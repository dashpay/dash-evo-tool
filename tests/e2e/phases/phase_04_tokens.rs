use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

pub fn run(harness: &mut Harness<'_, AppState>, _ctx: &mut TestContext) {
    for attempt in 1..=PLATFORM_MAX_RETRIES {
        // ─── 1. Navigate to token search screen (fresh each attempt) ───
        navigate_to_screen(harness, RootScreenType::RootScreenTokenSearch);

        let on_screen =
            wait_for_label(harness, "Enter Keyword", std::time::Duration::from_secs(10));
        assert!(
            on_screen,
            "'Enter Keyword' label must be visible on token search screen"
        );

        if attempt == 1 {
            println!("  Navigated to token search screen");
        }

        type_into_text_input(harness, 0, "dash");
        println!(
            "  Typed 'dash' in search input (attempt {}/{})",
            attempt, PLATFORM_MAX_RETRIES
        );

        // ─── 2. Click search button and wait for results ───────────────
        harness
            .query_by_label("Search")
            .expect("Search button must be visible on token search screen")
            .click();
        harness.run_steps(SETTLE_STEPS);

        let completed = wait_until(
            harness,
            |h| {
                h.query_by_label_contains("Contract ID").is_some()
                    || h.query_by_label_contains("No tokens match").is_some()
                    || h.query_by_label_contains("Error").is_some()
            },
            TOKEN_SEARCH_TIMEOUT,
            POLL_STEPS,
        );

        assert!(
            completed,
            "Token search must complete within {}s (timed out)",
            TOKEN_SEARCH_TIMEOUT.as_secs()
        );

        let has_results = harness.query_by_label_contains("Contract ID").is_some();
        let no_results = harness.query_by_label_contains("No tokens match").is_some();
        let has_error = harness.query_by_label_contains("Error").is_some();

        if has_results {
            println!("  Token search returned results for 'dash'");

            // Clear search before leaving
            if let Some(clear_btn) = harness.query_by_label_contains("Clear") {
                clear_btn.click();
                harness.run_steps(5);
                println!("  Search cleared");
            }

            println!("  Phase 04 complete: token search verified");
            return;
        }

        if no_results {
            panic!(
                "Token search for 'dash' returned no results. \
                 The keyword 'dash' consistently returns results on testnet — \
                 empty results means the query mechanism or token contract is broken."
            );
        }

        if has_error {
            handle_retry_error(harness, "Token search", attempt, false);
            continue;
        }

        panic!("Token search reached unexpected state");
    }
}
