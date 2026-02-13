use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use std::time::{Duration, Instant};

/// Create a test harness configured for E2E testing.
pub fn create_e2e_harness(rt: &tokio::runtime::Runtime) -> Harness<'static, AppState> {
    let _guard = rt.enter();
    let mut harness = Harness::builder()
        .with_max_steps(10000)
        .build_eframe(|ctx| AppState::new(ctx.egui_ctx.clone()).with_animations(false));
    harness.set_size(egui::vec2(1280.0, 800.0));
    harness
}

/// Poll harness until predicate returns true, or timeout.
/// Replacement for WebdriverIO's browser.waitUntil().
/// Runs `steps_per_check` frames between each predicate evaluation.
pub fn wait_until<F>(
    harness: &mut Harness<'_, AppState>,
    predicate: F,
    timeout: Duration,
    steps_per_check: usize,
) -> bool
where
    F: Fn(&Harness<'_, AppState>) -> bool,
{
    let start = Instant::now();
    while start.elapsed() < timeout {
        harness.run_steps(steps_per_check);
        if predicate(harness) {
            return true;
        }
    }
    false
}

/// Wait until a label containing `text` appears in the UI.
pub fn wait_for_label(harness: &mut Harness<'_, AppState>, text: &str, timeout: Duration) -> bool {
    wait_until(
        harness,
        |h| {
            use egui_kittest::kittest::Queryable;
            h.query_by_label_contains(text).is_some()
        },
        timeout,
        5,
    )
}

/// Wait until a label containing `text` disappears from the UI.
pub fn wait_for_label_gone(
    harness: &mut Harness<'_, AppState>,
    text: &str,
    timeout: Duration,
) -> bool {
    wait_until(
        harness,
        |h| {
            use egui_kittest::kittest::Queryable;
            h.query_by_label_contains(text).is_none()
        },
        timeout,
        5,
    )
}

/// Dismiss the welcome screen so tests start from the main app.
pub fn dismiss_welcome_screen(harness: &mut Harness<'_, AppState>) {
    harness.state_mut().show_welcome_screen = false;
    harness.state_mut().welcome_screen = None;
}

/// Navigate to a root screen by setting the selected screen directly.
pub fn navigate_to_screen(harness: &mut Harness<'_, AppState>, screen: RootScreenType) {
    harness.state_mut().selected_main_screen = screen;
    harness.state_mut().screen_stack.clear();
    harness.run_steps(15);
}
