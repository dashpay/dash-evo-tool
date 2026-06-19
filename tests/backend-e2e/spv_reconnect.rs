//! SPV reconnect regression test (scenario B).
//!
//! Verifies that `stop_spv` + `ensure_wallet_backend_and_start_spv` completes
//! cleanly without a `WalletStorageError::AlreadyOpen` panic/error.
//!
//! **Background**: `WalletBackend::shutdown` must stop the upstream
//! `SpvRuntime` run-loop *before* the `PlatformWalletManager` tears down its
//! coordinators.  The run-loop holds a transitive `Arc<SqlitePersister>` whose
//! path is registered in a global `OPEN_FILES` map (dash-spv
//! `storage/lockfile.rs`).  If the run-loop is still alive when the next
//! `WalletBackend::new` tries to open the same persistor, that path is still
//! registered and the open fails with `AlreadyOpen`. The fix joins / aborts
//! the background task inside `shutdown` so the persister can drop before the
//! next `new`.
//!
//! This test drives the full connect → disconnect → reconnect cycle with an
//! isolated `AppContext` (fresh temp dir, empty DB) to avoid disturbing the
//! shared harness context used by other tests.
//!
//! **Run command** (requires testnet egress; no funded wallet needed):
//! ```bash
//! RUST_MIN_STACK=16777216 \
//!   cargo test --test backend-e2e --all-features -- \
//!   --ignored --nocapture spv_reconnect_succeeds_without_already_open
//! ```

use crate::framework::wait;
use dash_evo_tool::app::TaskResult;
use dash_evo_tool::app_dir::ensure_env_file;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::context::connection_status::ConnectionStatus;
use dash_evo_tool::database::test_helpers::create_database_at_path;
use dash_evo_tool::utils::egui_mpsc::EguiMpscAsync;
use dash_evo_tool::utils::tasks::TaskManager;
use dash_sdk::dpp::dashcore::Network;
use std::sync::Arc;
use std::time::Duration;

/// Scenario B regression: connect → disconnect → connect must not produce
/// `WalletStorageError::AlreadyOpen` on the second connect.
///
/// An isolated AppContext is used so the shared harness SPV (and all other
/// tests) is not interrupted by the stop/restart cycle.
///
/// Funding requirement: none — this test never touches wallets or balances.
/// It only needs live testnet peers (outbound TCP on port 19999).
#[tokio_shared_rt::test(shared, flavor = "multi_thread", worker_threads = 12)]
#[ignore = "network-dependent; requires testnet egress — no funded wallet needed"]
async fn spv_reconnect_succeeds_without_already_open() {
    // ── Isolated context setup ──────────────────────────────────────────────
    let workdir =
        std::env::temp_dir().join(format!("dash-evo-e2e-reconnect-{}", std::process::id()));
    std::fs::create_dir_all(&workdir).expect("create reconnect test workdir");
    ensure_env_file(&workdir);

    let db_path = workdir.join("data.db");
    let db = Arc::new(create_database_at_path(&db_path).expect("create reconnect test DB"));

    let subtasks = Arc::new(TaskManager::new());
    let connection_status = Arc::new(ConnectionStatus::new());
    let egui_ctx = egui::Context::default();
    let app_kv = AppContext::open_app_kv(&workdir).expect("open app k/v");
    let secret_store = AppContext::open_secret_store(&workdir).expect("open secret store");

    let app_context = Arc::new(
        AppContext::new(
            workdir.clone(),
            Network::Testnet,
            db,
            subtasks,
            connection_status,
            egui_ctx.clone(),
            app_kv,
            secret_store,
        )
        .expect("create isolated AppContext"),
    );

    // ── Connect (first boot) ────────────────────────────────────────────────
    let (sender1, _rx1) =
        tokio::sync::mpsc::channel::<TaskResult>(256).with_egui_ctx(egui_ctx.clone());
    app_context
        .ensure_wallet_backend_and_start_spv(sender1)
        .await
        .expect("B: first ensure_wallet_backend_and_start_spv must succeed");

    tracing::info!("B: first connect complete; waiting for SPV peers...");
    wait::wait_for_spv_peers(&app_context, Duration::from_secs(60))
        .await
        .expect("B: SPV did not connect to peers on first boot within 60s");
    tracing::info!("B: first connect — SPV peers found");

    // ── Disconnect ──────────────────────────────────────────────────────────
    app_context.stop_spv().await;
    tracing::info!("B: SPV stopped (disconnect complete)");

    // The backend must have been torn down.
    assert!(
        app_context.wallet_backend().is_err(),
        "B: wallet backend must be None after stop_spv"
    );

    // ── Reconnect (must NOT fail with AlreadyOpen) ──────────────────────────
    let (sender2, _rx2) =
        tokio::sync::mpsc::channel::<TaskResult>(256).with_egui_ctx(egui_ctx.clone());
    app_context
        .ensure_wallet_backend_and_start_spv(sender2)
        .await
        .expect(
            "B: second ensure_wallet_backend_and_start_spv must succeed; \
             if 'AlreadyOpen' appears the fix has been reverted — \
             WalletBackend::shutdown must stop the SpvRuntime run-loop \
             before the persister is re-opened",
        );

    tracing::info!("B: reconnect complete; waiting for SPV peers...");
    wait::wait_for_spv_peers(&app_context, Duration::from_secs(60))
        .await
        .expect("B: SPV did not connect to peers after reconnect within 60s");
    tracing::info!("B: reconnect — SPV peers found; scenario B PASSED");

    // ── Cleanup ─────────────────────────────────────────────────────────────
    if let Ok(backend) = app_context.wallet_backend() {
        backend.shutdown().await;
    }
    if let Err(e) = std::fs::remove_dir_all(&workdir) {
        tracing::warn!("B: failed to clean up workdir {}: {e}", workdir.display());
    }
}
