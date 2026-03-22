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
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

impl AppContext {
    /// Search for unused asset locks by scanning the Core wallet for asset lock transactions
    /// that belong to this wallet but aren't tracked in the database.
    pub fn recover_asset_locks(
        &self,
        wallet: Arc<RwLock<Wallet>>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let (known_addresses, seed_hash, already_tracked_txids, core_wallet_name) = {
            let wallet_guard = wallet.read()?;
            let addresses: Vec<Address> = wallet_guard.known_addresses.keys().cloned().collect();
            let tracked: HashSet<_> = wallet_guard
                .unused_asset_locks
                .iter()
                .map(|(tx, _, _, _, _)| tx.txid())
                .collect();
            (
                addresses,
                wallet_guard.seed_hash(),
                tracked,
                wallet_guard.core_wallet_name.clone(),
            )
        };

        tracing::info!(
            "Searching for unused asset locks. Known addresses: {}, Already tracked: {}",
            known_addresses.len(),
            already_tracked_txids.len()
        );

        if known_addresses.is_empty() {
            tracing::warn!("No known addresses in wallet - cannot search for asset locks");
            return Ok(BackendTaskSuccessResult::RecoveredAssetLocks {
                recovered_count: 0,
                total_amount: 0,
            });
        }

        let client = self.core_client_for_wallet(core_wallet_name.as_deref())?;

        let mut recovered_count = 0;
        let mut total_amount = 0u64;

        // First, import all known addresses to Core to ensure it's watching them
        for address in &known_addresses {
            if let Err(e) = client.import_address(address, None, Some(false)) {
                tracing::debug!("import_address for {} returned: {:?}", address, e);
            }
        }

        // Method 1: Get unspent outputs for all known addresses
        let address_refs: Vec<&Address> = known_addresses.iter().collect();
        let unspent = client.list_unspent(None, None, Some(&address_refs), Some(true), None)?;

        tracing::info!(
            "Found {} unspent outputs for known addresses",
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
                    if known_addresses.contains(&addr) {
                        credit_address = Some(addr);
                        credit_amount = credit_output.value;
                        break;
                    }
                }
            }

            let Some(addr) = credit_address else {
                tracing::debug!("Asset lock {} credit address not in known addresses", txid);
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

            // Add to wallet's in-memory unused_asset_locks
            {
                let mut wallet_guard = wallet.write()?;

                let already_exists = wallet_guard
                    .unused_asset_locks
                    .iter()
                    .any(|(tx, _, _, _, _)| tx.txid() == txid);

                if !already_exists {
                    wallet_guard.unused_asset_locks.push((
                        raw_tx.clone(),
                        addr,
                        credit_amount,
                        None,
                        proof,
                    ));
                    recovered_count += 1;
                    total_amount += credit_amount;

                    tracing::info!(
                        "Found unused asset lock: txid={}, amount={} duffs",
                        txid,
                        credit_amount
                    );
                }
            }
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
                if !known_addresses.contains(&credit_addr) {
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

                // Add to wallet
                {
                    let mut wallet_guard = wallet.write()?;

                    let already_exists = wallet_guard
                        .unused_asset_locks
                        .iter()
                        .any(|(tx, _, _, _, _)| tx.txid() == txid);

                    if !already_exists {
                        wallet_guard.unused_asset_locks.push((
                            raw_tx.clone(),
                            credit_addr,
                            credit_amount,
                            None,
                            proof,
                        ));
                        recovered_count += 1;
                        total_amount += credit_amount;

                        tracing::info!(
                            "Found unused asset lock (full scan): txid={}, amount={} duffs",
                            txid,
                            credit_amount
                        );
                    }
                }
            }
        }

        // Clean up: Remove asset locks from wallet that don't belong to it
        // (credit address not in known_addresses)
        let mut txids_to_remove = Vec::new();
        let removed_count = {
            let mut wallet_guard = wallet.write()?;
            let before_count = wallet_guard.unused_asset_locks.len();

            wallet_guard.unused_asset_locks.retain(|(tx, _, _, _, _)| {
                // Get the credit output address from the transaction
                if let Some(TransactionPayload::AssetLockPayloadType(payload)) =
                    &tx.special_transaction_payload
                    && let Some(credit_output) = payload.credit_outputs.first()
                    && let Ok(addr) =
                        Address::from_script(&credit_output.script_pubkey, self.network)
                    && known_addresses.contains(&addr)
                {
                    return true; // Keep this asset lock
                }
                tracing::info!(
                    "Removing asset lock {} - credit address not in wallet",
                    tx.txid()
                );
                txids_to_remove.push(tx.txid());
                false // Remove this asset lock
            });

            before_count - wallet_guard.unused_asset_locks.len()
        };

        // Also delete from database
        for txid in &txids_to_remove {
            if let Err(e) = self.db.delete_asset_lock_transaction(txid.as_byte_array()) {
                tracing::warn!("Failed to delete asset lock {} from database: {}", txid, e);
            }
        }

        if removed_count > 0 {
            tracing::info!(
                "Removed {} asset locks that don't belong to this wallet",
                removed_count
            );
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
