use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::fee::Credits;
use std::sync::{Arc, RwLock};

impl AppContext {
    pub async fn create_registration_asset_lock(
        &self,
        wallet: Arc<RwLock<Wallet>>,
        amount: Credits,
        allow_take_fee_from_amount: bool,
        identity_index: u32,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Convert credits to duffs (1 duff = 1000 credits)
        let amount_duffs = amount / CREDITS_PER_DUFF;

        // Create the asset lock transaction
        let (asset_lock_transaction, _private_key, _change_address, _used_utxos) = {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;

            wallet_guard.registration_asset_lock_transaction(
                self,
                self.network,
                amount_duffs,
                allow_take_fee_from_amount,
                identity_index,
            )?
        };

        let tx_id = asset_lock_transaction.txid();

        // Insert the transaction into waiting for finality
        {
            let mut proofs = self
                .transactions_waiting_for_finality
                .lock()
                .map_err(|e| e.to_string())?;
            proofs.insert(tx_id, None);
        }

        // Broadcast the transaction.  If broadcast fails, the UTXOs have already
        // been removed from the wallet (inside the transaction builder) but were
        // never actually spent on-chain.  The caller should handle refreshing
        // the wallet so the next UTXO reload reconciles in-memory state with
        // the chain.  See also: https://github.com/dashpay/dash-evo-tool/issues/657
        if let Err(e) = self
            .broadcast_raw_transaction(&asset_lock_transaction)
            .await
        {
            // Clean up the finality tracking entry
            if let Ok(mut proofs) = self.transactions_waiting_for_finality.lock() {
                proofs.remove(&tx_id);
            } else {
                tracing::warn!("Failed to clean up finality tracking for tx {tx_id}: Mutex poisoned");
            }
            return Err(format!("Failed to broadcast asset lock transaction: {}", e));
        }

        Ok(BackendTaskSuccessResult::Message(format!(
            "Asset lock transaction broadcast successfully. TX ID: {}",
            tx_id
        )))
    }

    pub async fn create_top_up_asset_lock(
        &self,
        wallet: Arc<RwLock<Wallet>>,
        amount: Credits,
        allow_take_fee_from_amount: bool,
        identity_index: u32,
        top_up_index: u32,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Convert credits to duffs (1 duff = 1000 credits)
        let amount_duffs = amount / CREDITS_PER_DUFF;

        // Create the asset lock transaction
        let (asset_lock_transaction, _private_key, _change_address, _used_utxos) = {
            let mut wallet_guard = wallet.write().map_err(|e| e.to_string())?;

            wallet_guard.top_up_asset_lock_transaction(
                self,
                self.network,
                amount_duffs,
                allow_take_fee_from_amount,
                identity_index,
                top_up_index,
            )?
        };

        let tx_id = asset_lock_transaction.txid();

        // Insert the transaction into waiting for finality
        {
            let mut proofs = self
                .transactions_waiting_for_finality
                .lock()
                .map_err(|e| e.to_string())?;
            proofs.insert(tx_id, None);
        }

        // Broadcast the transaction (see registration path above for cleanup rationale)
        if let Err(e) = self
            .broadcast_raw_transaction(&asset_lock_transaction)
            .await
        {
            if let Ok(mut proofs) = self.transactions_waiting_for_finality.lock() {
                proofs.remove(&tx_id);
            } else {
                tracing::warn!("Failed to clean up finality tracking for tx {tx_id}: Mutex poisoned");
            }
            return Err(format!("Failed to broadcast asset lock transaction: {}", e));
        }

        Ok(BackendTaskSuccessResult::Message(format!(
            "Asset lock transaction broadcast successfully. TX ID: {}",
            tx_id
        )))
    }
}
