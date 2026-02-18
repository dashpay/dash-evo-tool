#![allow(dead_code)]

mod helpers;
mod phases;

/// Full E2E test against real testnet.
///
/// Required: E2E_WALLET_MNEMONIC env var (BIP39 testnet mnemonic)
/// The wallet must be pre-funded with at least 0.1 DASH.
///
/// Run: E2E_WALLET_MNEMONIC="word1 word2 ..." cargo test --test e2e_full -- --ignored --nocapture
#[test]
#[ignore]
fn e2e_full_testnet_journey() {
    // Validate presence early so we fail fast before booting the app
    std::env::var("E2E_WALLET_MNEMONIC")
        .expect("E2E_WALLET_MNEMONIC env var required (BIP39 testnet mnemonic, pre-funded)");

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();
    let mut harness = helpers::harness::create_e2e_harness(&rt);
    let mut ctx = helpers::context::TestContext::default();

    let test_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        println!("\n=== Smoke: App Initialization ===");
        phases::phase_smoke::run(&mut harness);

        println!("\n=== Phase 0: Setup (Wallet Import + SPV Sync) ===");
        phases::phase_00_setup::run(&mut harness, &mut ctx);

        println!("\n=== Phase 1: Wallet UI + Balance Display ===");
        phases::phase_01_faucet::run(&mut harness, &mut ctx);

        println!("\n=== Phase 2: Wallet UI Operations ===");
        phases::phase_02_wallet::run(&mut harness, &mut ctx);

        println!("\n=== Phase 3: Platform Reads ===");
        phases::phase_03_platform::run(&mut harness, &mut ctx);

        println!("\n=== Phase 4: Token Search ===");
        phases::phase_04_tokens::run(&mut harness, &mut ctx);

        println!("\n=== Phase 5: Identity Validation ===");
        phases::phase_05_identity::run(&mut harness, &mut ctx);

        // Phase 6 (DPNS) skipped — Phase 5 runs validation tests but actual
        // identity creation is disabled until SPV mempool support lands,
        // so no identity_id is produced for DPNS registration.
        println!("\n=== Phase 6: DPNS Name Registration — SKIPPED ===");

        println!("\n=== Phase 7: Teardown ===");
        phases::phase_07_teardown::run(&mut harness, &ctx);
    }));

    if let Err(payload) = test_result {
        eprintln!("\n=== PANIC: Running emergency cleanup ===");
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            helpers::harness::emergency_cleanup(&harness, &ctx);
        }));
        std::panic::resume_unwind(payload);
    }
}
