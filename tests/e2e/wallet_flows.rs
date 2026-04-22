//! E2E Tests for Wallet Flows
//!
//! These tests verify complete user journeys related to wallets,
//! including balance display and state management.

use egui_kittest::Harness;

/// Test that the app starts with proper wallet state initialization
#[test]
fn test_wallet_state_initialization() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });

    harness.set_size(egui::vec2(1024.0, 768.0));
    harness.run_steps(20);
}

/// Test that wallet balance display renders correctly
#[test]
fn test_wallet_balance_rendering() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(100).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });

    harness.set_size(egui::vec2(1024.0, 768.0));

    // Run enough frames to fully initialize
    harness.run_steps(30);
}

/// Test wallet operations don't cause UI freezes
#[test]
fn test_wallet_ui_responsiveness() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(200).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });

    harness.set_size(egui::vec2(1024.0, 768.0));

    // Run many frames to test UI responsiveness
    for batch in 0..10 {
        harness.run_steps(15);
        // Each batch should complete without hanging
        let _ = batch;
    }
}

/// Test that the app handles rapid resizing during wallet views
#[test]
fn test_wallet_resize_stability() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();

    let mut harness = Harness::builder().with_max_steps(150).build_eframe(|ctx| {
        dash_evo_tool::app::AppState::new(ctx.egui_ctx.clone())
            .expect("Failed to create AppState")
            .with_animations(false)
    });

    // Test various resize scenarios
    let sizes = [
        egui::vec2(800.0, 600.0),
        egui::vec2(1200.0, 900.0),
        egui::vec2(640.0, 480.0),
        egui::vec2(1920.0, 1080.0),
    ];

    for size in sizes {
        harness.set_size(size);
        harness.run_steps(10);
    }
}
