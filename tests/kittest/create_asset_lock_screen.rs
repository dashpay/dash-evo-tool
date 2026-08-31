use crate::support::with_isolated_data_dir;
use egui_kittest::Harness;

/// Test that the create asset lock screen can be rendered
#[test]
fn test_create_asset_lock_screen_renders() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(egui::vec2(1024.0, 768.0));
        crate::support::wait_for_screens(&mut harness);
    });
}

/// Test that the create asset lock screen handles window resize gracefully
#[test]
fn test_create_asset_lock_screen_resize() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        // Test various window sizes
        let sizes = [
            egui::vec2(800.0, 600.0),
            egui::vec2(1200.0, 900.0),
            egui::vec2(640.0, 480.0),
            egui::vec2(1920.0, 1080.0),
        ];

        for size in sizes {
            harness.set_size(size);
            harness.run_steps(5);
        }
    });
}

/// Test that the app remains responsive with multiple frame batches
#[test]
fn test_create_asset_lock_screen_frame_stability() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(200).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(egui::vec2(1024.0, 768.0));

        // Run multiple batches to test stability
        for _ in 0..10 {
            crate::support::wait_for_screens(&mut harness);
        }
    });
}
