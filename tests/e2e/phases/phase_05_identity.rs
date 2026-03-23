use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::model::amount::Amount;
use dash_evo_tool::ui::identities::add_new_identity_screen::{AddNewIdentityScreen, FundingMethod};
use dash_evo_tool::ui::identities::funding_common::WalletFundedScreenStep;
use dash_evo_tool::ui::{MessageType, Screen, ScreenLike, ScreenType};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    // Run validation sub-tests first — these are pure client-side (no network
    // calls) and should run regardless of SPV sync state.
    run_validation_tests(harness);

    // SPV readiness gate — identity creation builds an asset lock transaction
    ensure_spv_tx_ready(harness, ctx);

    // ─── Actual identity creation disabled ──────────────────────────────
    // Identity creation requires an asset lock transaction to be confirmed
    // in a block for the proof. Without SPV mempool support, this depends
    // on testnet block timing which is too slow/unreliable for CI.
    // TODO: Re-enable when SPV mempool support lands.
    println!("  Identity creation: SKIPPED (needs SPV mempool support for asset lock proof)");
    println!("  Phase 05 complete: validation tests passed");
}

/// Client-side validation tests for identity creation.
/// Each sub-test pushes a fresh AddNewIdentityScreen, configures an invalid
/// state, verifies the app handles it correctly, then pops the screen.
/// No network calls — these run in < 1 second total.
fn run_validation_tests(harness: &mut Harness<'_, AppState>) {
    // ─── Sub-test A: Zero funding amount → action button not rendered ───
    // Note: The top panel breadcrumb always shows a "Create Identity" button,
    // so we count matches: 1 = breadcrumb only, 2+ = breadcrumb + action button.
    push_screen(harness, ScreenType::AddNewIdentity);
    with_identity_screen_mut(harness, |screen| {
        set_wallet_funded_ready(screen, "Zero Amount Test");
        screen.set_funding_amount(None);
    });
    harness.run_steps(POLL_STEPS);
    let count = harness.query_all_by_label("Create Identity").count();
    assert_eq!(
        count, 1,
        "Only breadcrumb should show 'Create Identity' when funding_amount is None (found {})",
        count
    );
    println!("  Validation: zero-amount action button correctly hidden");
    pop_screen(harness);

    // ─── Sub-test B: No master key → click silently rejected ────────────
    push_screen(harness, ScreenType::AddNewIdentity);
    with_identity_screen_mut(harness, |screen| {
        set_wallet_funded_ready(screen, "No Keys Test");
        screen.set_funding_amount(Some(Amount::new_dash(0.01)));
        // Deliberately skip ensure_correct_identity_keys()
    });
    harness.run_steps(POLL_STEPS);
    // Skip the breadcrumb (nth 0) and click the action button (nth 1).
    // With funding_amount set, the action button should be rendered.
    let has_action_button = harness.query_all_by_label("Create Identity").count() >= 2;
    assert!(
        has_action_button,
        "Action button must be present when funding_amount is set (found only breadcrumb)"
    );
    harness
        .query_all_by_label("Create Identity")
        .nth(1)
        .unwrap()
        .click();
    harness.run_steps(POLL_STEPS);
    assert_identity_step(harness, WalletFundedScreenStep::ReadyToCreate);
    println!("  Validation: no-key click correctly rejected (stayed on ReadyToCreate)");
    pop_screen(harness);

    // ─── Sub-test C: Error message display and dismiss ──────────────────
    push_screen(harness, ScreenType::AddNewIdentity);
    with_identity_screen_mut(harness, |screen| {
        screen.display_message("Simulated identity error", MessageType::Error);
    });
    harness.run_steps(POLL_STEPS);
    assert!(
        harness
            .query_by_label_contains("Error registering identity")
            .is_some(),
        "Error message must be visible after display_message(Error)"
    );
    dismiss_if_present(harness);
    harness.run_steps(SETTLE_STEPS);
    assert!(
        harness
            .query_by_label_contains("Error registering identity")
            .is_none(),
        "Error message must be gone after dismiss"
    );
    println!("  Validation: error message displayed and dismissed");
    pop_screen(harness);

    // ─── Sub-test D: Step resets to ReadyToCreate on error ──────────────
    push_screen(harness, ScreenType::AddNewIdentity);
    with_identity_screen_mut(harness, |screen| {
        *screen.funding_method().write().unwrap() = FundingMethod::UseWalletBalance;
        *screen.step().write().unwrap() = WalletFundedScreenStep::WaitingForAssetLock;
        screen.display_message("Asset lock failed", MessageType::Error);
    });
    harness.run_steps(POLL_STEPS);
    assert_identity_step(harness, WalletFundedScreenStep::ReadyToCreate);
    println!("  Validation: step reset to ReadyToCreate on error");
    pop_screen(harness);
}

// ─── Validation test helpers ─────────────────────────────────────────────────

/// Access the AddNewIdentityScreen at the top of the screen stack mutably.
/// Panics if the top screen is not an AddNewIdentityScreen.
fn with_identity_screen_mut(
    harness: &mut Harness<'_, AppState>,
    f: impl FnOnce(&mut AddNewIdentityScreen),
) {
    let stack = &mut harness.state_mut().screen_stack;
    match stack.last_mut() {
        Some(Screen::AddNewIdentityScreen(screen)) => f(screen),
        _ => panic!("Expected AddNewIdentityScreen on screen stack"),
    }
}

/// Assert the identity screen's current step matches the expected value.
fn assert_identity_step(harness: &mut Harness<'_, AppState>, expected: WalletFundedScreenStep) {
    let stack = &harness.state().screen_stack;
    match stack.last() {
        Some(Screen::AddNewIdentityScreen(screen)) => {
            let step_arc = screen.step();
            let step = step_arc.read().unwrap();
            assert_eq!(*step, expected, "Identity screen step mismatch");
        }
        _ => panic!("Expected AddNewIdentityScreen on screen stack"),
    }
}

/// Set common fields for a wallet-funded identity screen in ReadyToCreate state.
fn set_wallet_funded_ready(screen: &mut AddNewIdentityScreen, alias: &str) {
    *screen.funding_method().write().unwrap() = FundingMethod::UseWalletBalance;
    *screen.step().write().unwrap() = WalletFundedScreenStep::ReadyToCreate;
    screen.set_alias_input(alias.to_string());
}
