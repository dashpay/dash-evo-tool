use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Routing rule for the tracked-lock funding dispatch: the orchestrated path is
/// eligible only when EVERY recipient is in the wallet's upstream
/// platform-payment pool (the orchestrator's pre-flight rejects mixed maps
/// outright), so a single non-pool recipient routes the whole map to the manual
/// path. `memberships` is each recipient's pre-resolved pool membership.
fn route_to_orchestrator(memberships: impl IntoIterator<Item = bool>) -> bool {
    memberships.into_iter().all(|in_pool| in_pool)
}

impl AppContext {
    /// Fund Platform addresses from a tracked asset lock.
    ///
    /// Branches on upstream pool membership. When every recipient is already
    /// revealed in this wallet's upstream platform-payment pool — the exact
    /// recipient set the orchestrator's pre-flight accepts — the funding routes
    /// through the orchestrated `fund_from_asset_lock` pipeline, which resumes
    /// the tracked lock, submits with CL-height retry and IS→CL fallback, and
    /// consumes the lock on acceptance. Any other destination — an advanced
    /// footgun users are trusted to take — keeps the manual `TopUpAddress` path.
    pub(crate) async fn fund_platform_address_from_asset_lock(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        out_point: OutPoint,
        outputs: BTreeMap<PlatformAddress, Option<Credits>>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let backend = self.wallet_backend()?;

        // The UI caller always supplies one `None` remainder recipient. A future
        // programmatic caller passing an empty map would otherwise route to the
        // orchestrator (vacuously all-in-pool) and rely on its pre-flight to
        // catch it before broadcast — reject it directly here instead, so the
        // precondition is enforced in every build, not just debug ones.
        if outputs.is_empty() {
            return Err(TaskError::NoFundingRecipients);
        }

        // Resolve each recipient's pool membership (short-circuiting on the first
        // miss to skip remaining network-touching queries), then apply the pure
        // routing rule. Splitting the async query from the decision keeps the
        // mixed-ownership rule unit-testable.
        let mut memberships: Vec<bool> = Vec::with_capacity(outputs.len());
        for addr in outputs.keys() {
            let in_pool = backend.platform_address_in_pool(&seed_hash, addr).await?;
            memberships.push(in_pool);
            if !in_pool {
                break;
            }
        }

        if route_to_orchestrator(memberships) {
            return self
                .fund_platform_address_from_asset_lock_orchestrated(seed_hash, out_point, outputs)
                .await;
        }

        self.fund_platform_address_from_asset_lock_manual(seed_hash, out_point, outputs)
            .await
    }

    /// Orchestrated tracked-lock funding for wallet-owned destinations: resumes
    /// the lock by outpoint, submits the address-funding transition with
    /// CL-height retry and IS→CL fallback, then consumes the lock on
    /// acceptance.
    async fn fund_platform_address_from_asset_lock_orchestrated(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        out_point: OutPoint,
        outputs: BTreeMap<PlatformAddress, Option<Credits>>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        use crate::wallet_backend::PlatformPathIndex;
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use platform_wallet::wallet::asset_lock::AssetLockFunding;

        let wallet_arc = self.wallet_arc(&seed_hash)?;
        let network = self.network;
        let path_index = {
            let wallet = wallet_arc.read()?;
            PlatformPathIndex::from_wallet(&wallet, network)
        };
        let fee_strategy = vec![AddressFundsFeeStrategyStep::ReduceOutput(0)];

        let backend = self.wallet_backend()?;
        backend
            .fund_platform_address(
                &seed_hash,
                // Generic platform-address funding resume, not the DashPay
                // invitation-voucher reclaim flow, so it must never consume a
                // bearer-voucher (invitation-typed) lock.
                AssetLockFunding::FromExistingAssetLock {
                    out_point,
                    consume_invitation_voucher: false,
                },
                0,
                outputs,
                fee_strategy,
                &path_index,
                None,
            )
            .await?;

        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }

    /// Manual tracked-lock funding: derive the credit-output key and submit the
    /// `TopUpAddress` transition directly. Used for non-owned destinations. A
    /// submit failure propagates via `?`, so the flow never reports a false
    /// success.
    async fn fund_platform_address_from_asset_lock_manual(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        out_point: OutPoint,
        outputs: BTreeMap<PlatformAddress, Option<Credits>>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::dpp::dashcore::Address;
        use dash_sdk::platform::transition::top_up_address::TopUpAddress;

        let backend = self.wallet_backend()?;
        let tracked = backend
            .list_tracked_asset_locks(&seed_hash)
            .await?
            .into_iter()
            .find(|t| t.out_point == out_point)
            .ok_or(TaskError::AssetLockAddressNotFound)?;

        let asset_lock_proof = tracked
            .proof
            .clone()
            .ok_or(TaskError::AssetLockAddressNotFound)?;

        // Recover the credit-output address from the asset-lock transaction
        // payload — the first credit output is the funded address.
        let payload = tracked
            .transaction
            .special_transaction_payload
            .as_ref()
            .ok_or(TaskError::AssetLockAddressNotFound)?;
        let asset_lock_payload = match payload {
            TransactionPayload::AssetLockPayloadType(p) => p,
            _ => return Err(TaskError::AssetLockAddressNotFound),
        };
        let credit_output = asset_lock_payload
            .credit_outputs
            .first()
            .ok_or(TaskError::AssetLockAddressNotFound)?;
        let asset_lock_address = Address::from_script(&credit_output.script_pubkey, self.network)
            .map_err(|_| TaskError::AssetLockAddressNotFound)?;

        let (wallet, sdk) = {
            let wallet = self.wallet_arc(&seed_hash)?.read()?.clone();
            let sdk = backend.sdk().clone();
            (wallet, sdk)
        };

        // Resolve the HD seed once through the chokepoint and, inside that same
        // scope, both derive the asset-lock address's private key AND build the
        // JIT platform signer that authorises each funded-output witness. The
        // seed is borrowed for the whole top-up and zeroizes when the closure
        // returns — it never enters this layer by value. The pure path index is
        // built before the scope.
        use crate::wallet_backend::{DetPlatformSigner, PlatformPathIndex};
        let network = self.network;
        let asset_lock_address_for_lookup = asset_lock_address.clone();
        let path_index = PlatformPathIndex::from_wallet(&wallet, network);
        let fee_strategy = vec![AddressFundsFeeStrategyStep::ReduceOutput(0)];

        let _result = backend
            .secret_access()
            .with_secret_session(
                &crate::wallet_backend::SecretScope::HdSeed { seed_hash },
                async |session| {
                    let plaintext = session.plaintext();
                    let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                    let asset_lock_private_key = wallet
                        .private_key_for_address_with_seed(
                            seed,
                            &asset_lock_address_for_lookup,
                            network,
                        )
                        .map_err(|detail| {
                            tracing::warn!(error = %detail, "Asset-lock key derivation failed");
                            TaskError::WalletKeyLookupFailed
                        })?
                        .ok_or(TaskError::AssetLockAddressNotFound)?;
                    let signer = DetPlatformSigner::from_held(seed, network, &path_index);
                    // Non-owned destination: the manual `TopUpAddress` submit
                    // has no orchestrated consume/retry. A failure propagates via
                    // `?`, so the flow never reports a false success; the
                    // orchestrated recovery is reserved for wallet-owned
                    // destinations (see the owned branch above).
                    outputs
                        .top_up(
                            &sdk,
                            asset_lock_proof,
                            asset_lock_private_key,
                            fee_strategy,
                            &signer,
                            None,
                        )
                        .await
                        .map_err(TaskError::from)
                },
            )
            .await?;

        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }
}

#[cfg(test)]
mod tests {
    use super::route_to_orchestrator;

    /// All recipients in-pool routes to the orchestrator.
    #[test]
    fn all_in_pool_routes_to_orchestrator() {
        assert!(route_to_orchestrator([true, true, true]));
        assert!(route_to_orchestrator([true]));
    }

    /// A mixed map (one in-pool, one foreign) routes to the manual path — the
    /// orchestrator's pre-flight would reject the foreign recipient, so the
    /// whole funding falls back rather than half-succeeding.
    #[test]
    fn mixed_ownership_routes_to_manual() {
        assert!(!route_to_orchestrator([true, false]));
        assert!(!route_to_orchestrator([false, true]));
        assert!(!route_to_orchestrator([false]));
    }

    /// An empty map (no recipients) is vacuously all-in-pool, so it routes to
    /// the orchestrator. Unreachable via the current UI caller, which always
    /// inserts exactly one `None` remainder recipient; a future programmatic /
    /// MCP caller passing an empty map would route here and be rejected by the
    /// orchestrator's own `validate_recipient_addresses` pre-flight before
    /// broadcast. The rule is pinned so the helper's contract is explicit.
    #[test]
    fn empty_is_vacuously_in_pool() {
        assert!(route_to_orchestrator(std::iter::empty()));
    }
}
