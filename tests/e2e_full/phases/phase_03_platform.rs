use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::{RootScreenType, ScreenType};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

/// Well-known DPNS contract ID (base58) present on all Dash Platform networks.
const DPNS_CONTRACT_ID: &str = "GWRSAVFMjXx8HpQFaNJMqBV7MBgMK4br5UESsB4S31Ec";

pub fn run(harness: &mut Harness<'_, AppState>, _ctx: &mut TestContext) {
    run_dpns_lookup(harness);
    run_contract_fetch(harness);

    println!(
        "  Platform info: network={:?}",
        harness.state().chosen_network
    );
    println!("  Phase 03 complete: platform reads verified");
}

fn run_dpns_lookup(harness: &mut Harness<'_, AppState>) {
    for attempt in 1..=PLATFORM_MAX_RETRIES {
        // Navigate to Load Identity screen each attempt (clean slate)
        navigate_to_screen(harness, RootScreenType::RootScreenIdentities);
        push_screen(harness, ScreenType::AddExistingIdentity);

        // Switch to "By DPNS Name" tab
        harness
            .query_by_label_contains("By DPNS Name")
            .expect("'By DPNS Name' tab must be visible on Load Identity screen")
            .click();
        harness.run_steps(15);

        // Type a DPNS name to search for
        type_into_text_input(harness, 0, "quantum");

        // Click "Search by Username" button
        harness
            .query_by_label("Search by Username")
            .expect("'Search by Username' button must be visible on DPNS lookup screen")
            .click();
        harness.run_steps(SETTLE_STEPS);

        // Wait for result
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
            PLATFORM_READ_TIMEOUT,
            POLL_STEPS,
        );
        assert!(
            completed,
            "DPNS lookup must complete within {}s (timed out)",
            PLATFORM_READ_TIMEOUT.as_secs()
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
        let has_error = harness.query_by_label_contains("Error").is_some()
            || harness.query_by_label_contains("Dismiss").is_some();

        if is_success {
            println!("  DPNS lookup succeeded: name \"quantum\" found");
        } else if is_not_found {
            println!("  DPNS lookup completed: name not found (acceptable)");
        } else if has_error {
            let error_text =
                capture_error_text(harness).unwrap_or_else(|| "unknown error".to_string());
            let category = classify_error(&error_text);
            println!(
                "  DPNS lookup error (attempt {}/{}): [{}] {}",
                attempt,
                PLATFORM_MAX_RETRIES,
                category.label(),
                error_text
            );

            if !category.is_retryable() {
                panic!(
                    "DPNS lookup failed with non-retryable error: {}",
                    error_text
                );
            }

            dismiss_if_present(harness);
            harness.state_mut().screen_stack.pop();
            harness.run_steps(SETTLE_STEPS * attempt as usize);

            if attempt == PLATFORM_MAX_RETRIES {
                panic!(
                    "DPNS lookup failed after {} retries. Last error: {}",
                    PLATFORM_MAX_RETRIES, error_text
                );
            }
            continue;
        } else {
            panic!("DPNS lookup reached unexpected state");
        }

        // Common cleanup for success/not-found paths
        dismiss_if_present(harness);
        harness.state_mut().screen_stack.pop();
        harness.run_steps(5);
        return;
    }
}

fn run_contract_fetch(harness: &mut Harness<'_, AppState>) {
    for attempt in 1..=PLATFORM_MAX_RETRIES {
        // Push AddContracts screen fresh each attempt so the text input is empty
        navigate_to_screen(harness, RootScreenType::RootScreenDocumentQuery);
        push_screen(harness, ScreenType::AddContracts);

        // Enter the DPNS contract ID
        type_into_text_input(harness, 0, DPNS_CONTRACT_ID);

        // Click "Fetch Contracts" submit button
        harness
            .query_by_label("Fetch Contracts")
            .or_else(|| harness.query_by_label_contains("Fetch Contracts"))
            .expect("'Fetch Contracts' button must be visible on Add Contracts screen")
            .click();
        harness.run_steps(SETTLE_STEPS);

        // Wait for fetch result
        let completed = wait_until(
            harness,
            |h| {
                h.query_by_label_contains("Successfully queried").is_some()
                    || h.query_by_label_contains("Error").is_some()
                    || h.query_by_label_contains("Dismiss").is_some()
                    || h.query_by_label_contains("not found").is_some()
            },
            CONTRACT_FETCH_TIMEOUT,
            POLL_STEPS,
        );
        assert!(
            completed,
            "Contract fetch must complete within {}s (timed out)",
            CONTRACT_FETCH_TIMEOUT.as_secs()
        );

        if harness
            .query_by_label_contains("Successfully queried")
            .is_some()
        {
            println!("  Contract fetch succeeded: DPNS contract found");
            harness.state_mut().screen_stack.pop();
            harness.run_steps(5);
            return;
        }

        // Error path — classify and decide whether to retry
        let error_text = capture_error_text(harness).unwrap_or_else(|| "unknown error".to_string());
        let category = classify_error(&error_text);
        println!(
            "  Contract fetch error (attempt {}/{}): [{}] {}",
            attempt,
            PLATFORM_MAX_RETRIES,
            category.label(),
            error_text
        );

        if !category.is_retryable() {
            panic!(
                "Contract fetch failed with non-retryable error: {}",
                error_text
            );
        }

        dismiss_if_present(harness);
        harness.state_mut().screen_stack.pop();
        harness.run_steps(SETTLE_STEPS * attempt as usize);

        if attempt == PLATFORM_MAX_RETRIES {
            panic!(
                "DPNS contract fetch failed after {} attempts. Last error: {}",
                PLATFORM_MAX_RETRIES, error_text
            );
        }
    }
}
