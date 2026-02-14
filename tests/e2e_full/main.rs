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
    let mnemonic = std::env::var("E2E_WALLET_MNEMONIC")
        .expect("E2E_WALLET_MNEMONIC env var required (BIP39 testnet mnemonic, pre-funded)");
    let word_count = mnemonic.split_whitespace().count();
    assert!(
        [12, 15, 18, 21, 24].contains(&word_count),
        "E2E_WALLET_MNEMONIC has {word_count} words, expected 12/15/18/21/24"
    );

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let _guard = rt.enter();
    let mut harness = helpers::harness::create_e2e_harness(&rt);
    let mut ctx = helpers::context::TestContext::default();

    println!("\n=== Smoke: App Initialization ===");
    phases::phase_smoke::run(&mut harness);

    println!("\n=== Phase 0: Setup (Wallet Import + SPV Sync) ===");
    phases::phase_00_setup::run(&mut harness, &mut ctx, &rt);

    println!("\n=== Phase 1: Balance Verification ===");
    phases::phase_01_faucet::run(&mut harness, &mut ctx);

    println!("\n=== Phase 2: Wallet UI Operations ===");
    phases::phase_02_wallet::run(&mut harness, &mut ctx);

    println!("\n=== Phase 3: Platform Reads ===");
    phases::phase_03_platform::run(&mut harness, &mut ctx);

    println!("\n=== Phase 4: Token Search ===");
    phases::phase_04_tokens::run(&mut harness, &mut ctx);

    println!("\n=== Phase 5: Teardown ===");
    phases::phase_05_teardown::run(&mut harness, &ctx);
}
