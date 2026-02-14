use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::Duration;

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    // 1. Navigate to wallets screen and verify wallet card
    navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);

    let has_wallet = wait_for_label(harness, "E2E Test Wallet", Duration::from_secs(10));
    assert!(
        has_wallet,
        "Wallet card should show 'E2E Test Wallet' alias"
    );
    println!("  Wallet card with alias visible");

    // 2. Verify Send/Receive buttons visible (wallet is already selected from Phase 0/1)
    // Use exact label match for "Receive" to avoid ambiguity with "Total Received (DASH)"
    let send_visible = harness.query_by_label("Send").is_some();
    assert!(
        send_visible,
        "Send button must be visible after selecting wallet"
    );
    verify_receive_button_visible(harness);
    println!("  Send/Receive buttons visible");

    // 3. Verify receive button visible (proves wallet is selected and action buttons rendered)
    verify_receive_button_visible(harness);

    // 4. Conditional send-to-self (requires >= 0.1 DASH)
    let min_balance_for_send: u64 = 10_000_000; // 0.1 DASH in duffs

    assert!(
        ctx.balance_duffs >= min_balance_for_send,
        "Wallet balance ({} duffs / {:.8} DASH) is below the minimum ({} duffs / {:.8} DASH) \
         required for the send-to-self test. Fund the E2E wallet and retry.",
        ctx.balance_duffs,
        ctx.balance_duffs as f64 / 1e8,
        min_balance_for_send,
        min_balance_for_send as f64 / 1e8,
    );

    println!("  Attempting send-to-self (0.001 DASH)...");

    // Click "Send" button to open the send screen
    let stack_before = harness.state().screen_stack.len();
    harness
        .query_by_label("Send")
        .expect("Send button not found")
        .click();
    harness.run_steps(30);
    let stack_after = harness.state().screen_stack.len();
    println!(
        "  Screen stack: {} -> {} after Send click",
        stack_before, stack_after
    );

    // The send screen should now be pushed onto the screen stack.
    // The "Send Dash" heading should be visible since the screen is on the stack.
    let send_screen_visible = wait_for_label(harness, "Send Dash", Duration::from_secs(10));
    assert!(
        send_screen_visible,
        "Send screen must be visible after clicking Send button (screen_stack={})",
        harness.state().screen_stack.len(),
    );

    // The "Send to" label should be visible (rendered by render_destination_input).
    // Note: hint_text on TextEdit may not appear in the AccessKit tree, so we check
    // the "Send to" label instead of the hint text.
    let send_to_visible = harness.query_by_label_contains("Send to").is_some();
    assert!(
        send_to_visible,
        "Send screen must show 'Send to' label for destination input"
    );

    // Find text inputs by role (hint_text doesn't appear in AccessKit labels).
    // The send screen's destination address input is the first TextInput.
    let addr = ctx
        .receive_address
        .as_ref()
        .expect("No receive address from Phase 01")
        .clone();

    // Click the destination input to focus it, then type the address.
    // We must drop the borrow from query before calling run_steps.
    harness
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .expect("Destination address TextInput must exist on send screen")
        .click();
    harness.run_steps(5);
    harness
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .unwrap()
        .type_text(&addr);
    harness.run_steps(10);
    println!("  Entered destination address");

    // The amount input: click to focus, then type (re-query each time to avoid borrow issues)
    harness
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .nth(1)
        .expect("Amount TextInput must exist on send screen")
        .click();
    harness.run_steps(5);
    harness
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .nth(1)
        .unwrap()
        .type_text("0.001");
    harness.run_steps(10);
    println!("  Entered amount: 0.001 DASH");

    // Click the send/transaction type button.
    // For Core→Core it will be "Core Transaction".
    // Use exact label match to target the Button, not the "Transaction type: Core Transaction" label.
    let tx_btn = harness
        .query_by_label("Core Transaction")
        .expect("Core Transaction button must be visible on send screen");
    tx_btn.click();
    harness.run_steps(10);
    println!("  Clicked transaction button");

    // Wait for the final result: success (Send Another / Back to Wallet) or
    // failure (Dismiss / any error dialog).  This skips the intermediate "Sending..."
    // state and waits for the transaction to complete.
    let got_final_result = wait_until(
        harness,
        |h| {
            h.query_by_label_contains("Send Another").is_some()
                || h.query_by_label_contains("Back to Wallet").is_some()
                || h.query_by_label_contains("Dismiss").is_some()
        },
        Duration::from_secs(180),
        30,
    );

    // Dump visible labels for diagnostics
    let all_labels: Vec<String> = harness
        .query_all_by_label_contains("")
        .take(40)
        .map(|n| format!("{:?}", n))
        .collect();
    println!("  After send — visible nodes (first 40):");
    for label in &all_labels {
        println!("    {}", label);
    }

    assert!(
        got_final_result,
        "Send transaction must complete within 180s"
    );

    // Assert success specifically — error is a test failure.
    let is_success = harness.query_by_label_contains("Send Another").is_some()
        || harness.query_by_label_contains("Back to Wallet").is_some();
    if !is_success {
        panic!("Send-to-self must succeed. Check the visible nodes above for error details.");
    }
    println!("  Send-to-self succeeded!");

    // Click "Back to Wallet" to return
    if let Some(back_btn) = harness.query_by_label_contains("Back to Wallet") {
        back_btn.click();
        harness.run_steps(10);
    }

    // Navigate back to wallets screen for next phase
    navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);
    println!("  Phase 02 complete: wallet UI operations verified");
}
