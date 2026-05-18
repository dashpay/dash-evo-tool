//! Refresh single-key wallet info.
//!
//! Single-key wallets are unsupported in this version (see
//! [`single-key-mock`](../../../../docs/ai-design/2026-05-18-platform-wallet-migration/single-key-mock.md)).

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::single_key::SingleKeyWallet;
use std::sync::{Arc, RwLock};

impl AppContext {
    pub fn refresh_single_key_wallet_info(
        &self,
        _wallet: Arc<RwLock<SingleKeyWallet>>,
    ) -> Result<(), TaskError> {
        Err(TaskError::SingleKeyWalletsUnsupported)
    }
}
