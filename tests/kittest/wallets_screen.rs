use egui_kittest::Harness;

/// Test that the wallets screen can be rendered
#[test]
fn test_wallets_screen_renders() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });

    harness.set_size(egui::vec2(1024.0, 768.0));
    harness.run_steps(10);
}

/// Test that the app can run many frames without issues
#[test]
fn test_app_stability_over_many_frames() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(200).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });

    harness.set_size(egui::vec2(1024.0, 768.0));

    // Run 50 frames to test stability
    harness.run_steps(50);
}

/// Test rapid frame stepping
#[test]
fn test_rapid_frame_stepping() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });

    harness.set_size(egui::vec2(800.0, 600.0));

    // Run single steps rapidly
    for _ in 0..20 {
        harness.run_steps(1);
    }
}
