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

    // ─── Send-to-self disabled ──────────────────────────────────────────
    // The send-to-self test and post-send SPV reconciliation are disabled
    // until dash-spv has mempool support. Without it, reconciliation
    // requires a block confirmation which is too slow/unreliable for CI.
    // TODO: Re-enable when SPV mempool support lands.
    println!("  Send-to-self: SKIPPED (needs SPV mempool support)");

    // Navigate back to wallets screen for next phase
    navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);
    println!("  Phase 02 complete: wallet UI buttons verified");
}
