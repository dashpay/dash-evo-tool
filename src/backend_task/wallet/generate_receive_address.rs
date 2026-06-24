use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use std::sync::Arc;

impl AppContext {
    /// Generate a fresh receive address via the wallet backend.
    pub(crate) async fn generate_receive_address(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // A seed hash that matches no wallet in the local store is a genuine
        // "not found". This is distinct from a known wallet whose backend is
        // still loading: the backend reports the latter as the transient,
        // retryable `WalletNotLoaded`. Resolving the existence question here,
        // where the DET-side wallet store lives, keeps that distinction honest
        // instead of collapsing both cases into `WalletNotLoaded`.
        if !self.wallets.read()?.contains_key(&seed_hash) {
            return Err(TaskError::WalletNotFound);
        }
        let backend = self.wallet_backend()?;
        let address = backend.next_receive_address(&seed_hash).await?;
        Ok(BackendTaskSuccessResult::GeneratedReceiveAddress { seed_hash, address })
    }
}
