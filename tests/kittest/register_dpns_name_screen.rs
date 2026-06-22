//! Kittest coverage for the Bucket A overlay adoption on the DPNS registration
//! screen (`RegisterDpnsNameScreen`).
//!
//! Proves the canonical contract the remaining transaction screens will copy:
//! dispatching the registration raises the global blocking overlay, and every
//! terminal result — success (`display_task_result`) or error
//! (`display_message`) — tears it down (the SEC-001 no-hard-lock guarantee).
//!
//! The screen needs an `Arc<AppContext>`, so each test borrows one from a
//! throwaway `AppState` built in an isolated data dir. The overlay raise/teardown
//! runs against an independent `egui::Context` the AppState never renders, so the
//! app's own SPV block can't perturb the `has_global` assertions.

use crate::support::with_isolated_data_dir;
use dash_evo_tool::app::AppState;
use dash_evo_tool::backend_task::{BackendTaskSuccessResult, FeeResult};
use dash_evo_tool::ui::MessageType;
use dash_evo_tool::ui::ScreenLike;
use dash_evo_tool::ui::components::ProgressOverlay;
use dash_evo_tool::ui::identities::register_dpns_name_screen::{
    RegisterDpnsNameScreen, RegisterDpnsNameSource,
};

/// Build a `RegisterDpnsNameScreen` over a fresh, isolated `AppContext`.
fn screen_with_context() -> RegisterDpnsNameScreen {
    let app_state = AppState::new(egui::Context::default()).expect("AppState builds");
    let app_context = app_state.current_app_context().clone();
    RegisterDpnsNameScreen::new(&app_context, RegisterDpnsNameSource::Dpns)
}

/// Dispatching the registration raises the global blocking overlay.
#[test]
fn dpns_dispatch_raises_blocking_overlay() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut screen = screen_with_context();
        let ctx = egui::Context::default();

        assert!(!ProgressOverlay::has_global(&ctx));
        screen.raise_progress_overlay_for_test(&ctx);
        assert!(
            ProgressOverlay::has_global(&ctx),
            "dispatching the registration must raise the blocking overlay"
        );
    });
}

/// A successful registration result tears the overlay down (success terminal path).
#[test]
fn dpns_success_result_clears_overlay() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut screen = screen_with_context();
        let ctx = egui::Context::default();

        screen.raise_progress_overlay_for_test(&ctx);
        assert!(ProgressOverlay::has_global(&ctx));

        screen.display_task_result(BackendTaskSuccessResult::RegisteredDpnsName(
            FeeResult::new(0, 0),
        ));
        assert!(
            !ProgressOverlay::has_global(&ctx),
            "a successful result must tear down the blocking overlay"
        );
    });
}

/// An error message tears the overlay down (error terminal path) — SEC-001: a
/// failed registration can never leave the window hard-locked.
#[test]
fn dpns_error_message_clears_overlay() {
    with_isolated_data_dir(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let _guard = rt.enter();

        let mut screen = screen_with_context();
        let ctx = egui::Context::default();

        screen.raise_progress_overlay_for_test(&ctx);
        assert!(ProgressOverlay::has_global(&ctx));

        screen.display_message("Registration failed. Try again.", MessageType::Error);
        assert!(
            !ProgressOverlay::has_global(&ctx),
            "an error result must tear down the blocking overlay"
        );
    });
}
