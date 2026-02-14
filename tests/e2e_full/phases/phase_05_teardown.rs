use crate::helpers::context::TestContext;
use crate::helpers::harness::wait_until;
use dash_evo_tool::app::AppState;
use dash_evo_tool::spv::SpvStatus;
use egui_kittest::Harness;
use std::time::Duration;

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &TestContext) {
    // ─── 1. Stop SPV sync ──────────────────────────────────────────────
    {
        let app_ctx = harness.state().current_app_context();
        app_ctx.spv_manager.stop();
    }
    println!("  SPV sync stopped");

    // ─── 2. Wait for SPV to finish stopping ────────────────────────────
    let spv_stopped = wait_until(
        harness,
        |h| {
            let app_ctx = h.state().current_app_context();
            app_ctx.spv_manager.status().status != SpvStatus::Running
        },
        Duration::from_secs(30),
        60,
    );
    assert!(
        spv_stopped,
        "SPV should not be running after stop (still Running after 30s)"
    );
    {
        let app_ctx = harness.state().current_app_context();
        let status = app_ctx.spv_manager.status();
        println!("  SPV status after stop: {:?}", status.status);
    }

    // ─── 3. Remove test wallet from database ───────────────────────────
    if let Some(seed_hash) = &ctx.wallet_seed_hash {
        let app_ctx = harness.state().current_app_context();
        match app_ctx.remove_wallet(seed_hash) {
            Ok(()) => println!("  Removed test wallet from database"),
            Err(e) => println!("  Warning: could not remove test wallet: {}", e),
        }
    }

    // ─── 4. Log test summary ───────────────────────────────────────────
    println!();
    println!("  =======================================");
    println!("  E2E Test Suite Summary");
    println!("  =======================================");
    println!("  Network:        {}", ctx.network);
    println!(
        "  Wallet seed:    {}",
        ctx.wallet_seed_hash
            .map(|h| format!("{:x?}...", &h[..4]))
            .unwrap_or_else(|| "N/A".to_string())
    );
    println!(
        "  Balance:        {} duffs ({:.8} DASH)",
        ctx.balance_duffs,
        ctx.balance_duffs as f64 / 1e8
    );
    println!("  SPV synced:     {}", ctx.spv_synced);
    println!("  Wallet reused:  {}", ctx.wallet_reused);
    println!(
        "  Receive addr:   {}",
        ctx.receive_address.as_deref().unwrap_or("N/A")
    );
    println!("  =======================================");
    println!();

    println!("  Phase 05 complete: teardown finished");
}
