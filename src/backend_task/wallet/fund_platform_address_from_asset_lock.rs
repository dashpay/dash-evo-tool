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

impl AppContext {
    /// Fund Platform addresses from a tracked asset lock.
    ///
    /// The lock is identified by its credit-output [`OutPoint`]. We pull the
    /// finalized proof, transaction, and credit-output address from the
    /// upstream `AssetLockManager` (DET no longer mirrors that state). The
    /// credit-output address is the BIP-32 address that originally received
    /// the credit output at lock-build time; its private key lives in the
    /// wallet's `known_addresses` map.
    pub(crate) async fn fund_platform_address_from_asset_lock(
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
            let wallet_arc = {
                let wallets = self.wallets.read()?;
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or(TaskError::WalletNotFound)?
            };
            let wallet = wallet_arc.read()?.clone();
            let sdk = self.sdk.load().as_ref().clone();
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
                    // A Platform submit failure here propagates via `?` below, so
                    // the flow never reports success on a failed top-up. What is
                    // missing is terminal accounting: on success the tracked lock
                    // is not marked `Consumed`, so it stays resumable.
                    //
                    // TODO(upstream-gated): route this through
                    // `platform_wallet::PlatformWallet::fund_from_asset_lock`
                    // (with `AssetLockFunding::FromExistingAssetLock { out_point }`),
                    // which runs resolve → `submit_with_cl_height_retry` →
                    // `consume_asset_lock`. That method is public on the public
                    // `PlatformWallet`, but DET reaches it only via
                    // `WalletBackend::resolve_wallet` (private, -> `Arc<PlatformWallet>`),
                    // and the route needs an external `Signer<PlatformAddress>` plus a
                    // `key_wallet::signer::Signer`. Wiring it is a funds-safety change
                    // gated on Smythe+Marvin review.
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
