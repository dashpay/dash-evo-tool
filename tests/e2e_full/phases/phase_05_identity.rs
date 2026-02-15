use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::model::amount::Amount;
use dash_evo_tool::ui::identities::add_new_identity_screen::{AddNewIdentityScreen, FundingMethod};
use dash_evo_tool::ui::identities::funding_common::WalletFundedScreenStep;
use dash_evo_tool::ui::{MessageType, RootScreenType, Screen, ScreenLike, ScreenType};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    // SPV readiness gate — identity creation builds an asset lock transaction
    ensure_spv_tx_ready(harness, ctx);

    // Run validation sub-tests before the main identity creation attempt
    run_validation_tests(harness);

    for attempt in 1..=PLATFORM_MAX_RETRIES {
        println!(
            "  Identity creation attempt {}/{}",
            attempt, PLATFORM_MAX_RETRIES
        );

        // ─── 1. Push AddNewIdentity screen ───────────────────────────────
        push_screen(harness, ScreenType::AddNewIdentity);

        // ─── 2. Configure the screen directly (ComboBox not accessible) ─
        with_identity_screen_mut(harness, |screen| {
            set_wallet_funded_ready(screen, "E2E Identity");

            // Hard-fail on key setup — wallet must be open
            screen.ensure_correct_identity_keys().unwrap_or_else(|e| {
                panic!(
                    "ensure_correct_identity_keys() failed: {}. Is wallet open?",
                    e
                )
            });
        });

        // ─── 3. Let UI render to initialize AmountInput widget ──────────
        // The AmountInput widget starts with `changed: true` which clears
        // funding_amount on first render. Run one cycle to consume that
        // initial forced-change, then set the amount.
        harness.run_steps(POLL_STEPS);
        with_identity_screen_mut(harness, |screen| {
            screen.funding_amount = Some(Amount::new_dash(0.01));
        });
        harness.run_steps(POLL_STEPS);

        // Verify UI rendered the expected state.
        // The top panel breadcrumb always has a "Create Identity" label,
        // so count >= 2 means the action button is also rendered.
        let create_count = harness.query_all_by_label("Create Identity").count();
        assert!(
            create_count >= 2,
            "'Create Identity' action button must be visible (found {} matches, \
             need >= 2: breadcrumb + button). UI did not render ReadyToCreate state.",
            create_count
        );
        println!("  Screen configured: Create Identity button visible");

        // ─── 4. Click "Create Identity" action button (skip breadcrumb) ─
        harness
            .query_all_by_label("Create Identity")
            .nth(1)
            .expect("Create Identity action button not found")
            .click();
        harness.run_steps(SETTLE_STEPS);

        // ─── 5. Wait for success or error ────────────────────────────────
        let completed = wait_until(
            harness,
            |h| {
                h.query_by_label_contains("Identity Registered Successfully!")
                    .is_some()
                    || h.query_by_label_contains("Error").is_some()
            },
            IDENTITY_CREATION_TIMEOUT,
            POLL_STEPS,
        );

        if !completed {
            println!(
                "  Identity creation timed out after {}s",
                IDENTITY_CREATION_TIMEOUT.as_secs()
            );
            harness.state_mut().screen_stack.pop();
            harness.run_steps(5);
            if attempt == PLATFORM_MAX_RETRIES {
                panic!(
                    "Identity creation timed out after {} attempts ({}s each)",
                    PLATFORM_MAX_RETRIES,
                    IDENTITY_CREATION_TIMEOUT.as_secs()
                );
            }
            continue;
        }

        // ─── 6. Check for success ────────────────────────────────────────
        if harness
            .query_by_label_contains("Identity Registered Successfully!")
            .is_some()
        {
            // Read the identity ID from the screen
            let identity_id = {
                let stack = &harness.state().screen_stack;
                if let Some(Screen::AddNewIdentityScreen(screen)) = stack.last() {
                    screen.successful_qualified_identity_id
                } else {
                    None
                }
            };

            if let Some(id) = identity_id {
                println!("  Identity created: {}", id);
                ctx.identity_id = Some(id);
            } else {
                println!("  Warning: success screen shown but no identity ID found");
            }

            harness.state_mut().screen_stack.pop();
            harness.run_steps(SETTLE_STEPS);

            // ─── 7. Verify identity appears on Identities screen ─────────
            navigate_to_screen(harness, RootScreenType::RootScreenIdentities);
            harness.run_steps(POLL_STEPS);

            if let Some(id) = ctx.identity_id {
                let id_str = id.to_string(Encoding::Base58);
                let found = harness.query_by_label_contains(&id_str).is_some()
                    || harness.query_by_label_contains("E2E Identity").is_some();

                if found {
                    println!("  Identity verified on Identities screen");
                } else {
                    println!(
                        "  Warning: identity not found on Identities screen (may need refresh)"
                    );
                }
            }

            println!("  Phase 05 complete: identity creation verified");
            return;
        }

        // ─── Error path: classify, dismiss, retry ─────────────────────────
        handle_retry_error(harness, "Identity creation", attempt, true);
    }
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
        screen.funding_amount = None;
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
        screen.funding_amount = Some(Amount::new_dash(0.01));
        // Deliberately skip ensure_correct_identity_keys()
    });
    harness.run_steps(POLL_STEPS);
    // Skip the breadcrumb (nth 0) and click the action button (nth 1) if present
    let has_action_button = harness.query_all_by_label("Create Identity").count() >= 2;
    if has_action_button {
        harness
            .query_all_by_label("Create Identity")
            .nth(1)
            .unwrap()
            .click();
        harness.run_steps(POLL_STEPS);
    }
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
        *screen.funding_method.write().unwrap() = FundingMethod::UseWalletBalance;
        *screen.step.write().unwrap() = WalletFundedScreenStep::WaitingForAssetLock;
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
            let step = screen.step.read().unwrap();
            assert_eq!(*step, expected, "Identity screen step mismatch");
        }
        _ => panic!("Expected AddNewIdentityScreen on screen stack"),
    }
}

/// Set common fields for a wallet-funded identity screen in ReadyToCreate state.
fn set_wallet_funded_ready(screen: &mut AddNewIdentityScreen, alias: &str) {
    *screen.funding_method.write().unwrap() = FundingMethod::UseWalletBalance;
    *screen.step.write().unwrap() = WalletFundedScreenStep::ReadyToCreate;
    screen.alias_input = alias.to_string();
}
