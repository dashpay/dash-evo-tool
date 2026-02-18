use dash_evo_tool::app::AppState;
use dash_evo_tool::spv::SpvStatus;
use egui_kittest::Harness;

/// Read-only smoke tests that verify the app boots correctly.
/// Runs BEFORE phase_00_setup — no TestContext needed.
pub fn run(harness: &mut Harness<'_, AppState>) {
    // 1. Harness is usable (AppState::new() succeeded)
    harness.run_steps(10);
    println!("  Harness usable: OK");

    // 2. AppContext is accessible
    let app_ctx = harness.state().current_app_context();
    println!("  AppContext valid: OK");

    // 3. SPV is idle or stopped at boot (not actively syncing)
    let spv_status = app_ctx.spv_manager().status().status;
    assert!(
        matches!(spv_status, SpvStatus::Idle | SpvStatus::Stopped),
        "SPV should be Idle or Stopped at boot, got: {:?}",
        spv_status
    );
    println!("  SPV idle at boot: {:?}", spv_status);

    // 4. Wallets lock is accessible (no deadlock)
    let wallet_count = app_ctx.wallets().read().unwrap().len();
    println!("  Wallets lock accessible: {} wallet(s)", wallet_count);

    // 5. Network is readable
    let network = harness.state().chosen_network;
    println!("  Network readable: {:?}", network);

    // 6. Welcome screen consistency: if show_welcome_screen, then welcome_screen.is_some()
    let show = harness.state().show_welcome_screen;
    let has_screen = harness.state().welcome_screen.is_some();
    if show {
        assert!(
            has_screen,
            "show_welcome_screen is true but welcome_screen is None"
        );
    }
    println!(
        "  Welcome screen consistent: show={}, present={}",
        show, has_screen
    );

    // 7. Testnet context exists (required for E2E)
    assert!(
        harness.state().testnet_app_context.is_some(),
        "Testnet AppContext must exist for E2E tests"
    );
    println!("  Testnet context exists: OK");

    println!("  Smoke tests passed!");
}
