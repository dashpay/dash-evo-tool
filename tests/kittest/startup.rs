use egui_kittest::Harness;

/// Test that demonstrates basic app startup and shutdown with kittest
#[test]
fn test_app_startup() {
    // Create a tokio runtime for async operations during app initialization
    // The app uses tokio::spawn internally for background tasks
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    // Create a test harness for the egui app
    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone()).with_animations(false)
    });

    // Set the window size
    harness.set_size(egui::vec2(800.0, 600.0));

    // Run a few frames to ensure the app initializes
    // Using run_steps instead of run() because the app may show spinners
    // which cause continuous repainting
    harness.run_steps(10);
}
