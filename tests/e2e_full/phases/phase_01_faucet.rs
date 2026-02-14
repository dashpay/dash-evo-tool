use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::RootScreenType;
use dash_sdk::dpp::dashcore::Network;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::Duration;

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    // 1. Navigate to wallets screen and verify wallet card
    navigate_to_screen_by_click(
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
    //    This proves the rendering pipeline works end-to-end — the wallet screen formats
    //    and displays the balance (format!(" Balance: {}", Self::format_dash(current_balance))).
    let has_balance_label = harness
        .query_all_by_label_contains("Balance:")
        .any(|node| format!("{:?}", node).contains("DASH"));
    assert!(
        has_balance_label,
        "Wallet screen must render a 'Balance:' label containing 'DASH'. \
         This means the UI rendering pipeline is broken or the wallet has no balance to display."
    );
    println!("  'Balance:' label with DASH unit found in UI");

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
