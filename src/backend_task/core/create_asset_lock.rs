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

        let (asset_lock_transaction, _private_key, _change_address, used_utxos, wallet_seed_hash) = {
            let mut wallet_guard = wallet.write()?;
            let seed_hash = wallet_guard.seed_hash();

            let (tx, key, addr, utxos) = wallet_guard
                .registration_asset_lock_transaction(
                    self,
                    self.network,
                    amount_duffs,
                    allow_take_fee_from_amount,
                    identity_index,
                )
                .map_err(|e| TaskError::AssetLockTransactionBuildFailed { detail: e })?;
            (tx, key, addr, utxos, seed_hash)
        };

        let tx_id = self
            .broadcast_and_commit_asset_lock(
                &asset_lock_transaction,
                amount_duffs,
                &wallet_seed_hash,
                &wallet,
                &used_utxos,
            )
            .await?;

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

        let (asset_lock_transaction, _private_key, _change_address, used_utxos, wallet_seed_hash) = {
            let mut wallet_guard = wallet.write()?;
            let seed_hash = wallet_guard.seed_hash();

            let (tx, key, addr, utxos) = wallet_guard
                .top_up_asset_lock_transaction(
                    self,
                    self.network,
                    amount_duffs,
                    allow_take_fee_from_amount,
                    identity_index,
                    top_up_index,
                )
                .map_err(|e| TaskError::AssetLockTransactionBuildFailed { detail: e })?;
            (tx, key, addr, utxos, seed_hash)
        };

        let tx_id = self
            .broadcast_and_commit_asset_lock(
                &asset_lock_transaction,
                amount_duffs,
                &wallet_seed_hash,
                &wallet,
                &used_utxos,
            )
            .await?;

        Ok(BackendTaskSuccessResult::Message(format!(
            "Asset lock transaction broadcast successfully. TX ID: {}",
            tx_id
        )))
    }
}
