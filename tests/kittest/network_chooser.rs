use egui_kittest::Harness;

/// Test that the network chooser screen renders without panicking
#[test]
fn test_network_chooser_renders() {
    // Create a tokio runtime for async operations during app initialization
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    // Create a test harness for the egui app
    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });

    // Set the window size
    harness.set_size(egui::vec2(1024.0, 768.0));

    // Run a few frames to ensure the app initializes
    harness.run_steps(10);
}

/// Test that the app can handle screen navigation
#[test]
fn test_app_handles_frame_stepping() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(50).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });

    harness.set_size(egui::vec2(800.0, 600.0));

    // Run multiple batches of frames
    for _ in 0..5 {
        harness.run_steps(5);
    }
}

/// Test that the app renders at different window sizes
#[test]
fn test_app_renders_at_various_sizes() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let sizes = [
        egui::vec2(640.0, 480.0),   // Small
        egui::vec2(1024.0, 768.0),  // Medium
        egui::vec2(1920.0, 1080.0), // Large
    ];

    for size in sizes {
        let mut harness = Harness::builder().with_max_steps(50).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(size);
        harness.run_steps(5);
    }
}
