use crate::helpers::context::TestContext;
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::spv::SpvStatus;
use dash_evo_tool::ui::{RootScreenType, Screen};
use dash_sdk::dpp::dashcore::Network;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::time::{Duration, Instant};

pub fn run(
    harness: &mut Harness<'_, AppState>,
    ctx: &mut TestContext,
    _rt: &tokio::runtime::Runtime,
) {
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

    // 4. Navigate to wallets screen
    navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);
    println!("  On wallets screen");

    // 5. Click "Import Wallet" button
    let found = wait_for_label(harness, "Import Wallet", Duration::from_secs(5));
    assert!(found, "Import Wallet button not found on wallets screen");
    harness
        .query_by_label_contains("Import Wallet")
        .expect("Import Wallet button should be visible")
        .click();
    harness.run_steps(10);
    println!("  Clicked Import Wallet");

    // 6. Handle word count selector if needed (default is 12)
    if words.len() != 12 {
        // Access ImportMnemonicScreen on the stack and set length directly,
        // since ComboBox interaction via AccessKit is unreliable.
        if let Some(Screen::ImportMnemonicScreen(screen)) =
            harness.state_mut().screen_stack.last_mut()
        {
            screen.set_seed_phrase_length(words.len());
        } else {
            panic!("Expected ImportMnemonicScreen on screen stack");
        }
        harness.run_steps(5);
        println!("  Set seed phrase length to {}", words.len());
    }

    // 7. Type mnemonic words into input fields
    // Each input has hint_text "Word N" from Phase 2 AccessKit labels.
    // We iterate sequentially: after typing into "Word 1", its accessible
    // name changes from the hint to the typed text, so subsequent queries
    // for "Word 2" etc. won't accidentally match filled fields.
    for (i, word) in words.iter().enumerate() {
        let label = format!("Word {}", i + 1);
        let found = wait_for_label(harness, &label, Duration::from_secs(5));
        assert!(found, "Seed word input '{}' not found", label);

        harness
            .query_by_label_contains(&label)
            .unwrap_or_else(|| panic!("Seed word input '{}' should be queryable", label))
            .type_text(word);
        harness.run_steps(2);
    }
    // Extra steps for seed phrase validation to complete
    harness.run_steps(10);
    println!("  Entered all {} mnemonic words", words.len());

    // 8. Set wallet alias
    let found = wait_for_label(harness, "Wallet name", Duration::from_secs(5));
    assert!(found, "Wallet name input not found");
    harness
        .query_by_label_contains("Wallet name")
        .expect("Wallet alias input should be visible")
        .type_text("E2E Test Wallet");
    harness.run_steps(5);
    println!("  Set wallet alias to 'E2E Test Wallet'");

    // 9. Click save button
    // Capture wallet count before save so we detect the new wallet specifically,
    // not a leftover from a previous test run.
    let initial_wallet_count = {
        let app_ctx = harness.state().current_app_context();
        app_ctx.wallets.read().unwrap().len()
    };

    let found = wait_for_label(harness, "Save Wallet", Duration::from_secs(5));
    assert!(found, "Save Wallet button not found");
    harness
        .query_by_label_contains("Save Wallet")
        .expect("Save Wallet button should be visible")
        .click();
    harness.run_steps(10);
    println!("  Clicked Save Wallet");

    // 10. Wait for NEW wallet to appear in AppContext
    let wallet_imported = wait_until(
        harness,
        |h| {
            let app_ctx = h.state().current_app_context();
            let wallets = app_ctx.wallets.read().unwrap();
            wallets.len() > initial_wallet_count
        },
        Duration::from_secs(60),
        10,
    );
    assert!(
        wallet_imported,
        "Wallet was not imported within 60s (count stayed at {})",
        initial_wallet_count
    );

    // Save the seed hash to TestContext
    {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        if let Some((hash, _)) = wallets.iter().next() {
            ctx.wallet_seed_hash = Some(*hash);
        }
    }
    println!(
        "  Wallet imported. Seed hash prefix: {:?}",
        ctx.wallet_seed_hash
            .map(|h| format!("{:02x}{:02x}{:02x}{:02x}", h[0], h[1], h[2], h[3]))
    );

    // Navigate back to wallets screen (dismiss the success screen)
    navigate_to_screen(harness, RootScreenType::RootScreenWalletsBalances);

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
        if let Some((_, wallet)) = wallets.iter().next() {
            let w = wallet.read().unwrap();
            ctx.balance_duffs = w.total_balance_duffs();
        }
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
        "  Setup complete. Balance: {} duffs ({:.8} DASH)",
        ctx.balance_duffs,
        ctx.balance_duffs as f64 / 1e8
    );
}
