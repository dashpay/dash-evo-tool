use crate::helpers::context::TestContext;
use dash_evo_tool::app::AppState;
use dash_evo_tool::spv::SpvStatus;
use egui_kittest::Harness;

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &TestContext) {
    // ─── 1. Stop SPV sync ──────────────────────────────────────────────
    {
        let app_ctx = harness.state().current_app_context();
        app_ctx.spv_manager.stop();
    }
    println!("  SPV sync stopped");

    // Give SPV a moment to wind down
    harness.run_steps(120);

    // ─── 2. Verify SPV stopped ─────────────────────────────────────────
    {
        let app_ctx = harness.state().current_app_context();
        let status = app_ctx.spv_manager.status();
        assert_ne!(
            status.status,
            SpvStatus::Running,
            "SPV should not be running after stop"
        );
        println!("  SPV status after stop: {:?}", status.status);
    }

    // ─── 3. Log test summary ───────────────────────────────────────────
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
    println!(
        "  Receive addr:   {}",
        ctx.receive_address.as_deref().unwrap_or("N/A")
    );
    println!("  =======================================");
    println!();

    println!("  Phase 05 complete: teardown finished");
}
