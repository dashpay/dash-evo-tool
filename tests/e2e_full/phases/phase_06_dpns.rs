use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::identities::register_dpns_name_screen::RegisterDpnsNameSource;
use dash_evo_tool::ui::{Screen, ScreenType};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_RETRIES: u32 = 3;
const DPNS_TIMEOUT: Duration = Duration::from_secs(180);

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    let identity_id = ctx
        .identity_id
        .expect("identity_id must be set (did Phase 5 complete?)");

    // Generate a unique, non-contested DPNS name.
    // Contains digits > 1 to avoid contested name fees.
    let unix_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let dpns_name = format!("e2etest{}", unix_secs);
    println!("  Target DPNS name: {}", dpns_name);

    for attempt in 1..=MAX_RETRIES {
        println!("  DPNS registration attempt {}/{}", attempt, MAX_RETRIES);

        // ─── 1. Push RegisterDpnsName screen ─────────────────────────────
        push_screen(
            harness,
            ScreenType::RegisterDpnsName(RegisterDpnsNameSource::Identities),
        );

        // ─── 2. Reload identities (just created in Phase 5) and configure ─
        let configured = {
            let stack = &mut harness.state_mut().screen_stack;
            if let Some(Screen::RegisterDpnsNameScreen(screen)) = stack.last_mut() {
                // Refresh identity list from database -- the identity was just
                // created in Phase 5 and may not have been loaded by the
                // screen's constructor.
                screen.qualified_identities = screen
                    .app_context
                    .load_local_user_identities()
                    .unwrap_or_default();
                screen.select_identity(identity_id);
                screen.show_identity_selector = false;
                screen.name_input = dpns_name.clone();
                if screen.selected_qualified_identity.is_none() {
                    println!("  Warning: select_identity did not find identity in list");
                }
                screen.selected_qualified_identity.is_some()
            } else {
                false
            }
        };

        if !configured {
            println!("  Failed to configure RegisterDpnsNameScreen, retrying...");
            harness.state_mut().screen_stack.pop();
            harness.run_steps(5);
            continue;
        }

        // ─── 3. Let UI render with configured state ──────────────────────
        harness.run_steps(30);

        // ─── 4. Click "Register Name" button ─────────────────────────────
        if let Some(btn) = harness.query_by_label("Register Name") {
            btn.click();
            harness.run_steps(10);
        } else {
            println!("  'Register Name' button not found, retrying...");
            harness.state_mut().screen_stack.pop();
            harness.run_steps(5);
            continue;
        }

        // ─── 5. Wait for success or error ────────────────────────────────
        let completed = wait_until(
            harness,
            |h| {
                h.query_by_label_contains("DPNS Name Registered!").is_some()
                    || h.query_by_label_contains("Error").is_some()
            },
            DPNS_TIMEOUT,
            30,
        );

        if !completed {
            println!(
                "  DPNS registration timed out after {}s",
                DPNS_TIMEOUT.as_secs()
            );
            harness.state_mut().screen_stack.pop();
            harness.run_steps(5);
            if attempt == MAX_RETRIES {
                panic!(
                    "DPNS registration timed out after {} attempts ({}s each)",
                    MAX_RETRIES,
                    DPNS_TIMEOUT.as_secs()
                );
            }
            continue;
        }

        // ─── 6. Check for success ────────────────────────────────────────
        if harness
            .query_by_label_contains("DPNS Name Registered!")
            .is_some()
        {
            ctx.dpns_name = Some(format!("{}.dash", dpns_name));
            println!("  DPNS name registered: {}.dash", dpns_name);

            harness.state_mut().screen_stack.pop();
            harness.run_steps(10);

            println!("  Phase 06 complete: DPNS registration verified");
            return;
        }

        // ─── Error path: capture, dismiss, retry ─────────────────────────
        let error_detail =
            capture_error_text(harness).unwrap_or_else(|| "unknown error".to_string());
        println!(
            "  DPNS registration error (attempt {}/{}): {}",
            attempt, MAX_RETRIES, error_detail
        );
        dismiss_if_present(harness);
        harness.state_mut().screen_stack.pop();
        harness.run_steps(10);

        if attempt == MAX_RETRIES {
            panic!(
                "DPNS registration failed after {} retries. Last error: {}",
                MAX_RETRIES, error_detail
            );
        }
    }
}
