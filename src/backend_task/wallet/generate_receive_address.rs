use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use std::sync::Arc;

impl AppContext {
    /// Generate a fresh receive address via the wallet backend.
    pub(crate) async fn generate_receive_address(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        let backend = self.wallet_backend()?;
        let address = backend.next_receive_address(&seed_hash).await?;
        Ok(BackendTaskSuccessResult::GeneratedReceiveAddress { seed_hash, address })
    }
}
