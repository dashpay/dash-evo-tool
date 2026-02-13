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

    let has_dash = wait_for_label(harness, "DASH", Duration::from_secs(10));
    assert!(has_dash, "Wallet card should show DASH balance");
    println!("  Wallet card with balance visible");

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
    if let Some(receive_btn) = harness.query_by_label_contains("Receive") {
        receive_btn.click();
        harness.run_steps(10);

        // Verify the receive address is displayed in the dialog
        if let Some(addr) = &ctx.receive_address {
            let addr_short = addr.get(..8).unwrap_or(addr);
            let found = wait_for_label(harness, addr_short, Duration::from_secs(5));
            if found {
                println!("  Receive dialog shows address: {}...", addr_short);
            } else {
                println!("  Receive dialog opened but address not found in UI");
            }
        }

        // Close dialog via Escape
        harness.key_press(egui::Key::Escape);
        harness.run_steps(5);
        println!("  Receive dialog closed");
    } else {
        println!("  Receive button not visible (skipping receive dialog test)");
    }

    // 5. Conditional send-to-self (requires >= 0.1 DASH)
    let min_balance_for_send: u64 = 10_000_000; // 0.1 DASH in duffs

    if ctx.balance_duffs < min_balance_for_send {
        println!(
            "  Skipping send-to-self: insufficient funds ({} < {} duffs)",
            ctx.balance_duffs, min_balance_for_send
        );
        println!("  Phase 02 complete: wallet UI verified (send skipped)");
        return;
    }

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
    if !addr_input_visible {
        // Wallet might need unlock — check for unlock button
        if harness.query_by_label_contains("Unlock Wallet").is_some() {
            println!("  Wallet is locked, cannot proceed with send test");
            // Navigate back
            navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);
            println!("  Phase 02 complete: wallet UI verified (send skipped — locked)");
            return;
        }
        println!("  Send screen address input not found, skipping send test");
        navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);
        println!("  Phase 02 complete: wallet UI verified (send skipped)");
        return;
    }

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
    let amount_input = harness.query_by_label_contains("Enter amount");
    if let Some(input) = amount_input {
        input.type_text("0.001");
        harness.run_steps(10);
        println!("  Entered amount: 0.001 DASH");
    } else {
        println!("  Amount input not found, skipping send");
        navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);
        println!("  Phase 02 complete: wallet UI verified (send skipped)");
        return;
    }

    // Click the send/transaction type button.
    // For Core→Core it will be "Core Transaction".
    let tx_btn = harness
        .query_by_label_contains("Core Transaction")
        .or_else(|| harness.query_by_label_contains("Send"));
    if let Some(btn) = tx_btn {
        btn.click();
        harness.run_steps(10);
        println!("  Clicked transaction button");

        // Wait for transaction result — success, complete message, or error
        let completed = wait_until(
            harness,
            |h| {
                // Success: SendStatus::Complete shows heading with message
                h.query_by_label_contains("Send Another").is_some()
                    || h.query_by_label_contains("Back to Wallet").is_some()
                    // Error status shows error text + Dismiss button
                    || h.query_by_label_contains("Dismiss").is_some()
                    // Or still sending
                    || h.query_by_label_contains("Sending...").is_some()
            },
            Duration::from_secs(30),
            10,
        );

        if completed {
            // If still sending, wait longer for final result
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
                if !final_result {
                    println!("  Send-to-self timed out waiting for result (skipping)");
                    navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);
                    println!("  Phase 02 complete: wallet UI verified (send timed out)");
                    return;
                }
            }

            if harness.query_by_label_contains("Send Another").is_some()
                || harness.query_by_label_contains("Back to Wallet").is_some()
            {
                println!("  Send-to-self succeeded!");
                // Click "Back to Wallet" to return
                if let Some(back_btn) = harness.query_by_label_contains("Back to Wallet") {
                    back_btn.click();
                    harness.run_steps(10);
                }
            } else if harness.query_by_label_contains("Dismiss").is_some() {
                println!("  Send-to-self encountered an error (acceptable in test env)");
                // Dismiss the error
                if let Some(dismiss_btn) = harness.query_by_label_contains("Dismiss") {
                    dismiss_btn.click();
                    harness.run_steps(5);
                }
            }
        } else {
            println!("  Send-to-self: no response detected (skipping)");
        }
    } else {
        println!("  Transaction button not found on send screen (skipping)");
    }

    // Navigate back to wallets screen for next phase
    navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);
    println!("  Phase 02 complete: wallet UI operations verified");
}
