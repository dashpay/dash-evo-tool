use crate::support::{mount_app, with_isolated_data_dir};
use dash_evo_tool::model::user_role::UserRole;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// Test that the network chooser screen renders without panicking
#[test]
fn test_network_chooser_renders() {
    with_isolated_data_dir(|| {
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
    });
}

/// The Settings interface-mode selector sets the app-global role and persists
/// it to AppSettings — the single source of truth the runtime role and the
/// gates read from.
#[test]
fn interface_mode_selector_sets_and_persists_role() {
    with_isolated_data_dir(|| {
        let mut harness = mount_app(RootScreenType::RootScreenNetworkChooser);
        let app_context = harness.state().current_app_context().clone();

        // An account that never chose a role starts at the unset default.
        assert_eq!(app_context.user_role(), UserRole::WHEN_UNSET);

        harness.get_by_label("Developer tools").click();
        harness.run_steps(3);

        assert_eq!(
            app_context.user_role(),
            UserRole::Developer,
            "selecting 'Developer tools' must raise the app-global role"
        );
        assert_eq!(
            app_context.get_app_settings().user_role,
            Some(UserRole::Developer),
            "the selected role must be persisted to AppSettings"
        );
    });
}

/// Test that the app can handle screen navigation
#[test]
fn test_app_handles_frame_stepping() {
    with_isolated_data_dir(|| {
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
    });
}

/// Test that the app renders at different window sizes
#[test]
fn test_app_renders_at_various_sizes() {
    let sizes = [
        egui::vec2(640.0, 480.0),   // Small
        egui::vec2(1024.0, 768.0),  // Medium
        egui::vec2(1920.0, 1080.0), // Large
    ];

    // A fresh data dir per size: each `AppState` opens the shared seed vault,
    // whose exclusive advisory lock outlives the harness drop (background
    // subtasks keep the `Arc<AppContext>` graph alive). Per-iteration isolation
    // gives each its own vault file so the lock never collides. Production opens
    // the vault once per process, so this multi-AppState pattern is test-only.
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
            harness.run_steps(5);
        });
    }
}
