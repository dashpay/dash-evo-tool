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
use dash_evo_tool::app_dir::{ENV_FILE, ensure_data_dir_exists, ensure_env_file};
use dash_evo_tool::backend_task::BackendTask;
use dash_evo_tool::backend_task::core::{CoreTask, PaymentRecipient, WalletPaymentRequest};
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::platform_info::PlatformInfoTaskRequestType;
use dash_evo_tool::config::EnvFileError;
use dash_evo_tool::context::AppContext;
use dash_evo_tool::context::connection_status::ConnectionStatus;
use dash_evo_tool::database::test_helpers::create_database_at_path;
use dash_evo_tool::model::user_role::{UserRole, UserRoleCell};
use dash_evo_tool::model::wallet::WalletSeedHash;
use dash_evo_tool::utils::egui_mpsc::EguiMpscAsync;
use dash_evo_tool::utils::tasks::TaskManager;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::dashcore::blockdata::constants::MAX_MONEY;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env::VarError;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
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
/// `get_or_init` serializes concurrent callers, so parallel test threads never
/// race into `init()` together — but it does not cache a panicked init, so the
/// next test retries from scratch.
///
/// A retry cannot reuse the failed attempt's workdir. `AppContext::new` ends in
/// `SpvProvider::bind_app_context`, which stores a strong `Arc<AppContext>`
/// inside a field of that same `AppContext` — the resulting cycle means no
/// `AppContext` is ever dropped. Its `Arc<DetKv>` therefore keeps
/// `det-app.sqlite` in the upstream `SqlitePersister` process-wide open-path
/// registry (released only on `Drop`), and reopening it returns `AlreadyOpen`.
/// [`open_available_workdir`] handles this by treating a slot whose stores are
/// still held open as taken, so the real init failure surfaces on the retry
/// instead of a misleading `AlreadyOpen`.
///
/// None of that is why the suite needs `--test-threads=1`. Serial execution is
/// required because the tests share mutable chain state: one framework wallet's
/// UTXO set (`FUNDING_MUTEX` serializes only the broadcast, not the balance
/// waits), the `SHARED_IDENTITY` fixture, and a cleanup sweep that removes every
/// wallet but the framework one.
static CTX: tokio::sync::OnceCell<BackendTestContext> = tokio::sync::OnceCell::const_new();

/// Init attempts allowed per test process before [`BackendTestContext::init`]
/// stops retrying. A transient failure (SPV peer timeout) can clear on a retry;
/// a standing one (unfunded framework wallet, bad `E2E_WALLET_MNEMONIC`) cannot,
/// and retrying it once per remaining test buries the first, real failure.
const MAX_INIT_ATTEMPTS: usize = 3;

/// Init attempts made in this process, counted by [`BackendTestContext::init`].
static INIT_ATTEMPTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

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

/// Number of distinct workdir slots `open_available_workdir` cycles through.
/// The scan wraps modulo this value, so the preferred floor can climb without
/// ever emptying the scan range.
const MAX_WORKDIR_SLOTS: usize = 10;

/// Preferred starting workdir slot for the next `init()`. Bumped each time a
/// panicked init leaks an un-droppable SPV `LockFile`, so the retry prefers a
/// fresh directory instead of colliding with the locked one. The scan in
/// [`open_available_workdir`] wraps modulo [`MAX_WORKDIR_SLOTS`], so even if
/// this floor exceeds the slot count it never exhausts the available slots — a
/// freed lower slot is still picked up.
static WORKDIR_SLOT_FLOOR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Load the project .env before any workdir .env can change the process environment.
pub fn load_project_env() {
    static PROJECT_ENV: OnceLock<()> = OnceLock::new();
    PROJECT_ENV.get_or_init(|| {
        if let Err(error) = dotenvy::dotenv()
            && !error.not_found()
        {
            panic!(
                "Could not load the project .env. Correct the file and retry. ({})",
                EnvFileError::from(error)
            );
        }
    });
}

struct E2eSettings {
    network: Network,
    env_contents: Option<Vec<u8>>,
    devnet: Option<DevnetSettings>,
    bootstrap_amount: Result<Option<u64>, BootstrapAmountError>,
    mnemonic: Option<String>,
}

fn settings() -> &'static E2eSettings {
    static SETTINGS: OnceLock<E2eSettings> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        load_project_env();
        let network = match std::env::var("E2E_NETWORK").as_deref() {
            Err(VarError::NotPresent) | Ok("") | Ok("testnet") => Network::Testnet,
            Ok("devnet") => Network::Devnet,
            _ => panic!("E2E_NETWORK is invalid. Set it to testnet or devnet and retry."),
        };
        let env_contents = std::env::var_os("E2E_ENV_FILE")
            .map(std::fs::read)
            .transpose()
            .expect("Could not read E2E_ENV_FILE. Check the path and permissions and retry.");
        let devnet = if network == Network::Devnet {
            let contents = env_contents.as_deref().expect(
                "Devnet settings are missing. Point E2E_ENV_FILE at the devnet .env and retry.",
            );
            Some(devnet_settings_from_env(contents).unwrap_or_else(|error| panic!("{error}")))
        } else {
            None
        };
        let bootstrap_amount = if network == Network::Devnet
            && std::env::var("E2E_DEVNET_BOOTSTRAP").as_deref() == Ok("1")
        {
            match std::env::var("E2E_PLATFORM_FUND_DUFFS") {
                Ok(value) => parse_bootstrap_amount(Some(&value)).map(Some),
                Err(VarError::NotPresent) => parse_bootstrap_amount(None).map(Some),
                Err(error) => Err(BootstrapAmountError::Encoding { source: error }),
            }
        } else {
            Ok(None)
        };
        E2eSettings {
            network,
            env_contents,
            devnet,
            bootstrap_amount,
            mnemonic: std::env::var("E2E_WALLET_MNEMONIC").ok(),
        }
    })
}

#[derive(Debug, thiserror::Error)]
enum DevnetConfigError {
    #[error("E2E_ENV_FILE is invalid. Correct the .env file and retry.")]
    InvalidFile(#[source] EnvFileError),
    #[error("The devnet name is missing. Set DEVNET_devnet_name in E2E_ENV_FILE and retry.")]
    MissingName,
}

#[derive(PartialEq, Eq)]
struct DevnetSettings {
    name: String,
    entries: BTreeMap<String, String>,
}

fn devnet_config_field(key: &str) -> Option<String> {
    key.strip_prefix("DEVNET_").map(str::to_ascii_lowercase)
}

fn devnet_settings_from_env(contents: &[u8]) -> Result<DevnetSettings, DevnetConfigError> {
    let contents = contents.strip_prefix(b"\xef\xbb\xbf").unwrap_or(contents);
    let mut file_entries = BTreeMap::new();
    for entry in dotenvy::Iter::new(contents) {
        let (key, value) = entry.map_err(|error| DevnetConfigError::InvalidFile(error.into()))?;
        if let Some(field) = devnet_config_field(&key) {
            file_entries.insert(field, value);
        }
    }
    let name = file_entries
        .get("devnet_name")
        .ok_or(DevnetConfigError::MissingName)?
        .trim();
    // Use the same name as the SPV handshake, without Core's datadir prefix.
    let name = name.strip_prefix("devnet-").unwrap_or(name);
    if name.is_empty() {
        return Err(DevnetConfigError::MissingName);
    }
    let name = name.to_owned();

    // Match Config::load_from: file entries override process settings.
    let mut entries: BTreeMap<_, _> = std::env::vars_os()
        .filter_map(|(key, value)| {
            let field = devnet_config_field(key.to_str()?)?;
            Some((field, value.into_string().ok()?))
        })
        .collect();
    entries.extend(file_entries);
    Ok(DevnetSettings { name, entries })
}

#[derive(Debug, thiserror::Error)]
enum BootstrapAmountError {
    #[error(
        "E2E_PLATFORM_FUND_DUFFS is not valid UTF-8. Set it to a whole number of duffs and retry."
    )]
    Encoding {
        #[source]
        source: VarError,
    },
    #[error("E2E_PLATFORM_FUND_DUFFS is invalid. Set it to a whole number of duffs and retry.")]
    InvalidNumber(#[from] std::num::ParseIntError),
    #[error(
        "E2E_PLATFORM_FUND_DUFFS is out of range. Set it to between 1 and {MAX_MONEY} duffs and retry."
    )]
    OutOfRange,
}

fn parse_bootstrap_amount(amount: Option<&str>) -> Result<u64, BootstrapAmountError> {
    const DEFAULT_PLATFORM_FUND_DUFFS: u64 = 2_500_000_000;
    let amount = amount.map_or(Ok(DEFAULT_PLATFORM_FUND_DUFFS), str::parse)?;
    if !(1..=MAX_MONEY).contains(&amount) {
        return Err(BootstrapAmountError::OutOfRange);
    }
    Ok(amount)
}

/// Network selected for this test process.
pub fn e2e_network() -> Network {
    settings().network
}

/// Validated amount when funding is explicitly enabled for Devnet.
pub fn devnet_bootstrap_amount() -> Option<u64> {
    settings()
        .bootstrap_amount
        .as_ref()
        .copied()
        .unwrap_or_else(|error| panic!("{error}"))
}

/// Derive the seed from the framework mnemonic configured for this test process.
pub fn framework_seed() -> [u8; 64] {
    let phrase = settings().mnemonic.as_deref().expect(
        "The framework wallet mnemonic is missing or invalid. Set E2E_WALLET_MNEMONIC to a funded wallet's BIP-39 mnemonic and retry.",
    );
    Mnemonic::parse_in(Language::English, phrase)
        .expect("Invalid E2E_WALLET_MNEMONIC. Correct the BIP-39 mnemonic and retry.")
        .to_seed("")
}

/// A workdir configuration could not be safely prepared or reused.
#[derive(Debug, thiserror::Error)]
pub enum WorkdirEnvError {
    #[error("Could not prepare the workdir .env. Check the directory permissions and retry.")]
    Io(#[from] io::Error),
    #[error(
        "The workdir .env differs from E2E_ENV_FILE. Move the workdir aside or retry with the original E2E_ENV_FILE."
    )]
    Mismatch,
    #[error(
        "The workdir contains test data but its .env is missing. Move the workdir aside or restore its configuration and retry."
    )]
    Missing,
    #[error(
        "The resolved devnet settings changed during this run. Set stable devnet values in E2E_ENV_FILE and restart the tests."
    )]
    DevnetConfigChanged,
}

/// Prepare an isolated workdir with the resolved network configuration.
pub fn write_workdir_env(workdir: &Path) -> Result<(), WorkdirEnvError> {
    let settings = settings();
    prepare_workdir_env(
        workdir,
        settings.env_contents.as_deref(),
        settings.devnet.as_ref(),
    )
}

fn prepare_workdir_env(
    workdir: &Path,
    contents: Option<&[u8]>,
    expected_devnet: Option<&DevnetSettings>,
) -> Result<(), WorkdirEnvError> {
    use std::io::Write;

    ensure_data_dir_exists(workdir)?;
    let Some(contents) = contents else {
        return Ok(ensure_env_file(workdir)?);
    };
    // AppContext loads the file into the process environment. An earlier
    // context can change how dotenv substitutions resolve without changing
    // the file bytes. Reject that drift before opening stores.
    if let Some(expected) = expected_devnet
        && !devnet_settings_from_env(contents).is_ok_and(|current| &current == expected)
    {
        return Err(WorkdirEnvError::DevnetConfigChanged);
    }
    let destination = workdir.join(ENV_FILE);
    match std::fs::read(&destination) {
        Ok(existing) if existing == contents => return Ok(()),
        Ok(_) => return Err(WorkdirEnvError::Mismatch),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    for entry in std::fs::read_dir(workdir)? {
        if entry?.file_name() != ".lock" {
            return Err(WorkdirEnvError::Missing);
        }
    }

    // NamedTempFile starts at 0600 on Unix. Publish only after the full write.
    let mut temporary = tempfile::NamedTempFile::new_in(workdir)?;
    temporary.write_all(contents)?;
    temporary
        .persist_noclobber(destination)
        .map_err(|error| error.error)?;
    Ok(())
}

fn workdir_base(root: &Path, network: Network, devnet_name: Option<&str>) -> PathBuf {
    match devnet_name {
        Some(name) => root.join(format!(
            "dash-evo-e2e-devnet-{}",
            hex::encode(Sha256::digest(name))
        )),
        None => root.join(format!("dash-evo-e2e-{network}")),
    }
}

/// Get (or initialize) the shared test context.
pub async fn ctx() -> &'static BackendTestContext {
    CTX.get_or_init(BackendTestContext::init).await
}

/// Shared backend context for E2E tests.
pub struct BackendTestContext {
    pub app_context: Arc<AppContext>,
    pub framework_wallet_hash: WalletSeedHash,
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
        let attempt = INIT_ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        assert!(
            attempt <= MAX_INIT_ATTEMPTS,
            "backend-e2e context init already failed {MAX_INIT_ATTEMPTS} times in this process; \
             not retrying. Fix the first failure reported above (the earliest `harness.rs` panic \
             in this log) — every later test reports this message instead."
        );

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
        // `open_available_workdir`'s slot fallback exists for).
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

        // E2E_WALLET_MNEMONIC is required. Read and parse it BEFORE any store is
        // opened: a bad value here is the most common init failure, and failing
        // ahead of `open_available_workdir` keeps the first attempt from leaving
        // a workdir slot holding an un-droppable `det-app.sqlite` handle.
        let settings = settings();
        let network = settings.network;
        let seed = framework_seed();
        let framework_wallet_hash = {
            let tmp_wallet =
                dash_evo_tool::model::wallet::Wallet::new_from_seed(seed, network, None, None)
                    .expect("Failed to compute framework wallet hash");
            tmp_wallet.seed_hash()
        };

        // Deterministic workdir — always the same path so the database, wallets,
        // and SPV data persist across runs. If the primary path is locked by
        // another process, fall back to numbered alternatives (slot 1, 2, ...).
        let base = workdir_base(
            &std::env::temp_dir(),
            network,
            settings.devnet.as_ref().map(|devnet| devnet.name.as_str()),
        );
        let slot_floor = WORKDIR_SLOT_FLOOR.load(std::sync::atomic::Ordering::SeqCst);
        let WorkdirHandles {
            dir: workdir,
            lock_file,
            app_kv,
            secret_store,
        } = open_available_workdir(
            &base,
            slot_floor,
            settings.env_contents.as_deref(),
            settings.devnet.as_ref(),
        )
        .unwrap_or_else(|e| panic!("{}", error_report(&e)));
        tracing::info!("E2E workdir: {}", workdir.display());

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
            network,
            db,
            subtasks,
            connection_status,
            egui_ctx,
            app_kv,
            secret_store,
            UserRoleCell::new(UserRole::Power),
        )
        .expect("Failed to create AppContext");

        // Purge stale wallets from the persistent DB before SPV starts.
        // SPV builds a bloom filter for every loaded wallet address — accumulated
        // test wallets from previous runs slow SPV sync past its stall window and cap.
        {
            let stale: Vec<WalletSeedHash> = {
                let wallets = app_context.wallet_context().wallets();
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
            network,
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
            Err(TaskError::WalletAlreadyImported { .. }) => {
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
        // Progress-aware, bounded wait on this runtime: a slow sync keeps its
        // slot and stored headers/filters instead of panicking into an init
        // retry that would restart from genesis on a fresh slot.
        wait::wait_for_spv_sync(&app_context, wait::SPV_STALL_WINDOW, wait::SPV_SYNC_CAP)
            .await
            .unwrap_or_else(|e| panic!("SPV did not finish its initial sync: {e}"));
        tracing::info!("SPV fully synced — mempool bloom filter active");

        run_task(
            &app_context,
            BackendTask::PlatformInfo(PlatformInfoTaskRequestType::CurrentEpochInfo),
        )
        .await
        .expect("Failed to fetch current epoch information");

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
            app_context.network(),
            // Wallet aliases are unique per app; a fixed one fails the second call.
            Some(format!("E2E Test Wallet {:08x}", rand::random::<u32>())),
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

        let framework_wallet_arc = app_context
            .wallet_context()
            .hd_wallet(&self.framework_wallet_hash)
            .expect("framework wallet must exist");

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

/// A workdir slot with every handle the harness needs to hold exclusively.
struct WorkdirHandles {
    dir: PathBuf,
    /// Held for the process lifetime; excludes concurrent test *processes*.
    lock_file: std::fs::File,
    app_kv: Arc<dash_evo_tool::wallet_backend::DetKv>,
    secret_store: Arc<platform_wallet_storage::secrets::SecretStore>,
}

/// Which exclusive store of a workdir slot failed to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkdirStore {
    AppKv,
    SecretVault,
}

impl std::fmt::Display for WorkdirStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AppKv => "app k/v store (det-app.sqlite)",
            Self::SecretVault => "secret vault",
        })
    }
}

/// Why [`open_available_workdir`] could not hand out a workdir slot.
#[derive(Debug, thiserror::Error)]
enum WorkdirOpenError {
    /// The slot directory or its lock file could not be created or opened for a
    /// reason that is not another user owning the slot — an unwritable or
    /// read-only `TMPDIR`, a full disk. Every other slot lives under the same
    /// parent, so the scan stops instead of reporting them all as busy.
    #[error("E2E workdir {} could not be prepared", dir.display())]
    DirectoryUnusable {
        dir: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// `flock` failed for a reason other than the lock being held — the
    /// filesystem does not support locking, or the kernel ran out of lock
    /// records. Retrying other slots on the same filesystem cannot help.
    #[error("E2E workdir {} could not be locked", dir.display())]
    LockUnsupported {
        dir: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("E2E workdir {} could not use the requested configuration", dir.display())]
    ConfigUnusable {
        dir: PathBuf,
        #[source]
        source: WorkdirEnvError,
    },

    /// A store refused to open for a reason another slot cannot avoid, such as
    /// an ancestor directory other users can write to. Cycling the remaining
    /// slots would only repeat the refusal and hide its cause.
    #[error("E2E workdir {} is unusable: its {store} refused to open", dir.display())]
    StoreRefused {
        dir: PathBuf,
        store: WorkdirStore,
        #[source]
        source: Box<TaskError>,
    },

    /// Every slot is held: locked by another test process, still open in this
    /// one after a panicked init, or owned by another user of this machine.
    #[error(
        "All {MAX_WORKDIR_SLOTS} E2E workdir slots are unavailable (locked by another process, \
         or still open in this one after a panicked init). Kill other test processes or remove \
         lock files in {}*",
        base.display()
    )]
    AllSlotsBusy {
        base: PathBuf,
        /// The last in-process "still open" refusal seen during the scan.
        #[source]
        last_held: Option<Box<TaskError>>,
    },
}

/// Whether an I/O failure on a slot's directory or lock file is specific to that
/// slot, so the next one may still work.
///
/// Only the permission cases are: in a shared `TMPDIR`, a slot directory (or its
/// `.lock`) can belong to another user of the machine, and the next slot is
/// usually free. Everything else — a read-only or full filesystem, a missing
/// parent — is a property of the parent directory every slot lives under, so
/// reporting it as "all slots busy" hides the real cause.
/// A read-only or full filesystem cannot be mistaken for a foreign slot, so
/// those stop the scan where they happen. `PermissionDenied` is ambiguous —
/// another user's slot and an unwritable `TMPDIR` raise the same errno — so it
/// skips to the next slot, and the scan decides at its end which of the two it
/// was: a scan that never saw real contention reports the I/O failure instead
/// of blaming locks.
fn is_slot_specific_io_error(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
}

/// Whether a store-open failure means the slot is held by a handle already open
/// in this process (leaked by a panicked init), so another slot may succeed.
///
/// Matches the typed upstream variants only; every other failure is a property
/// of the environment, not of the slot, and must stop the scan.
fn is_slot_held(error: &TaskError) -> bool {
    use platform_wallet_storage::WalletStorageError;
    use platform_wallet_storage::secrets::SecretStoreError;
    match error {
        TaskError::WalletStorage { source } => {
            matches!(source.as_ref(), WalletStorageError::AlreadyOpen { .. })
        }
        TaskError::SecretStore { source } => matches!(**source, SecretStoreError::AlreadyLocked),
        _ => false,
    }
}

/// Render an error with its full `source()` chain, one cause per line.
///
/// `TaskError`'s `Display` is user-facing copy; the actionable diagnostics (for
/// an insecure ancestor: the directory and the command to fix it) live further
/// down the chain.
fn error_report(error: &dyn std::error::Error) -> String {
    let mut report = error.to_string();
    let mut cause = error.source();
    while let Some(e) = cause {
        report.push_str("\n  caused by: ");
        report.push_str(&e.to_string());
        cause = e.source();
    }
    report
}

/// Open a deterministic workdir, acquiring its lock file and exclusive stores.
///
/// Tries the primary path first (`base`), then falls back to `base-1`, `base-2`,
/// etc. up to [`MAX_WORKDIR_SLOTS`]. A slot is usable only when all three
/// exclusive claims succeed: its `.lock` file (`flock`, excludes other test
/// processes), its app k/v store, and its secret vault. Both stores are
/// single-open-per-process — the k/v via the upstream `SqlitePersister`
/// open-path registry, the vault via an advisory lock — and a panicked `init()`
/// leaks both (see [`CTX`]), so "still open in this process" is exactly as
/// disqualifying as "locked by another process" and is handled the same way.
///
/// Any other store failure ([`is_slot_held`] is false) stops the scan with
/// [`WorkdirOpenError::StoreRefused`]: it stems from the environment (e.g. a
/// group-writable `TMPDIR`), so every other slot would fail the same way.
///
/// `slot_floor` is the *preferred* first slot to try — bumped across init
/// retries when a panicked predecessor leaked an un-droppable SPV `LockFile`,
/// so the retry prefers a fresh directory over the still-locked one. The scan
/// wraps around and tries every slot, so a `slot_floor` that has climbed to (or
/// past) `MAX_WORKDIR_SLOTS` still falls back to lower slots whose harness
/// `.lock` was released on a previous init's unwind — it never produces an empty
/// scan and a spurious "all slots locked" error.
///
/// This ensures:
/// - The same workdir is reused across runs (wallets, SPV data, DB persist)
/// - Concurrent test processes get separate workdirs automatically
/// - A retried init after a leaked lock or store handle lands on a clean
///   directory, but still recovers a freed lower slot
fn open_available_workdir(
    base: &std::path::Path,
    slot_floor: usize,
    contents: Option<&[u8]>,
    expected_devnet: Option<&DevnetSettings>,
) -> Result<WorkdirHandles, WorkdirOpenError> {
    use std::io::Write;

    let preferred = slot_floor % MAX_WORKDIR_SLOTS;
    let mut last_held: Option<Box<TaskError>> = None;
    // The last ambiguous `PermissionDenied`, and whether any slot was genuinely
    // taken (its lock held, or its stores still open here). Together they tell a
    // busy machine from an unusable `TMPDIR` once the scan is exhausted.
    let mut last_io_error: Option<(PathBuf, std::io::Error)> = None;
    let mut saw_contention = false;

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

        // Create the directory so the lock file can live inside it. A slot
        // another user owns is skipped; anything else is fatal for every slot.
        if let Err(source) = std::fs::create_dir_all(&dir) {
            if is_slot_specific_io_error(&source) {
                tracing::warn!(
                    "Workdir slot {} is not ours to create, trying next: {source}",
                    dir.display()
                );
                last_io_error = Some((dir, source));
                continue;
            }
            return Err(WorkdirOpenError::DirectoryUnusable { dir, source });
        }

        let lock_path = dir.join(".lock");
        let lock_file = match std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
        {
            Ok(f) => f,
            Err(source) if is_slot_specific_io_error(&source) => {
                tracing::warn!(
                    "Workdir slot {} has a lock file we may not open, trying next: {source}",
                    dir.display()
                );
                last_io_error = Some((dir, source));
                continue;
            }
            Err(source) => return Err(WorkdirOpenError::DirectoryUnusable { dir, source }),
        };

        // Try to acquire an exclusive non-blocking lock. Contention means the
        // slot is taken; any other locking failure applies to every slot.
        match try_lock_exclusive(&lock_file) {
            Ok(true) => {}
            Ok(false) => {
                tracing::debug!(
                    "Workdir slot {} locked by another process, trying next...",
                    dir.display()
                );
                saw_contention = true;
                continue;
            }
            Err(source) => return Err(WorkdirOpenError::LockUnsupported { dir, source }),
        }

        prepare_workdir_env(&dir, contents, expected_devnet).map_err(|source| {
            WorkdirOpenError::ConfigUnusable {
                dir: dir.clone(),
                source,
            }
        })?;

        // Both stores refuse a second open, so opening them here is what makes
        // a slot leaked by a panicked init unusable rather than fatal. Dropping
        // `app_kv` on the vault's error path releases its registry claim.
        let app_kv = match AppContext::open_app_kv(&dir) {
            Ok(app_kv) => app_kv,
            Err(e) if is_slot_held(&e) => {
                tracing::warn!(
                    "Workdir slot {} has an app k/v still open in this process, trying next: {e}",
                    dir.display()
                );
                last_held = Some(Box::new(e));
                saw_contention = true;
                continue;
            }
            Err(source) => {
                return Err(WorkdirOpenError::StoreRefused {
                    dir,
                    store: WorkdirStore::AppKv,
                    source: Box::new(source),
                });
            }
        };
        let secret_store = match AppContext::open_secret_store(&dir) {
            Ok(secret_store) => secret_store,
            Err(e) if is_slot_held(&e) => {
                tracing::warn!(
                    "Workdir slot {} has a secret vault still open in this process, trying next: {e}",
                    dir.display()
                );
                last_held = Some(Box::new(e));
                saw_contention = true;
                continue;
            }
            Err(source) => {
                return Err(WorkdirOpenError::StoreRefused {
                    dir,
                    store: WorkdirStore::SecretVault,
                    source: Box::new(source),
                });
            }
        };

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

        return Ok(WorkdirHandles {
            dir,
            lock_file: f,
            app_kv,
            secret_store,
        });
    }

    // Nothing was ever taken, yet no slot could be prepared: the parent, not the
    // slots, is the problem — report that instead of "kill other test processes".
    if !saw_contention && let Some((dir, source)) = last_io_error {
        return Err(WorkdirOpenError::DirectoryUnusable { dir, source });
    }

    Err(WorkdirOpenError::AllSlotsBusy {
        base: base.to_path_buf(),
        last_held,
    })
}

/// Try to acquire an exclusive non-blocking file lock using POSIX `flock()`.
///
/// `Ok(true)` acquired it, `Ok(false)` another process holds it (`EWOULDBLOCK`),
/// and `Err` is a locking failure that says nothing about contention — the
/// filesystem not supporting locks (`ENOLCK`, `EOPNOTSUPP`, common on network
/// mounts), for instance. The two must not be conflated: only contention means
/// another slot is worth trying.
#[cfg(unix)]
fn try_lock_exclusive(file: &std::fs::File) -> Result<bool, std::io::Error> {
    use std::os::unix::io::AsRawFd;
    // LOCK_EX (2) | LOCK_NB (4) = exclusive + non-blocking
    // Safety: flock on a valid fd is safe; non-blocking so it won't deadlock.
    if unsafe { nix::libc::flock(file.as_raw_fd(), nix::libc::LOCK_EX | nix::libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(nix::libc::EWOULDBLOCK) {
        return Ok(false);
    }
    Err(error)
}

// Non-Unix has no file locking here, so concurrent processes share a workdir;
// acceptable because CI is Linux and Windows E2E runs are rare.
#[cfg(not(unix))]
fn try_lock_exclusive(_file: &std::fs::File) -> Result<bool, std::io::Error> {
    Ok(true)
}

#[cfg(test)]
mod workdir_tests {
    use super::*;
    use platform_wallet_storage::WalletStorageError;
    use platform_wallet_storage::secrets::SecretStoreError;

    #[test]
    fn config_regression_named_devnets_have_distinct_workdir_bases() {
        let root = std::path::Path::new("tmp");
        let alpha = workdir_base(root, Network::Devnet, Some("alpha"));
        assert_ne!(alpha, workdir_base(root, Network::Devnet, Some("beta")));
        assert_ne!(alpha, workdir_base(root, Network::Devnet, Some("Alpha")));
        assert_eq!(alpha, workdir_base(root, Network::Devnet, Some("alpha")));
        assert_eq!(
            workdir_base(root, Network::Devnet, Some("../alpha")).parent(),
            Some(root)
        );
        assert_eq!(
            workdir_base(root, Network::Testnet, None),
            root.join("dash-evo-e2e-testnet")
        );
    }

    #[test]
    fn config_regression_stale_env_is_refused_before_opening_stores() {
        let scratch = tempfile::tempdir().expect("scratch dir");
        let workdir = scratch.path();
        let old = b"DEVNET_core_host=old.example";
        std::fs::write(workdir.join(".env"), old).expect("write old .env");

        let result =
            open_available_workdir(workdir, 0, Some(b"DEVNET_core_host=new.example"), None);

        assert!(matches!(
            result,
            Err(WorkdirOpenError::ConfigUnusable {
                source: WorkdirEnvError::Mismatch,
                ..
            })
        ));
        assert_eq!(std::fs::read(workdir.join(".env")).unwrap(), old);
        assert!(!workdir.join("det-app.sqlite").exists());
        assert!(!workdir.join("secrets").exists());
    }

    #[test]
    fn config_regression_refuses_resolved_devnet_name_drift_before_stores() {
        let expected =
            devnet_settings_from_env(b"DEVNET_devnet_name=alpha\nDEVNET_core_host=alpha.example")
                .unwrap();
        for contents in [
            b"DEVNET_devnet_name=beta\nDEVNET_core_host=alpha.example".as_slice(),
            b"DEVNET_devnet_name=alpha\nDEVNET_core_host=beta.example",
        ] {
            let scratch = tempfile::tempdir().expect("scratch dir");
            let workdir = scratch.path();
            std::fs::write(workdir.join(".env"), contents).unwrap();

            let result = open_available_workdir(workdir, 0, Some(contents), Some(&expected));

            assert!(matches!(
                result,
                Err(WorkdirOpenError::ConfigUnusable {
                    source: WorkdirEnvError::DevnetConfigChanged,
                    ..
                })
            ));
            assert!(!workdir.join("det-app.sqlite").exists());
            assert!(!workdir.join("secrets").exists());
        }
    }

    #[test]
    fn config_regression_accepts_matching_env_and_refuses_missing_config_for_existing_data() {
        let scratch = tempfile::tempdir().expect("scratch dir");
        let workdir = scratch.path();
        let contents = b"DEVNET_core_host=devnet.example";
        std::fs::write(workdir.join(".env"), contents).unwrap();
        prepare_workdir_env(workdir, Some(contents), None).expect("reuse an identical .env");

        let other = workdir.join("used");
        std::fs::create_dir(&other).unwrap();
        std::fs::write(other.join("data.db"), []).unwrap();
        assert!(matches!(
            prepare_workdir_env(&other, Some(contents), None),
            Err(WorkdirEnvError::Missing)
        ));
        assert!(!other.join(".env").exists());
    }

    #[test]
    fn config_regression_devnet_name_uses_last_entry_and_core_prefix() {
        let contents =
            b"\xef\xbb\xbfDEVNET_devnet_name=first\nexport DEVNET_DEVNET_NAME=' devnet-second '\n";
        assert_eq!(devnet_settings_from_env(contents).unwrap().name, "second");
        for contents in [b"".as_slice(), b"DEVNET_devnet_name=devnet-"] {
            assert!(matches!(
                devnet_settings_from_env(contents),
                Err(DevnetConfigError::MissingName)
            ));
        }
        assert!(matches!(
            devnet_settings_from_env(b"DEVNET_devnet_name='"),
            Err(DevnetConfigError::InvalidFile(_))
        ));
    }

    #[test]
    fn config_regression_bootstrap_amount_is_positive_and_in_range() {
        assert_eq!(parse_bootstrap_amount(None).unwrap(), 2_500_000_000);
        assert_eq!(parse_bootstrap_amount(Some("12345")).unwrap(), 12_345);
        assert_eq!(
            parse_bootstrap_amount(Some(&MAX_MONEY.to_string())).unwrap(),
            MAX_MONEY
        );
        for invalid in [
            "",
            "-1",
            "0",
            "0.5",
            "18446744073709551616",
            &(MAX_MONEY + 1).to_string(),
        ] {
            assert!(
                parse_bootstrap_amount(Some(invalid)).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn config_regression_copied_env_and_workdir_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = tempfile::tempdir().expect("scratch dir");
        let workdir = scratch.path().join("isolated");
        std::fs::create_dir(&workdir).unwrap();
        std::fs::set_permissions(&workdir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let contents = b"DEVNET_core_host=devnet.example";
        prepare_workdir_env(&workdir, Some(contents), None).expect("prepare workdir .env");

        let path = workdir.join(".env");
        assert_eq!(std::fs::read(&path).unwrap(), contents);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(&workdir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn slot_held_only_for_in_process_open_handles() {
        assert!(is_slot_held(&TaskError::WalletStorage {
            source: WalletStorageError::AlreadyOpen {
                path: PathBuf::from("/w/det-app.sqlite"),
            }
            .into(),
        }));
        assert!(is_slot_held(&TaskError::SecretStore {
            source: Box::new(SecretStoreError::AlreadyLocked),
        }));

        assert!(!is_slot_held(&TaskError::SecretStore {
            source: Box::new(SecretStoreError::Corruption),
        }));
    }

    #[test]
    fn only_permission_errors_are_treated_as_a_foreign_slot() {
        use std::io::ErrorKind;

        assert!(is_slot_specific_io_error(&std::io::Error::from(
            ErrorKind::PermissionDenied
        )));
        for kind in [
            ErrorKind::ReadOnlyFilesystem,
            ErrorKind::StorageFull,
            ErrorKind::NotFound,
            ErrorKind::InvalidInput,
        ] {
            assert!(
                !is_slot_specific_io_error(&std::io::Error::from(kind)),
                "{kind:?} is a property of the parent, not of one slot"
            );
        }
    }

    /// A read-only parent fails every slot with the same ambiguous
    /// `PermissionDenied`. With no slot ever actually taken, the scan must
    /// report the I/O cause instead of telling the operator to kill test
    /// processes that do not exist.
    #[cfg(unix)]
    #[test]
    fn an_unwritable_parent_reports_io_rather_than_busy_slots() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = tempfile::tempdir().expect("scratch dir");
        let readonly = scratch.path().join("readonly");
        std::fs::create_dir(&readonly).expect("create parent");
        std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o500))
            .expect("chmod 0500");

        let result = open_available_workdir(&readonly.join("dash-evo-e2e-testnet"), 0, None, None);

        // Restore write permission so the tempdir can clean itself up.
        std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o700))
            .expect("chmod 0700");

        let Err(error) = result else {
            panic!("an unwritable parent must not yield a workdir");
        };
        match error {
            WorkdirOpenError::DirectoryUnusable { dir, source } => {
                assert!(
                    dir.starts_with(&readonly),
                    "the named directory must be the slot under the unwritable parent: {dir:?}"
                );
                assert_eq!(source.kind(), std::io::ErrorKind::PermissionDenied);
            }
            other => panic!("expected DirectoryUnusable, got: {other:?}"),
        }
    }
}
