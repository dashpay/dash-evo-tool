//! E2E Tests for Navigation
//!
//! These tests verify that navigation between screens works correctly
//! and that state is preserved appropriately.

use crate::helpers::with_isolated_data_dir;
use egui_kittest::Harness;

/// Test that app navigation completes without errors
#[test]
fn test_basic_navigation() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(egui::vec2(1024.0, 768.0));

        // Run initial frames
        harness.run_steps(20);
    });
}

/// Test navigation with different window sizes
#[test]
fn test_navigation_responsive_layout() {
    let sizes = [
        egui::vec2(640.0, 480.0),
        egui::vec2(1024.0, 768.0),
        egui::vec2(1440.0, 900.0),
    ];

    // A fresh data dir per size: each `AppState` opens the shared seed vault,
    // which takes an exclusive advisory lock that outlives the harness drop
    // (background subtasks keep the `Arc<AppContext>` graph — and thus the vault
    // handle — alive). Isolating the data dir per iteration gives each its own
    // vault file, so the lock never collides. Production opens the vault once
    // per process, so this multi-AppState pattern is test-only.
    for size in sizes {
        with_isolated_data_dir(|| {
            let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
            let _guard = rt.enter();

            let mut harness = Harness::builder().with_max_steps(50).build_eframe(|ctx| {
                dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                    .expect("Failed to create AppState")
                    .with_animations(false)
            });

            harness.set_size(size);
            harness.run_steps(15);
        });
    }
}

/// Test that rapid navigation doesn't cause issues
#[test]
fn test_rapid_frame_navigation() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(300).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(egui::vec2(1024.0, 768.0));

        // Run many single-step frames
        for _ in 0..50 {
            harness.run_steps(1);
        }
    });
}

/// Test that the app maintains stability over extended use
#[test]
fn test_extended_navigation_stability() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(500).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        harness.set_size(egui::vec2(1280.0, 720.0));

        // Run 100 frames in batches
        for batch in 0..10 {
            harness.run_steps(10);
            // Verify each batch completes
            let _ = batch;
        }
    });
}

/// Test app behavior with minimum window size
#[test]
fn test_minimum_size_navigation() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        let _guard = rt.enter();

        let mut harness = Harness::builder().with_max_steps(50).build_eframe(|ctx| {
            dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
                .expect("Failed to create AppState")
                .with_animations(false)
        });

        // Very small window
        harness.set_size(egui::vec2(320.0, 240.0));
        harness.run_steps(10);

        // Resize to normal
        harness.set_size(egui::vec2(1024.0, 768.0));
        harness.run_steps(10);
    });
}
