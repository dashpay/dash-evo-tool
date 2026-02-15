use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::RootScreenType;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::Duration;

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    // SPV readiness gate — re-verify before building core transactions
    ensure_spv_tx_ready(harness, ctx);

    // 1. Navigate to wallets screen and verify wallet card
    navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);

    let has_wallet = wait_for_label(harness, "E2E Test Wallet", Duration::from_secs(10));
    assert!(
        has_wallet,
        "Wallet card should show 'E2E Test Wallet' alias"
    );
    println!("  Wallet card with alias visible");

    // 2. Verify Send/Receive buttons visible (wallet is already selected from Phase 0/1)
    assert!(
        harness.query_by_label("Send").is_some(),
        "Send button must be visible after selecting wallet"
    );
    verify_receive_button_visible(harness);
    println!("  Send/Receive buttons visible");

    // 3. Snapshot pre-send state for post-send verification
    {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        let wallet = wallets
            .get(ctx.seed_hash())
            .expect("Wallet not found for pre-send snapshot");
        let w = wallet.read().unwrap();
        ctx.pre_send_balance = w.max_balance();
        ctx.pre_send_tx_count = w.transactions.len();
        println!(
            "  Pre-send snapshot: balance={} duffs, tx_count={}",
            ctx.pre_send_balance, ctx.pre_send_tx_count
        );
    }

    // 4. Conditional send-to-self (requires >= 0.1 DASH)
    assert!(
        ctx.balance_duffs >= MIN_BALANCE_FOR_SEND,
        "Wallet balance ({} duffs / {:.8} DASH) is below the minimum ({} duffs / {:.8} DASH) \
         required for the send-to-self test. Fund the E2E wallet and retry.",
        ctx.balance_duffs,
        ctx.balance_duffs as f64 / 1e8,
        MIN_BALANCE_FOR_SEND,
        MIN_BALANCE_FOR_SEND as f64 / 1e8,
    );

    println!("  Attempting send-to-self (0.001 DASH)...");

    // Click "Send" button to open the send screen
    let stack_before = harness.state().screen_stack.len();
    harness
        .query_by_label("Send")
        .expect("Send button not found")
        .click();
    harness.run_steps(POLL_STEPS);
    let stack_after = harness.state().screen_stack.len();
    println!(
        "  Screen stack: {} -> {} after Send click",
        stack_before, stack_after
    );

    // The send screen should now be pushed onto the screen stack.
    let send_screen_visible = wait_for_label(harness, "Send Dash", Duration::from_secs(10));
    assert!(
        send_screen_visible,
        "Send screen must be visible after clicking Send button (screen_stack={})",
        harness.state().screen_stack.len(),
    );

    // The "Send to" label should be visible
    let send_to_visible = harness.query_by_label_contains("Send to").is_some();
    assert!(
        send_to_visible,
        "Send screen must show 'Send to' label for destination input"
    );

    // Type destination address into the first TextInput
    let addr = ctx
        .receive_address
        .as_ref()
        .expect("No receive address from Phase 01")
        .clone();
    type_into_text_input(harness, 0, &addr);
    println!("  Entered destination address");

    // Type amount into the second TextInput
    type_into_text_input(harness, 1, "0.001");
    println!("  Entered amount: 0.001 DASH");

    // Click the send/transaction type button
    let tx_btn = harness
        .query_by_label("Core Transaction")
        .expect("Core Transaction button must be visible on send screen");
    tx_btn.click();
    harness.run_steps(SETTLE_STEPS);
    println!("  Clicked transaction button");

    // Wait for the final result
    let got_final_result = wait_until(
        harness,
        |h| {
            h.query_by_label_contains("Send Another").is_some()
                || h.query_by_label_contains("Back to Wallet").is_some()
                || h.query_by_label_contains("Dismiss").is_some()
        },
        SEND_TX_TIMEOUT,
        POLL_STEPS,
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
        "Send transaction must complete within {}s",
        SEND_TX_TIMEOUT.as_secs()
    );

    // Assert success specifically — error is a test failure.
    let is_success = harness.query_by_label_contains("Send Another").is_some()
        || harness.query_by_label_contains("Back to Wallet").is_some();
    if !is_success {
        panic!("Send-to-self must succeed. Check the visible nodes above for error details.");
    }
    println!("  Send-to-self succeeded!");

    // ─── Post-send verification ────────────────────────────────────────
    // The broadcast was successful but SPV peers may not have relayed the
    // transaction back through the bloom filter yet.  Poll until the wallet
    // state reflects the send (balance decreases or tx count increases).
    println!("  Waiting for SPV to reconcile post-send state...");
    let seed_hash = *ctx.seed_hash();
    let pre_balance = ctx.pre_send_balance;
    let pre_tx_count = ctx.pre_send_tx_count;

    let reconciled = wait_until(
        harness,
        |h| {
            let app_ctx = h.state().current_app_context();
            let wallets = app_ctx.wallets.read().unwrap();
            if let Some(wallet) = wallets.get(&seed_hash) {
                let w = wallet.read().unwrap();
                w.max_balance() < pre_balance || w.transactions.len() > pre_tx_count
            } else {
                false
            }
        },
        POST_SEND_RECONCILE_TIMEOUT,
        POLL_STEPS,
    );

    // Read final state for assertions and diagnostics
    {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        let wallet = wallets
            .get(ctx.seed_hash())
            .expect("Wallet not found for post-send verification");
        let w = wallet.read().unwrap();

        let post_balance = w.max_balance();
        let post_tx_count = w.transactions.len();

        println!(
            "  Post-send: balance={} duffs (was {}), tx_count={} (was {})",
            post_balance, ctx.pre_send_balance, post_tx_count, ctx.pre_send_tx_count
        );

        assert!(
            reconciled,
            "Wallet state must update within {}s after send-to-self. \
             Balance: {} -> {} duffs, tx_count: {} -> {}. \
             SPV reconciliation may be broken.",
            POST_SEND_RECONCILE_TIMEOUT.as_secs(),
            ctx.pre_send_balance,
            post_balance,
            ctx.pre_send_tx_count,
            post_tx_count
        );

        // a) Balance must decrease (self-send returns the amount, only fee is lost)
        assert!(
            post_balance < ctx.pre_send_balance,
            "Balance must decrease after send-to-self (fee deducted). \
             Pre: {} duffs, Post: {} duffs.",
            ctx.pre_send_balance,
            post_balance
        );

        // b) Fee must be reasonable
        let fee = ctx.pre_send_balance - post_balance;
        assert!(
            fee < MAX_SEND_FEE,
            "Transaction fee is unreasonably high: {} duffs ({:.8} DASH). \
             Expected < {} duffs ({:.8} DASH).",
            fee,
            fee as f64 / 1e8,
            MAX_SEND_FEE,
            MAX_SEND_FEE as f64 / 1e8
        );
        println!("  Fee paid: {} duffs ({:.8} DASH)", fee, fee as f64 / 1e8);

        // c) Transaction count — unconfirmed txs may not appear in SPV history
        //    until the next block, so this is a soft check.  The balance decrease
        //    already proves the send worked and UTXOs reconciled.
        if post_tx_count > ctx.pre_send_tx_count {
            println!(
                "  Transaction count increased: {} -> {}",
                ctx.pre_send_tx_count, post_tx_count
            );

            // d) Verify newest transaction is outgoing.
            //    The transactions vector order depends on the SDK, not position,
            //    so find the newest by timestamp rather than using .last().
            if let Some(newest_tx) = w.transactions.iter().max_by_key(|tx| tx.timestamp) {
                assert!(
                    newest_tx.is_outgoing(),
                    "Newest transaction (by timestamp) must be outgoing for a send-to-self. \
                     net_amount={}, timestamp={}, txid={}",
                    newest_tx.net_amount,
                    newest_tx.timestamp,
                    newest_tx.txid
                );
                println!(
                    "  Newest tx: net_amount={} duffs (outgoing={})",
                    newest_tx.net_amount,
                    newest_tx.is_outgoing()
                );
            }
        } else {
            println!(
                "  Transaction count unchanged ({}) — unconfirmed tx not yet in SPV history (expected)",
                post_tx_count
            );
        }

        // e) Update ctx.balance_duffs for subsequent phases
        ctx.balance_duffs = w.total_balance_duffs();
    }

    // Click "Back to Wallet" to return
    if let Some(back_btn) = harness.query_by_label_contains("Back to Wallet") {
        back_btn.click();
        harness.run_steps(SETTLE_STEPS);
    }

    // Navigate back to wallets screen for next phase
    navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);
    println!("  Phase 02 complete: wallet UI operations verified with post-send assertions");
}
