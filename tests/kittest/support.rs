//! Shared kittest helpers.

#[path = "../common/data_dir.rs"]
mod data_dir;

use dash_evo_tool::context::AppContext;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use std::sync::Arc;

pub use data_dir::with_isolated_data_dir;

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
    bootstrap.run_steps(5);
    let app_context = bootstrap.state().current_app_context().clone();
    drop(bootstrap);
    drop(guard);
    (rt, app_context)
}
