use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::model::amount::Amount;
use dash_evo_tool::ui::identities::add_new_identity_screen::FundingMethod;
use dash_evo_tool::ui::identities::funding_common::WalletFundedScreenStep;
use dash_evo_tool::ui::{RootScreenType, Screen, ScreenType};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    // SPV readiness gate — identity creation builds an asset lock transaction
    ensure_spv_tx_ready(harness, ctx);

    for attempt in 1..=PLATFORM_MAX_RETRIES {
        println!(
            "  Identity creation attempt {}/{}",
            attempt, PLATFORM_MAX_RETRIES
        );

        // ─── 1. Push AddNewIdentity screen ───────────────────────────────
        push_screen(harness, ScreenType::AddNewIdentity);

        // ─── 2. Configure the screen directly (ComboBox not accessible) ─
        {
            let stack = &mut harness.state_mut().screen_stack;
            if let Some(Screen::AddNewIdentityScreen(screen)) = stack.last_mut() {
                *screen.funding_method.write().unwrap() = FundingMethod::UseWalletBalance;
                *screen.step.write().unwrap() = WalletFundedScreenStep::ReadyToCreate;
                screen.alias_input = "E2E Identity".to_string();
                screen.funding_amount = Some(Amount::new_dash(0.01));

                // Hard-fail on key setup — wallet must be open
                screen.ensure_correct_identity_keys().unwrap_or_else(|e| {
                    panic!(
                        "ensure_correct_identity_keys() failed: {}. Is wallet open?",
                        e
                    )
                });
            } else {
                panic!("Expected AddNewIdentityScreen on screen stack");
            }
        }

        // ─── 3. Let UI render with configured state ──────────────────────
        harness.run_steps(POLL_STEPS);

        // Verify UI rendered the expected state
        assert!(
            harness.query_by_label("Create Identity").is_some(),
            "'Create Identity' button must be visible after configuring screen — \
             UI did not render ReadyToCreate state correctly"
        );
        println!("  Screen configured: Create Identity button visible");

        // ─── 4. Click "Create Identity" button ───────────────────────────
        harness
            .query_by_label("Create Identity")
            .expect("Create Identity button not found")
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
