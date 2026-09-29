use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::OutPoint;
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
        use crate::wallet_backend::PlatformPathIndex;

        let path_index = {
            let wallet = self.wallet_arc(&seed_hash)?;
            let wallet = wallet.read()?;
            PlatformPathIndex::from_wallet(&wallet, self.network)
        };
        self.wallet_backend()?
            .fund_platform_address_from_tracked_lock_manual(
                &seed_hash,
                out_point,
                outputs,
                &path_index,
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
