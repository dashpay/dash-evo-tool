use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::{RootScreenType, Screen};
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

    // 2. Select wallet — the wallet should already be selected from Phase 0/1,
    //    but verify by checking the combo box shows our alias
    let has_wallet = wait_for_label(harness, "E2E Test Wallet", Duration::from_secs(5));
    if !has_wallet {
        // Try clicking the wallet selector to find our wallet
        println!("  Wallet not pre-selected, attempting to select...");
        // Set it programmatically via the wallets screen's selected wallet
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        if let Some(seed_hash) = &ctx.wallet_seed_hash
            && let Some(wallet_arc) = wallets.get(seed_hash)
        {
            let wallet_clone = wallet_arc.clone();
            drop(wallets);
            if let Some(Screen::WalletsBalancesScreen(screen)) = harness
                .state_mut()
                .main_screens
                .get_mut(&RootScreenType::RootScreenWalletsBalances)
            {
                screen.select_hd_wallet(wallet_clone);
            }
            harness.run_steps(10);
        }
    }
    println!("  Wallet selected");

    // 3. Verify Send/Receive buttons visible
    let send_visible = harness.query_by_label_contains("Send").is_some();
    let receive_visible = harness.query_by_label_contains("Receive").is_some();
    assert!(
        send_visible || receive_visible,
        "Send and/or Receive buttons should be visible after selecting wallet"
    );
    println!("  Send/Receive buttons visible");

    // 4. Open receive dialog and verify address
    let receive_btn = harness
        .query_by_label_contains("Receive")
        .expect("Receive button must be visible on wallets screen");
    receive_btn.click();
    harness.run_steps(10);

    let addr = ctx
        .receive_address
        .as_ref()
        .expect("Receive address must be set by Phase 01");
    let addr_short = addr.get(..8).unwrap_or(addr);
    let found = wait_for_label(harness, addr_short, Duration::from_secs(5));
    assert!(
        found,
        "Receive dialog must display wallet address (expected prefix: {})",
        addr_short
    );
    println!("  Receive dialog shows address: {}...", addr_short);

    // Close dialog via Escape
    harness.key_press(egui::Key::Escape);
    harness.run_steps(5);
    println!("  Receive dialog closed");

    // 5. Conditional send-to-self (requires >= 0.1 DASH)
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
    let send_btn = harness.query_by_label_contains("Send");
    assert!(send_btn.is_some(), "Send button not found");
    send_btn.unwrap().click();
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
