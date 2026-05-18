//! Live EventBridge validation: a real upstream `SpvRuntime` sync must drive
//! the DET `EventBridge` (`on_progress` / `on_sync_event`) → `ConnectionStatus`
//! → frame-loop `TaskResult::Refresh` → repaint.
//!
//! Network-dependent, so `#[ignore]`. The deterministic mapping logic is
//! covered by unit tests in `src/wallet_backend/event_bridge.rs`; this test
//! exercises the full live path once the backend-e2e harness drives a real
//! `WalletBackend` sync.
//!
//! TODO(P2-followup): the shared harness `start_spv()` is still the P1 inert
//! stub. Wiring it to build + start a real `WalletBackend` (so this assertion
//! observes a genuine sync) is test-infra work tracked for P3's migration QA
//! lane. Until then this test documents the intended live contract.

use crate::framework::harness::ctx;
use dash_evo_tool::model::spv_status::SpvStatus;
use std::time::Duration;

#[tokio::test]
#[ignore = "network-dependent; requires harness WalletBackend wiring (P2-followup)"]
async fn event_bridge_drives_connection_status_on_live_sync() {
    let app_ctx = ctx().await.app_context.clone();

    // Once the harness starts a real WalletBackend, the upstream SpvRuntime
    // sync loop fires EventBridge callbacks; ConnectionStatus must leave Idle
    // and reach Running on completion.
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    loop {
        let status = app_ctx.connection_status().spv_status();
        if status == SpvStatus::Running {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "EventBridge did not drive ConnectionStatus to Running within 5m \
             (last status: {status:?})"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
