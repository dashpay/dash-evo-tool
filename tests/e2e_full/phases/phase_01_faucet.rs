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

    // 2. Verify the wallet screen renders a balance label containing "Balance:" and "DASH",
    //    then parse and verify the actual balance value matches ctx.balance_duffs.
    let balance_text = harness
        .query_all_by_label_contains("Balance:")
        .find_map(|node| {
            let s = format!("{:?}", node);
            if s.contains("DASH") { Some(s) } else { None }
        })
        .expect(
            "Wallet screen must render a 'Balance:' label containing 'DASH'. \
             This means the UI rendering pipeline is broken or the wallet has no balance to display.",
        );

    // Parse numeric value from the label text.
    // The label format is: " Balance: X.XXXXXXXX DASH"
    // We extract the substring between "Balance:" and "DASH".
    let ui_balance: f64 = {
        let start = balance_text
            .find("Balance:")
            .expect("Balance: prefix not found in label");
        let after_prefix = &balance_text[start + "Balance:".len()..];
        let end = after_prefix
            .find("DASH")
            .expect("DASH suffix not found in label");
        after_prefix[..end]
            .trim()
            .parse::<f64>()
            .expect("Could not parse balance value as a number")
    };
    let expected_balance = ctx.balance_duffs as f64 / 1e8;

    // Allow small floating-point tolerance
    assert!(
        (ui_balance - expected_balance).abs() < 0.00000002,
        "UI balance ({} DASH) doesn't match wallet balance ({} DASH / {} duffs)",
        ui_balance,
        expected_balance,
        ctx.balance_duffs
    );
    println!(
        "  Balance value verified: {:.8} DASH matches wallet state",
        ui_balance
    );

    // 3. Get receive address and verify it's a valid testnet P2PKH address (starts with 'y')
    {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        let wallet = wallets.get(ctx.seed_hash()).unwrap();
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
