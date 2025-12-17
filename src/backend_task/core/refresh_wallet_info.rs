use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::{DerivationPathHelpers, Wallet};
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::Address;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

impl AppContext {
    pub fn refresh_wallet_info(
        &self,
        wallet: Arc<RwLock<Wallet>>,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Step 1: Collect Core chain addresses from the wallet (excluding Platform addresses)
        // Platform addresses  are NOT valid on Core chain and must be skipped
        let addresses = {
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
            wallet_guard
                .known_addresses
                .iter()
                .filter(|(_, path)| !path.is_platform_payment(self.network))
                .map(|(addr, _)| addr.clone())
                .collect::<Vec<_>>()
        };

        // Step 2: Import addresses to Core (needed for UTXO queries)
        let client = self
            .core_client
            .read()
            .expect("Core client lock was poisoned");

        for address in &addresses {
            if let Err(e) = client.import_address(address, None, Some(false)) {
                tracing::debug!(?e, address = %address, "import_address failed during refresh");
            }
        }
        drop(client);

        // Step 3: Reload UTXOs using the wallet's existing method
        let utxo_map = {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
            match wallet_guard.reload_utxos(
                &self
                    .core_client
                    .read()
                    .expect("Core client lock was poisoned"),
                self.network,
                Some(self),
            ) {
                Ok(utxo_map) => utxo_map,
                Err(e) => {
                    eprintln!("Error reloading UTXOs: {}", e);
                    return Err(e);
                }
            }
        };

        // Step 4: Calculate actual balances from UTXOs and update wallet
        // Group UTXOs by address and sum their values
        let mut address_balances: HashMap<Address, u64> = HashMap::new();
        for tx_out in utxo_map.values() {
            if let Ok(address) = Address::from_script(&tx_out.script_pubkey, self.network) {
                *address_balances.entry(address).or_insert(0) += tx_out.value;
            }
        }

        // Update wallet's address_balances with UTXO-based balances
        {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
            for address in &addresses {
                let balance = address_balances.get(address).cloned().unwrap_or(0);
                wallet_guard.update_address_balance(address, balance, self)?;
            }
        }

        // Step 5: Fetch total received for each address from Core RPC
        {
            let client = self
                .core_client
                .read()
                .expect("Core client lock was poisoned");

            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
            for address in &addresses {
                // get_received_by_address returns the total historical amount received
                match client.get_received_by_address(address, None) {
                    Ok(amount) => {
                        let total_received = amount.to_sat();
                        if let Err(e) = wallet_guard.update_address_total_received(
                            address,
                            total_received,
                            self,
                        ) {
                            tracing::debug!(
                                ?e,
                                address = %address,
                                "Failed to update total received"
                            );
                        }
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

        // Step 6: Insert updated UTXOs into the database
        for (outpoint, tx_out) in &utxo_map {
            if let Ok(address) = Address::from_script(&tx_out.script_pubkey, self.network) {
                self.db
                    .insert_utxo(
                        outpoint.txid.as_ref(),           // txid: &[u8]
                        outpoint.vout,                    // vout: i64
                        &address,                         // address: &str
                        tx_out.value,                     // value: i64
                        &tx_out.script_pubkey.to_bytes(), // script_pubkey: &[u8]
                        self.network,                     // network: &str
                    )
                    .map_err(|e| e.to_string())?;
            }
        }

        // Step 7: Calculate and persist wallet-level balance
        // In RPC mode, we use the UTXO sum as the confirmed/total balance
        let total_balance: u64 = utxo_map.values().map(|tx_out| tx_out.value).sum();
        {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
            let seed_hash = wallet_guard.seed_hash();
            wallet_guard.update_spv_balances(total_balance, 0, total_balance);
            if let Err(e) =
                self.db
                    .update_wallet_balances(&seed_hash, total_balance, 0, total_balance)
            {
                tracing::warn!(error = %e, "Failed to persist wallet balances");
            }
        }

        // Step 8: Return a success result
        Ok(BackendTaskSuccessResult::RefreshedWallet)
    }
}
