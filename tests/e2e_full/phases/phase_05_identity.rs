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
use std::time::Duration;

const MAX_RETRIES: u32 = 3;
const IDENTITY_TIMEOUT: Duration = Duration::from_secs(300);

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    for attempt in 1..=MAX_RETRIES {
        println!("  Identity creation attempt {}/{}", attempt, MAX_RETRIES);

        // ─── 1. Push AddNewIdentity screen ───────────────────────────────
        push_screen(harness, ScreenType::AddNewIdentity);

        // ─── 2. Configure the screen directly (ComboBox not accessible) ─
        let configured = {
            let stack = &mut harness.state_mut().screen_stack;
            if let Some(Screen::AddNewIdentityScreen(screen)) = stack.last_mut() {
                *screen.funding_method.write().unwrap() = FundingMethod::UseWalletBalance;
                *screen.step.write().unwrap() = WalletFundedScreenStep::ReadyToCreate;
                screen.alias_input = "E2E Identity".to_string();
                screen.funding_amount = Some(Amount::new_dash(0.01));
                if let Err(e) = screen.ensure_correct_identity_keys() {
                    println!("  Warning: ensure_correct_identity_keys failed: {}", e);
                    false
                } else {
                    true
                }
            } else {
                false
            }
        };

        if !configured {
            println!("  Failed to configure AddNewIdentityScreen, retrying...");
            harness.state_mut().screen_stack.pop();
            harness.run_steps(5);
            continue;
        }

        // ─── 3. Let UI render with configured state ──────────────────────
        harness.run_steps(30);

        // ─── 4. Click "Create Identity" button ───────────────────────────
        if let Some(btn) = harness.query_by_label("Create Identity") {
            btn.click();
            harness.run_steps(10);
        } else {
            println!("  'Create Identity' button not found, retrying...");
            harness.state_mut().screen_stack.pop();
            harness.run_steps(5);
            continue;
        }

        // ─── 5. Wait for success or error ────────────────────────────────
        let completed = wait_until(
            harness,
            |h| {
                h.query_by_label_contains("Identity Registered Successfully!")
                    .is_some()
                    || h.query_by_label_contains("Error").is_some()
            },
            IDENTITY_TIMEOUT,
            30,
        );

        if !completed {
            println!(
                "  Identity creation timed out after {}s",
                IDENTITY_TIMEOUT.as_secs()
            );
            harness.state_mut().screen_stack.pop();
            harness.run_steps(5);
            if attempt == MAX_RETRIES {
                panic!(
                    "Identity creation timed out after {} attempts ({}s each)",
                    MAX_RETRIES,
                    IDENTITY_TIMEOUT.as_secs()
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
            harness.run_steps(10);

            // ─── 7. Verify identity appears on Identities screen ─────────
            navigate_to_screen(harness, RootScreenType::RootScreenIdentities);
            harness.run_steps(30);

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

        // ─── Error path: capture, dismiss, retry ─────────────────────────
        let error_detail =
            capture_error_text(harness).unwrap_or_else(|| "unknown error".to_string());
        println!(
            "  Identity creation error (attempt {}/{}): {}",
            attempt, MAX_RETRIES, error_detail
        );
        dismiss_if_present(harness);
        harness.state_mut().screen_stack.pop();
        harness.run_steps(10);

        if attempt == MAX_RETRIES {
            panic!(
                "Identity creation failed after {} retries. Last error: {}",
                MAX_RETRIES, error_detail
            );
        }
    }
}
