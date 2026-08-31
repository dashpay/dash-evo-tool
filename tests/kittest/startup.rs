use crate::support::with_isolated_data_dir;
use egui_kittest::Harness;

/// Test that demonstrates basic app startup and shutdown with kittest
#[test]
fn test_app_startup() {
    with_isolated_data_dir(|| {
        // Create a tokio runtime for async operations during app initialization
        // The app uses tokio::spawn internally for background tasks
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        // Create a test harness for the egui app
        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        // Set the window size
        harness.set_size(egui::vec2(800.0, 600.0));

        // Step until the storage-preparation gate lifts rather than for a fixed
        // count: `run_steps(10)` can end mid-gate, leaving the app in a phase
        // where no root screen has been built yet.
        crate::support::wait_for_wallet_backend(&mut harness);
        harness.run_steps(10);
    });
}
