use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::fee::Credits;
use std::sync::{Arc, RwLock};

impl AppContext {
    pub fn create_registration_asset_lock(
        &self,
        wallet: Arc<RwLock<Wallet>>,
        amount: Credits,
        allow_take_fee_from_amount: bool,
        identity_index: u32,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Create the asset lock transaction
        let (asset_lock_transaction, _private_key, _change_address, used_utxos) = {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;

            wallet_guard.registration_asset_lock_transaction(
                self.network,
                amount,
                allow_take_fee_from_amount,
                identity_index,
                Some(self),
            )?
        };

        let tx_id = asset_lock_transaction.txid();

        // Insert the transaction into waiting for finality
        {
            let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
            proofs.insert(tx_id, None);
        }

        // Broadcast the transaction
        self.core_client
            .read()
            .expect("Core client lock was poisoned")
            .send_raw_transaction(&asset_lock_transaction)
            .map_err(|e| format!("Failed to broadcast asset lock transaction: {}", e))?;

        // Update wallet UTXOs
        {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
            wallet_guard.utxos.retain(|_, utxo_map| {
                utxo_map.retain(|outpoint, _| !used_utxos.contains_key(outpoint));
                !utxo_map.is_empty() // Keep addresses that still have UTXOs
            });

            // Drop used UTXOs from database
            for utxo in used_utxos.keys() {
                self.db
                    .drop_utxo(utxo, &self.network.to_string())
                    .map_err(|e| e.to_string())?;
            }
        }

        Ok(BackendTaskSuccessResult::Message(format!(
            "Asset lock transaction broadcast successfully. TX ID: {}",
            tx_id
        )))
    }

    pub fn create_top_up_asset_lock(
        &self,
        wallet: Arc<RwLock<Wallet>>,
        amount: Credits,
        allow_take_fee_from_amount: bool,
        identity_index: u32,
        top_up_index: u32,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Create the asset lock transaction
        let (asset_lock_transaction, _private_key, _change_address, used_utxos) = {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;

            wallet_guard.top_up_asset_lock_transaction(
                self.network,
                amount,
                allow_take_fee_from_amount,
                identity_index,
                top_up_index,
                Some(self),
            )?
        };

        let tx_id = asset_lock_transaction.txid();

        // Insert the transaction into waiting for finality
        {
            let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
            proofs.insert(tx_id, None);
        }

        // Broadcast the transaction
        self.core_client
            .read()
            .expect("Core client lock was poisoned")
            .send_raw_transaction(&asset_lock_transaction)
            .map_err(|e| format!("Failed to broadcast asset lock transaction: {}", e))?;

        // Update wallet UTXOs
        {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;
            wallet_guard.utxos.retain(|_, utxo_map| {
                utxo_map.retain(|outpoint, _| !used_utxos.contains_key(outpoint));
                !utxo_map.is_empty() // Keep addresses that still have UTXOs
            });

            // Drop used UTXOs from database
            for utxo in used_utxos.keys() {
                self.db
                    .drop_utxo(utxo, &self.network.to_string())
                    .map_err(|e| e.to_string())?;
            }
        }

        Ok(BackendTaskSuccessResult::Message(format!(
            "Asset lock transaction broadcast successfully. TX ID: {}",
            tx_id
        )))
    }
}
