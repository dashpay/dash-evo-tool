use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::RootScreenType;
use dash_sdk::dpp::dashcore::Network;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::Duration;

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    // 1. Check balance via AppContext (programmatic)
    let balance = {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        let seed_hash = ctx
            .wallet_seed_hash
            .as_ref()
            .expect("No wallet seed hash from Phase 0");
        let wallet = wallets
            .get(seed_hash)
            .expect("Wallet not found in AppContext");
        let w = wallet.read().unwrap();
        w.total_balance_duffs()
    };

    assert!(
        balance > 0,
        "Wallet balance is 0. E2E_WALLET_MNEMONIC must point to a pre-funded testnet wallet. \
         Fund it at https://faucet.thepasta.org and retry."
    );
    ctx.balance_duffs = balance;
    println!(
        "  Wallet balance: {} duffs ({:.8} DASH)",
        balance,
        balance as f64 / 1e8
    );

    // 2. Get receive address (programmatic)
    {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        let seed_hash = ctx.wallet_seed_hash.as_ref().unwrap();
        let wallet = wallets.get(seed_hash).unwrap();
        let mut w = wallet.write().unwrap();
        let addr = w
            .receive_address(Network::Testnet, true, Some(app_ctx))
            .expect("Failed to get receive address from imported wallet");
        ctx.receive_address = Some(addr.to_string());
    }
    println!(
        "  Receive address: {}",
        ctx.receive_address.as_deref().unwrap_or("N/A")
    );

    // 3. Navigate to wallets screen via sidebar click and verify balance in UI
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

    // 4. Open receive dialog and verify address display
    let receive_btn = harness
        .query_by_label_contains("Receive")
        .expect("Receive button must be visible on wallets screen");
    receive_btn.click();
    harness.run_steps(10);

    let addr = ctx
        .receive_address
        .as_ref()
        .expect("Receive address must be set by this point");
    let addr_short = addr.get(..8).unwrap_or(addr);
    let found = wait_for_label(harness, addr_short, Duration::from_secs(5));
    assert!(
        found,
        "Receive dialog must display wallet address (expected prefix: {})",
        addr_short
    );
    println!("  Receive dialog shows address: {}...", addr_short);

    // Dismiss the dialog and verify it closed
    harness.key_press(egui::Key::Escape);
    harness.run_steps(5);
    let dismissed = wait_for_label_gone(harness, addr_short, Duration::from_secs(5));
    assert!(dismissed, "Receive dialog must close after pressing Escape");

    println!("  Phase 01 complete: balance verified, receive address obtained");
}
