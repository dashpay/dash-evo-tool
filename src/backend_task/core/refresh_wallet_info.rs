use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use std::sync::{Arc, RwLock};

impl AppContext {
    /// Refresh wallet balances/UTXOs/transactions.
    ///
    /// Inert at the P0.5 compile floor: chain sync is owned by upstream
    /// `platform-wallet`, which keeps wallet state current and pushes
    /// updates via events. P2 wires this to the upstream runtime.
    pub fn refresh_wallet_info(
        &self,
        _wallet: Arc<RwLock<Wallet>>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        Err(TaskError::WalletBackendNotYetWired)
    }
}
