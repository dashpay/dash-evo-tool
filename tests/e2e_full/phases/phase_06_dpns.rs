use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::identities::register_dpns_name_screen::RegisterDpnsNameSource;
use dash_evo_tool::ui::{Screen, ScreenType};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::{SystemTime, UNIX_EPOCH};

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

    for attempt in 1..=PLATFORM_MAX_RETRIES {
        println!(
            "  DPNS registration attempt {}/{}",
            attempt, PLATFORM_MAX_RETRIES
        );

        // ─── 1. Push RegisterDpnsName screen ─────────────────────────────
        push_screen(
            harness,
            ScreenType::RegisterDpnsName(RegisterDpnsNameSource::Identities),
        );

        // ─── 2. Reload identities (just created in Phase 5) and configure ─
        {
            let stack = &mut harness.state_mut().screen_stack;
            if let Some(Screen::RegisterDpnsNameScreen(screen)) = stack.last_mut() {
                // Refresh identity list from database -- the identity was just
                // created in Phase 5 and may not have been loaded by the
                // screen's constructor.
                screen.qualified_identities = screen
                    .app_context
                    .load_local_user_identities()
                    .unwrap_or_default();

                // Hard assert: identity list must not be empty
                assert!(
                    !screen.qualified_identities.is_empty(),
                    "No identities in database. Did Phase 5 complete?"
                );

                screen.select_identity(identity_id);

                // Hard assert: identity must be selected
                assert!(
                    screen.selected_qualified_identity.is_some(),
                    "select_identity({}) failed — identity not in list of {} identities",
                    identity_id,
                    screen.qualified_identities.len()
                );

                screen.show_identity_selector = false;
                screen.name_input = dpns_name.clone();
            } else {
                panic!("Expected RegisterDpnsNameScreen on screen stack");
            }
        }

        // ─── 3. Let UI render with configured state ──────────────────────
        harness.run_steps(POLL_STEPS);

        // Verify UI rendered the expected state.
        // The top panel breadcrumb also has "Register Name", so count >= 2
        // means the action button is rendered.
        let reg_count = harness.query_all_by_label("Register Name").count();
        assert!(
            reg_count >= 2,
            "'Register Name' action button must be visible (found {} matches, \
             need >= 2: breadcrumb + button)",
            reg_count
        );
        println!("  Screen configured: Register Name button visible");

        // ─── 4. Click "Register Name" action button (skip breadcrumb) ───
        harness
            .query_all_by_label("Register Name")
            .nth(1)
            .expect("Register Name action button not found")
            .click();
        harness.run_steps(SETTLE_STEPS);

        // ─── 5. Wait for success or error ────────────────────────────────
        let completed = wait_until(
            harness,
            |h| {
                h.query_by_label_contains("DPNS Name Registered!").is_some()
                    || h.query_by_label_contains("Error").is_some()
            },
            DPNS_REGISTRATION_TIMEOUT,
            POLL_STEPS,
        );

        if !completed {
            println!(
                "  DPNS registration timed out after {}s",
                DPNS_REGISTRATION_TIMEOUT.as_secs()
            );
            harness.state_mut().screen_stack.pop();
            harness.run_steps(5);
            if attempt == PLATFORM_MAX_RETRIES {
                panic!(
                    "DPNS registration timed out after {} attempts ({}s each)",
                    PLATFORM_MAX_RETRIES,
                    DPNS_REGISTRATION_TIMEOUT.as_secs()
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
            harness.run_steps(SETTLE_STEPS);

            println!("  Phase 06 complete: DPNS registration verified");
            return;
        }

        // ─── Error path: classify, dismiss, retry ─────────────────────────
        handle_retry_error(harness, "DPNS registration", attempt, true);
    }
}
