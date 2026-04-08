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

        // Persistent workdir — stable across commits so SPV header/filter
        // cache is reused. Wallet state (UTXOs, balances) is NOT persisted
        // yet (apply() TODO), so each run must rescan filters from birth_height.
        // We clear the SPV state directory to reset filter_committed_height,
        // but keep headers/filters cached for fast re-download.
        let workdir = std::env::temp_dir().join("dash-evo-e2e-testnet");
        // TODO: Once apply() restores wallet state from persistence, remove
        // this filter_committed_height reset. Currently wallet UTXOs/balances
        // are lost on restart, so we must force a filter rescan from
        // birth_height each run. We do NOT delete the SPV cache — headers
        // and filters take ~90 min to re-download from scratch.
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

        // Register wallet BEFORE starting SPV so the wallet's addresses
        // are in the bloom filter from the start. This is important because
        // the SPV filter scan checks monitored_addresses() once at startup.
        match app_context.register_wallet(wallet) {
            Ok((hash, _)) => {
                tracing::info!("Registered framework wallet (seed_hash: {:?})", &hash[..4]);
            }
            Err(TaskError::WalletAlreadyImported) => {
                tracing::info!("Framework wallet already registered (reusing from persistent DB)");
            }
            Err(e) => panic!("Failed to register framework wallet: {}", e),
        }

        // Set birth_height on the framework wallet so SPV filter scanning
        // starts from a recent block instead of genesis. Without this, a
        // fresh testnet scan (~1.4M blocks) takes >90 minutes and exceeds
        // the test timeout. E2E_WALLET_BIRTH_HEIGHT can override the default.
        {
            use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::wallet_info_interface::WalletInfoInterface;

            let birth_height: u32 = std::env::var("E2E_WALLET_BIRTH_HEIGHT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1_400_000);

            // Access the PlatformWallet through the old Wallet model.
            let wallets = app_context.wallets().read().expect("wallets lock");
            if let Some(wallet_arc) = wallets.get(&framework_wallet_hash) {
                let wallet_guard = wallet_arc.read().expect("wallet lock");
                if let Some(pw) = &wallet_guard.platform_wallet {
                    if let Some(mut wi) = pw.try_state_mut() {
                        if wi.wallet_info.birth_height() == 0 {
                            wi.wallet_info.set_birth_height(birth_height);
                            tracing::info!("Set framework wallet birth_height to {}", birth_height);
                        }
                    }
                }
            }
        }

        // Switch to SPV mode and start (wallet already registered above)
        app_context.set_core_backend_mode(CoreBackendMode::Spv);
        // Reset filter_committed_height so the filter scan restarts from
        // birth_height. Without this, cached committed height from a previous
        // run causes the scan to skip historical blocks, and since wallet
        // state isn't persisted yet (apply() TODO), the balance stays 0.
        app_context.reset_spv_filter_committed_height();
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

        // Wait for wallet to appear in SPV
        wait::wait_for_wallet_in_spv(&app_context, framework_wallet_hash, Duration::from_secs(30))
            .await
            .expect("Framework wallet not picked up by SPV");

        // Wait for SPV to sync and funds to become spendable
        // First run with a fresh SPV cache requires downloading ~54K filter
        // headers + filters (from birth_height). This takes 2-5 minutes.
        // Subsequent runs use cached data and complete in ~8 seconds.
        let balance_timeout = std::env::var("E2E_BALANCE_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300);
        tracing::info!("Waiting for SPV to sync framework wallet spendable balance (timeout: {}s)...", balance_timeout);
        match wait::wait_for_spendable_balance(
            &app_context,
            framework_wallet_hash,
            1, // at least 1 duff spendable
            Duration::from_secs(balance_timeout),
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
                                .platform_wallet
                                .as_ref()
                                .and_then(|pw| pw.try_state())
                                .and_then(|info| {
                                    info.wallet_info.accounts.standard_bip44_accounts.get(&0)
                                        .and_then(|a| {
                                            let addrs = a.account_type.all_addresses();
                                            addrs.into_iter().next()
                                        })
                                        .map(|a| a.to_string())
                                })
                                .unwrap_or_else(|| "<unknown>".to_string());
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

        // Wait for SPV filters to sync so mempool bloom filter is active
        // before any test broadcasts. We don't require masternodes to sync
        // because testnet quorum rotation data can fail (QRInfo errors).
        // The wallet is fully functional for transactions without masternodes.
        tracing::info!("Waiting for SPV filters to sync...");
        wait::wait_for_spv_syncing_or_running(&app_context, Duration::from_secs(120))
            .await
            .expect("SPV did not start syncing within 120s");
        tracing::info!("SPV filters synced — ready for transactions");

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

        // Get test wallet's receive address — extract PlatformWallet under
        // short sync lock, then drop before .await to avoid deadlock.
        let test_address = {
            let pw = {
                let w = wallet_arc.read().expect("wallet lock");
                w.platform_wallet.clone().expect("platform wallet must exist")
            };
            pw.core()
                .next_receive_address()
                .await
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
        tracing::trace!(seed_hash = ?&seed_hash[..4], min = amount_duffs, "create_funded_test_wallet: waiting for spendable balance (IS lock)...");
        wait::wait_for_spendable_balance(
            app_context,
            seed_hash,
            amount_duffs,
            Duration::from_secs(120),
        )
        .await
        .expect("Test wallet funds did not become spendable");
        tracing::trace!(
            seed_hash = ?&seed_hash[..4],
            elapsed_ms = funding_start.elapsed().as_millis(),
            "create_funded_test_wallet: funds spendable (IS-locked)"
        );

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
