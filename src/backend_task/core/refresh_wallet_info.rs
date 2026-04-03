use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::{DerivationPathHelpers, TransactionStatus, Wallet, WalletTransaction};
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dashcore_rpc::json::GetTransactionResultDetailCategory;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Address, BlockHash, OutPoint, Transaction, TxOut, Txid};
use dash_sdk::dpp::key_wallet::WalletCoreBalance;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

impl AppContext {
    /// Refresh wallet info with minimal lock contention to avoid UI freezes.
    ///
    /// Strategy: Collect data with brief read locks, do all RPC calls without locks,
    /// then update wallet with a single brief write lock at the end.
    pub fn refresh_wallet_info(
        &self,
        wallet: Arc<RwLock<Wallet>>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Step 1: Collect data from wallet with brief read lock
        let (addresses, asset_lock_txs, seed_hash, core_wallet_name) = {
            let wallet_guard = wallet.read()?;
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
            let cwn = wallet_guard.core_wallet_name.clone();
            (addrs, asset_locks, seed, cwn)
        };

        let is_spv_mode = self.core_backend_mode() == crate::spv::CoreBackendMode::Spv;

        // Build an RPC client targeting the wallet's Core wallet (if set)
        let client = self.core_client_for_wallet(core_wallet_name.as_deref())?;

        // Step 2: Import addresses to Core (no wallet lock needed)
        for address in &addresses {
            if let Err(e) = client.import_address(address, None, Some(false)) {
                tracing::debug!(?e, address = %address, "import_address failed during refresh");
            }
        }

        // Step 3: Fetch UTXOs from Core RPC (no wallet lock needed)
        let utxo_map: HashMap<OutPoint, TxOut> = {
            let utxos = if addresses.is_empty() {
                Vec::new()
            } else {
                client.list_unspent(
                    None,
                    None,
                    Some(&addresses.iter().collect::<Vec<_>>()),
                    Some(false),
                    None,
                )?
            };

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

        // Step 6: Load transaction history from Core RPC (skip in SPV mode)
        // None = skip update (SPV mode or RPC failure), Some = replace DB/in-memory state
        let mut tx_truncated = false;
        let rpc_transactions: Option<Vec<WalletTransaction>> = if is_spv_mode {
            None
        } else {
            let wallet_address_set: HashSet<Address> = addresses.iter().cloned().collect();

            struct TxAggregate {
                net_amount: i64,
                fee: Option<u64>,
                timestamp: u64,
                height: Option<u32>,
                block_hash: Option<BlockHash>,
                label: Option<String>,
                is_ours: bool,
            }

            match client.list_transactions(None, Some(200), None, Some(true)) {
                Ok(list_results) => {
                    if list_results.len() >= 200 {
                        tx_truncated = true;
                        tracing::warn!(
                            "list_transactions returned 200 entries — transaction history may be truncated"
                        );
                    }

                    let mut tx_aggregates: HashMap<Txid, TxAggregate> = HashMap::new();

                    for entry in &list_results {
                        let txid = entry.info.txid;
                        let amount_sat = entry.detail.amount.to_sat();
                        let fee_sat = entry.detail.fee.map(|f| f.to_sat().unsigned_abs());

                        // Safety: list_transactions targets a wallet-scoped RPC endpoint
                        // (/wallet/<name>), so all entries belong to this wallet. Send
                        // entries are included unconditionally because outgoing transactions
                        // may reference external addresses not in our known set.
                        let entry_involves_wallet = entry
                            .detail
                            .address
                            .as_ref()
                            .map(|a| wallet_address_set.contains(a.assume_checked_ref()))
                            .unwrap_or(false)
                            || entry.detail.category == GetTransactionResultDetailCategory::Send;

                        if !entry_involves_wallet {
                            continue;
                        }

                        let is_send =
                            entry.detail.category == GetTransactionResultDetailCategory::Send;
                        let addr_is_ours = entry
                            .detail
                            .address
                            .as_ref()
                            .map(|a| wallet_address_set.contains(a.assume_checked_ref()))
                            .unwrap_or(false);

                        match tx_aggregates.get_mut(&txid) {
                            Some(agg) => {
                                agg.net_amount += amount_sat;
                                if agg.fee.is_none() {
                                    agg.fee = fee_sat;
                                }
                                if agg.label.is_none() {
                                    agg.label = entry.detail.label.clone();
                                }
                                if is_send || addr_is_ours {
                                    agg.is_ours = true;
                                }
                            }
                            None => {
                                tx_aggregates.insert(
                                    txid,
                                    TxAggregate {
                                        net_amount: amount_sat,
                                        fee: fee_sat,
                                        timestamp: entry.info.time,
                                        height: entry.info.blockheight,
                                        block_hash: entry.info.blockhash,
                                        label: entry.detail.label.clone(),
                                        is_ours: is_send || addr_is_ours,
                                    },
                                );
                            }
                        }
                    }

                    let mut wallet_txs = Vec::new();
                    let mut raw_fetch_failed = false;
                    for (txid, agg) in tx_aggregates {
                        let raw_tx =
                            match client.get_raw_transaction(&txid, agg.block_hash.as_ref()) {
                                Ok(tx) => tx,
                                Err(e) => {
                                    tracing::debug!(
                                        ?e,
                                        %txid,
                                        "get_raw_transaction failed, skipping"
                                    );
                                    raw_fetch_failed = true;
                                    continue;
                                }
                            };

                        wallet_txs.push(WalletTransaction {
                            txid,
                            transaction: raw_tx,
                            timestamp: agg.timestamp,
                            height: agg.height,
                            block_hash: agg.block_hash,
                            net_amount: agg.net_amount,
                            fee: agg.fee,
                            label: agg.label,
                            is_ours: agg.is_ours,
                            // RPC only returns confirmed transactions
                            status: TransactionStatus::from_height(agg.height),
                        });
                    }

                    if raw_fetch_failed {
                        tracing::warn!(
                            "one or more get_raw_transaction calls failed; \
                             preserving existing transaction history"
                        );
                        tx_truncated = false;
                        None
                    } else {
                        tracing::info!(
                            rpc_transactions = wallet_txs.len(),
                            "loaded transaction history from Core RPC"
                        );
                        Some(wallet_txs)
                    }
                }
                Err(e) => {
                    tracing::debug!(?e, "list_transactions failed, skipping transaction load");
                    None
                }
            }
        };

        // Step 7: Check which asset locks are stale (no wallet lock needed)
        let stale_txids: Vec<_> = {
            asset_lock_txs
                .iter()
                .filter_map(|tx| {
                    let txid = tx.txid();
                    match client.get_tx_out(&txid, 0, Some(true)) {
                        Ok(Some(_)) => None,
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

        // Step 8: Insert UTXOs into database (no wallet lock needed)
        for (outpoint, tx_out) in &utxo_map {
            if let Ok(address) = Address::from_script(&tx_out.script_pubkey, self.network) {
                self.db.insert_utxo(
                    outpoint.txid.as_ref(),
                    outpoint.vout,
                    &address,
                    tx_out.value,
                    &tx_out.script_pubkey.to_bytes(),
                    self.network,
                )?;
            }
        }

        // Step 9: Delete stale asset locks from database (no wallet lock needed)
        for txid in &stale_txids {
            if let Err(e) = self.db.delete_asset_lock_transaction(txid.as_byte_array()) {
                tracing::warn!("Failed to delete stale asset lock from database: {}", e);
            }
        }

        // Step 10: Calculate total balance (no lock needed)
        let total_balance: u64 = utxo_map.values().map(|tx_out| tx_out.value).sum();

        // Step 11: Persist transactions to database BEFORE the write lock so we
        // can move (not clone) rpc_transactions into the wallet afterwards.
        if let Some(ref txs) = rpc_transactions {
            self.db
                .replace_wallet_transactions(&seed_hash, &self.network, txs)?;
        }

        // Step 12: Update wallet IN-MEMORY state only (brief write lock, no I/O)
        let (changed_balances, changed_total_received): (Vec<_>, Vec<_>) = {
            let mut wallet_guard = wallet.write()?;

            let new_outpoints: std::collections::HashSet<_> = utxo_map.keys().cloned().collect();

            for utxos in wallet_guard.utxos.values_mut() {
                utxos.retain(|outpoint, _| new_outpoints.contains(outpoint));
            }
            wallet_guard.utxos.retain(|_, utxos| !utxos.is_empty());

            for (outpoint, tx_out) in &utxo_map {
                if let Ok(address) = Address::from_script(&tx_out.script_pubkey, self.network) {
                    wallet_guard
                        .utxos
                        .entry(address)
                        .or_default()
                        .insert(*outpoint, tx_out.clone());
                }
            }

            let mut balance_changes = Vec::new();
            for address in &addresses {
                let balance = address_balances.get(address).cloned().unwrap_or(0);
                let current = wallet_guard.address_balances.get(address).cloned();
                if current != Some(balance) {
                    wallet_guard
                        .address_balances
                        .insert(address.clone(), balance);
                    balance_changes.push((address.clone(), balance));
                }
            }

            let mut received_changes = Vec::new();
            for (address, total_received) in &total_received_map {
                let current = wallet_guard.address_total_received.get(address).cloned();
                if current != Some(*total_received) {
                    wallet_guard
                        .address_total_received
                        .insert(address.clone(), *total_received);
                    received_changes.push((address.clone(), *total_received));
                }
            }

            if !stale_txids.is_empty() {
                let stale_count = stale_txids.len();
                wallet_guard
                    .unused_asset_locks
                    .retain(|(tx, _, _, _, _)| !stale_txids.contains(&tx.txid()));
                tracing::info!("Removed {} stale asset locks", stale_count);
            }

            if let Some(txs) = rpc_transactions {
                wallet_guard.set_transactions(txs);
            }

            // Update PlatformWallet balance via WalletInfoWriteGuard (auto-refreshes WalletBalance)
            if let Some(pw) = wallet_guard.platform_wallet.as_ref() {
                if let Some(mut pw_info) = pw.core().try_wallet_info_mut() {
                    pw_info.balance = WalletCoreBalance::new(
                        total_balance,
                        0, // unconfirmed
                        0, // immature
                        0, // locked
                    );
                }
            }

            (balance_changes, received_changes)
        };

        // Step 13: Persist remaining changes to database (no wallet lock needed)
        for (address, balance) in &changed_balances {
            self.db
                .update_address_balance(&seed_hash, address, *balance)?;
        }

        for (address, total_received) in &changed_total_received {
            self.db
                .update_address_total_received(&seed_hash, address, *total_received)?;
        }

        self.db
            .update_wallet_balances(&seed_hash, total_balance, 0, total_balance)?;

        // Sync balance to PlatformWallet's ManagedWalletInfo so it stays
        // current and can serve as the canonical read source.
        // Uses try_wallet_info_mut() because we are in a blocking context
        // (spawn_blocking) where awaiting is not possible.
        if let Some(pw) = self.get_platform_wallet(&seed_hash) {
            if let Some(mut pw_info) = pw.core().try_wallet_info_mut() {
                pw_info.balance = WalletCoreBalance::new(
                    total_balance, // spendable
                    0,             // unconfirmed
                    0,             // immature
                    0,             // locked
                );
            }
        }

        let warning = if tx_truncated {
            Some(
                "Transaction history may be incomplete. The most recent transactions are shown."
                    .to_string(),
            )
        } else {
            None
        };

        Ok(BackendTaskSuccessResult::RefreshedWallet { warning })
    }
}
