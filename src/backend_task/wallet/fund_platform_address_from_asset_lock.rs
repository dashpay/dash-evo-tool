use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::wallet::PlatformSyncMode;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::dashcore::Address;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::AssetLockProof;
use std::collections::BTreeMap;
use std::sync::Arc;

impl AppContext {
    /// Fund Platform addresses from an asset lock
    pub(crate) async fn fund_platform_address_from_asset_lock(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        asset_lock_proof: AssetLockProof,
        asset_lock_address: Address,
        outputs: BTreeMap<PlatformAddress, Option<Credits>>,
    ) -> Result<BackendTaskSuccessResult, String> {
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::dpp::dashcore::OutPoint;
        use dash_sdk::platform::transition::top_up_address::TopUpAddress;

        // Clone wallet and SDK before the async operation to avoid holding guards across await
        let (wallet, sdk, asset_lock_private_key) = {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or_else(|| "Wallet not found".to_string())?
            };
            let wallet = wallet_arc.read().map_err(|e| e.to_string())?.clone();
            let sdk = self.sdk.read().map_err(|e| e.to_string())?.clone();

            // Get the private key for the asset lock address
            let private_key = wallet
                .private_key_for_address(&asset_lock_address, self.network)
                .map_err(|e| format!("Failed to get private key: {}", e))?
                .ok_or_else(|| "Asset lock address not found in wallet".to_string())?;

            (wallet, sdk, private_key)
        };

        // Check if we need to convert an old instant lock proof to a chain lock proof
        use crate::context::get_transaction_info_via_dapi;
        use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
        use dash_sdk::platform::Fetch;

        let asset_lock_proof = if let AssetLockProof::Instant(instant_asset_lock_proof) =
            &asset_lock_proof
        {
            // Get the transaction ID from the instant lock proof
            let tx_id = instant_asset_lock_proof.transaction().txid();

            // Query DAPI to check if the transaction has been chain-locked
            let tx_info = get_transaction_info_via_dapi(&sdk, &tx_id).await?;

            if tx_info.is_chain_locked && tx_info.height > 0 && tx_info.confirmations > 8 {
                // Transaction has been chain-locked with sufficient confirmations
                let tx_block_height = tx_info.height;

                // Check if the platform has caught up to this block height
                let (_, metadata) = ExtendedEpochInfo::fetch_with_metadata(&sdk, 0, None)
                    .await
                    .map_err(|e| format!("Failed to get platform metadata: {}", e))?;

                if tx_block_height <= metadata.core_chain_locked_height {
                    // Platform has synced past this block, use chain lock proof
                    AssetLockProof::Chain(ChainAssetLockProof {
                        core_chain_locked_height: tx_block_height,
                        out_point: OutPoint::new(tx_id, 0),
                    })
                } else {
                    // Platform hasn't verified this Core block yet - can't use chain lock proof
                    // and instant lock is stale. User needs to wait.
                    return Err(format!(
                        "Cannot use this asset lock yet. The instant lock proof has expired (quorum rotated), \
                            and Platform hasn't verified Core block {} yet (Platform has verified up to Core block {}). \
                            Please wait for Platform to sync with Core chain.",
                        tx_block_height, metadata.core_chain_locked_height
                    ));
                }
            } else {
                // Use the instant lock proof as-is (transaction is recent)
                asset_lock_proof
            }
        } else {
            // Already a chain lock proof, use as-is
            asset_lock_proof
        };

        // Simple fee strategy: reduce from first output
        let fee_strategy = vec![AddressFundsFeeStrategyStep::ReduceOutput(0)];

        // Get the transaction ID before consuming the asset lock proof
        let tx_id = match &asset_lock_proof {
            AssetLockProof::Instant(instant) => instant.transaction().txid(),
            AssetLockProof::Chain(chain) => chain.out_point.txid,
        };

        // Use the SDK to top up Platform addresses from asset lock
        let _result = outputs
            .top_up(
                &sdk,
                asset_lock_proof,
                asset_lock_private_key,
                fee_strategy,
                &wallet,
                None,
            )
            .await
            .map_err(|e| format!("Failed to fund Platform address from asset lock: {}", e))?;

        // Remove the used asset lock from the wallet and database
        {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets.get(&seed_hash).cloned()
            };
            if let Some(wallet_arc) = wallet_arc {
                let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;
                wallet
                    .unused_asset_locks
                    .retain(|(tx, _, _, _, _)| tx.txid() != tx_id);
            }
            // Also remove from database
            if let Err(e) = self
                .db
                .delete_asset_lock_transaction(&tx_id.to_byte_array())
            {
                tracing::warn!("Failed to delete asset lock from database: {}", e);
            }
        }

        // Trigger a balance refresh
        self.fetch_platform_address_balances(seed_hash, PlatformSyncMode::Auto)
            .await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }
}
