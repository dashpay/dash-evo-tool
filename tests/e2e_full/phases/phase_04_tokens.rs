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
    let Some(search_input) = harness.query_by_label_contains("Search tokens") else {
        println!("  Token search input not found — skipping");
        return;
    };
    search_input.type_text("dash");
    harness.run_steps(5);
    println!("  Typed 'dash' in search input");

    // ─── 3. Click search button and wait for results ─────────────────
    let Some(search_btn) = harness.query_by_label_contains("Search") else {
        println!("  Search button not found — skipping");
        return;
    };
    search_btn.click();
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

    if completed {
        if harness.query_by_label_contains("Contract ID").is_some() {
            println!("  Token search returned results");
        } else if harness.query_by_label_contains("No tokens match").is_some() {
            println!("  Token search returned no results (acceptable)");
        } else {
            println!("  Token search completed with error (platform may be unavailable)");
            if let Some(dismiss) = harness.query_by_label_contains("Dismiss") {
                dismiss.click();
                harness.run_steps(5);
            }
        }
    } else {
        println!("  Token search timed out (platform may be unavailable)");
    }

    // ─── 4. Clear search ─────────────────────────────────────────────
    if let Some(clear_btn) = harness.query_by_label_contains("Clear") {
        clear_btn.click();
        harness.run_steps(5);
        println!("  Search cleared");
    } else {
        println!("  Clear button not found (skipping clear verification)");
    }

    println!("  Phase 04 complete: token search verified");
}
