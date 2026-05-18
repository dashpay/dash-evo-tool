use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use std::sync::{Arc, RwLock};

impl AppContext {
    /// Recover unused asset locks.
    ///
    /// Inert at the P0.5 compile floor: asset-lock tracking is owned by
    /// upstream `platform-wallet`'s `AssetLockManager`, which tracks locks
    /// continuously. P2 wires this to the upstream runtime.
    pub fn recover_asset_locks(
        &self,
        _wallet: Arc<RwLock<Wallet>>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        Err(TaskError::WalletBackendNotYetWired)
    }
}
