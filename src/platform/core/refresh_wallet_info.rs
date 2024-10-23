use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::platform::BackendTaskSuccessResult;
use dash_sdk::dashcore_rpc::RpcApi;
use std::sync::{Arc, RwLock};
use dash_sdk::dpp::dashcore::Address;

impl AppContext {
    pub fn refresh_wallet_info(
        &self,
        wallet: Arc<RwLock<Wallet>>,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Step 1: Collect all addresses from the wallet without holding the lock
        let addresses = {
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
            wallet_guard.address_balances.keys().cloned().collect::<Vec<_>>()
        };

        // Step 2: Iterate over each address and update balances
        for address in &addresses {
            // Fetch balance for the address from Dash Core
            match self.core_client.get_received_by_address(address, Some(1)) {
                Ok(new_balance) => {
                    // Update the wallet's address_balances and database
                    {
                        let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
                        wallet_guard.update_address_balance(address, new_balance.to_sat(), self)?;
                    }
                }
                Err(e) => {
                    eprintln!("Error fetching balance for address {}: {:?}", address, e);
                }
            }
        }

        // Step 3: Reload UTXOs using the wallet's existing method
        let utxo_map = {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
            match wallet_guard.reload_utxos(&self.core_client) {
                Ok(utxo_map) => utxo_map,
                Err(e) => {
                    eprintln!("Error reloading UTXOs: {}", e);
                    return Err(e);
                }
            }
        };

        // Insert updated UTXOs into the database
        for (outpoint, tx_out) in &utxo_map {
            // You can get the address from the tx_out's script_pubkey
            let address = Address::from_script(&tx_out.script_pubkey, self.network)
                .map_err(|e| e.to_string())?;
            self.db.insert_utxo(
                &outpoint.txid.as_ref(),          // txid: &[u8]
                outpoint.vout as i64,             // vout: i64
                &address.to_string(),             // address: &str
                tx_out.value as i64,              // value: i64
                &tx_out.script_pubkey.to_bytes(), // script_pubkey: &[u8]
                &self.network.to_string(),        // network: &str
            ).map_err(|e| e.to_string())?;
        }

        // Step 5: Return a success result
        Ok(BackendTaskSuccessResult::None)
    }
}