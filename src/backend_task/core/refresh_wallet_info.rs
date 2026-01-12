use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::{DerivationPathHelpers, Wallet};
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Address, OutPoint, Transaction, TxOut};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

impl AppContext {
    /// Refresh wallet info with minimal lock contention to avoid UI freezes.
    ///
    /// Strategy: Collect data with brief read locks, do all RPC calls without locks,
    /// then update wallet with a single brief write lock at the end.
    pub fn refresh_wallet_info(
        &self,
        wallet: Arc<RwLock<Wallet>>,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Step 1: Collect data from wallet with brief read lock
        let (addresses, asset_lock_txs, seed_hash) = {
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
            let addrs = wallet_guard
                .known_addresses
                .iter()
                .filter(|(_, path)| !path.is_platform_payment(self.network))
                .map(|(addr, _)| addr.clone())
                .collect::<Vec<_>>();
            let asset_locks: Vec<Transaction> = wallet_guard
                .unused_asset_locks
                .iter()
                .map(|(tx, _, _, _, _)| tx.clone())
                .collect();
            let seed = wallet_guard.seed_hash();
            (addrs, asset_locks, seed)
        };
        // Read lock released here

        // Step 2: Import addresses to Core (no wallet lock needed)
        {
            let client = self
                .core_client
                .read()
                .expect("Core client lock was poisoned");

            for address in &addresses {
                if let Err(e) = client.import_address(address, None, Some(false)) {
                    tracing::debug!(?e, address = %address, "import_address failed during refresh");
                }
            }
        }

        // Step 3: Fetch UTXOs from Core RPC (no wallet lock needed)
        let utxo_map: HashMap<OutPoint, TxOut> = {
            let client = self
                .core_client
                .read()
                .expect("Core client lock was poisoned");

            // Get UTXOs for all addresses
            let utxos = if addresses.is_empty() {
                Vec::new()
            } else {
                client
                    .list_unspent(
                        None,
                        None,
                        Some(&addresses.iter().collect::<Vec<_>>()),
                        Some(false),
                        None,
                    )
                    .map_err(|e| format!("Failed to list UTXOs: {}", e))?
            };

            // Build the UTXO map
            let mut map = HashMap::new();
            for utxo in utxos {
                let outpoint = OutPoint::new(utxo.txid, utxo.vout);
                let tx_out = TxOut {
                    value: utxo.amount.to_sat(),
                    script_pubkey: utxo.script_pub_key,
                };
                map.insert(outpoint, tx_out);
            }
            map
        };
        // No lock was held during RPC call

        // Step 4: Calculate balances from UTXOs (no lock needed)
        let mut address_balances: HashMap<Address, u64> = HashMap::new();
        for tx_out in utxo_map.values() {
            if let Ok(address) = Address::from_script(&tx_out.script_pubkey, self.network) {
                *address_balances.entry(address).or_insert(0) += tx_out.value;
            }
        }

        // Step 5: Fetch total received for each address from Core RPC (no wallet lock)
        let mut total_received_map: HashMap<Address, u64> = HashMap::new();
        {
            let client = self
                .core_client
                .read()
                .expect("Core client lock was poisoned");

            for address in &addresses {
                match client.get_received_by_address(address, None) {
                    Ok(amount) => {
                        total_received_map.insert(address.clone(), amount.to_sat());
                    }
                    Err(e) => {
                        tracing::debug!(
                            ?e,
                            address = %address,
                            "get_received_by_address failed"
                        );
                    }
                }
            }
        }

        // Step 6: Check which asset locks are stale (no wallet lock needed)
        let stale_txids: Vec<_> = {
            let client = self
                .core_client
                .read()
                .expect("Core client lock was poisoned");

            asset_lock_txs
                .iter()
                .filter_map(|tx| {
                    let txid = tx.txid();
                    match client.get_tx_out(&txid, 0, Some(true)) {
                        Ok(Some(_)) => None, // UTXO exists, keep it
                        Ok(None) => {
                            tracing::info!(
                                "Asset lock {} has been used (UTXO spent), removing from unused list",
                                txid
                            );
                            Some(txid)
                        }
                        Err(e) => {
                            tracing::debug!("Error checking asset lock UTXO {}: {}", txid, e);
                            None
                        }
                    }
                })
                .collect()
        };

        // Step 7: Insert UTXOs into database (no wallet lock needed)
        for (outpoint, tx_out) in &utxo_map {
            if let Ok(address) = Address::from_script(&tx_out.script_pubkey, self.network) {
                self.db
                    .insert_utxo(
                        outpoint.txid.as_ref(),
                        outpoint.vout,
                        &address,
                        tx_out.value,
                        &tx_out.script_pubkey.to_bytes(),
                        self.network,
                    )
                    .map_err(|e| e.to_string())?;
            }
        }

        // Step 8: Delete stale asset locks from database (no wallet lock needed)
        for txid in &stale_txids {
            if let Err(e) = self.db.delete_asset_lock_transaction(txid.as_byte_array()) {
                tracing::warn!("Failed to delete stale asset lock from database: {}", e);
            }
        }

        // Step 9: Calculate total balance (no lock needed)
        let total_balance: u64 = utxo_map.values().map(|tx_out| tx_out.value).sum();

        // Step 10: Update wallet IN-MEMORY state only (brief write lock, no I/O)
        // Collect which balances actually changed for later database update
        let (changed_balances, changed_total_received): (Vec<_>, Vec<_>) = {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;

            // Update wallet's UTXO map
            let new_outpoints: std::collections::HashSet<_> = utxo_map.keys().cloned().collect();

            // Remove UTXOs that are no longer unspent
            for utxos in wallet_guard.utxos.values_mut() {
                utxos.retain(|outpoint, _| new_outpoints.contains(outpoint));
            }
            wallet_guard.utxos.retain(|_, utxos| !utxos.is_empty());

            // Add new UTXOs
            for (outpoint, tx_out) in &utxo_map {
                if let Ok(address) = Address::from_script(&tx_out.script_pubkey, self.network) {
                    wallet_guard
                        .utxos
                        .entry(address)
                        .or_default()
                        .insert(*outpoint, tx_out.clone());
                }
            }

            // Update address balances IN-MEMORY and collect changes
            let mut balance_changes = Vec::new();
            for address in &addresses {
                let balance = address_balances.get(address).cloned().unwrap_or(0);
                // Only track if balance changed
                let current = wallet_guard.address_balances.get(address).cloned();
                if current != Some(balance) {
                    wallet_guard
                        .address_balances
                        .insert(address.clone(), balance);
                    balance_changes.push((address.clone(), balance));
                }
            }

            // Update total received IN-MEMORY and collect changes
            let mut received_changes = Vec::new();
            for (address, total_received) in &total_received_map {
                // Only track if changed
                let current = wallet_guard.address_total_received.get(address).cloned();
                if current != Some(*total_received) {
                    wallet_guard
                        .address_total_received
                        .insert(address.clone(), *total_received);
                    received_changes.push((address.clone(), *total_received));
                }
            }

            // Remove stale asset locks
            if !stale_txids.is_empty() {
                let stale_count = stale_txids.len();
                wallet_guard
                    .unused_asset_locks
                    .retain(|(tx, _, _, _, _)| !stale_txids.contains(&tx.txid()));
                tracing::info!("Removed {} stale asset locks", stale_count);
            }

            // Update wallet-level balances
            wallet_guard.update_spv_balances(total_balance, 0, total_balance);

            (balance_changes, received_changes)
        };
        // Write lock released here - all I/O happens below without any wallet lock

        // Step 11: Persist all changes to database (no wallet lock needed)
        // Update address balances in database
        for (address, balance) in &changed_balances {
            if let Err(e) = self
                .db
                .update_address_balance(&seed_hash, address, *balance)
            {
                tracing::debug!(error = %e, address = %address, "Failed to persist address balance");
            }
        }

        // Update total received in database
        for (address, total_received) in &changed_total_received {
            if let Err(e) =
                self.db
                    .update_address_total_received(&seed_hash, address, *total_received)
            {
                tracing::debug!(error = %e, address = %address, "Failed to persist total received");
            }
        }

        // Update wallet-level balances
        if let Err(e) = self
            .db
            .update_wallet_balances(&seed_hash, total_balance, 0, total_balance)
        {
            tracing::warn!(error = %e, "Failed to persist wallet balances");
        }

        Ok(BackendTaskSuccessResult::RefreshedWallet)
    }
}
