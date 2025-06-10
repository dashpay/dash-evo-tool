use egui_kittest::Harness;

/// Test that demonstrates basic app startup and shutdown with kittest
#[test]
fn test_app_startup() {
    // Create a test harness for the egui app
    //
    let mut harness = Harness::builder()
        .with_max_steps(100)
        .build_eframe(|ctx| dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone()));

    // Set the window size
    harness.set_size(egui::vec2(800.0, 600.0));

    // Run one frame to ensure the app initializes
    harness.run();
}
