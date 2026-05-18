use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use std::sync::Arc;

impl AppContext {
    /// Generate a fresh receive address.
    ///
    /// Inert at the P0.5 compile floor: address derivation moves to the
    /// upstream `platform-wallet` runtime. P2 wires this to
    /// `wallet_backend.next_receive_address`.
    pub(crate) async fn generate_receive_address(
        self: &Arc<Self>,
        _seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        Err(crate::backend_task::error::TaskError::WalletBackendNotYetWired)
    }
}
