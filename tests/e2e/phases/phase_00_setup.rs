use crate::helpers::context::{TestContext, seed_hash_prefix};
use crate::helpers::harness::*;
use dash_evo_tool::app::AppState;
use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_evo_tool::spv::CoreBackendMode;
use dash_evo_tool::spv::SpvStatus;
use dash_evo_tool::ui::{Screen, ScreenType};
use dash_sdk::dash_spv::sync::ProgressPercentage;
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
    push_screen(harness, ScreenType::ImportMnemonic);

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
    harness.run_steps(SETTLE_STEPS);

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
        assert_eq!(
            new_keys.len(),
            1,
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
    harness.run_steps(SETTLE_STEPS);

    // 3. Switch to testnet and enable SPV mode
    harness.state_mut().change_network(Network::Testnet);
    harness.run_steps(SETTLE_STEPS);
    let app_ctx = harness.state().current_app_context().clone();
    app_ctx.set_core_backend_mode(CoreBackendMode::Spv);
    println!("  Switched to testnet (SPV mode)");

    // 4. Check if wallet already exists (idempotent re-run support)
    if let Some(seed_hash) = find_existing_e2e_wallet(harness) {
        ctx.wallet_seed_hash = Some(seed_hash);
        ctx.wallet_reused = true;
        println!(
            "  Reusing existing wallet. Seed hash prefix: {}",
            seed_hash_prefix(&seed_hash)
        );

        // Unlock the wallet so SPV can register its addresses.
        // Wallets loaded from DB are in a closed state — SPV needs the
        // seed bytes to build the bloom filter and discover transactions.
        let app_ctx = harness.state().current_app_context().clone();
        let wallet_arc = {
            let wallets = app_ctx.wallets.read().unwrap();
            wallets.get(&seed_hash).cloned()
        };
        // Drop wallets lock before calling bootstrap/unlock methods
        if let Some(wallet_arc) = wallet_arc {
            {
                let mut w = wallet_arc.write().unwrap();
                if !w.is_open() {
                    w.wallet_seed
                        .open_no_password()
                        .unwrap_or_else(|e| panic!("Failed to unlock reused wallet: {}", e));
                    println!("  Wallet unlocked (no password)");
                } else {
                    println!("  Wallet already open");
                }
            }
            app_ctx.bootstrap_wallet_addresses(&wallet_arc);
            app_ctx.handle_wallet_unlocked(&wallet_arc);
            println!("  Wallet bootstrapped for SPV");
        }
    } else {
        // 5–10. Import wallet via UI
        import_wallet_via_ui(harness, ctx, &words);
    }

    // 11. Clear stale cached wallet state so the balance/UTXO check below only
    //     passes after a FRESH SPV reconciliation (not from DB-cached values
    //     left over from a previous run).
    {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        if let Some(seed_hash) = &ctx.wallet_seed_hash
            && let Some(wallet) = wallets.get(seed_hash)
        {
            let mut w = wallet.write().unwrap();
            w.utxos.clear();
            w.update_spv_balances(0, 0, 0);
        }
    }

    // 12. Start SPV via AppContext::start_spv() which registers the reconcile
    //     and finality listeners BEFORE launching the sync loop.  Without the
    //     reconcile listener, balance updates from SPV never propagate to wallets.
    {
        let app_ctx = harness.state().current_app_context().clone();
        app_ctx.start_spv().expect("SPV start failed");
    }
    println!("  SPV sync started (with reconcile listener), waiting for completion...");

    // 13. Poll for balance availability (primary) or SPV Running status.
    //
    // Balance is populated via the reconcile listener as soon as filters + blocks
    // are scanned — this completes MUCH earlier than full SPV sync because
    // masternode list validation (Syncing 0/N) can stall for extended periods
    // on testnet with limited peers.  We treat the balance being ready as
    // sufficient to proceed; SpvStatus::Running is a nice-to-have.
    let timeout_secs: u64 = std::env::var("E2E_SPV_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(SPV_SYNC_TIMEOUT_SECS);

    let start = Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let mut retry_count: u32 = 0;
    let mut spv_synced = false;
    let mut balance_ready = false;
    let mut last_log = Instant::now();
    let mut last_spv_status = SpvStatus::Idle;

    while start.elapsed() < timeout {
        harness.run_steps(60); // ~1s at 60fps

        let status = {
            let app_ctx = harness.state().current_app_context();
            app_ctx.spv_manager.status()
        };

        // Check balance using max_balance() (sum of UTXOs in memory) rather than
        // total_balance_duffs() which can return a cached DB value from a previous run.
        // UTXOs are populated by reconcile_spv_wallets() which also syncs the SPV
        // WalletManager state — so non-zero max_balance proves the WalletManager
        // has UTXOs available for building transactions.
        if !balance_ready {
            let app_ctx = harness.state().current_app_context();
            let wallets = app_ctx.wallets.read().unwrap();
            if let Some(seed_hash) = &ctx.wallet_seed_hash
                && let Some(wallet) = wallets.get(seed_hash)
            {
                let w = wallet.read().unwrap();
                let utxo_balance = w.max_balance();
                if utxo_balance >= MIN_BALANCE_DUFFS {
                    balance_ready = true;
                    println!(
                        "  UTXOs ready: {} duffs ({:.8} DASH) at {:.0}s",
                        utxo_balance,
                        utxo_balance as f64 / 1e8,
                        start.elapsed().as_secs_f64(),
                    );
                }
            }
        }

        // Check whether SPV has synced headers (required for building transactions —
        // the chain height is used for lock time calculation).
        let header_height = status
            .sync_progress
            .as_ref()
            .and_then(|p| p.headers().ok())
            .map(|h| h.current_height())
            .unwrap_or(0);

        // Check SPV status
        last_spv_status = status.status;
        match status.status {
            SpvStatus::Running => {
                spv_synced = true;
            }
            SpvStatus::Error => {
                let err_msg = status.last_error.as_deref().unwrap_or("unknown");
                println!("  SPV error detected: {}", err_msg);

                if retry_count < PLATFORM_MAX_RETRIES {
                    retry_count += 1;
                    println!(
                        "  Retrying SPV sync ({}/{})...",
                        retry_count, PLATFORM_MAX_RETRIES
                    );
                    let app_ctx = harness.state().current_app_context().clone();
                    app_ctx.spv_manager.stop();
                    harness.run_steps(120); // ~2s cooldown
                    app_ctx
                        .start_spv()
                        .unwrap_or_else(|e| panic!("SPV restart failed: {}", e));
                } else {
                    panic!(
                        "SPV sync failed after {} retries. Last error: {}",
                        PLATFORM_MAX_RETRIES, err_msg
                    );
                }
            }
            _ => {}
        }

        // We can proceed once:
        // 1. Balance is ready (wallet has funds)
        // 2. SPV is at least Syncing (network manager live, can broadcast)
        // 3. Header height is known (required for transaction lock time)
        // We don't require Running (full masternode validation can stall on testnet).
        let spv_network_ready = matches!(status.status, SpvStatus::Syncing | SpvStatus::Running);
        if balance_ready && spv_network_ready && header_height > 0 {
            ctx.header_height = header_height;
            println!(
                "  SPV {:?}, header_height={} — proceeding{}",
                status.status,
                header_height,
                if spv_synced { "" } else { " without full sync" },
            );
            break;
        }

        // Log progress every 30s so we can diagnose hangs
        if last_log.elapsed() > Duration::from_secs(30) {
            let stage_info = status
                .sync_progress
                .as_ref()
                .map(|p| format!("state={:?}, synced={}", p.state(), p.is_synced()))
                .unwrap_or_else(|| "no progress info".to_string());
            println!(
                "  SPV status: {:?} ({:.0}s elapsed, peers={}, {})",
                status.status,
                start.elapsed().as_secs_f64(),
                status.connected_peers,
                stage_info,
            );
            last_log = Instant::now();
        }
    }

    ctx.spv_synced = spv_synced;

    // Read final balance and print wallet diagnostics
    {
        let app_ctx = harness.state().current_app_context();
        let wallets = app_ctx.wallets.read().unwrap();
        let wallet = wallets
            .get(ctx.seed_hash())
            .expect("Wallet not found by seed hash after SPV sync");
        let w = wallet.read().unwrap();
        ctx.balance_duffs = w.total_balance_duffs();
        println!(
            "  Wallet diagnostics: total_balance={}, confirmed={}, max_balance(utxos)={}, utxo_addrs={}, tx_count={}, is_open={}",
            w.total_balance_duffs(),
            w.confirmed_balance_duffs(),
            w.max_balance(),
            w.utxos.len(),
            w.transactions.len(),
            w.is_open(),
        );
    }

    assert!(
        balance_ready,
        "Wallet balance ({} duffs / {:.8} DASH) did not reach minimum ({} duffs) \
         within {}s. SPV status: {:?}. Please fund the wallet.",
        ctx.balance_duffs,
        ctx.balance_duffs as f64 / 1e8,
        MIN_BALANCE_DUFFS,
        timeout_secs,
        last_spv_status,
    );

    println!(
        "  Setup complete. Balance: {} duffs ({:.8} DASH){}",
        ctx.balance_duffs,
        ctx.balance_duffs as f64 / 1e8,
        if ctx.wallet_reused { " (reused)" } else { "" }
    );
}
