//! Core shared state for backend E2E tests.
//!
//! Provides a lazily-initialized `BackendTestContext` that sets up
//! `AppContext`, SPV, and a funded framework wallet.
//!
//! ## Shared Runtime
//!
//! All tests use `#[tokio_shared_rt::test(shared)]` instead of `#[tokio::test]`.
//! SPV spawns background tasks via `tokio::spawn` that are bound to the runtime
//! that created them. With `#[tokio::test]`, each test creates its own runtime —
//! when the first test exits, its runtime drops and kills the SPV tasks, causing
//! "channel closed" errors in later tests. The shared runtime from
//! `tokio-shared-rt` keeps everything alive for the entire test binary.

use crate::framework::funding;
use crate::framework::task_runner::run_task;
use crate::framework::wait;
use bip39::{Language, Mnemonic};
use dash_evo_tool::app_dir::ensure_env_file;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::error::TaskError;
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

/// Shared test context, initialized once across all backend E2E tests.
///
/// Uses `tokio::sync::OnceCell` so initialization runs inside the shared
/// runtime context (via `block_on`) rather than spawning a nested one.
static CTX: tokio::sync::OnceCell<BackendTestContext> = tokio::sync::OnceCell::const_new();

/// Cancellation token for the task manager that owns SPV tasks.
///
/// `tokio::sync::OnceCell` does not cache panicked inits — if `init()`
/// panics (e.g. framework wallet unfunded), the next test retries from
/// scratch. But the orphaned SPV tasks from the panicked init still run
/// on the shared tokio runtime, holding the data directory lock. A global
/// panic hook cancels this token, which stops SPV (its stop_token is a
/// child) and releases the lock file.
static SPV_CANCEL: std::sync::Mutex<Option<tokio_util::sync::CancellationToken>> =
    std::sync::Mutex::new(None);

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
        // Cancel orphaned SPV tasks from a previous panicked init (if any).
        if let Some(token) = SPV_CANCEL
            .lock()
            .inspect_err(|e| {
                eprintln!("SPV_CANCEL mutex poisoned during init retry: {e}");
            })
            .ok()
            .and_then(|mut g| g.take())
        {
            tracing::warn!("Cancelling orphaned SPV tasks from a previous init attempt");
            token.cancel();
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        // Initialize tracing for test output
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("backend_e2e=info")),
            )
            .with_target(false)
            .try_init();

        // Load .env from the project root so E2E_WALLET_MNEMONIC is available.
        if let Err(e) = dotenvy::dotenv() {
            tracing::debug!(".env not loaded ({e}), relying on environment");
        }

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
        tracing::info!("E2E workdir: {}", workdir.display());

        // Ensure .env is present in the workdir (no env var mutation needed).
        ensure_env_file(&workdir);

        // Create database
        let db_path = workdir.join("data.db");
        let db =
            Arc::new(create_database_at_path(&db_path).expect("Failed to create test database"));

        // Create AppContext
        let subtasks = Arc::new(TaskManager::new());
        let cancel_token = subtasks.cancellation_token.clone();
        let connection_status = Arc::new(ConnectionStatus::new());
        let egui_ctx = egui::Context::default();

        let app_context = AppContext::new(
            workdir.clone(),
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

        // Stash the cancellation token so the panic hook can stop SPV if
        // init panics later (e.g. framework wallet unfunded).
        if let Ok(mut guard) = SPV_CANCEL.lock() {
            *guard = Some(cancel_token);
        }

        // Install a panic hook (once) that cancels SPV tasks on any panic
        // during init. The token is cleared after init succeeds (below) so
        // test panics don't kill SPV for other parallel tests.
        static HOOK_INSTALLED: std::sync::Once = std::sync::Once::new();
        HOOK_INSTALLED.call_once(|| {
            let prev_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if let Some(token) = SPV_CANCEL
                    .lock()
                    .inspect_err(|e| {
                        eprintln!("Panic hook: SPV_CANCEL mutex poisoned, cannot cancel SPV: {e}");
                    })
                    .ok()
                    .and_then(|g| g.clone())
                {
                    tracing::warn!(
                        "Panic hook: cancelling SPV tasks to release data directory lock"
                    );
                    token.cancel();
                }
                prev_hook(info);
            }));
        });

        // Wait for SPV peers
        wait::wait_for_spv_peers(&app_context, Duration::from_secs(60))
            .await
            .expect("SPV failed to connect to any peers within 60s");
        tracing::info!("SPV connected to peers");

        // E2E_WALLET_MNEMONIC is required
        let mnemonic_phrase = std::env::var("E2E_WALLET_MNEMONIC").unwrap_or_else(|_| {
            panic!(
                "E2E_WALLET_MNEMONIC is not set.\n\
                 This environment variable is required for backend E2E tests.\n\
                 Set it to a BIP-39 mnemonic of a pre-funded testnet wallet.\n\
                 Example: E2E_WALLET_MNEMONIC=\"word1 word2 word3 ... word12\"\n\
                 You can also add it to the project root .env file."
            );
        });

        tracing::info!("Restoring framework wallet from E2E_WALLET_MNEMONIC");
        let mnemonic = Mnemonic::parse_in(Language::English, &mnemonic_phrase)
            .expect("Invalid E2E_WALLET_MNEMONIC");

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
                tracing::info!("Registered framework wallet (seed_hash: {:?})", &hash[..4]);
            }
            Err(TaskError::WalletAlreadyImported) => {
                tracing::info!("Framework wallet already registered (reusing from persistent DB)");
            }
            Err(e) => panic!("Failed to register framework wallet: {}", e),
        }

        // Wait for wallet to appear in SPV
        wait::wait_for_wallet_in_spv(&app_context, framework_wallet_hash, Duration::from_secs(30))
            .await
            .expect("Framework wallet not picked up by SPV");

        // Wait for SPV to fully sync (including masternodes) so MempoolManager
        // is active and bloom filter is built before any test broadcasts.
        // This must come BEFORE the spendable balance check — wallet balances
        // are only available after compact filter sync completes.
        tracing::info!("Waiting for SPV to complete full sync (masternodes + mempool)...");
        wait::wait_for_spv_running(&app_context, Duration::from_secs(600))
            .await
            .expect("SPV did not reach Running state within 600s");
        tracing::info!("SPV fully synced — mempool bloom filter active");

        // Now check framework wallet balance — SPV has synced, so balances
        // should be available immediately (no need for a long timeout).
        tracing::info!("Waiting for SPV to sync framework wallet spendable balance...");
        match wait::wait_for_spendable_balance(
            &app_context,
            framework_wallet_hash,
            1, // at least 1 duff spendable
            Duration::from_secs(30),
        )
        .await
        {
            Ok(balance) => {
                tracing::info!("Framework wallet spendable: {} duffs", balance);
            }
            Err(e) => {
                let (confirmed, total, address) = {
                    let wallets = app_context.wallets().read().expect("wallets lock");
                    wallets
                        .get(&framework_wallet_hash)
                        .map(|w| {
                            let mut guard = w.write().expect("wallet lock");
                            let bal =
                                (guard.confirmed_balance_duffs(), guard.total_balance_duffs());
                            let addr = guard
                                .receive_address(Network::Testnet, false, Some(&app_context))
                                .map(|a| a.to_string())
                                .unwrap_or_else(|_| "<unknown>".to_string());
                            (bal.0, bal.1, addr)
                        })
                        .unwrap_or((0, 0, "<unknown>".to_string()))
                };
                panic!(
                    "Framework wallet has no spendable balance: {} \
                     (confirmed: {}, total: {})\n  \
                     Fund this address manually: {}",
                    e, confirmed, total, address
                );
            }
        }

        // Verify balance is above minimum threshold
        funding::verify_framework_funded(&app_context, framework_wallet_hash).await;

        // Sweep orphaned test wallets from previous runs (e.g., a test panicked
        // before cleanup). Wallets persist in the DB, so AppContext loaded them
        // automatically and SPV synced their balances.
        crate::framework::cleanup::cleanup_test_wallets(&app_context, framework_wallet_hash).await;

        // Init succeeded — clear the cancellation token so the panic hook
        // won't kill SPV when individual tests panic. The hook is only
        // needed during init to prevent orphaned SPV holding the lock file.
        if let Ok(mut guard) = SPV_CANCEL.lock() {
            *guard = None;
        }

        BackendTestContext {
            app_context,
            framework_wallet_hash,
            _workdir: workdir,
        }
    }

    /// Create a new wallet, fund it from the framework wallet, and wait for balance.
    ///
    /// Sends `amount_duffs` from the framework wallet in a single attempt,
    /// then waits for the full amount to become spendable in the test wallet
    /// and for the framework wallet's change output to settle.
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
        tracing::trace!(
            seed_hash = ?&seed_hash[..4],
            amount_duffs,
            "create_funded_test_wallet: registered new wallet"
        );

        // Wait for SPV to pick up the wallet
        wait::wait_for_wallet_in_spv(app_context, seed_hash, Duration::from_secs(30))
            .await
            .expect("Test wallet not picked up by SPV");
        tracing::trace!(seed_hash = ?&seed_hash[..4], "create_funded_test_wallet: wallet visible in SPV");

        // Allow mempool manager tick (100ms) to detect wallet address change
        // and rebuild bloom filter before we broadcast.
        tokio::time::sleep(Duration::from_millis(200)).await;
        tracing::trace!(seed_hash = ?&seed_hash[..4], "create_funded_test_wallet: waited for bloom filter rebuild tick");

        // Get test wallet's receive address
        let test_address = {
            let mut w = wallet_arc.write().expect("wallet lock");
            w.receive_address(Network::Testnet, false, Some(app_context))
                .expect("Failed to get test wallet receive address")
                .to_string()
        };
        tracing::trace!(address = %test_address, "create_funded_test_wallet: receive address derived");

        let framework_wallet_arc = {
            let wallets = app_context.wallets().read().expect("wallets lock");
            wallets
                .get(&self.framework_wallet_hash)
                .expect("framework wallet must exist")
                .clone()
        };

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
            wallet: framework_wallet_arc,
            request,
        });

        tracing::trace!(seed_hash = ?&seed_hash[..4], "create_funded_test_wallet: broadcasting funding tx...");
        let funding_start = std::time::Instant::now();
        run_task(app_context, task)
            .await
            .expect("Failed to send funds to test wallet");
        tracing::trace!(
            seed_hash = ?&seed_hash[..4],
            elapsed_ms = funding_start.elapsed().as_millis(),
            "create_funded_test_wallet: funding tx broadcast"
        );

        // Wait for test wallet to see the funds
        tracing::trace!(seed_hash = ?&seed_hash[..4], min = amount_duffs, "create_funded_test_wallet: waiting for total balance...");
        wait::wait_for_balance(
            app_context,
            seed_hash,
            amount_duffs,
            Duration::from_secs(120),
        )
        .await
        .expect("Test wallet did not receive expected funds");
        tracing::trace!(
            seed_hash = ?&seed_hash[..4],
            elapsed_ms = funding_start.elapsed().as_millis(),
            "create_funded_test_wallet: total balance reached"
        );

        // Wait for the full funded amount to become spendable so callers can
        // immediately build transactions without racing confirmations/IS locks.
        // Large amounts (>10M duffs) sometimes don't get IS-locked on testnet
        // within a reasonable timeout, so we fall back gracefully if total
        // balance is present — the IS lock will arrive eventually.
        tracing::trace!(seed_hash = ?&seed_hash[..4], min = amount_duffs, "create_funded_test_wallet: waiting for spendable balance (IS lock)...");
        match wait::wait_for_spendable_balance(
            app_context,
            seed_hash,
            amount_duffs,
            Duration::from_secs(180),
        )
        .await
        {
            Ok(_) => {
                tracing::trace!(
                    seed_hash = ?&seed_hash[..4],
                    elapsed_ms = funding_start.elapsed().as_millis(),
                    "create_funded_test_wallet: funds spendable (IS-locked)"
                );
            }
            Err(e) => {
                // Check if total balance arrived even though IS lock didn't
                let total = {
                    let wallets = app_context.wallets().read().expect("wallets lock");
                    wallets
                        .get(&seed_hash)
                        .map(|w| w.read().expect("wallet lock").total_balance_duffs())
                        .unwrap_or(0)
                };
                if total >= amount_duffs {
                    tracing::warn!(
                        seed_hash = ?&seed_hash[..4],
                        total,
                        amount_duffs,
                        "create_funded_test_wallet: IS lock timed out but total balance \
                         is sufficient — continuing (lock will arrive eventually)"
                    );
                } else {
                    panic!("Test wallet funds did not become spendable: {e}");
                }
            }
        }

        // Wait for framework wallet change output to become spendable.
        tracing::trace!(seed_hash = ?&seed_hash[..4], "create_funded_test_wallet: waiting for framework change to settle...");
        let _ = wait::wait_for_spendable_balance(
            app_context,
            self.framework_wallet_hash,
            1,
            Duration::from_secs(30),
        )
        .await;
        tracing::info!(
            seed_hash = ?&seed_hash[..4],
            total_elapsed_ms = funding_start.elapsed().as_millis(),
            "create_funded_test_wallet: complete"
        );

        (seed_hash, wallet_arc)
    }
}
