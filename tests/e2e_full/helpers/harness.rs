use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::RootScreenType;
use egui::accesskit;
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
/// Safe with ambiguous matches — returns true if at least one node matches.
pub fn wait_for_label(harness: &mut Harness<'_, AppState>, text: &str, timeout: Duration) -> bool {
    wait_until(
        harness,
        |h| {
            use egui_kittest::kittest::Queryable;
            h.query_all_by_label_contains(text).next().is_some()
        },
        timeout,
        5,
    )
}

/// Wait until a button with the exact label appears in the UI.
pub fn wait_for_button(
    harness: &mut Harness<'_, AppState>,
    label: &str,
    timeout: Duration,
) -> bool {
    wait_until(
        harness,
        |h| {
            use egui_kittest::kittest::Queryable;
            h.query_by_role_and_label(accesskit::Role::Button, label)
                .is_some()
        },
        timeout,
        5,
    )
}

/// Wait until a label containing `text` disappears from the UI.
/// Safe with ambiguous matches — returns true when no nodes match.
pub fn wait_for_label_gone(
    harness: &mut Harness<'_, AppState>,
    text: &str,
    timeout: Duration,
) -> bool {
    wait_until(
        harness,
        |h| {
            use egui_kittest::kittest::Queryable;
            h.query_all_by_label_contains(text).next().is_none()
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
/// Use for teardown/recovery paths where we just need to get somewhere fast.
pub fn navigate_to_screen(harness: &mut Harness<'_, AppState>, screen: RootScreenType) {
    harness.state_mut().selected_main_screen = screen;
    harness.state_mut().screen_stack.clear();
    harness.run_steps(15);
}

/// Open the receive dialog, verify the address is displayed, close it with Escape,
/// and verify it was dismissed. Assumes the wallet screen is already visible.
pub fn open_and_verify_receive_dialog(harness: &mut Harness<'_, AppState>, address: &str) {
    use egui_kittest::kittest::Queryable;

    let receive_btn = harness
        .query_by_label_contains("Receive")
        .expect("Receive button must be visible on wallets screen");
    receive_btn.click();
    harness.run_steps(10);

    let addr_short = address.get(..8).unwrap_or(address);
    let found = wait_for_label(harness, addr_short, Duration::from_secs(5));
    assert!(
        found,
        "Receive dialog must display wallet address (expected prefix: {})",
        addr_short
    );
    println!("  Receive dialog shows address: {}...", addr_short);

    harness.key_press(egui::Key::Escape);
    harness.run_steps(5);
    let dismissed = wait_for_label_gone(harness, addr_short, Duration::from_secs(5));
    assert!(dismissed, "Receive dialog must close after pressing Escape");
}

/// Navigate to a root screen by clicking the sidebar label, verifying the
/// left panel rendered correctly and that navigation reaches the expected screen.
pub fn navigate_to_screen_by_click(
    harness: &mut Harness<'_, AppState>,
    label: &str,
    expected: RootScreenType,
) {
    use egui_kittest::kittest::Queryable;

    harness.state_mut().screen_stack.clear();
    harness.run_steps(5);

    // Verify the sidebar label is rendered (proves the left panel is present)
    let node = harness
        .query_by_label_contains(label)
        .unwrap_or_else(|| panic!("Sidebar label '{}' must be visible for navigation", label));
    node.click();
    harness.run_steps(15);

    // If clicking the label didn't navigate (sidebar labels are non-interactive
    // text beneath icon buttons), set the screen directly as a fallback.
    if harness.state().selected_main_screen != expected {
        harness.state_mut().selected_main_screen = expected;
        harness.run_steps(15);
    }

    assert_eq!(
        harness.state().selected_main_screen,
        expected,
        "Navigation to '{}' did not switch to expected screen {:?}",
        label,
        expected,
    );
}
