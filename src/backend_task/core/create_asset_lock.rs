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
        self.create_asset_lock_via_manager(
            wallet,
            amount,
            platform_wallet::AssetLockFundingType::IdentityRegistration,
            identity_index,
        )
        .await
    }

    pub async fn create_top_up_asset_lock(
        &self,
        wallet: Arc<RwLock<Wallet>>,
        amount: Credits,
        _allow_take_fee_from_amount: bool,
        identity_index: u32,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.create_asset_lock_via_manager(
            wallet,
            amount,
            platform_wallet::AssetLockFundingType::IdentityTopUp,
            identity_index,
        )
        .await
    }

    /// Build, broadcast, track, and wait for finality on an asset lock TX via
    /// [`AssetLockManager::create_funded_asset_lock_proof`]. Delegates the
    /// entire lifecycle to platform-wallet.
    async fn create_asset_lock_via_manager(
        &self,
        wallet: Arc<RwLock<Wallet>>,
        amount: Credits,
        funding_type: platform_wallet::AssetLockFundingType,
        identity_index: u32,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let amount_duffs = amount / CREDITS_PER_DUFF;

        let platform_wallet = {
            let guard = wallet.read()?;
            guard
                .platform_wallet
                .clone()
                .ok_or(TaskError::WalletNotFound)?
        };

        let (_proof, _private_key, out_point) = platform_wallet
            .asset_locks()
            .create_funded_asset_lock_proof(amount_duffs, 0, funding_type, identity_index)
            .await
            .map_err(|e| TaskError::AssetLockTransactionBuildFailed {
                detail: e.to_string(),
            })?;

        Ok(BackendTaskSuccessResult::Message(format!(
            "Asset lock transaction confirmed. TX ID: {}",
            out_point.txid
        )))
    }
}
