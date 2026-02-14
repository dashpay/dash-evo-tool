use crate::helpers::context::{TestContext, seed_hash_prefix};
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_evo_tool::spv::SpvStatus;
use dash_evo_tool::ui::{Screen, ScreenType};
use dash_sdk::dpp::dashcore::Network;
use egui_kittest::Harness;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

const E2E_WALLET_ALIAS: &str = "E2E Test Wallet";

/// Check if a wallet with the E2E alias already exists that we can reuse.
/// Only matches by exact alias — never grabs unrelated wallets.
fn find_existing_e2e_wallet(harness: &Harness<'_, AppState>) -> Option<WalletSeedHash> {
    let app_ctx = harness.state().current_app_context();
    let wallets = app_ctx.wallets.read().unwrap();
    for (seed_hash, wallet) in wallets.iter() {
        let w = wallet.read().unwrap();
        if w.alias.as_deref() == Some(E2E_WALLET_ALIAS) {
            return Some(*seed_hash);
        }
    }
    None
}

/// Import a wallet by pushing ImportMnemonicScreen and setting values directly.
/// AccessKit interactions (button clicks, text input via hint_text) are unreliable
/// in egui_kittest, so we manipulate the screen state programmatically.
fn import_wallet_via_ui(
    harness: &mut Harness<'_, AppState>,
    ctx: &mut TestContext,
    words: &[&str],
) {
    // Capture wallet keys before import so we can diff to find the new one
    let initial_wallet_keys: BTreeSet<WalletSeedHash> = {
        let app_ctx = harness.state().current_app_context();
        app_ctx.wallets.read().unwrap().keys().copied().collect()
    };

    // Push ImportMnemonicScreen and configure it directly
    {
        let app_ctx = harness.state().current_app_context();
        let screen = ScreenType::ImportMnemonic.create_screen(app_ctx);
        harness.state_mut().screen_stack.push(screen);
    }
    harness.run_steps(5);

    if let Some(Screen::ImportMnemonicScreen(screen)) = harness.state_mut().screen_stack.last_mut()
    {
        screen.set_seed_phrase_words(words);
        screen.set_alias(E2E_WALLET_ALIAS);
        println!(
            "  Set {} mnemonic words + alias '{}'",
            words.len(),
            E2E_WALLET_ALIAS
        );

        screen
            .trigger_save()
            .unwrap_or_else(|e| panic!("Wallet import failed: {}", e));
        println!("  Wallet saved to DB");
    } else {
        panic!("Expected ImportMnemonicScreen on screen stack");
    }

    // Let the UI process the save
    harness.run_steps(10);

    // Verify wallet appeared in AppContext
    {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        assert!(
            wallets.len() > initial_wallet_keys.len(),
            "Wallet count didn't increase after save (still {})",
            initial_wallet_keys.len()
        );
        let current_keys: BTreeSet<WalletSeedHash> = wallets.keys().copied().collect();
        let new_keys: Vec<_> = current_keys.difference(&initial_wallet_keys).collect();
        assert!(
            new_keys.len() == 1,
            "Expected exactly 1 new wallet after import, found {}",
            new_keys.len()
        );
        ctx.wallet_seed_hash = Some(*new_keys[0]);
    }
    println!(
        "  Wallet imported. Seed hash prefix: {}",
        seed_hash_prefix(ctx.seed_hash())
    );

    // Pop the import screen
    harness.state_mut().screen_stack.pop();
    harness.run_steps(5);
}

pub fn run(harness: &mut Harness<'_, AppState>, ctx: &mut TestContext) {
    // 1. Read & validate mnemonic
    let mnemonic =
        std::env::var("E2E_WALLET_MNEMONIC").expect("E2E_WALLET_MNEMONIC env var required");
    let words: Vec<&str> = mnemonic.split_whitespace().collect();
    assert!(
        [12, 15, 18, 21, 24].contains(&words.len()),
        "Mnemonic has {} words, expected 12/15/18/21/24",
        words.len()
    );
    println!("  Mnemonic: {} words", words.len());

    // 2. Dismiss welcome screen
    dismiss_welcome_screen(harness);
    harness.run_steps(10);

    // 3. Switch to testnet
    harness.state_mut().change_network(Network::Testnet);
    harness.run_steps(10);
    println!("  Switched to testnet");

    // 4. Check if wallet already exists (idempotent re-run support)
    if let Some(seed_hash) = find_existing_e2e_wallet(harness) {
        ctx.wallet_seed_hash = Some(seed_hash);
        ctx.wallet_reused = true;
        println!(
            "  Reusing existing wallet. Seed hash prefix: {}",
            seed_hash_prefix(&seed_hash)
        );
    } else {
        // 5–10. Import wallet via UI
        import_wallet_via_ui(harness, ctx, &words);
    }

    // 11. Start SPV sync
    {
        let app_ctx = harness.state().current_app_context().clone();
        let wallet_count = app_ctx.wallets.read().unwrap().len();
        app_ctx
            .spv_manager
            .start(wallet_count)
            .expect("SPV start failed");
    }
    println!("  SPV sync started, waiting for completion...");

    // 12. Poll for SPV Running status
    let timeout_secs: u64 = std::env::var("E2E_SPV_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1800); // 30 min default

    const MAX_SPV_RETRIES: u32 = 3;

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let mut retry_count: u32 = 0;
    let mut spv_synced = false;

    while start.elapsed() < timeout {
        harness.run_steps(60); // ~1s at 60fps

        let status = {
            let app_ctx = harness.state().current_app_context();
            app_ctx.spv_manager.status()
        };

        match status.status {
            SpvStatus::Running => {
                spv_synced = true;
                break;
            }
            SpvStatus::Error => {
                let err_msg = status.last_error.as_deref().unwrap_or("unknown");
                println!("  SPV error detected: {}", err_msg);

                if retry_count < MAX_SPV_RETRIES {
                    retry_count += 1;
                    println!(
                        "  Retrying SPV sync ({}/{})...",
                        retry_count, MAX_SPV_RETRIES
                    );
                    let app_ctx = harness.state().current_app_context().clone();
                    app_ctx.spv_manager.stop();
                    harness.run_steps(120); // ~2s cooldown (non-blocking)
                    let wallet_count = app_ctx.wallets.read().unwrap().len();
                    app_ctx
                        .spv_manager
                        .start(wallet_count)
                        .unwrap_or_else(|e| panic!("SPV restart failed: {}", e));
                }
            }
            _ => {}
        }
    }

    assert!(
        spv_synced,
        "SPV sync did not reach Running within {}s (retries: {}/{})",
        timeout_secs, retry_count, MAX_SPV_RETRIES
    );
    ctx.spv_synced = true;
    println!("  SPV sync complete!");

    // 13. Save wallet balance to context and validate
    {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        let wallet = wallets
            .get(ctx.seed_hash())
            .expect("Wallet not found by seed hash after SPV sync");
        ctx.balance_duffs = wallet.read().unwrap().total_balance_duffs();
    }

    // Minimum balance required for identity operations in later phases
    const MIN_BALANCE_DUFFS: u64 = 100_000; // 0.001 DASH
    assert!(
        ctx.balance_duffs >= MIN_BALANCE_DUFFS,
        "Wallet balance ({} duffs / {:.8} DASH) is below the minimum ({} duffs) \
         required for E2E tests. Please fund the wallet.",
        ctx.balance_duffs,
        ctx.balance_duffs as f64 / 1e8,
        MIN_BALANCE_DUFFS,
    );

    println!(
        "  Setup complete. Balance: {} duffs ({:.8} DASH){}",
        ctx.balance_duffs,
        ctx.balance_duffs as f64 / 1e8,
        if ctx.wallet_reused { " (reused)" } else { "" }
    );
}
