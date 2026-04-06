use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::dashcore::{Address, OutPoint};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::AssetLockProof;
use platform_wallet::CoreAddressInfo;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// Register a recovered asset lock with the PlatformWallet's AssetLockManager.
///
/// This keeps the AssetLockManager in sync with evo-tool's own tracking. The
/// `funding_type` is set to `IdentityRegistration` as a default since recovery
/// cannot determine the original funding type.
fn register_with_asset_lock_manager(
    wallet: &Wallet,
    tx: &dash_sdk::dpp::dashcore::Transaction,
    amount: u64,
    proof: Option<AssetLockProof>,
) {
    if let Some(pw) = wallet.platform_wallet.as_ref() {
        pw.asset_locks().recover_asset_lock_blocking(
            tx.clone(),
            amount,
            0, // account_index unknown for recovered locks, default to 0
            platform_wallet::AssetLockFundingType::IdentityRegistration,
            0, // identity_index unknown for recovered locks
            dash_sdk::dpp::dashcore::OutPoint::new(tx.txid(), 0),
            proof,
        );
    }
}

impl AppContext {
    /// Search for unused asset locks by scanning the Core wallet for asset lock transactions
    /// that belong to this wallet but aren't tracked in the database.
    pub fn recover_asset_locks(
        &self,
        wallet: Arc<RwLock<Wallet>>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let (wallet_addresses, seed_hash, already_tracked_txids, core_wallet_name) = {
            let wallet_guard = wallet.read()?;

            // Read addresses from PlatformWallet (canonical source).
            // Locked wallets (no PlatformWallet) have no addresses — return empty.
            let addresses: Vec<Address> = if let Some(pw) = wallet_guard.platform_wallet.as_ref() {
                let info = pw.core().wallet_info_blocking();
                CoreAddressInfo::all_from_wallet_info(&info)
                    .into_iter()
                    .map(|a| a.address)
                    .collect()
            } else {
                Vec::new()
            };

            let tracked: HashSet<_> = wallet_guard
                .platform_wallet
                .as_ref()
                .map(|pw| {
                    pw.asset_locks()
                        .list_tracked_locks_blocking()
                        .into_iter()
                        .map(|lock| lock.out_point.txid)
                        .collect()
                })
                .unwrap_or_default();
            (
                addresses,
                wallet_guard.seed_hash(),
                tracked,
                wallet_guard.core_wallet_name.clone(),
            )
        };

        tracing::info!(
            "Searching for unused asset locks. Wallet addresses: {}, Already tracked: {}",
            wallet_addresses.len(),
            already_tracked_txids.len()
        );

        if wallet_addresses.is_empty() {
            tracing::warn!("No addresses in wallet - cannot search for asset locks");
            return Ok(BackendTaskSuccessResult::RecoveredAssetLocks {
                recovered_count: 0,
                total_amount: 0,
            });
        }

        let client = self.core_client_for_wallet(core_wallet_name.as_deref())?;

        let mut recovered_count = 0;
        let mut total_amount = 0u64;

        // First, import all wallet addresses to Core to ensure it's watching them
        for address in &wallet_addresses {
            if let Err(e) = client.import_address(address, None, Some(false)) {
                tracing::debug!("import_address for {} returned: {:?}", address, e);
            }
        }

        // Method 1: Get unspent outputs for all wallet addresses
        let address_refs: Vec<&Address> = wallet_addresses.iter().collect();
        let unspent = client.list_unspent(None, None, Some(&address_refs), Some(true), None)?;

        tracing::info!(
            "Found {} unspent outputs for wallet addresses",
            unspent.len()
        );

        // Check each unspent output to see if it's an asset lock
        for utxo in &unspent {
            let txid = utxo.txid;

            // Skip if already tracked
            if already_tracked_txids.contains(&txid) {
                tracing::debug!("Skipping {} - already tracked in wallet", txid);
                continue;
            }

            // Check if already in database
            if let Ok(Some(_)) = self.db.get_asset_lock_transaction(txid.as_byte_array()) {
                tracing::debug!("Skipping {} - already in database", txid);
                continue;
            }

            // Get the raw transaction to check if it's an asset lock
            let raw_tx = match client.get_raw_transaction(&txid, None) {
                Ok(tx) => tx,
                Err(e) => {
                    tracing::debug!("Failed to get raw transaction {}: {}", txid, e);
                    continue;
                }
            };

            // Check if this is an asset lock transaction
            let Some(TransactionPayload::AssetLockPayloadType(payload)) =
                &raw_tx.special_transaction_payload
            else {
                continue;
            };

            tracing::info!("Found asset lock transaction: {}", txid);

            // Find the credit output that belongs to our wallet
            let mut credit_address = None;
            let mut credit_amount = 0u64;

            for credit_output in &payload.credit_outputs {
                if let Ok(addr) = Address::from_script(&credit_output.script_pubkey, self.network) {
                    tracing::debug!("Asset lock credit output address: {}", addr);
                    if wallet_addresses.contains(&addr) {
                        credit_address = Some(addr);
                        credit_amount = credit_output.value;
                        break;
                    }
                }
            }

            let Some(addr) = credit_address else {
                tracing::debug!("Asset lock {} credit address not in wallet addresses", txid);
                continue;
            };

            // Note: We cannot check if asset lock is "spent" via get_tx_out because
            // asset lock transactions use OP_RETURN outputs which are never UTXOs.
            // Platform tracks whether asset locks are used, not Core.
            // We add the asset lock and let the user try to use it - Platform will
            // reject if it's already been consumed.

            // Get transaction info for chain lock status
            let tx_info = client.get_raw_transaction_info(&txid, None).ok();

            // Build the proof
            let (chain_locked_height, proof) = if let Some(ref info) = tx_info {
                if info.chainlock && info.height.is_some() {
                    let height = info.height.unwrap() as u32;
                    tracing::debug!("Asset lock {} is chain-locked at height {}", txid, height);
                    (
                        Some(height),
                        Some(AssetLockProof::Chain(ChainAssetLockProof {
                            core_chain_locked_height: height,
                            out_point: OutPoint::new(txid, 0),
                        })),
                    )
                } else {
                    tracing::debug!(
                        "Asset lock {} not chain-locked yet (chainlock={}, height={:?}) — proof unavailable",
                        txid,
                        info.chainlock,
                        info.height
                    );
                    (None, None)
                }
            } else {
                tracing::debug!(
                    "Could not retrieve transaction info for asset lock {} — proof unavailable",
                    txid
                );
                (None, None)
            };

            // Store the asset lock in the database
            if let Err(e) = self.db.store_asset_lock_transaction(
                &raw_tx,
                credit_amount,
                None,
                &seed_hash,
                self.network,
            ) {
                tracing::warn!("Failed to store asset lock {}: {}", txid, e);
                continue;
            }

            // Also store the chain locked height if available
            if let Some(height) = chain_locked_height
                && let Err(e) = self
                    .db
                    .update_asset_lock_chain_locked_height(txid.as_byte_array(), Some(height))
            {
                tracing::warn!("Failed to update chain locked height for {}: {}", txid, e);
            }

            // Register with PlatformWallet's AssetLockManager
            {
                let wallet_guard = wallet.read()?;
                register_with_asset_lock_manager(&wallet_guard, &raw_tx, credit_amount, proof);
            }

            recovered_count += 1;
            total_amount += credit_amount;

            tracing::info!(
                "Found unused asset lock: txid={}, amount={} duffs",
                txid,
                credit_amount
            );
        }

        // Method 2: Also check Core's wallet for any transactions we might have missed
        // by scanning ALL unspent outputs (not filtered by address)
        tracing::info!("Scanning all Core wallet unspent outputs...");
        if let Ok(all_unspent) = client.list_unspent(None, None, None, Some(true), None) {
            tracing::info!(
                "Core wallet has {} total unspent outputs",
                all_unspent.len()
            );

            for utxo in all_unspent {
                let txid = utxo.txid;

                // Skip if already processed or tracked
                if already_tracked_txids.contains(&txid) {
                    continue;
                }
                if let Ok(Some(_)) = self.db.get_asset_lock_transaction(txid.as_byte_array()) {
                    continue;
                }

                // Get the raw transaction
                let raw_tx = match client.get_raw_transaction(&txid, None) {
                    Ok(tx) => tx,
                    Err(_) => continue,
                };

                // Check if this is an asset lock transaction
                let Some(TransactionPayload::AssetLockPayloadType(payload)) =
                    &raw_tx.special_transaction_payload
                else {
                    continue;
                };

                tracing::info!("Found asset lock in Core wallet scan: {}", txid);

                // Get the credit output address and amount
                let Some(credit_output) = payload.credit_outputs.first() else {
                    continue;
                };

                let Ok(credit_addr) =
                    Address::from_script(&credit_output.script_pubkey, self.network)
                else {
                    continue;
                };

                // Verify the credit address belongs to our wallet
                if !wallet_addresses.contains(&credit_addr) {
                    tracing::debug!(
                        "Asset lock {} credit address {} not in wallet, skipping",
                        txid,
                        credit_addr
                    );
                    continue;
                }

                let credit_amount = credit_output.value;

                // Note: We cannot check if asset lock is "spent" via get_tx_out because
                // asset lock transactions use OP_RETURN outputs which are never UTXOs.
                // Platform tracks whether asset locks are used, not Core.

                // Get chain lock info
                let tx_info = client.get_raw_transaction_info(&txid, None).ok();
                let (chain_locked_height, proof) = if let Some(ref info) = tx_info {
                    if info.chainlock && info.height.is_some() {
                        let height = info.height.unwrap() as u32;
                        tracing::debug!("Asset lock {} is chain-locked at height {}", txid, height);
                        (
                            Some(height),
                            Some(AssetLockProof::Chain(ChainAssetLockProof {
                                core_chain_locked_height: height,
                                out_point: OutPoint::new(txid, 0),
                            })),
                        )
                    } else {
                        tracing::debug!(
                            "Asset lock {} not chain-locked yet (chainlock={}, height={:?}) — proof unavailable",
                            txid,
                            info.chainlock,
                            info.height
                        );
                        (None, None)
                    }
                } else {
                    tracing::debug!(
                        "Could not retrieve transaction info for asset lock {} — proof unavailable",
                        txid
                    );
                    (None, None)
                };

                // Store in database
                if let Err(e) = self.db.store_asset_lock_transaction(
                    &raw_tx,
                    credit_amount,
                    None,
                    &seed_hash,
                    self.network,
                ) {
                    tracing::warn!("Failed to store asset lock {}: {}", txid, e);
                    continue;
                }

                // Also store the chain locked height if available
                if let Some(height) = chain_locked_height
                    && let Err(e) = self
                        .db
                        .update_asset_lock_chain_locked_height(txid.as_byte_array(), Some(height))
                {
                    tracing::warn!("Failed to update chain locked height for {}: {}", txid, e);
                }

                // Register with PlatformWallet's AssetLockManager
                {
                    let wallet_guard = wallet.read()?;
                    register_with_asset_lock_manager(&wallet_guard, &raw_tx, credit_amount, proof);
                }

                recovered_count += 1;
                total_amount += credit_amount;

                tracing::info!(
                    "Found unused asset lock (full scan): txid={}, amount={} duffs",
                    txid,
                    credit_amount
                );
            }
        }

        tracing::info!(
            "Asset lock search complete. Found {} unused asset locks worth {} duffs",
            recovered_count,
            total_amount
        );

        Ok(BackendTaskSuccessResult::RecoveredAssetLocks {
            recovered_count,
            total_amount,
        })
    }
}
