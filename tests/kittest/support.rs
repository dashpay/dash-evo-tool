//! Shared kittest helpers.

#[path = "../common/data_dir.rs"]
mod data_dir;

use dash_evo_tool::app::BootPhase;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use data_dir::with_isolated_data_dir;

/// How long storage preparation may go **without visible progress** before a
/// mount helper gives up.
///
/// A stall budget, not a total: it restarts whenever the boot phase or the
/// published migration step changes. A total budget cannot work here — the
/// suite runs dozens of tests in parallel, each with its own multi-worker tokio
/// runtime, so a preparation that takes 0.75 s alone takes far longer when it is
/// starved, and any fixed total is a coin flip rather than a safety margin
/// (30 s lost that flip on a loaded machine; 60 s would only move it).
///
/// The size is set by the worst *legitimate* single step, which is wiring when
/// it hydrates a password-protected wallet: that runs a deliberately
/// memory-hard Argon2id KDF, publishes nothing while it does, and was measured
/// still going after 90 s on a box running the suite plus twelve CPU hogs. This
/// budget is therefore not a performance assertion — it exists so a gate that
/// never lifts fails in minutes instead of hanging CI, and exceeding it means
/// something is genuinely wedged. Do not "optimise" it down to make a slow
/// machine look fast; the number is deliberately far above any real duration.
const STORAGE_PREP_STALL_TIMEOUT: Duration = Duration::from_secs(300);

/// Step `harness` until root screens exist, then let them settle.
///
/// A test that mounts the real `AppState` and asserts on a screen must wait for
/// the storage-preparation gate to lift: nothing below it is rendered, and the
/// preparation behind it does file IO, so a fixed frame budget is a race that
/// only shows up as a flake on a loaded machine.
pub fn wait_for_screens(harness: &mut Harness<'static, dash_evo_tool::app::AppState>) {
    wait_for_wallet_backend(harness);
    harness.run_steps(3);
}

/// Step `harness` until the storage-preparation gate has lifted, then return the
/// live `AppContext`. Panics once preparation has sat on one step for
/// [`STORAGE_PREP_STALL_TIMEOUT`].
///
/// `AppState::new` spawns preparation as a background tokio task and the frame
/// loop polls it, so a fixed `run_steps(N)` gives no guarantee it has completed.
/// Gate on [`BootPhase::Ready`] rather than on `wallet_backend().is_ok()`: root
/// screens do not exist before that point, and tests that seed the DB via
/// `insert_local_qualified_identity` (which reaches through the backend's k/v
/// store) need the whole sequence done, not just its first step.
///
/// # Panics
///
/// Names the phase and published step it gave up on, so a future failure says
/// which part of the sequence stopped rather than only that time ran out.
pub fn wait_for_wallet_backend(
    harness: &mut Harness<'static, dash_evo_tool::app::AppState>,
) -> Arc<AppContext> {
    let started = Instant::now();
    let mut progress = boot_progress(harness);
    let mut last_change = Instant::now();
    loop {
        harness.step();
        if harness.state().boot_phase() == BootPhase::Ready {
            return harness.state().current_app_context().clone();
        }
        let current = boot_progress(harness);
        if current != progress {
            progress = current;
            last_change = Instant::now();
        }
        assert!(
            last_change.elapsed() < STORAGE_PREP_STALL_TIMEOUT,
            "storage preparation stopped advancing for {STORAGE_PREP_STALL_TIMEOUT:?} \
             (total wait {:?}); it is stuck on phase {:?}, published state {}",
            started.elapsed(),
            harness.state().boot_phase(),
            progress.1,
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The pair a stalled preparation is detected by: the boot phase and the
/// migration state it publishes. Debug-formatted rather than compared as values
/// so a new [`MigrationState`](dash_evo_tool::context::migration_status::MigrationState)
/// variant counts as progress without needing anything derived on it.
fn boot_progress(harness: &Harness<'static, dash_evo_tool::app::AppState>) -> (BootPhase, String) {
    let state = harness.state();
    (
        state.boot_phase(),
        format!(
            "{:?}",
            state.current_app_context().migration_status().state()
        ),
    )
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
