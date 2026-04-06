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
        _allow_take_fee_from_amount: bool,
        identity_index: u32,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let amount_duffs = amount / CREDITS_PER_DUFF;

        let (platform_wallet, seed_hash) = {
            let guard = wallet.read()?;
            let pw = guard
                .platform_wallet
                .clone()
                .ok_or(TaskError::WalletNotFound)?;
            (pw, guard.seed_hash())
        };

        let (tx, _private_key) = platform_wallet
            .core()
            .build_asset_lock_transaction(
                amount_duffs,
                platform_wallet::AssetLockFundingType::IdentityRegistration,
                identity_index,
            )
            .await
            .map_err(|e| TaskError::AssetLockTransactionBuildFailed {
                detail: e.to_string(),
            })?;

        let result = self.broadcast_and_track_asset_lock(tx).await?;

        // Wallet changes (UTXO updates) are auto-flushed via
        // FlushStrategy::Immediate when queued by the platform wallet.

        Ok(result)
    }

    pub async fn create_top_up_asset_lock(
        &self,
        wallet: Arc<RwLock<Wallet>>,
        amount: Credits,
        _allow_take_fee_from_amount: bool,
        identity_index: u32,
        _topup_index: u32,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let amount_duffs = amount / CREDITS_PER_DUFF;

        let (platform_wallet, seed_hash) = {
            let guard = wallet.read()?;
            let pw = guard
                .platform_wallet
                .clone()
                .ok_or(TaskError::WalletNotFound)?;
            (pw, guard.seed_hash())
        };

        let (tx, _private_key) = platform_wallet
            .core()
            .build_asset_lock_transaction(
                amount_duffs,
                platform_wallet::AssetLockFundingType::IdentityTopUp,
                identity_index,
            )
            .await
            .map_err(|e| TaskError::AssetLockTransactionBuildFailed {
                detail: e.to_string(),
            })?;

        let result = self.broadcast_and_track_asset_lock(tx).await?;

        // Wallet changes (UTXO updates) are auto-flushed via
        // FlushStrategy::Immediate when queued by the platform wallet.

        Ok(result)
    }

    /// Broadcast an asset lock transaction and register it for finality tracking.
    async fn broadcast_and_track_asset_lock(
        &self,
        asset_lock_transaction: dash_sdk::dpp::dashcore::Transaction,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
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
