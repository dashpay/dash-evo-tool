use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
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
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let amount_duffs = amount / CREDITS_PER_DUFF;

        let (asset_lock_transaction, _private_key, _change_address, _used_utxos) = {
            let mut wallet_guard = wallet.write()?;

            wallet_guard
                .registration_asset_lock_transaction(
                    self,
                    self.network,
                    amount_duffs,
                    allow_take_fee_from_amount,
                    identity_index,
                    None,
                )
                .map_err(|e| TaskError::AssetLockTransactionBuildFailed { detail: e })?
        };

        let tx_id = asset_lock_transaction.txid();

        {
            let mut proofs = self.transactions_waiting_for_finality.lock()?;
            proofs.insert(tx_id, None);
        }

        if let Err(e) = self
            .broadcast_raw_transaction(&asset_lock_transaction)
            .await
        {
            if let Ok(mut proofs) = self.transactions_waiting_for_finality.lock() {
                proofs.remove(&tx_id);
            } else {
                tracing::warn!(
                    "Failed to clean up finality tracking for tx {tx_id}: Mutex poisoned"
                );
            }
            return Err(e);
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
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let amount_duffs = amount / CREDITS_PER_DUFF;

        let (asset_lock_transaction, _private_key, _change_address, _used_utxos) = {
            let mut wallet_guard = wallet.write()?;

            wallet_guard
                .top_up_asset_lock_transaction(
                    self,
                    self.network,
                    amount_duffs,
                    allow_take_fee_from_amount,
                    identity_index,
                    top_up_index,
                    None,
                )
                .map_err(|e| TaskError::AssetLockTransactionBuildFailed { detail: e })?
        };

        let tx_id = asset_lock_transaction.txid();

        {
            let mut proofs = self.transactions_waiting_for_finality.lock()?;
            proofs.insert(tx_id, None);
        }

        if let Err(e) = self
            .broadcast_raw_transaction(&asset_lock_transaction)
            .await
        {
            if let Ok(mut proofs) = self.transactions_waiting_for_finality.lock() {
                proofs.remove(&tx_id);
            } else {
                tracing::warn!(
                    "Failed to clean up finality tracking for tx {tx_id}: Mutex poisoned"
                );
            }
            return Err(e);
        }

        Ok(BackendTaskSuccessResult::Message(format!(
            "Asset lock transaction broadcast successfully. TX ID: {}",
            tx_id
        )))
    }
}
