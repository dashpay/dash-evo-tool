use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::ui::RootScreenType;
use dash_sdk::dpp::dashcore::Network;
use egui_kittest::Harness;
use std::time::Duration;

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    // 1. Check balance via AppContext (programmatic)
    let balance = {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        let wallet = wallets
            .get(ctx.seed_hash())
            .expect("Wallet not found in AppContext");
        wallet.read().unwrap().total_balance_duffs()
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
        let wallet = wallets.get(ctx.seed_hash()).unwrap();
        let addr = wallet
            .write()
            .unwrap()
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
    let addr = ctx
        .receive_address
        .as_ref()
        .expect("Receive address must be set by this point");
    open_and_verify_receive_dialog(harness, addr);

    println!("  Phase 01 complete: balance verified, receive address obtained");
}
