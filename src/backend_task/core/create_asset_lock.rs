use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
use dash_sdk::dpp::dashcore::Transaction;
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

        let asset_lock_transaction = if let Some(tx) = self
            .try_build_registration_via_platform_wallet(&wallet, amount_duffs, identity_index)
            .await
        {
            tx
        } else {
            let (tx, _private_key, _change_address, _used_utxos) = {
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
            tx
        };

        self.broadcast_and_track_asset_lock(asset_lock_transaction)
            .await
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

        let asset_lock_transaction = if let Some(tx) = self
            .try_build_topup_via_platform_wallet(
                &wallet,
                amount_duffs,
                identity_index,
                top_up_index,
            )
            .await
        {
            tx
        } else {
            let (tx, _private_key, _change_address, _used_utxos) = {
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
            tx
        };

        self.broadcast_and_track_asset_lock(asset_lock_transaction)
            .await
    }

    /// Try to build a registration asset lock transaction using the new
    /// `CoreWallet::build_registration_asset_lock_transaction` API.
    ///
    /// Returns `Some(Transaction)` on success, or `None` if the platform wallet
    /// is not available for this wallet (e.g., wallet is locked or not yet
    /// registered with the bridge). Callers fall back to the old path on `None`.
    async fn try_build_registration_via_platform_wallet(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
        amount_duffs: u64,
        identity_index: u32,
    ) -> Option<Transaction> {
        let seed_hash = wallet.read().ok()?.seed_hash();
        let platform_wallet = self.get_platform_wallet(&seed_hash)?;

        match platform_wallet
            .core()
            .build_registration_asset_lock_transaction(amount_duffs, identity_index)
            .await
        {
            Ok((tx, _private_key)) => {
                tracing::info!(
                    tx_id = %tx.txid(),
                    "Built registration asset lock via CoreWallet"
                );
                Some(tx)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "CoreWallet registration asset lock build failed, falling back to legacy path"
                );
                None
            }
        }
    }

    /// Try to build a top-up asset lock transaction using the new
    /// `CoreWallet::build_topup_asset_lock_transaction` API.
    ///
    /// Returns `Some(Transaction)` on success, or `None` if the platform wallet
    /// is not available for this wallet. Callers fall back to the old path on `None`.
    async fn try_build_topup_via_platform_wallet(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
        amount_duffs: u64,
        identity_index: u32,
        topup_index: u32,
    ) -> Option<Transaction> {
        let seed_hash = wallet.read().ok()?.seed_hash();
        let platform_wallet = self.get_platform_wallet(&seed_hash)?;

        match platform_wallet
            .core()
            .build_topup_asset_lock_transaction(amount_duffs, identity_index, topup_index)
            .await
        {
            Ok((tx, _private_key)) => {
                tracing::info!(
                    tx_id = %tx.txid(),
                    "Built top-up asset lock via CoreWallet"
                );
                Some(tx)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "CoreWallet top-up asset lock build failed, falling back to legacy path"
                );
                None
            }
        }
    }

    /// Broadcast an asset lock transaction and register it for finality tracking.
    ///
    /// This is the shared broadcast+track logic used by both registration and
    /// top-up asset lock creation, regardless of which path built the transaction.
    async fn broadcast_and_track_asset_lock(
        &self,
        asset_lock_transaction: Transaction,
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
