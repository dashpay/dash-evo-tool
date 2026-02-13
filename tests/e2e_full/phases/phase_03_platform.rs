use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::{RootScreenType, ScreenType};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::Duration;

/// Well-known DPNS contract ID (base58) present on all Dash Platform networks.
const DPNS_CONTRACT_ID: &str = "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec";

pub fn run(harness: &mut Harness<'_, AppState>, _ctx: &mut TestContext) {
    // ─── 1. DPNS Name Lookup via UI ─────────────────────────────────
    run_dpns_lookup(harness);

    // ─── 2. Contract Fetch via UI ───────────────────────────────────
    run_contract_fetch(harness);

    // ─── 3. Platform Info ───────────────────────────────────────────
    println!(
        "  Platform info: network={:?}",
        harness.state().chosen_network
    );
    println!("  Phase 03 complete: platform reads verified");
}

/// Click a button by label, falling back to pushing the screen directly if not found.
fn click_or_push_screen(
    harness: &mut Harness<'_, AppState>,
    button_label: &str,
    screen_type: ScreenType,
) {
    if let Some(btn) = harness.query_by_label_contains(button_label) {
        btn.click();
    } else {
        println!("  {button_label} button not found — pushing screen directly");
        let app_ctx = harness.state().current_app_context();
        let screen = screen_type.create_screen(app_ctx);
        harness.state_mut().screen_stack.push(screen);
    }
    harness.run_steps(10);
}

/// Dismiss an error dialog if the "Dismiss" button is present.
fn dismiss_if_present(harness: &mut Harness<'_, AppState>) {
    if let Some(dismiss) = harness.query_by_label_contains("Dismiss") {
        dismiss.click();
        harness.run_steps(5);
    }
}

fn run_dpns_lookup(harness: &mut Harness<'_, AppState>) {
    navigate_to_screen(harness, RootScreenType::RootScreenIdentities);
    click_or_push_screen(harness, "Load Identity", ScreenType::AddExistingIdentity);

    // Switch to "By DPNS Name" tab
    harness
        .query_by_label_contains("By DPNS Name")
        .expect("'By DPNS Name' tab must be visible on Load Identity screen")
        .click();
    harness.run_steps(5);

    // Type a DPNS name to search for
    harness
        .query_by_label_contains("DPNS name")
        .expect("DPNS name input must be visible after selecting 'By DPNS Name' tab")
        .type_text("quantum");
    harness.run_steps(5);

    // Click "Search by Username" button
    harness
        .query_by_label_contains("Search by Username")
        .expect("'Search by Username' button must be visible on DPNS lookup screen")
        .click();
    harness.run_steps(10);

    // Wait for result — success or not-found are acceptable; error/timeout are failures
    let completed = wait_until(
        harness,
        |h| {
            h.query_by_label_contains("Successfully loaded").is_some()
                || h.query_by_label_contains("Finished loading").is_some()
                || h.query_by_label_contains("not found").is_some()
                || h.query_by_label_contains("No identity").is_some()
                || h.query_by_label_contains("Error").is_some()
                || h.query_by_label_contains("Dismiss").is_some()
        },
        Duration::from_secs(60),
        30,
    );
    assert!(
        completed,
        "DPNS lookup must complete within 60s (timed out)"
    );

    let is_success = harness
        .query_by_label_contains("Successfully loaded")
        .is_some()
        || harness
            .query_by_label_contains("Finished loading")
            .is_some();
    let is_not_found = harness.query_by_label_contains("not found").is_some()
        || harness
            .query_by_label_contains("No identity found")
            .is_some();

    // Detect platform errors that aren't a clean "not found"
    let has_error_label = harness.query_by_label_contains("Error").is_some();
    let is_platform_error = has_error_label && !is_not_found;
    assert!(
        !is_platform_error,
        "DPNS lookup returned a platform error (not a clean not-found)"
    );
    assert!(
        is_success || is_not_found,
        "DPNS lookup must succeed or return not-found (got unexpected state)"
    );
    if is_success {
        println!("  DPNS lookup succeeded: name \"quantum\" found");
    } else {
        println!("  DPNS lookup completed: name not found (acceptable)");
    }
    dismiss_if_present(harness);

    navigate_to_screen(harness, RootScreenType::RootScreenIdentities);
}

fn run_contract_fetch(harness: &mut Harness<'_, AppState>) {
    navigate_to_screen(harness, RootScreenType::RootScreenDocumentQuery);
    click_or_push_screen(harness, "Load Contracts", ScreenType::AddContracts);

    // Enter the DPNS contract ID
    harness
        .query_by_label_contains("Contract ID")
        .expect("Contract ID input must be visible on Add Contracts screen")
        .type_text(DPNS_CONTRACT_ID);
    harness.run_steps(5);

    // Click "Fetch Contracts" submit button
    harness
        .query_by_label_contains("Fetch Contracts")
        .expect("'Fetch Contracts' button must be visible on Add Contracts screen")
        .click();
    harness.run_steps(10);

    // Wait for fetch result
    let completed = wait_until(
        harness,
        |h| {
            h.query_by_label_contains("Successfully queried").is_some()
                || h.query_by_label_contains("Error").is_some()
                || h.query_by_label_contains("Dismiss").is_some()
                || h.query_by_label_contains("not found").is_some()
        },
        Duration::from_secs(90),
        30,
    );
    assert!(
        completed,
        "Contract fetch must complete within 90s (timed out)"
    );

    let is_success = harness
        .query_by_label_contains("Successfully queried")
        .is_some();
    assert!(
        is_success,
        "DPNS contract fetch must succeed — {} exists on all networks",
        DPNS_CONTRACT_ID
    );
    println!("  Contract fetch succeeded: DPNS contract found");

    if let Some(back_btn) = harness.query_by_label_contains("Back to Contracts") {
        back_btn.click();
        harness.run_steps(10);
    } else {
        navigate_to_screen(harness, RootScreenType::RootScreenDocumentQuery);
    }
}
