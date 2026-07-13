//! Shared kittest helpers.

#[path = "../common/data_dir.rs"]
mod data_dir;

use dash_evo_tool::context::AppContext;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use data_dir::with_isolated_data_dir;

/// Upper bound a mount helper waits for the wallet backend to finish wiring.
/// Generous on purpose: the poll runs under whole-suite CPU/swap contention
/// (dozens of parallel tests), where a fixed frame count races the async init
/// and intermittently panics `WalletBackendNotYetWired`.
const WALLET_BACKEND_WIRE_TIMEOUT: Duration = Duration::from_secs(30);

/// Step `harness` until its live `AppContext` has a wired wallet backend, then
/// return that context. Panics if the backend is not wired within
/// [`WALLET_BACKEND_WIRE_TIMEOUT`].
///
/// `AppState::new` spawns wallet-backend wiring as a background tokio task, so a
/// fixed `run_steps(N)` gives no guarantee it has completed. Tests that seed the
/// DB via `insert_local_qualified_identity` (which reaches through the backend's
/// k/v store) must gate on this instead of a fixed step count to close the race
/// deterministically — `wallet_backend().is_ok()` is the exact precondition that
/// seeding needs.
pub fn wait_for_wallet_backend(
    harness: &mut Harness<'static, dash_evo_tool::app::AppState>,
) -> Arc<AppContext> {
    let deadline = Instant::now() + WALLET_BACKEND_WIRE_TIMEOUT;
    loop {
        harness.step();
        let ctx = harness.state().current_app_context().clone();
        if ctx.wallet_backend().is_ok() {
            return ctx;
        }
        assert!(
            Instant::now() < deadline,
            "wallet backend was not wired within {WALLET_BACKEND_WIRE_TIMEOUT:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Mounts the full `AppState` on `root_screen` and steps the frame loop until
/// it settles. Skips the app's first-run welcome screen so the requested root
/// screen renders directly. Owns a private tokio runtime for the duration of
/// construction only — callers that need a runtime alive afterwards (e.g. to
/// seed the DB through `AppContext` methods that spawn tasks) must enter their
/// own around the call, same as any other kittest.
pub fn mount_app(root_screen: RootScreenType) -> Harness<'static, dash_evo_tool::app::AppState> {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder()
        .with_max_steps(100)
        .build_eframe(move |ctx| {
            let mut app = dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false);
            app.show_welcome_screen = false;
            app.welcome_screen = None;
            app.selected_main_screen = root_screen;
            app
        });
    harness.set_size(egui::vec2(1280.0, 800.0));
    wait_for_wallet_backend(&mut harness);
    harness.run_steps(10);
    harness
}

/// Builds a real `AppContext` from the default first-run database via the
/// same `AppState::new` factory `mount_app` uses, without mounting a
/// particular root screen. Returns the runtime so the caller keeps it alive
/// for the duration of the test.
pub fn fresh_app_context() -> (tokio::runtime::Runtime, Arc<AppContext>) {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let guard = rt.enter();
    let mut bootstrap = Harness::builder().with_max_steps(20).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });
    let app_context = wait_for_wallet_backend(&mut bootstrap);
    drop(bootstrap);
    drop(guard);
    (rt, app_context)
}
