//! Core shared state for backend E2E tests.
//!
//! Provides a lazily-initialized `BackendTestContext` that sets up
//! `AppContext`, SPV, and a funded framework wallet.

use crate::funding;
use crate::task_runner::run_task;
use crate::wait;
use bip39::{Language, Mnemonic};
use dash_evo_tool::app_dir::copy_env_file_if_not_exists;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::context::connection_status::ConnectionStatus;
use dash_evo_tool::database::test_helpers::create_database_at_path;
use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_evo_tool::spv::CoreBackendMode;
use dash_evo_tool::utils::tasks::TaskManager;
use dash_sdk::dpp::dashcore::Network;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Maximum retries for sends that fail with InsufficientFunds.
const SEND_MAX_RETRIES: u32 = 5;
/// Delay between send retries (waiting for change to become spendable).
const SEND_RETRY_DELAY: Duration = Duration::from_secs(10);

/// Shared test context, initialized once across all backend E2E tests.
///
/// Uses `tokio::sync::OnceCell` so initialization runs inside the existing
/// tokio runtime (from `#[tokio::test]`) rather than spawning a nested one.
static CTX: tokio::sync::OnceCell<BackendTestContext> = tokio::sync::OnceCell::const_new();

/// Get (or initialize) the shared test context.
pub async fn ctx() -> &'static BackendTestContext {
    CTX.get_or_init(BackendTestContext::init).await
}

/// Shared backend context for E2E tests.
pub struct BackendTestContext {
    pub app_context: Arc<AppContext>,
    pub framework_wallet_hash: WalletSeedHash,
    pub _workdir: PathBuf,
}

impl BackendTestContext {
    async fn init() -> Self {
        // Persistent workdir keyed by git revision
        let git_hash = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let workdir = std::env::temp_dir().join(format!("dash-evo-e2e-testnet-{}", git_hash));
        std::fs::create_dir_all(&workdir).expect("Failed to create workdir");
        println!("  E2E workdir: {}", workdir.display());

        // Point the app data dir to our workdir so config/env files live there.
        // SAFETY: tests run with --test-threads=1, so no concurrent env var access.
        unsafe {
            std::env::set_var("DASH_EVO_DATA_DIR", &workdir);
        }

        // Ensure .env is present
        copy_env_file_if_not_exists();

        // Create database
        let db_path = workdir.join("data.db");
        let db =
            Arc::new(create_database_at_path(&db_path).expect("Failed to create test database"));

        // Create AppContext
        let subtasks = Arc::new(TaskManager::new());
        let connection_status = Arc::new(ConnectionStatus::new());
        let egui_ctx = egui::Context::default();

        let app_context = AppContext::new(
            Network::Testnet,
            db,
            None, // no password
            subtasks,
            connection_status,
            egui_ctx,
        )
        .expect("Failed to create AppContext for testnet");

        // Switch to SPV mode and start
        app_context.set_core_backend_mode(CoreBackendMode::Spv);
        app_context.start_spv().expect("Failed to start SPV");

        // Wait for SPV peers
        wait::wait_for_spv_peers(&app_context, Duration::from_secs(60))
            .await
            .expect("SPV failed to connect to any peers within 60s");
        println!("  SPV connected to peers");

        // Create or restore framework wallet
        let mnemonic = match std::env::var("E2E_WALLET_MNEMONIC") {
            Ok(phrase) => {
                println!("  Restoring framework wallet from E2E_WALLET_MNEMONIC");
                Mnemonic::parse_in(Language::English, &phrase).expect("Invalid E2E_WALLET_MNEMONIC")
            }
            Err(_) => {
                println!("  Generating fresh framework wallet mnemonic");
                Mnemonic::generate_in(Language::English, 12).expect("Mnemonic generation failed")
            }
        };

        let seed = mnemonic.to_seed("");
        let wallet = dash_evo_tool::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("E2E Framework Wallet".to_string()),
            None,
        )
        .expect("Failed to create framework wallet");

        let framework_wallet_hash = wallet.seed_hash();

        // Try to register; if the wallet already exists (persistent DB), just look it up.
        match app_context.register_wallet(wallet) {
            Ok((hash, _)) => {
                println!(
                    "  Registered framework wallet (seed_hash: {:?})",
                    &hash[..4]
                );
            }
            Err(e) if e.contains("already been imported") => {
                println!("  Framework wallet already registered (reusing from persistent DB)");
            }
            Err(e) => panic!("Failed to register framework wallet: {}", e),
        }

        // Wait for wallet to appear in SPV
        wait::wait_for_wallet_in_spv(&app_context, framework_wallet_hash, Duration::from_secs(30))
            .await
            .expect("Framework wallet not picked up by SPV");

        // Wait for SPV to sync the wallet's existing UTXOs and make them spendable.
        // Use spendable balance (confirmed/IS-locked) — total balance includes unconfirmed
        // funds that can't actually be used for transaction building.
        println!("  Waiting for SPV to sync framework wallet spendable balance...");
        match wait::wait_for_spendable_balance(
            &app_context,
            framework_wallet_hash,
            1, // at least 1 duff spendable
            Duration::from_secs(120),
        )
        .await
        {
            Ok(balance) => {
                println!(
                    "  Framework wallet spendable balance after SPV sync: {} duffs",
                    balance
                );
            }
            Err(_) => {
                // Fall back to checking total balance — funds may be unconfirmed from faucet
                println!("  No spendable balance yet, checking total...");
                match wait::wait_for_balance(
                    &app_context,
                    framework_wallet_hash,
                    1,
                    Duration::from_secs(30),
                )
                .await
                {
                    Ok(balance) => {
                        println!(
                            "  Framework wallet total balance: {} duffs (unconfirmed, will wait for confirmation)",
                            balance
                        );
                    }
                    Err(_) => {
                        println!("  No existing balance found, will try faucet");
                    }
                }
            }
        }

        // Top up from faucet if balance is still below threshold
        funding::ensure_framework_funded(&app_context, framework_wallet_hash).await;

        // Wait for funds to become SPENDABLE (not just visible as unconfirmed).
        // This ensures the first create_funded_test_wallet() call can actually use them.
        println!("  Waiting for framework wallet funds to become spendable...");
        match wait::wait_for_spendable_balance(
            &app_context,
            framework_wallet_hash,
            1,
            Duration::from_secs(180),
        )
        .await
        {
            Ok(balance) => println!("  Framework wallet ready, spendable: {} duffs", balance),
            Err(e) => {
                // Log detailed diagnostics but don't panic — the send retry loop
                // in create_funded_test_wallet will handle transient UTXO issues.
                let (confirmed, total) = {
                    let wallets = app_context.wallets().read().expect("wallets lock");
                    wallets
                        .get(&framework_wallet_hash)
                        .map(|w| {
                            let guard = w.read().expect("wallet lock");
                            (guard.confirmed_balance_duffs(), guard.total_balance_duffs())
                        })
                        .unwrap_or((0, 0))
                };
                eprintln!(
                    "  Warning: {} (confirmed: {}, total: {})",
                    e, confirmed, total
                );
            }
        }

        BackendTestContext {
            app_context,
            framework_wallet_hash,
            _workdir: workdir,
        }
    }

    /// Create a new wallet, fund it from the framework wallet, and wait for balance.
    ///
    /// Retries the send if it fails with "Insufficient funds" (change output from a
    /// previous send not yet spendable). Waits for framework wallet change to become
    /// spendable after each successful send so subsequent calls don't fail.
    pub async fn create_funded_test_wallet(
        &self,
        amount_duffs: u64,
    ) -> (
        WalletSeedHash,
        Arc<std::sync::RwLock<dash_evo_tool::model::wallet::Wallet>>,
    ) {
        let app_context = &self.app_context;

        // Generate fresh wallet
        let mnemonic =
            Mnemonic::generate_in(Language::English, 12).expect("Mnemonic generation failed");
        let seed = mnemonic.to_seed("");

        let wallet = dash_evo_tool::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("E2E Test Wallet".to_string()),
            None,
        )
        .expect("Failed to create test wallet");

        let (seed_hash, wallet_arc) = app_context
            .register_wallet(wallet)
            .expect("Failed to register test wallet");

        // Wait for SPV to pick up the wallet
        wait::wait_for_wallet_in_spv(app_context, seed_hash, Duration::from_secs(30))
            .await
            .expect("Test wallet not picked up by SPV");

        // Get test wallet's receive address
        let test_address = {
            let mut w = wallet_arc.write().expect("wallet lock");
            w.receive_address(Network::Testnet, false, Some(app_context))
                .expect("Failed to get test wallet receive address")
                .to_string()
        };

        // Send funds from framework wallet with retry logic.
        // After a previous send, the change output may not be spendable yet
        // (waiting for InstantSend lock or block confirmation).
        let framework_wallet_arc = {
            let wallets = app_context.wallets().read().expect("wallets lock");
            wallets
                .get(&self.framework_wallet_hash)
                .expect("framework wallet must exist")
                .clone()
        };

        let mut last_error = String::new();
        for attempt in 1..=SEND_MAX_RETRIES {
            // Ensure framework wallet has spendable UTXOs before attempting send
            if attempt > 1 {
                println!(
                    "  Retry {}/{}: waiting for framework wallet UTXOs to become spendable...",
                    attempt, SEND_MAX_RETRIES
                );
                match wait::wait_for_spendable_balance(
                    app_context,
                    self.framework_wallet_hash,
                    amount_duffs,
                    SEND_RETRY_DELAY,
                )
                .await
                {
                    Ok(bal) => println!("  Framework wallet spendable: {} duffs", bal),
                    Err(_) => {
                        println!("  Still no spendable balance, retrying send anyway...");
                    }
                }
            }

            let request = WalletPaymentRequest {
                recipients: vec![PaymentRecipient {
                    address: test_address.clone(),
                    amount_duffs,
                }],
                subtract_fee_from_amount: false,
                memo: Some("E2E test funding".to_string()),
                override_fee: None,
            };

            let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
                wallet: framework_wallet_arc.clone(),
                request,
            });

            match run_task(app_context, task).await {
                Ok(_result) => {
                    println!(
                        "  Funded test wallet on attempt {}/{}",
                        attempt, SEND_MAX_RETRIES
                    );
                    last_error.clear();
                    break;
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("Insufficient") && attempt < SEND_MAX_RETRIES {
                        println!(
                            "  Send attempt {}/{} failed (Insufficient funds), will retry...",
                            attempt, SEND_MAX_RETRIES
                        );
                        last_error = err_str;
                        tokio::time::sleep(SEND_RETRY_DELAY).await;
                        continue;
                    }
                    panic!(
                        "Failed to send funds to test wallet after {} attempts: {}",
                        attempt, e
                    );
                }
            }
        }
        if !last_error.is_empty() {
            panic!(
                "Failed to send funds to test wallet after {} attempts: {}",
                SEND_MAX_RETRIES, last_error
            );
        }

        // Wait for test wallet to see the funds (total, not just spendable —
        // the test wallet just needs to confirm the tx arrived)
        wait::wait_for_balance(
            app_context,
            seed_hash,
            amount_duffs,
            Duration::from_secs(120),
        )
        .await
        .expect("Test wallet did not receive expected funds");

        // Wait for framework wallet change output to become spendable.
        // This ensures the NEXT call to create_funded_test_wallet() won't fail
        // with "Insufficient funds" because the change hasn't confirmed yet.
        // Use a short timeout — if it doesn't confirm quickly, the retry loop
        // in the next call will handle it.
        let _ = wait::wait_for_spendable_balance(
            app_context,
            self.framework_wallet_hash,
            1, // any spendable amount means change arrived
            Duration::from_secs(30),
        )
        .await;

        (seed_hash, wallet_arc)
    }
}
