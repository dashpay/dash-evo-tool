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
    let send_visible = harness.query_by_label_contains("Send").is_some();
    assert!(
        send_visible,
        "Send button must be visible after selecting wallet"
    );
    let receive_visible = harness.query_by_label_contains("Receive").is_some();
    assert!(
        receive_visible,
        "Receive button must be visible after selecting wallet"
    );
    println!("  Send/Receive buttons visible");

    // 3. Open receive dialog and verify address
    let addr = ctx
        .receive_address
        .as_ref()
        .expect("Receive address must be set by Phase 01");
    open_and_verify_receive_dialog(harness, addr);

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
    harness
        .query_by_label_contains("Send")
        .expect("Send button not found")
        .click();
    harness.run_steps(15);

    // The send screen should now be pushed onto the screen stack.
    // The wallet is imported without a password, so it should auto-unlock via
    // try_open_wallet_no_password(). Wait for the address input to appear.
    let addr_input_visible = wait_for_label(harness, "Enter address", Duration::from_secs(10));
    assert!(
        addr_input_visible,
        "Address input must be visible on send screen. {}",
        if harness.query_by_label_contains("Unlock Wallet").is_some() {
            "Wallet is locked — a passwordless import should auto-unlock"
        } else {
            "Send screen did not render the address input"
        }
    );

    // Fill destination address (send to self)
    let addr = ctx
        .receive_address
        .as_ref()
        .expect("No receive address from Phase 01");
    harness
        .query_by_label_contains("Enter address")
        .expect("Address input should be visible")
        .type_text(addr);
    harness.run_steps(10);
    println!("  Entered destination address");

    // Fill amount (0.001 DASH)
    harness
        .query_by_label_contains("Enter amount")
        .expect("Amount input must be visible on send screen")
        .type_text("0.001");
    harness.run_steps(10);
    println!("  Entered amount: 0.001 DASH");

    // Click the send/transaction type button.
    // For Core→Core it will be "Core Transaction".
    let tx_btn = harness
        .query_by_label_contains("Core Transaction")
        .or_else(|| harness.query_by_label_contains("Send"))
        .expect("Transaction button must be visible on send screen");
    tx_btn.click();
    harness.run_steps(10);
    println!("  Clicked transaction button");

    // Wait for any response (sending indicator, success, or error)
    let got_response = wait_until(
        harness,
        |h| {
            h.query_by_label_contains("Send Another").is_some()
                || h.query_by_label_contains("Back to Wallet").is_some()
                || h.query_by_label_contains("Dismiss").is_some()
                || h.query_by_label_contains("Sending...").is_some()
        },
        Duration::from_secs(30),
        10,
    );
    assert!(
        got_response,
        "Send screen must show a response within 30s (sending indicator, success, or error)"
    );

    // If still sending, wait for final result
    if harness.query_by_label_contains("Sending...").is_some() {
        let final_result = wait_until(
            harness,
            |h| {
                h.query_by_label_contains("Send Another").is_some()
                    || h.query_by_label_contains("Back to Wallet").is_some()
                    || h.query_by_label_contains("Dismiss").is_some()
            },
            Duration::from_secs(180),
            30,
        );
        assert!(
            final_result,
            "Send transaction must complete within 180s (stuck on 'Sending...')"
        );
    }

    // Assert success specifically — error is a test failure
    let is_success = harness.query_by_label_contains("Send Another").is_some()
        || harness.query_by_label_contains("Back to Wallet").is_some();
    assert!(
        is_success,
        "Send-to-self must succeed (got error/dismiss instead of success)"
    );
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
