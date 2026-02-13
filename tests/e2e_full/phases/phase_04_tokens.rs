use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::Duration;

pub fn run(harness: &mut Harness<'_, AppState>, _ctx: &mut TestContext) {
    // ─── 1. Navigate to token search screen ──────────────────────────
    navigate_to_screen(harness, RootScreenType::RootScreenTokenSearch);
    println!("  Navigated to token search screen");

    // ─── 2. Find and fill search input ───────────────────────────────
    harness
        .query_by_label_contains("Search tokens")
        .expect("Token search input must be visible on token search screen")
        .type_text("dash");
    harness.run_steps(5);
    println!("  Typed 'dash' in search input");

    // ─── 3. Click search button and wait for results ─────────────────
    harness
        .query_by_label_contains("Search")
        .expect("Search button must be visible on token search screen")
        .click();
    harness.run_steps(10);

    let completed = wait_until(
        harness,
        |h| {
            // Results found (table has Contract ID column header)
            h.query_by_label_contains("Contract ID").is_some()
                // No results
                || h.query_by_label_contains("No tokens match").is_some()
                // Error
                || h.query_by_label_contains("Error").is_some()
        },
        Duration::from_secs(60),
        30,
    );

    assert!(
        completed,
        "Token search must complete within 60s (timed out)"
    );

    let has_results = harness.query_by_label_contains("Contract ID").is_some();
    let no_results = harness.query_by_label_contains("No tokens match").is_some();
    assert!(
        has_results || no_results,
        "Token search must return results or 'no results' (got platform error)"
    );
    if has_results {
        println!("  Token search returned results");
    } else {
        println!("  Token search returned no results (acceptable)");
    }

    // ─── 4. Clear search ─────────────────────────────────────────────
    harness
        .query_by_label_contains("Clear")
        .expect("Clear button must be visible after performing a search")
        .click();
    harness.run_steps(5);
    println!("  Search cleared");

    println!("  Phase 04 complete: token search verified");
}
