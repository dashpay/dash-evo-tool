use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::RootScreenType;
use dash_sdk::dpp::dashcore::Network;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::Duration;

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    // 1. Navigate to wallets screen and verify sidebar label
    verify_sidebar_label_and_navigate(
        harness,
        "Wallets",
        RootScreenType::RootScreenWalletsBalances,
    );

    let has_wallet_label = wait_for_label(harness, "E2E Test Wallet", Duration::from_secs(10));
    assert!(
        has_wallet_label,
        "Wallet card should show 'E2E Test Wallet' alias"
    );
    println!("  UI shows wallet card with alias");

    // 2. Verify the wallet screen renders a balance label containing "Balance:" and "DASH".
    //    SPV runs continuously in the background and can update the wallet balance at any
    //    time via reconciliation callbacks. To avoid a race between the rendered UI and our
    //    wallet read, we: run a few frames to get a fresh render, immediately parse the UI
    //    balance, then immediately read the wallet balance — minimizing the time window.
    harness.run_steps(5); // fresh render
    use egui_kittest::kittest::NodeT;
    let balance_label = harness
        .query_all_by_label_contains("Balance:")
        .find_map(|node| {
            let label = node.accesskit_node().label()?;
            if label.contains("DASH") { Some(label) } else { None }
        })
        .expect(
            "Wallet screen must render a 'Balance:' label containing 'DASH'. \
             This means the UI rendering pipeline is broken or the wallet has no balance to display.",
        );

    // Parse numeric value from the label text.
    // The label format is: " Balance: X.XXXXXXXX DASH"
    // We extract the substring between "Balance:" and "DASH".
    let ui_balance: f64 = {
        let start = balance_label
            .find("Balance:")
            .expect("Balance: prefix not found in label");
        let after_prefix = &balance_label[start + "Balance:".len()..];
        let end = after_prefix
            .find("DASH")
            .expect("DASH suffix not found in label");
        after_prefix[..end]
            .trim()
            .parse::<f64>()
            .expect("Could not parse balance value as a number")
    };

    // Read the wallet's live balance immediately after parsing the UI —
    // both should reflect the same SPV state since no frames ran between them.
    let live_balance_duffs = {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        let wallet = wallets
            .get(ctx.seed_hash())
            .expect("wallet not found by seed hash (phase 01 - did phase 00 import succeed?)");
        wallet.read().unwrap().total_balance_duffs()
    };
    ctx.balance_duffs = live_balance_duffs;
    let expected_balance = live_balance_duffs as f64 / 1e8;

    // Tolerance: SPV can still update between UI render and our read (background thread),
    // so allow up to 1000 duffs (0.00001 DASH) of drift beyond floating-point rounding.
    assert!(
        (ui_balance - expected_balance).abs() < 0.00001,
        "UI balance ({} DASH) doesn't match wallet balance ({} DASH / {} duffs)",
        ui_balance,
        expected_balance,
        live_balance_duffs
    );
    println!(
        "  Balance value verified: {:.8} DASH matches wallet state ({:.8} DASH)",
        ui_balance, expected_balance
    );

    // 3. Get receive address and verify it's a valid testnet P2PKH address (starts with 'y')
    {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        let wallet = wallets
            .get(ctx.seed_hash())
            .expect("wallet not found by seed hash (phase 01 - receive address)");
        let addr = wallet
            .write()
            .unwrap()
            .receive_address(Network::Testnet, false, None)
            .expect("Failed to get receive address from imported wallet");
        ctx.receive_address = Some(addr.to_string());
    }
    let addr_str = ctx.receive_address.as_deref().unwrap();
    assert!(
        addr_str.starts_with('y'),
        "Testnet P2PKH receive address must start with 'y', got: {}",
        addr_str
    );
    println!("  Receive address: {} (valid testnet prefix)", addr_str);

    // 4. Verify Receive button is visible (proves wallet is selected)
    verify_receive_button_visible(harness);

    println!("  Phase 01 complete: wallet UI renders balance, receive address valid");
}
