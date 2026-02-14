use crate::helpers::context::TestContext;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::{RootScreenType, ScreenLike, ScreenType};
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
/// Calls `refresh_on_arrival()` on the target screen so it picks up new
/// wallets, identities, etc. that were added after initial screen creation.
pub fn navigate_to_screen(harness: &mut Harness<'_, AppState>, screen: RootScreenType) {
    harness.state_mut().selected_main_screen = screen;
    harness.state_mut().screen_stack.clear();
    harness
        .state_mut()
        .active_root_screen_mut()
        .refresh_on_arrival();
    harness.run_steps(15);
}

/// Verify the Receive button is visible (proves wallet is selected on the
/// wallets screen). In kittest, opening modal dialogs and verifying their
/// content is unreliable because AccessKit interactions don't always propagate,
/// so we limit this to checking the button exists.
pub fn verify_receive_button_visible(harness: &mut Harness<'_, AppState>) {
    use egui_kittest::kittest::Queryable;
    // Use exact match to avoid "Total Received (DASH)"
    let found = harness.query_by_label("Receive").is_some();
    assert!(
        found,
        "Receive button must be visible on wallets screen (wallet selected)"
    );
    println!("  Receive button visible (wallet is selected)");
}

/// Verify the sidebar renders a label for the given screen, then navigate
/// directly. AccessKit cannot click sidebar labels (they're non-interactive
/// text beneath icon buttons), so we verify presence and navigate directly.
pub fn verify_sidebar_label_and_navigate(
    harness: &mut Harness<'_, AppState>,
    label: &str,
    target: RootScreenType,
) {
    use egui_kittest::kittest::Queryable;

    harness.state_mut().screen_stack.clear();
    harness.run_steps(5);

    // Verify the sidebar label is rendered (proves the left panel works)
    assert!(
        harness.query_by_label_contains(label).is_some(),
        "Sidebar label '{}' must be visible (left panel rendering broken?)",
        label
    );
    println!("  Sidebar label '{}' verified", label);

    navigate_to_screen(harness, target);
}

/// Push a screen onto the screen stack by type.
/// Creates the screen from the current AppContext and runs a few frames
/// to let the UI settle.
pub fn push_screen(harness: &mut Harness<'_, AppState>, screen_type: ScreenType) {
    let app_ctx = harness.state().current_app_context();
    let screen = screen_type.create_screen(app_ctx);
    harness.state_mut().screen_stack.push(screen);
    harness.run_steps(10);
}

/// Click the Nth TextInput (by AccessKit role), type text into it, and
/// run a few frames for the UI to process the input.
///
/// `nth` is zero-indexed: 0 = first TextInput, 1 = second, etc.
pub fn type_into_text_input(harness: &mut Harness<'_, AppState>, nth: usize, text: &str) {
    use egui_kittest::kittest::Queryable;

    harness
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .nth(nth)
        .unwrap_or_else(|| panic!("TextInput #{} must exist on screen", nth))
        .click();
    harness.run_steps(5);
    harness
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .nth(nth)
        .unwrap()
        .type_text(text);
    harness.run_steps(10);
}

/// Dismiss an error/info dialog if the "Dismiss" button is present.
pub fn dismiss_if_present(harness: &mut Harness<'_, AppState>) {
    use egui_kittest::kittest::Queryable;

    if let Some(dismiss) = harness.query_by_label_contains("Dismiss") {
        dismiss.click();
        harness.run_steps(5);
    }
}

/// Capture the text of a visible error label (format: "Error: <message>").
/// Returns None if no error label is visible.
pub fn capture_error_text(harness: &Harness<'_, AppState>) -> Option<String> {
    use egui_kittest::kittest::Queryable;
    harness
        .query_all_by_label_contains("Error:")
        .next()
        .map(|node| format!("{:?}", node))
}

/// Emergency cleanup after a panic — stop SPV and remove the test wallet.
/// Both operations are synchronous (CancellationToken + rusqlite/RwLock),
/// so they are safe to call in a panic handler.
pub fn emergency_cleanup(harness: &Harness<'_, AppState>, ctx: &TestContext) {
    harness.state().current_app_context().spv_manager.stop();
    eprintln!("  Emergency: SPV stop requested");

    if let Some(seed_hash) = &ctx.wallet_seed_hash {
        let app_ctx = harness.state().current_app_context();
        match app_ctx.remove_wallet(seed_hash) {
            Ok(()) => eprintln!("  Emergency: wallet removed"),
            Err(e) => eprintln!("  Emergency: wallet removal failed: {}", e),
        }
    }
}
