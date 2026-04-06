use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::dashcore::OutPoint;
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

        let (platform_wallet, _seed_hash) = {
            let guard = wallet.read()?;
            let pw = guard
                .platform_wallet
                .clone()
                .ok_or(TaskError::WalletNotFound)?;
            (pw, guard.seed_hash())
        };

        let (tx, _private_key) = platform_wallet
            .asset_locks()
            .build_asset_lock_transaction(
                amount_duffs,
                0,
                platform_wallet::AssetLockFundingType::IdentityRegistration,
                identity_index,
            )
            .await
            .map_err(|e| TaskError::AssetLockTransactionBuildFailed {
                detail: e.to_string(),
            })?;

        let result = self
            .broadcast_and_track_asset_lock_via_manager(
                tx,
                amount_duffs,
                platform_wallet::AssetLockFundingType::IdentityRegistration,
                identity_index,
                &platform_wallet,
            )
            .await?;

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

        let (platform_wallet, _seed_hash) = {
            let guard = wallet.read()?;
            let pw = guard
                .platform_wallet
                .clone()
                .ok_or(TaskError::WalletNotFound)?;
            (pw, guard.seed_hash())
        };

        let (tx, _private_key) = platform_wallet
            .asset_locks()
            .build_asset_lock_transaction(
                amount_duffs,
                0,
                platform_wallet::AssetLockFundingType::IdentityTopUp,
                identity_index,
            )
            .await
            .map_err(|e| TaskError::AssetLockTransactionBuildFailed {
                detail: e.to_string(),
            })?;

        let result = self
            .broadcast_and_track_asset_lock_via_manager(
                tx,
                amount_duffs,
                platform_wallet::AssetLockFundingType::IdentityTopUp,
                identity_index,
                &platform_wallet,
            )
            .await?;

        Ok(result)
    }

    /// Build, register, and broadcast an asset lock transaction via
    /// AssetLockManager. The TX is tracked internally by the manager;
    /// the proof will arrive later via SPV events.
    async fn broadcast_and_track_asset_lock_via_manager(
        &self,
        asset_lock_transaction: dash_sdk::dpp::dashcore::Transaction,
        amount_duffs: u64,
        funding_type: platform_wallet::AssetLockFundingType,
        identity_index: u32,
        platform_wallet: &platform_wallet::PlatformWallet,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let tx_id = asset_lock_transaction.txid();
        let out_point = OutPoint::new(tx_id, 0);

        // Register with AssetLockManager so SPV events can resolve it.
        platform_wallet.asset_locks().recover_asset_lock_blocking(
            asset_lock_transaction.clone(),
            amount_duffs,
            0,
            funding_type,
            identity_index,
            out_point,
            None,
        );

        // Broadcast via SPV P2P peers.
        if let Err(e) = self
            .wallet_manager
            .broadcast_transaction(&asset_lock_transaction)
            .await
        {
            return Err(TaskError::AssetLockTransactionBuildFailed {
                detail: e.to_string(),
            });
        }

        Ok(BackendTaskSuccessResult::Message(format!(
            "Asset lock transaction broadcast successfully. TX ID: {}",
            tx_id
        )))
    }
}
