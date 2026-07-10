//! SPV reconnect regression test (scenario B).
//!
//! Verifies that `stop_spv` + `ensure_wallet_backend_and_start_spv` completes
//! cleanly without a `WalletStorageError::AlreadyOpen` panic/error.
//!
//! **Background**: the disconnect → reconnect path is *restart-in-place*.
//! `stop_spv` stops the upstream `SpvRuntime` run-loop and quiesces the
//! coordinators but KEEPS the `WalletBackend` (and its transitive
//! `Arc<SqlitePersister>`) wired in the `AppContext` slot. The next Connect
//! fast-paths on that populated slot — no `WalletBackend::new`, no
//! `SqlitePersister::open` — so the SAME instance restarts on a re-armed latch.
//! Because the persister DB is never closed and reopened, the path registered
//! in dash-spv's global `OPEN_FILES` map (`storage/lockfile.rs`) is never
//! re-registered, and `AlreadyOpen` is impossible by construction.
//!
//! This is the live-network counterpart to the offline unit test
//! `reconnect_restart_in_place_reuses_backend` in `src/context/wallet_lifecycle.rs`:
//! it asserts the same reuse/restart contract against real testnet peers.
//!
//! This test drives the full connect → disconnect → reconnect cycle with an
//! isolated `AppContext` (fresh temp dir, empty DB) to avoid disturbing the
//! shared harness context used by other tests.
//!
//! **Run command** (requires testnet egress; no funded wallet needed):
//! ```bash
//! cargo test --test backend-e2e --all-features -- \
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
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
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

    // Record the backend instance so the reconnect can be proven to REUSE it.
    let first_ptr = {
        let backend = app_context
            .wallet_backend()
            .expect("B: backend must be wired after the first connect");
        assert!(
            backend.is_started(),
            "B: first connect must start chain sync"
        );
        Arc::as_ptr(&backend)
    };

    // ── Disconnect ──────────────────────────────────────────────────────────
    app_context.stop_spv().await;
    tracing::info!("B: SPV stopped (disconnect complete)");

    // Restart-in-place: the backend stays wired (slot not taken) with its
    // start latch re-armed, so the next Connect restarts the SAME instance and
    // never reopens the persister.
    {
        let backend = app_context
            .wallet_backend()
            .expect("B: stop_spv must KEEP the backend wired for restart-in-place (NOT unwire it)");
        assert!(
            !backend.is_started(),
            "B: stop_spv must re-arm the start latch so the next Connect can restart"
        );
    }

    // ── Reconnect (must NOT fail with AlreadyOpen) ──────────────────────────
    let (sender2, _rx2) =
        tokio::sync::mpsc::channel::<TaskResult>(256).with_egui_ctx(egui_ctx.clone());
    app_context
        .ensure_wallet_backend_and_start_spv(sender2)
        .await
        .expect(
            "B: second ensure_wallet_backend_and_start_spv must succeed; \
             if 'AlreadyOpen' appears the restart-in-place contract has been \
             broken — stop_spv must keep the backend wired so the persister is \
             never closed and reopened",
        );

    // The reconnect must reuse the SAME backend instance, not rebuild it.
    {
        let backend = app_context
            .wallet_backend()
            .expect("B: backend must still be wired after reconnect");
        assert_eq!(
            first_ptr,
            Arc::as_ptr(&backend),
            "B: restart-in-place must REUSE the same backend, not rebuild it"
        );
        assert!(
            backend.is_started(),
            "B: reconnect must restart chain sync on the reused backend"
        );
    }

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
