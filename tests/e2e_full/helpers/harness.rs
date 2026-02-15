use crate::helpers::context::TestContext;
use dash_evo_tool::app::AppState;
use dash_evo_tool::spv::SpvStatus;
use dash_evo_tool::ui::{RootScreenType, ScreenLike, ScreenType};
use egui_kittest::Harness;
use std::time::{Duration, Instant};

// ─── Centralized constants ───────────────────────────────────────────────────

/// SPV sync: max wait for headers + balance. Configurable via E2E_SPV_TIMEOUT_SECS.
pub const SPV_SYNC_TIMEOUT_SECS: u64 = 600;
/// Minimum wallet balance (duffs) for SPV sync to be considered successful.
pub const MIN_BALANCE_DUFFS: u64 = 100_000;
/// Send-to-self transaction broadcast + confirmation.
pub const SEND_TX_TIMEOUT: Duration = Duration::from_secs(180);
/// Post-send SPV reconciliation (balance/UTXO update).
pub const POST_SEND_RECONCILE_TIMEOUT: Duration = Duration::from_secs(120);
/// Platform read operations (DPNS lookup, contract fetch).
pub const PLATFORM_READ_TIMEOUT: Duration = Duration::from_secs(120);
/// Contract fetch (simpler than full platform reads).
pub const CONTRACT_FETCH_TIMEOUT: Duration = Duration::from_secs(90);
/// Token search.
pub const TOKEN_SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
/// Identity creation (asset lock + platform broadcast + confirmation).
pub const IDENTITY_CREATION_TIMEOUT: Duration = Duration::from_secs(300);
/// DPNS name registration.
pub const DPNS_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(180);
/// SPV stop timeout during teardown.
pub const SPV_STOP_TIMEOUT: Duration = Duration::from_secs(30);
/// Default retry count for transient platform operations.
pub const PLATFORM_MAX_RETRIES: u32 = 3;
/// Frames per poll cycle in wait loops (~0.5s at 60fps).
pub const POLL_STEPS: usize = 30;
/// Frames to run after navigation/screen push for UI settle.
pub const SETTLE_STEPS: usize = 10;
/// Minimum balance (duffs) required before send-to-self (0.1 DASH).
pub const MIN_BALANCE_FOR_SEND: u64 = 10_000_000;
/// Maximum acceptable fee for send-to-self (0.01 DASH).
pub const MAX_SEND_FEE: u64 = 1_000_000;

// ─── Error classification ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Network,
    Validation,
    TransientPlatform,
    Fatal,
}

impl ErrorCategory {
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::Network | Self::TransientPlatform)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Network => "NETWORK",
            Self::Validation => "VALIDATION",
            Self::TransientPlatform => "TRANSIENT",
            Self::Fatal => "FATAL",
        }
    }
}

const ERROR_PATTERNS: &[(ErrorCategory, &[&str])] = &[
    (
        ErrorCategory::Network,
        &[
            "timeout",
            "connection",
            "network",
            "unavailable",
            "timed out",
            "refused",
            "unreachable",
        ],
    ),
    (
        ErrorCategory::Validation,
        &[
            "invalid",
            "insufficient",
            "already exists",
            "not found",
            "duplicate",
            "too low",
            "too high",
        ],
    ),
    (
        ErrorCategory::TransientPlatform,
        &[
            "consensus",
            "retry",
            "temporarily",
            "try again",
            "rate limit",
        ],
    ),
];

pub fn classify_error(error_text: &str) -> ErrorCategory {
    let lower = error_text.to_lowercase();
    for (category, patterns) in ERROR_PATTERNS {
        if patterns.iter().any(|p| lower.contains(p)) {
            return *category;
        }
    }
    ErrorCategory::Fatal
}

// ─── Harness creation ────────────────────────────────────────────────────────

/// Create a test harness configured for E2E testing.
pub fn create_e2e_harness(rt: &tokio::runtime::Runtime) -> Harness<'static, AppState> {
    let _guard = rt.enter();
    let mut harness = Harness::builder()
        .with_max_steps(10000)
        .build_eframe(|ctx| AppState::new(ctx.egui_ctx.clone()).with_animations(false));
    harness.set_size(egui::vec2(1280.0, 800.0));
    harness
}

// ─── Wait helpers ────────────────────────────────────────────────────────────

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

// ─── Navigation helpers ──────────────────────────────────────────────────────

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
    harness.run_steps(SETTLE_STEPS);
}

// ─── Input helpers ───────────────────────────────────────────────────────────

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
    harness.run_steps(SETTLE_STEPS);
}

// ─── Error / dismiss helpers ─────────────────────────────────────────────────

/// Dismiss an error/info dialog if the "Dismiss" button is present.
pub fn dismiss_if_present(harness: &mut Harness<'_, AppState>) {
    use egui_kittest::kittest::Queryable;

    if let Some(dismiss) = harness.query_by_label_contains("Dismiss") {
        dismiss.click();
        harness.run_steps(5);
    }
}

/// Capture the text of a visible error label.
/// Searches multiple common error patterns and extracts the label name
/// from the AccessKit node debug output.
/// Returns None if no error label is visible.
pub fn capture_error_text(harness: &Harness<'_, AppState>) -> Option<String> {
    use egui_kittest::kittest::Queryable;
    const PATTERNS: &[&str] = &["Error:", "Error registering", "Error "];
    const NAME_PREFIX: &str = "name: \"";

    for pattern in PATTERNS {
        if let Some(node) = harness.query_all_by_label_contains(pattern).next() {
            let debug = format!("{:?}", node);
            // Extract the name field from the AccessKit Debug output
            if let Some(name_start) = debug.find(NAME_PREFIX) {
                let value_start = name_start + NAME_PREFIX.len();
                if let Some(end) = debug[value_start..].find('"') {
                    return Some(debug[value_start..value_start + end].to_string());
                }
            }
            return Some(debug.chars().take(200).collect());
        }
    }
    None
}

/// Handle a retryable error during a platform operation.
///
/// Captures the error text, classifies it, logs it, and either panics (for
/// non-retryable errors or final attempt) or dismisses the dialog and prepares
/// for the next attempt. If `pop_screen` is true, pops the top screen off the
/// stack before backoff.
///
/// Panics if the error is non-retryable or this was the last attempt.
/// Returns normally when the caller should `continue` the retry loop.
pub fn handle_retry_error(
    harness: &mut Harness<'_, AppState>,
    operation: &str,
    attempt: u32,
    pop_screen: bool,
) {
    let error_text = capture_error_text(harness).unwrap_or_else(|| "unknown error".to_string());
    let category = classify_error(&error_text);
    println!(
        "  {} error (attempt {}/{}): [{}] {}",
        operation,
        attempt,
        PLATFORM_MAX_RETRIES,
        category.label(),
        error_text
    );

    if !category.is_retryable() {
        panic!(
            "{} failed with non-retryable error: {}",
            operation, error_text
        );
    }

    dismiss_if_present(harness);
    if pop_screen {
        harness.state_mut().screen_stack.pop();
    }
    harness.run_steps(SETTLE_STEPS * attempt as usize);

    if attempt == PLATFORM_MAX_RETRIES {
        panic!(
            "{} failed after {} retries. Last error: {}",
            operation, PLATFORM_MAX_RETRIES, error_text
        );
    }
}

// ─── SPV readiness gate ──────────────────────────────────────────────────────

/// Re-verify SPV state before phases that build core transactions.
/// Asserts that SPV is syncing/running, headers are available, and
/// the wallet still has the minimum balance from Phase 0.
pub fn ensure_spv_tx_ready(harness: &mut Harness<'_, AppState>, ctx: &TestContext) {
    let app_ctx = harness.state().current_app_context();
    let status = app_ctx.spv_manager.status();
    let header_height = status
        .sync_progress
        .as_ref()
        .map(|p| p.header_height)
        .unwrap_or(0);

    assert!(
        matches!(status.status, SpvStatus::Syncing | SpvStatus::Running),
        "SPV must be Syncing or Running for transactions, got {:?}. Last error: {}",
        status.status,
        status.last_error.as_deref().unwrap_or("none")
    );
    assert!(
        header_height > 0,
        "SPV header height must be > 0 for transaction building (got 0)"
    );

    let wallets = app_ctx.wallets.read().unwrap();
    let wallet = wallets
        .get(ctx.seed_hash())
        .expect("Test wallet must exist in AppContext during tx phases");
    let w = wallet.read().unwrap();
    assert!(
        w.max_balance() >= MIN_BALANCE_DUFFS,
        "Wallet balance ({} duffs) below minimum since Phase 0",
        w.max_balance()
    );

    println!(
        "  SPV tx-ready: {:?}, header_height={}, peers={}",
        status.status, header_height, status.connected_peers
    );
}

// ─── Emergency cleanup ───────────────────────────────────────────────────────

/// Emergency cleanup after a panic — stop SPV, remove test identity and wallet.
/// All operations are synchronous (CancellationToken + rusqlite/RwLock),
/// so they are safe to call in a panic handler.
/// Identity is removed before wallet (identity references may depend on wallet state).
pub fn emergency_cleanup(harness: &Harness<'_, AppState>, ctx: &TestContext) {
    let app_ctx = harness.state().current_app_context();
    app_ctx.spv_manager.stop();
    eprintln!("  Emergency: SPV stop requested");

    if let Some(identity_id) = &ctx.identity_id {
        match app_ctx
            .db
            .delete_local_qualified_identity(identity_id, app_ctx)
        {
            Ok(()) => eprintln!("  Emergency: identity removed"),
            Err(e) => eprintln!("  Emergency: identity removal failed: {}", e),
        }
    }

    if let Some(seed_hash) = &ctx.wallet_seed_hash {
        match app_ctx.remove_wallet(seed_hash) {
            Ok(()) => eprintln!("  Emergency: wallet removed"),
            Err(e) => eprintln!("  Emergency: wallet removal failed: {}", e),
        }
    }
}
