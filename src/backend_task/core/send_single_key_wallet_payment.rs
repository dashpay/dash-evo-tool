//! Send single-key wallet payment.
//!
//! Single-key wallets are unsupported in this version (see
//! [`single-key-mock`](../../../../docs/ai-design/2026-05-18-platform-wallet-migration/single-key-mock.md)).

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::core::WalletPaymentRequest;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::single_key::SingleKeyWallet;
use std::sync::{Arc, RwLock};

impl AppContext {
    pub async fn send_single_key_wallet_payment(
        &self,
        _wallet: Arc<RwLock<SingleKeyWallet>>,
        _request: WalletPaymentRequest,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        Err(TaskError::SingleKeyWalletsUnsupported)
    }
}
