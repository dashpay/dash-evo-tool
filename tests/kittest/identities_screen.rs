use egui_kittest::Harness;

/// Test that the identities screen can be rendered
#[test]
fn test_identities_screen_renders() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone()).with_animations(false)
    });

    harness.set_size(egui::vec2(1024.0, 768.0));
    harness.run_steps(10);
}

/// Test that the app renders correctly at minimum size
#[test]
fn test_minimum_window_size() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(50).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone()).with_animations(false)
    });

    // Test with a small window size
    harness.set_size(egui::vec2(400.0, 300.0));
    harness.run_steps(10);
}

/// Test that the app handles resize gracefully
#[test]
fn test_window_resize() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone()).with_animations(false)
    });

    // Start small
    harness.set_size(egui::vec2(640.0, 480.0));
    harness.run_steps(5);

    // Resize larger
    harness.set_size(egui::vec2(1280.0, 720.0));
    harness.run_steps(5);

    // Resize smaller again
    harness.set_size(egui::vec2(800.0, 600.0));
    harness.run_steps(5);
}

/// Test multiple frame batches
#[test]
fn test_frame_batch_processing() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(150).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone()).with_animations(false)
    });

    harness.set_size(egui::vec2(1024.0, 768.0));

    // Process frames in batches
    for batch in 0..10 {
        harness.run_steps(10);
        // Just ensure we can run multiple batches without error
        let _ = batch;
    }
}
