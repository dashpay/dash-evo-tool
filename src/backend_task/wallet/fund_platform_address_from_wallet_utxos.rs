use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::wallet::WalletSeedHash;
use dash_sdk::dpp::address_funds::PlatformAddress;
use std::sync::Arc;

impl AppContext {
    /// Fund a platform address directly from wallet UTXOs.
    ///
    /// Inert at the P0.5 compile floor: asset-lock creation and broadcast
    /// move to the upstream `platform-wallet` runtime. P2 wires the real
    /// flow (build via upstream, broadcast via `SpvRuntime`).
    pub(crate) async fn fund_platform_address_from_wallet_utxos(
        self: &Arc<Self>,
        _seed_hash: WalletSeedHash,
        _amount: u64,
        _destination: PlatformAddress,
        _fee_deduct_from_output: bool,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        Err(TaskError::WalletBackendNotYetWired)
    }
}
