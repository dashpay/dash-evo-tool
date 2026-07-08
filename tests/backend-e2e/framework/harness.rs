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
use dash_evo_tool::app::TaskResult;
use dash_evo_tool::app_dir::ensure_env_file;
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::context::connection_status::ConnectionStatus;
use dash_evo_tool::database::test_helpers::create_database_at_path;
use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_evo_tool::utils::egui_mpsc::EguiMpscAsync;
use dash_evo_tool::utils::tasks::TaskManager;
use dash_sdk::dpp::dashcore::Network;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Maximum timeout for any single test operation (poll loop, broadcast wait,
/// balance query, etc.). Only the initial SPV sync may exceed this.
pub const MAX_TEST_TIMEOUT: Duration = Duration::from_secs(360);

/// Budget for the framework wallet to register with the upstream wallet
/// backend during `init`. Registration emits the transient
/// `TaskError::WalletBackend` ("retry in a moment") signal under the slower
/// platform revision; `wait_for_wallet_in_spv` retries that typed variant with
/// backoff inside this budget, so it must be comfortably longer than a single
/// registration round-trip.
const FRAMEWORK_WALLET_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(120);

/// Budget for a per-test funded wallet to be picked up by the upstream SPV
/// backend in [`BackendTestContext::create_funded_test_wallet`]. Matches
/// [`FRAMEWORK_WALLET_REGISTRATION_TIMEOUT`]: the suite runs serially
/// (`--test-threads=1`), so as more wallets accumulate in the upstream manager
/// across the run, each later `wait_for_wallet_in_spv` round (filter rebuild +
/// re-sync) takes longer and needs the same 120s headroom as the framework wallet.
const FUNDED_WALLET_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(120);

/// Shared test context, initialized once across all backend E2E tests.
///
/// Uses `tokio::sync::OnceCell` so initialization runs inside the shared
/// runtime context (via `block_on`) rather than spawning a nested one.
static CTX: tokio::sync::OnceCell<BackendTestContext> = tokio::sync::OnceCell::const_new();

/// Serializes the UTXO-critical section of `create_funded_test_wallet`.
///
/// Only the payment broadcast (UTXO selection → broadcast → UTXO removal) is
/// serialized. The long waits (recipient balance, IS lock) run concurrently.
static FUNDING_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Cancellation token for the DET `TaskManager` that owns DET-side subtasks.
///
/// `tokio::sync::OnceCell` does not cache panicked inits — if `init()`
/// panics (e.g. framework wallet unfunded), the next test retries from
/// scratch. A global panic hook cancels this token to stop DET subtasks
/// from a panicked init. The upstream `SpvRuntime` (which holds the SPV
/// data-directory lock) is a separate concern handled by [`LEAKED_BACKEND`].
static SPV_CANCEL: std::sync::Mutex<Option<tokio_util::sync::CancellationToken>> =
    std::sync::Mutex::new(None);

/// The most recently constructed `WalletBackend`.
///
/// The upstream `SpvRuntime` takes an exclusive lock on its data directory.
/// If `init()` panics after the backend is built, that runtime keeps running
/// on the shared tokio runtime and holds the lock, so every retried `init()`
/// fails at `start()` with "Data directory locked". Before constructing a
/// new backend, a retry calls `shutdown()` on the leaked predecessor stored
/// here to release the lock.
static LEAKED_BACKEND: tokio::sync::Mutex<
    Option<Arc<dash_evo_tool::wallet_backend::WalletBackend>>,
> = tokio::sync::Mutex::const_new(None);

/// Number of distinct workdir slots `pick_available_workdir` cycles through.
/// The scan wraps modulo this value, so the preferred floor can climb without
/// ever emptying the scan range.
const MAX_WORKDIR_SLOTS: usize = 10;

/// Preferred starting workdir slot for the next `init()`. Bumped each time a
/// panicked init leaks an un-droppable SPV `LockFile`, so the retry prefers a
/// fresh directory instead of colliding with the locked one. The scan in
/// [`pick_available_workdir`] wraps modulo [`MAX_WORKDIR_SLOTS`], so even if
/// this floor exceeds the slot count it never exhausts the available slots — a
/// freed lower slot is still picked up.
static WORKDIR_SLOT_FLOOR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Get (or initialize) the shared test context.
pub async fn ctx() -> &'static BackendTestContext {
    CTX.get_or_init(BackendTestContext::init).await
}

/// Shared backend context for E2E tests.
pub struct BackendTestContext {
    pub app_context: Arc<AppContext>,
    pub framework_wallet_hash: WalletSeedHash,
    pub _workdir: PathBuf,
    /// Lock file held for the lifetime of the test process to prevent
    /// concurrent test runs from using the same workdir.
    _lock_file: std::fs::File,
    /// Receiver for the `TaskResult` channel handed to `WalletBackend`'s
    /// `EventBridge`. Kept alive for the whole test process so the bridge's
    /// non-blocking `try_send(Refresh)` never hits a closed channel — this
    /// mirrors `AppState` owning the receiver in production.
    _task_result_rx: tokio::sync::mpsc::Receiver<TaskResult>,
}

/// Whether a `register_wallet` failure is worth retrying: transient storage
/// contention or a not-yet-ready wallet backend, as opposed to a permanent
/// error (bad input, poisoned lock) or the idempotent `WalletAlreadyImported`.
fn is_transient_registration_error(error: &TaskError) -> bool {
    matches!(
        error,
        TaskError::WalletBackend { .. }
            | TaskError::WalletBackendNotYetWired
            | TaskError::WalletSeedStorage { .. }
            | TaskError::KvSidecarStorage { .. }
    )
}

/// Register a wallet, retrying transient storage/backend errors with bounded
/// backoff (~30s total).
///
/// Under the shared-runtime backend-e2e harness, the fail-closed sidecar writes
/// (`WalletSeedStorage` / `KvSidecarStorage`) can briefly lose a SQLite race,
/// and upstream registration can surface the typed transient `WalletBackend`
/// ("retry in a moment") signal. A single attempt then panics and masks the test
/// under exercise (e.g. identity_create / identity_cold_boot). Retry those
/// transient variants until they clear or the deadline passes; a permanent error
/// still surfaces after the bounded attempts. `WalletAlreadyImported` is returned
/// as-is so callers can treat it as the idempotent success it is.
async fn register_wallet_with_retry(
    app_context: &Arc<AppContext>,
    wallet: dash_evo_tool::model::wallet::Wallet,
    seed: &[u8; 64],
    origin: dash_evo_tool::model::wallet::birth_height::WalletOrigin,
) -> Result<
    (
        WalletSeedHash,
        Arc<std::sync::RwLock<dash_evo_tool::model::wallet::Wallet>>,
    ),
    TaskError,
> {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        // `register_wallet` consumes the wallet; clone per attempt so a retry
        // can submit a fresh copy.
        match app_context.register_wallet(wallet.clone(), seed, origin) {
            Ok(registered) => return Ok(registered),
            Err(e)
                if is_transient_registration_error(&e) && std::time::Instant::now() < deadline =>
            {
                let backoff = Duration::from_millis(500 * u64::from(attempt.min(6)));
                tracing::warn!(
                    attempt,
                    error = %e,
                    "wallet registration hit a transient error; retrying after backoff"
                );
                tokio::time::sleep(backoff).await;
            }
            Err(e) => return Err(e),
        }
    }
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

        // Handle a leaked upstream SpvRuntime from a previous panicked init.
        // `OnceCell` does not cache panicked inits, so this runs on every
        // retry until one init succeeds. We `shutdown()` to stop its
        // background tasks, but the SPV data-directory `LockFile` releases
        // only on `Drop` (dash-spv `storage/lockfile.rs`), and leaked task
        // clones keep the backend's `Arc` graph alive — so the lock cannot
        // be reclaimed in-process. Instead, advance the workdir slot so the
        // retry's SpvRuntime locks a fresh directory (this is exactly what
        // `pick_available_workdir`'s slot fallback exists for).
        if let Some(stale_backend) = LEAKED_BACKEND.lock().await.take() {
            tracing::warn!(
                "Leaked wallet backend from a previous init attempt; \
                 shutting it down and advancing to a fresh workdir slot"
            );
            stale_backend.shutdown().await;
            WORKDIR_SLOT_FLOOR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

        // Deterministic workdir — always the same path so the database, wallets,
        // and SPV data persist across runs. If the primary path is locked by
        // another process, fall back to numbered alternatives (slot 1, 2, ...).
        let base = std::env::temp_dir().join("dash-evo-e2e-testnet");
        let slot_floor = WORKDIR_SLOT_FLOOR.load(std::sync::atomic::Ordering::SeqCst);
        let (workdir, lock_file) = pick_available_workdir(&base, slot_floor);
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

        let app_kv = AppContext::open_app_kv(&workdir).expect("open app k/v");
        let secret_store = AppContext::open_secret_store(&workdir).expect("open secret store");
        let app_context = AppContext::new(
            workdir.clone(),
            Network::Testnet,
            db,
            subtasks,
            connection_status,
            egui_ctx,
            app_kv,
            secret_store,
        )
        .expect("Failed to create AppContext for testnet");

        // E2E_WALLET_MNEMONIC is required — read it early so we know which
        // wallet to keep before SPV starts.
        let mnemonic_phrase = std::env::var("E2E_WALLET_MNEMONIC").unwrap_or_else(|_| {
            panic!(
                "E2E_WALLET_MNEMONIC is not set.\n\
                 This environment variable is required for backend E2E tests.\n\
                 Set it to a BIP-39 mnemonic of a pre-funded testnet wallet.\n\
                 Example: E2E_WALLET_MNEMONIC=\"word1 word2 word3 ... word12\"\n\
                 You can also add it to the project root .env file."
            );
        });

        let mnemonic = Mnemonic::parse_in(Language::English, &mnemonic_phrase)
            .expect("Invalid E2E_WALLET_MNEMONIC");
        let seed = mnemonic.to_seed("");
        let framework_wallet_hash = {
            let tmp_wallet = dash_evo_tool::model::wallet::Wallet::new_from_seed(
                seed,
                Network::Testnet,
                None,
                None,
            )
            .expect("Failed to compute framework wallet hash");
            tmp_wallet.seed_hash()
        };

        // Purge stale wallets from the persistent DB before SPV starts.
        // SPV builds a bloom filter for every loaded wallet address — accumulated
        // test wallets from previous runs cause SPV sync to exceed the 600s timeout.
        {
            let stale: Vec<WalletSeedHash> = {
                let wallets = app_context.wallets().read().expect("wallets lock");
                wallets
                    .keys()
                    .filter(|h| **h != framework_wallet_hash)
                    .copied()
                    .collect()
            };
            if !stale.is_empty() {
                tracing::info!(
                    "Purging {} stale wallet(s) from DB before SPV starts",
                    stale.len()
                );
                for hash in stale {
                    // Log the wallet's balance before removal for audit trail
                    let balance = app_context.snapshot_balance(&hash).total;
                    if balance > 0 {
                        tracing::warn!(
                            "Purging stale wallet {:?} with {} duffs (not swept!)",
                            &hash[..4],
                            balance
                        );
                    }
                    match app_context.remove_wallet(&hash) {
                        Ok(()) => tracing::debug!("Purged stale wallet {:?}", &hash[..4]),
                        Err(e) => {
                            tracing::warn!("Failed to purge stale wallet {:?}: {}", &hash[..4], e)
                        }
                    }
                }
            }
        }

        // Register the framework wallet BEFORE the backend is built so its
        // sidecars (wallet-meta xpub + encrypted-seed envelope) are
        // persisted. `load_from_persistor_seedless` then loads it watch-only
        // from the persister at `WalletBackend::new` time and the upstream
        // manager monitors its addresses from the first sync.
        tracing::info!("Restoring framework wallet from E2E_WALLET_MNEMONIC");
        let wallet = dash_evo_tool::model::wallet::Wallet::new_from_seed(
            seed,
            Network::Testnet,
            Some("E2E Framework Wallet".to_string()),
            None,
        )
        .expect("Failed to create framework wallet");
        match register_wallet_with_retry(
            &app_context,
            wallet,
            &seed,
            dash_evo_tool::model::wallet::birth_height::WalletOrigin::Imported,
        )
        .await
        {
            Ok((hash, _)) => {
                tracing::info!("Registered framework wallet (seed_hash: {:?})", &hash[..4]);
            }
            Err(TaskError::WalletAlreadyImported) => {
                tracing::info!("Framework wallet already registered (reusing from persistent DB)");
            }
            Err(e) => panic!("Failed to register framework wallet: {}", e),
        }

        // Build the long-lived `TaskResult` channel the `EventBridge` pushes
        // `Refresh` nudges onto. Production hands `WalletBackend` the
        // `AppState`-owned sender; the harness owns both ends and keeps the
        // receiver alive in `BackendTestContext` so the bridge's
        // non-blocking `try_send` never hits a closed channel.
        let (task_result_sender, task_result_rx) = tokio::sync::mpsc::channel::<TaskResult>(256)
            .with_egui_ctx(app_context.egui_ctx().clone());

        // Construct + start the real wallet backend exactly as production
        // does: `ensure_wallet_backend` builds `WalletBackend` (which loads
        // the framework wallet watch-only via `load_from_persistor_seedless`), then
        // `WalletBackend::start()` starts the upstream `SpvRuntime` sync.
        app_context
            .ensure_wallet_backend(task_result_sender)
            .await
            .expect("Failed to construct wallet backend");
        let backend = app_context
            .wallet_backend()
            .expect("wallet backend must be wired after ensure_wallet_backend");

        // `ensure_wallet_backend` runs the unlock chokepoint
        // (`bootstrap_loaded_wallets`) at its tail, so the framework wallet's
        // seed is already handed to the backend here — no manual re-run needed.

        // Track the backend before starting sync so a later panic in init
        // (peer wait, balance check, …) doesn't leak the SpvRuntime's
        // data-directory lock — the next retry shuts this down first.
        *LEAKED_BACKEND.lock().await = Some(Arc::clone(&backend));

        backend
            .start()
            .await
            .expect("Failed to start wallet backend chain sync");

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

        // Wait for the upstream SpvRuntime to connect to peers (push-based
        // via the EventBridge → ConnectionStatus).
        wait::wait_for_spv_peers(&app_context, Duration::from_secs(60))
            .await
            .expect("SPV failed to connect to any peers within 60s");
        tracing::info!("SPV connected to peers");

        // Confirm the framework wallet is registered with the upstream
        // manager so its addresses are watched by the SpvRuntime.
        // `wait_for_wallet_in_spv` retries the transient
        // `TaskError::WalletBackend` signal with backoff inside this budget.
        wait::wait_for_wallet_in_spv(
            &app_context,
            framework_wallet_hash,
            FRAMEWORK_WALLET_REGISTRATION_TIMEOUT,
        )
        .await
        .expect("Framework wallet not registered with the wallet backend");

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
                let snap = app_context.snapshot_balance(&framework_wallet_hash);
                let address = app_context
                    .wallet_backend()
                    .expect("wallet backend wired")
                    .next_receive_address(&framework_wallet_hash)
                    .await
                    .unwrap_or_else(|_| "<unknown>".to_string());
                let (confirmed, total) = (snap.confirmed, snap.total);
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

        // Reset the retry bookkeeping: `OnceCell` caches this success, so no
        // further init runs. Clearing the floor keeps the slot accounting
        // deterministic if a future change ever re-enters init.
        WORKDIR_SLOT_FLOOR.store(0, std::sync::atomic::Ordering::SeqCst);

        BackendTestContext {
            app_context,
            framework_wallet_hash,
            _workdir: workdir,
            _lock_file: lock_file,
            _task_result_rx: task_result_rx,
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

        let (seed_hash, wallet_arc) = register_wallet_with_retry(
            app_context,
            wallet,
            &seed,
            dash_evo_tool::model::wallet::birth_height::WalletOrigin::Imported,
        )
        .await
        .expect("Failed to register test wallet");
        tracing::trace!(
            seed_hash = ?&seed_hash[..4],
            amount_duffs,
            "create_funded_test_wallet: registered new wallet"
        );

        // Wait for SPV to pick up the wallet. Budgeted for the cumulative
        // upstream load late in a serial run — see FUNDED_WALLET_REGISTRATION_TIMEOUT.
        wait::wait_for_wallet_in_spv(app_context, seed_hash, FUNDED_WALLET_REGISTRATION_TIMEOUT)
            .await
            .expect("Test wallet not picked up by SPV");
        tracing::trace!(seed_hash = ?&seed_hash[..4], "create_funded_test_wallet: wallet visible in SPV");

        // Allow mempool manager tick (100ms) to detect wallet address change
        // and rebuild bloom filter before we broadcast.
        tokio::time::sleep(Duration::from_millis(200)).await;
        tracing::trace!(seed_hash = ?&seed_hash[..4], "create_funded_test_wallet: waited for bloom filter rebuild tick");

        // Get test wallet's receive address from the SPV-watched upstream pool.
        let test_address = app_context
            .wallet_backend()
            .expect("wallet backend wired")
            .next_receive_address(&seed_hash)
            .await
            .expect("Failed to get test wallet receive address");
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
            override_fee: None,
        };

        let task = BackendTask::CoreTask(CoreTask::SendWalletPayment {
            wallet: framework_wallet_arc,
            request,
        });

        tracing::trace!(seed_hash = ?&seed_hash[..4], "create_funded_test_wallet: acquiring funding mutex...");
        let funding_start = std::time::Instant::now();
        // Critical section — serialize UTXO selection/broadcast so concurrent
        // callers don't double-spend the same outputs from the framework wallet.
        {
            let _guard = FUNDING_MUTEX.lock().await;
            tracing::trace!(seed_hash = ?&seed_hash[..4], "create_funded_test_wallet: broadcasting funding tx...");
            run_task(app_context, task)
                .await
                .expect("Failed to send funds to test wallet");
        }
        tracing::trace!(
            seed_hash = ?&seed_hash[..4],
            elapsed_ms = funding_start.elapsed().as_millis(),
            "create_funded_test_wallet: funding tx broadcast"
        );

        // Wait for test wallet to see the funds
        tracing::trace!(seed_hash = ?&seed_hash[..4], min = amount_duffs, "create_funded_test_wallet: waiting for total balance...");
        wait::wait_for_balance(app_context, seed_hash, amount_duffs, MAX_TEST_TIMEOUT / 3)
            .await
            .expect("Test wallet did not receive expected funds");
        tracing::trace!(
            seed_hash = ?&seed_hash[..4],
            elapsed_ms = funding_start.elapsed().as_millis(),
            "create_funded_test_wallet: total balance reached"
        );

        // Wait for the full funded amount to become spendable so callers can
        // immediately build transactions without racing confirmations/IS locks.
        // Funds MUST be confirmed or IS-locked before proceeding — unconfirmed
        // UTXOs cannot be used for asset-lock transactions.
        tracing::trace!(seed_hash = ?&seed_hash[..4], min = amount_duffs, "create_funded_test_wallet: waiting for spendable balance (IS lock)...");
        match wait::wait_for_spendable_balance(
            app_context,
            seed_hash,
            amount_duffs,
            MAX_TEST_TIMEOUT / 2,
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
            Err(_) => {
                // IS lock timed out — fall back to waiting for block confirmation.
                // A Core block (~2.5 min) will confirm the transaction, making
                // the UTXOs spendable. Use a longer timeout (~2 blocks).
                tracing::warn!(
                    seed_hash = ?&seed_hash[..4],
                    amount_duffs,
                    "create_funded_test_wallet: IS lock timed out, waiting for block confirmation..."
                );
                wait::wait_for_spendable_balance(
                    app_context,
                    seed_hash,
                    amount_duffs,
                    MAX_TEST_TIMEOUT,
                )
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "Test wallet funds not spendable after IS lock + block confirmation \
                         timeouts (elapsed {:?}): {e}",
                        funding_start.elapsed()
                    )
                });
                tracing::info!(
                    seed_hash = ?&seed_hash[..4],
                    elapsed_ms = funding_start.elapsed().as_millis(),
                    "create_funded_test_wallet: funds spendable (block-confirmed)"
                );
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

/// Pick a deterministic workdir, acquiring an exclusive lock file.
///
/// Tries the primary path first (`base`), then falls back to `base-1`, `base-2`,
/// etc. up to 10 slots. Each slot has a `.lock` file that is held for the
/// lifetime of the returned `File` handle (via `flock` / `LockFile`).
///
/// `slot_floor` is the *preferred* first slot to try — bumped across init
/// retries when a panicked predecessor leaked an un-droppable SPV `LockFile`,
/// so the retry prefers a fresh directory over the still-locked one. The scan
/// wraps around and tries every slot, so a `slot_floor` that has climbed to (or
/// past) `MAX_WORKDIR_SLOTS` still falls back to lower slots whose harness
/// `.lock` was released on a previous init's unwind — it never produces an empty
/// scan and a spurious "all slots locked" panic.
///
/// This ensures:
/// - The same workdir is reused across runs (wallets, SPV data, DB persist)
/// - Concurrent test processes get separate workdirs automatically
/// - A retried init after a leaked SPV lock prefers a clean directory, but
///   still recovers a freed lower slot once the leaked floor exceeds the range
fn pick_available_workdir(base: &std::path::Path, slot_floor: usize) -> (PathBuf, std::fs::File) {
    use std::io::Write;

    let preferred = slot_floor % MAX_WORKDIR_SLOTS;
    for offset in 0..MAX_WORKDIR_SLOTS {
        let slot = (preferred + offset) % MAX_WORKDIR_SLOTS;
        let dir = if slot == 0 {
            base.to_path_buf()
        } else {
            base.with_file_name(format!(
                "{}-{}",
                base.file_name().unwrap().to_str().unwrap(),
                slot
            ))
        };

        // Create the directory so the lock file can live inside it
        std::fs::create_dir_all(&dir).ok();

        let lock_path = dir.join(".lock");
        let lock_file = match std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
        {
            Ok(f) => f,
            Err(_) => continue,
        };

        // Try to acquire an exclusive non-blocking lock
        if try_lock_exclusive(&lock_file) {
            // Write PID for debugging
            let mut f = lock_file;
            let _ = f.set_len(0);
            let _ = write!(f, "{}", std::process::id());
            let _ = f.flush();

            if offset > 0 {
                tracing::info!(
                    "Preferred workdir slot {preferred} unavailable, using slot {slot}: {}",
                    dir.display()
                );
            } else if slot != 0 {
                tracing::info!("Using preferred workdir slot {slot}: {}", dir.display());
            }
            return (dir, f);
        }

        tracing::debug!(
            "Workdir slot {} locked by another process, trying next...",
            dir.display()
        );
    }

    panic!(
        "All {MAX_WORKDIR_SLOTS} E2E workdir slots are locked. \
         Kill other test processes or remove lock files in {}*",
        base.display()
    );
}

/// Try to acquire an exclusive non-blocking file lock using POSIX `flock()`.
#[cfg(unix)]
fn try_lock_exclusive(file: &std::fs::File) -> bool {
    use std::os::unix::io::AsRawFd;
    // LOCK_EX (2) | LOCK_NB (4) = exclusive + non-blocking
    // Safety: flock on a valid fd is safe; non-blocking so it won't deadlock.
    unsafe { nix::libc::flock(file.as_raw_fd(), nix::libc::LOCK_EX | nix::libc::LOCK_NB) == 0 }
}

// Non-Unix has no file locking here, so concurrent processes share a workdir;
// acceptable because CI is Linux and Windows E2E runs are rare.
#[cfg(not(unix))]
fn try_lock_exclusive(_file: &std::fs::File) -> bool {
    true
}
