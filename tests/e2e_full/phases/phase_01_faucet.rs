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
        match w.receive_address(Network::Testnet, true, Some(app_ctx)) {
            Ok(addr) => ctx.receive_address = Some(addr.to_string()),
            Err(e) => println!("  Warning: could not get receive address: {}", e),
        }
    }
    println!(
        "  Receive address: {}",
        ctx.receive_address.as_deref().unwrap_or("N/A")
    );

    // 3. Navigate to wallets screen and verify balance in UI
    navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);

    let has_dash_label = wait_for_label(harness, "DASH", Duration::from_secs(10));
    assert!(
        has_dash_label,
        "Wallet should show 'DASH' in balance display"
    );
    println!("  UI shows DASH balance");

    // 4. Open receive dialog via UI if button is available
    if let Some(receive_btn) = harness.query_by_label_contains("Receive") {
        receive_btn.click();
        harness.run_steps(10);

        if let Some(addr) = &ctx.receive_address {
            let addr_short = addr.get(..8).unwrap_or(addr);
            let found = wait_for_label(harness, addr_short, Duration::from_secs(5));
            if found {
                println!("  Receive dialog shows address: {}...", addr_short);
            }
        }

        // Dismiss the dialog
        harness.key_press(egui::Key::Escape);
        harness.run_steps(5);
    }

    println!("  Phase 01 complete: balance verified, receive address obtained");
}
