pub mod connection_status;
mod contested_names_db;
mod contract_token_db;
mod identity_db;
pub mod migration_status;
mod settings_db;
mod wallet_lifecycle;

use crate::app_dir::core_cookie_path;
use crate::backend_task::error::{TaskError, is_rpc_connection_error};
use crate::config::{Config, NetworkConfig};
use crate::context_provider_spv::SpvProvider;
use crate::database::Database;
use crate::model::feature_gate::FeatureGate;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::proof_log_item::RequestType;
use crate::model::wallet::single_key::{SingleKeyHash, SingleKeyWallet};
use crate::model::wallet::{PlatformAddressUpdates, Wallet, WalletSeedHash};
use crate::sdk_wrapper::initialize_sdk;
use crate::utils::tasks::TaskManager;
use crate::wallet_backend::{
    DetKv, DetWalletBalance, NullSecretPrompt, SecretPrompt, UpstreamFromPersisted, WalletBackend,
};
use arc_swap::{ArcSwap, ArcSwapOption};
use connection_status::ConnectionStatus;
use dash_sdk::Sdk;
use dash_sdk::dapi_client::AddressList;
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::Network;
#[cfg(any(test, feature = "testing"))]
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::state_transition::StateTransitionSigningOptions;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::version::v11::PLATFORM_V11;
use dash_sdk::platform::DataContract;
#[cfg(any(test, feature = "testing"))]
use dash_sdk::platform::Identifier;
use egui::Context;
use migration_status::MigrationStatus;
use platform_wallet_storage::secrets::SecretStore;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard};

use crate::model::settings::AppSettings;

const ANIMATION_REFRESH_TIME: std::time::Duration = std::time::Duration::from_millis(100);

/// A guard that ensures settings cache invalidation happens atomically
///
/// This guard holds a write lock on the cached settings, preventing reads
/// until the k/v update is complete and the cache is properly invalidated.
pub(crate) type SettingsCacheGuard<'a> = RwLockWriteGuard<'a, Option<AppSettings>>;

#[derive(Debug)]
pub struct AppContext {
    pub(crate) data_dir: PathBuf,
    pub(crate) network: Network,
    developer_mode: AtomicBool,
    pub(crate) db: Arc<Database>,
    pub(crate) sdk: ArcSwap<Sdk>,
    // SDK context provider (quorum keys via DAPI). Chain sync is SPV-only,
    // owned by upstream platform-wallet.
    spv_context_provider: RwLock<SpvProvider>,
    pub(crate) config: Arc<RwLock<NetworkConfig>>,
    pub(crate) dpns_contract: Arc<DataContract>,
    pub(crate) withdraws_contract: Arc<DataContract>,
    pub(crate) dashpay_contract: Arc<DataContract>,
    pub(crate) token_history_contract: Arc<DataContract>,
    pub(crate) keyword_search_contract: Arc<DataContract>,
    pub(crate) core_client: RwLock<Client>,
    pub(crate) has_wallet: AtomicBool,
    /// One-shot-per-session latch for the automatic all-wallets identity sweep.
    /// Set the first time Platform becomes reachable (masternode list `Synced`)
    /// so the sweep runs once; cleared in
    /// [`stop_spv`](Self::stop_spv) so a reconnect re-arms it.
    identity_autodiscovery_fired: AtomicBool,
    pub(crate) wallets: RwLock<BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>>,
    pub(crate) single_key_wallets: RwLock<BTreeMap<SingleKeyHash, Arc<RwLock<SingleKeyWallet>>>>,
    /// Whether to animate the UI elements.
    ///
    /// This is used to control animations in the UI, such as loading spinners or transitions.
    /// Disable for automated tests.
    animate: AtomicBool,
    /// Cached settings to avoid repeated k/v reads + bincode decoding.
    /// Use RwLock to allow multiple readers but exclusive writers for cache invalidation.
    cached_settings: RwLock<Option<AppSettings>>,
    /// Shared app-level k/v store at `<data_dir>/det-app.sqlite`.
    /// Cross-network, global-scoped slot used for `AppSettings` and other
    /// DET-owned application data that must outlive a single network's
    /// wallet persister. Cheap to clone (`Arc<DetKv>` is `Arc`-backed).
    app_kv: Arc<DetKv>,
    /// Shared encrypted HD-seed vault at `<data_dir>/secrets/det-secrets.pwsvault`.
    /// Opened once and handed to every per-network `AppContext` and to the
    /// `WalletBackend`, because the file backend takes an exclusive advisory
    /// lock — a second open of the same vault returns `AlreadyLocked`. Holding
    /// the handle here lets `register_wallet` persist the seed-envelope sidecar
    /// before the wallet backend is wired (PROJ-010). Cross-network: a single
    /// imported WIF / HD seed is keyed by its hash, not by the chain prefix.
    secret_store: Arc<SecretStore>,
    // subtasks started by the app context, used for graceful shutdown
    pub(crate) subtasks: Arc<TaskManager>,
    /// Tracks the connection status to currently active network
    pub(crate) connection_status: Arc<ConnectionStatus>,
    /// Tracks the legacy-data migration progress. Cheap to read each
    /// frame from the UI. Always present and idle on fresh installs;
    /// driven by [`MigrationTask::FinishUnwire`](crate::backend_task::migration::MigrationTask).
    pub(crate) migration_status: Arc<MigrationStatus>,
    /// Pending wallet selection - set after creating/importing a wallet
    /// so the wallet screen can auto-select the new wallet
    pub(crate) pending_wallet_selection: Mutex<Option<WalletSeedHash>>,
    /// Currently selected HD wallet (persisted across screen navigation)
    pub(crate) selected_wallet_hash: Mutex<Option<WalletSeedHash>>,
    /// Currently selected single key wallet (persisted across screen navigation)
    pub(crate) selected_single_key_hash: Mutex<Option<SingleKeyHash>>,
    /// Cached fee multiplier permille from current epoch (1000 = 1x, 2000 = 2x)
    /// Updated when epoch info is fetched from Platform
    fee_multiplier_permille: AtomicU64,
    /// Cached protocol version from the current epoch on the connected network.
    /// Updated alongside fee_multiplier when epoch info is fetched.
    /// 0 means not yet fetched from the network.
    platform_protocol_version: AtomicU32,
    /// Frame-safe shielded balance snapshot (credits, summed across all Orchard accounts).
    ///
    /// Written by the Phase-E `on_shielded_sync_completed` event handler from the
    /// upstream `ShieldedSyncManager` 60-second loop; read synchronously in the frame
    /// loop via [`Self::shielded_balance_duffs`].  Starts empty — returns 0 until the
    /// first completed sync delivers a balance.  Must never be written from the frame
    /// loop (Nagatha ruling: no `block_in_place`/`block_on` on the UI thread).
    pub(crate) shielded_balances: Arc<Mutex<std::collections::HashMap<WalletSeedHash, u64>>>,
    /// Frame-safe platform-address balance snapshot (duffs, summed across owned addresses).
    ///
    /// Written by `on_platform_address_sync_completed` in [`EventBridge`] after each
    /// coordinator pass; read synchronously in the frame loop via
    /// [`Self::platform_balance_duffs`].
    ///
    /// **Cold-start behaviour:** seeded at boot by [`Self::warm_start_platform_addresses`]
    /// from the persisted upstream `platform_addresses` rows (clean per-wallet owned
    /// state — no orphan-inflation risk), so the total renders immediately and
    /// network-independently; the first coordinator pass then overwrites it.
    ///
    /// **Wallet-removal behaviour (QA-B-004, accepted):** removing a wallet does NOT
    /// evict its entry from this map (consistent with `shielded_balances` — same known
    /// gap, see SEC-003 in the grumpy review). The stale value is never displayed because
    /// removed wallets are not iterated in the UI; this is a memory leak, not a display
    /// bug. A future cleanup pass should mirror `SnapshotStore::forget_wallet`.
    ///
    /// Must never be written from the frame loop (Nagatha ruling).
    pub(crate) platform_balances: Arc<Mutex<std::collections::HashMap<WalletSeedHash, u64>>>,
    /// Frame-safe `(last_sync_timestamp, sync_height)` snapshot keyed by
    /// `WalletSeedHash`, written by the coordinator's platform-address sync pass
    /// (the same pass that fills `platform_balances`) and read each frame via
    /// [`AppContext::platform_sync_info`] to drive the "Addresses synced" label.
    /// Shares `platform_balances`' wallet-removal leak (accepted, QA-B-004) and
    /// must never be written from the frame loop.
    pub(crate) platform_sync_cursors:
        Arc<Mutex<std::collections::HashMap<WalletSeedHash, (u64, u64)>>>,
    /// The egui context, stored for use in non-UI code paths (e.g. display_task_result).
    /// Clone is O(1) — egui::Context is Arc-backed and the same instance for the app lifetime.
    egui_ctx: egui::Context,
    /// The wallet seam. Lazily built once `AppState` has wired the
    /// `TaskResult` sender (it lives on `AppState`, not here) — see
    /// [`Self::ensure_wallet_backend`]. `None` until that first call;
    /// wallet/identity task arms degrade to `WalletBackendNotYetWired`
    /// while unset.
    wallet_backend: ArcSwapOption<WalletBackend>,
    /// Serializes the lazy wallet-backend construction so two concurrent
    /// first-tasks cannot both run the expensive `WalletBackend::new` (which
    /// takes an exclusive advisory lock on the persistor file). The
    /// double-checked `ArcSwapOption` read still serves the steady state
    /// lock-free; this `Mutex` is held only across the one-time build.
    wallet_backend_build: tokio::sync::Mutex<()>,
    /// The just-in-time secret prompt host (UI seam). Defaults to
    /// [`NullSecretPrompt`] (headless: no interactive unlock); the GUI installs
    /// an `EguiSecretPromptHost` before the wallet backend is built via
    /// [`Self::install_secret_prompt`]. Read by [`Self::ensure_wallet_backend`]
    /// to construct the backend's `SecretAccess` chokepoint. `Mutex` (not
    /// `ArcSwap`, which needs a `Sized` payload) so the host can be installed
    /// after `AppContext::new` but before the backend reads it; contention is
    /// nil (touched only at install and backend construction).
    secret_prompt: SecretPromptSlot,
}

/// Mutex-guarded slot for the installable secret-prompt host, with an opaque
/// `Debug` (the host is `dyn` and not `Debug`) so [`AppContext`] keeps its
/// derived `Debug`.
struct SecretPromptSlot(Mutex<Arc<dyn SecretPrompt>>);

impl std::fmt::Debug for SecretPromptSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretPromptSlot")
    }
}

impl AppContext {
    // The constructor takes the app's foundational dependencies — the shared
    // db, k/v store, and seed vault all have to be opened once and threaded in
    // so every per-network context reuses the same handle (the vault's
    // exclusive advisory lock forbids a second open).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data_dir: PathBuf,
        network: Network,
        db: Arc<Database>,
        subtasks: Arc<TaskManager>,
        connection_status: Arc<ConnectionStatus>,
        egui_ctx: egui::Context,
        app_kv: Arc<DetKv>,
        secret_store: Arc<SecretStore>,
    ) -> Option<Arc<Self>> {
        let config = match Config::load_from(&data_dir) {
            Ok(config) => config,
            Err(e) => {
                tracing::error!("Failed to load config: {e}");
                return None;
            }
        };

        let network_config = config.config_for_network(network).clone()?;
        let config_lock = Arc::new(RwLock::new(network_config.clone()));

        // Create the SDK context provider; bind to app context later
        // (post construction) due to circularity.
        let spv_provider = match SpvProvider::new(db.clone(), network) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(?network, "Failed to initialize SPV provider: {e}");
                return None;
            }
        };

        // Parse configured DAPI addresses directly (no auto-discovery at startup)
        let address_list = match &network_config.dapi_addresses {
            Some(addrs) if !addrs.trim().is_empty() => match AddressList::from_str(addrs.trim()) {
                Ok(list) => list,
                Err(e) => {
                    tracing::error!(
                        ?network,
                        error = %e,
                        "Failed to parse configured DAPI addresses"
                    );
                    return None;
                }
            },
            _ => {
                tracing::error!(
                    ?network,
                    "No DAPI addresses configured. Use Refresh DAPI endpoints in Network Settings or add addresses to .env."
                );
                return None;
            }
        };

        // Default to SPV provider initially; UI can switch backend after
        let sdk = match initialize_sdk(address_list, network, spv_provider.clone()) {
            Ok(sdk) => sdk,
            Err(e) => {
                tracing::error!("Failed to initialize SDK: {e}");
                return None;
            }
        };
        let platform_version = sdk.version();

        let dpns_contract =
            match load_system_data_contract(SystemDataContract::DPNS, platform_version) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(?network, "Failed to load DPNS contract: {e}");
                    return None;
                }
            };

        let withdrawal_contract =
            match load_system_data_contract(SystemDataContract::Withdrawals, platform_version) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(?network, "Failed to load Withdrawals contract: {e}");
                    return None;
                }
            };

        let token_history_contract =
            match load_system_data_contract(SystemDataContract::TokenHistory, platform_version) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(?network, "Failed to load TokenHistory contract: {e}");
                    return None;
                }
            };

        let keyword_search_contract =
            match load_system_data_contract(SystemDataContract::KeywordSearch, platform_version) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(?network, "Failed to load KeywordSearch contract: {e}");
                    return None;
                }
            };

        let dashpay_contract =
            match load_system_data_contract(SystemDataContract::Dashpay, platform_version) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(?network, "Failed to load Dashpay contract: {e}");
                    return None;
                }
            };

        let addr = format!(
            "http://{}:{}",
            network_config.rpc_host(),
            network_config.rpc_port(network)
        );
        let core_client = match Self::create_core_rpc_client(
            &addr,
            network,
            &network_config.devnet_name,
            &network_config,
        ) {
            Ok(client) => client,
            Err(e) => {
                tracing::error!(?network, "Failed to create CoreClient: {e}");
                return None;
            }
        };

        // T-W-01 / T-W-01b: both HD and single-key wallets are now
        // rehydrated from the upstream `SecretStore` + DET k/v sidecars
        // by `WalletBackend::new`, not from the legacy `wallet` /
        // `single_key_wallet` SQLite tables. The maps start empty here
        // and are filled inside `ensure_wallet_backend` (see
        // `WalletBackend::hydrate_context_wallets`).
        let wallets: BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>> = BTreeMap::new();
        let single_key_wallets: BTreeMap<SingleKeyHash, Arc<RwLock<SingleKeyWallet>>> =
            BTreeMap::new();

        let developer_mode_enabled = config.developer_mode.unwrap_or(false);

        let animate = match developer_mode_enabled {
            true => {
                tracing::debug!("developer_mode is enabled, disabling animations");
                AtomicBool::new(false)
            }
            false => AtomicBool::new(true), // Animations are enabled by default
        };

        // Wallet selection is restored from the per-network wallet k/v
        // store inside `ensure_wallet_backend` once the backend is
        // wired. At `AppContext::new` time the backend does not exist
        // yet, so both selections start as `None` and are populated
        // lazily by the eager backend init in `AppState`.
        let selected_wallet_hash: Option<WalletSeedHash> = None;
        let selected_single_key_hash: Option<SingleKeyHash> = None;

        let app_context = AppContext {
            data_dir,
            network,
            developer_mode: AtomicBool::new(developer_mode_enabled),
            db,
            sdk: ArcSwap::from_pointee(sdk),
            spv_context_provider: spv_provider.into(),
            config: config_lock,
            dpns_contract: Arc::new(dpns_contract),
            withdraws_contract: Arc::new(withdrawal_contract),
            dashpay_contract: Arc::new(dashpay_contract),
            token_history_contract: Arc::new(token_history_contract),
            keyword_search_contract: Arc::new(keyword_search_contract),
            core_client: core_client.into(),
            has_wallet: (!wallets.is_empty() || !single_key_wallets.is_empty()).into(),
            identity_autodiscovery_fired: AtomicBool::new(false),
            wallets: RwLock::new(wallets),
            single_key_wallets: RwLock::new(single_key_wallets),
            animate,
            cached_settings: RwLock::new(None),
            app_kv,
            secret_store,
            subtasks,
            connection_status,
            migration_status: Arc::new(MigrationStatus::new_idle()),
            pending_wallet_selection: Mutex::new(None),
            selected_wallet_hash: Mutex::new(selected_wallet_hash),
            selected_single_key_hash: Mutex::new(selected_single_key_hash),
            fee_multiplier_permille: AtomicU64::new(
                PlatformFeeEstimator::DEFAULT_FEE_MULTIPLIER_PERMILLE,
            ),
            platform_protocol_version: AtomicU32::new(0),
            shielded_balances: Arc::new(Mutex::new(std::collections::HashMap::new())),
            platform_balances: Arc::new(Mutex::new(std::collections::HashMap::new())),
            platform_sync_cursors: Arc::new(Mutex::new(std::collections::HashMap::new())),
            egui_ctx,
            wallet_backend: ArcSwapOption::const_empty(),
            wallet_backend_build: tokio::sync::Mutex::new(()),
            secret_prompt: SecretPromptSlot(Mutex::new(
                Arc::new(NullSecretPrompt) as Arc<dyn SecretPrompt>
            )),
        };

        let app_context = Arc::new(app_context);
        // Bind the SDK context provider. Chain sync is SPV-only (owned by
        // upstream platform-wallet); the SPV provider is the sole SDK
        // quorum/context provider.
        if let Err(e) = app_context
            .spv_context_provider
            .read()
            .map_err(|e| e.to_string())
            .and_then(|provider| provider.bind_app_context(app_context.clone()))
        {
            tracing::error!("Failed to bind SPV provider: {}", e);
            return None;
        }

        Some(app_context)
    }

    /// Enables animations in the UI.
    ///
    /// This is used to control whether UI elements should animate, such as loading spinners or transitions.
    pub fn enable_animations(&self, animate: bool) {
        self.animate.store(animate, Ordering::Relaxed);
    }

    pub fn enable_developer_mode(&self, enable: bool) {
        self.developer_mode.store(enable, Ordering::Relaxed);
        // Animations are reverse of developer mode
        self.enable_animations(!enable);
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    /// Shared app-level k/v store. Cheap clone — `Arc<DetKv>` is `Arc`-backed.
    pub fn app_kv(&self) -> Arc<DetKv> {
        Arc::clone(&self.app_kv)
    }

    /// Shared encrypted HD-seed vault. Cheap clone — `Arc<SecretStore>` is
    /// `Arc`-backed. The wallet backend reuses this same handle rather than
    /// opening its own, because the file vault takes an exclusive advisory
    /// lock that a second open would trip.
    pub fn secret_store(&self) -> Arc<SecretStore> {
        Arc::clone(&self.secret_store)
    }

    /// Shared migration-status handle. Cheap to clone — backed by `Arc`.
    /// Readers (the UI hot path) call `.state()` each frame; writers
    /// (the [`MigrationTask`](crate::backend_task::migration::MigrationTask)
    /// orchestrator) call `.set_state()`.
    pub fn migration_status(&self) -> Arc<MigrationStatus> {
        Arc::clone(&self.migration_status)
    }

    /// Open (or create) the shared app k/v store at
    /// `<data_dir>/det-app.sqlite`. Used by every `AppContext::new`
    /// callsite — pass a single `Arc<DetKv>` to all per-network
    /// contexts so they share the same blob.
    pub fn open_app_kv(
        data_dir: &std::path::Path,
    ) -> Result<Arc<DetKv>, platform_wallet_storage::WalletStorageError> {
        use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig};
        let path = data_dir.join("det-app.sqlite");
        let config = SqlitePersisterConfig::new(path);
        let persister = Arc::new(SqlitePersister::open(config)?);
        Ok(Arc::new(DetKv::new(persister)))
    }

    /// Open (or create) the shared encrypted seed vault at
    /// `<data_dir>/secrets/det-secrets.pwsvault`. Called once per process and
    /// the resulting `Arc<SecretStore>` is handed to every per-network
    /// `AppContext` and reused by the `WalletBackend` — the file backend holds
    /// an exclusive advisory lock for the handle's lifetime, so a second open
    /// of the same vault would fail with `AlreadyLocked`. The vault is
    /// cross-network: seeds and imported keys are scoped by hash, not chain.
    pub fn open_secret_store(data_dir: &std::path::Path) -> Result<Arc<SecretStore>, TaskError> {
        let mut path = data_dir.to_path_buf();
        path.push("secrets");
        path.push("det-secrets.pwsvault");
        crate::wallet_backend::single_key::open_secret_store(&path)
            .map(Arc::new)
            .map_err(|source| TaskError::SecretStore {
                source: Box::new(source),
            })
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn connection_status(&self) -> &ConnectionStatus {
        &self.connection_status
    }

    pub fn egui_ctx(&self) -> &egui::Context {
        &self.egui_ctx
    }

    /// Get the cached fee multiplier permille (1000 = 1x, 2000 = 2x)
    pub fn fee_multiplier_permille(&self) -> u64 {
        self.fee_multiplier_permille.load(Ordering::Relaxed)
    }

    /// Update the cached fee multiplier from epoch info
    pub fn set_fee_multiplier_permille(&self, multiplier: u64) {
        self.fee_multiplier_permille
            .store(multiplier, Ordering::Relaxed);
    }

    /// Get the cached platform protocol version from the connected network.
    /// Returns 0 if not yet fetched from the network.
    pub fn platform_protocol_version(&self) -> u32 {
        self.platform_protocol_version.load(Ordering::Relaxed)
    }

    /// Update the cached platform protocol version from epoch info.
    ///
    /// When the version crosses the shielded threshold for the first time,
    /// retroactively initializes shielded wallets that were unlocked before
    /// the protocol version was known.
    pub fn set_platform_protocol_version(self: &Arc<Self>, version: u32) {
        let was_shielded = FeatureGate::Shielded.is_available(self);

        self.platform_protocol_version
            .swap(version, Ordering::Relaxed);

        if !was_shielded && FeatureGate::Shielded.is_available(self) {
            self.init_missing_shielded_wallets();
        }
    }

    /// Synchronous read of the frame-safe shielded balance for `seed_hash`.
    ///
    /// Returns the total shielded balance in duffs, summed across all Orchard accounts.
    /// Returns `0` if no sync has completed yet (field is empty on cold boot).
    ///
    /// This is the **read side** of the push snapshot; the write side is
    /// `on_shielded_sync_completed` in Phase E.  Safe to call from the egui frame
    /// loop — no blocking I/O, no async (Nagatha ruling).
    pub fn shielded_balance_duffs(&self, seed_hash: &WalletSeedHash) -> u64 {
        use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
        self.shielded_balances
            .lock()
            .ok()
            .and_then(|map| map.get(seed_hash).copied())
            .unwrap_or(0)
            / CREDITS_PER_DUFF
    }

    /// Synchronous, frame-safe reader for the per-wallet shielded balance in
    /// Platform **credits** (the native unit for Platform/Orchard operations).
    /// Returns 0 when no snapshot has been written yet.
    ///
    /// Use this for screens that display or operate on credits (shielded send,
    /// unshield, coin-selection).  For screens that sum Core + Platform +
    /// Shielded in duffs, use [`Self::shielded_balance_duffs`] instead.
    pub fn shielded_balance_credits(&self, seed_hash: &WalletSeedHash) -> u64 {
        self.shielded_balances
            .lock()
            .ok()
            .and_then(|map| map.get(seed_hash).copied())
            .unwrap_or(0)
    }

    /// Synchronous read of the frame-safe platform-address balance for `seed_hash`.
    ///
    /// Returns the total platform balance in **duffs**, summed across all OWNED platform
    /// payment addresses for the wallet.  Returns `0` if no sync has completed yet.
    ///
    /// This is the **read side** of the coordinator-push snapshot; the write side is
    /// `on_platform_address_sync_completed` in [`EventBridge`]. Safe to call from
    /// the egui frame loop — no blocking I/O, no async (Nagatha ruling).
    pub fn platform_balance_duffs(&self, seed_hash: &WalletSeedHash) -> u64 {
        self.platform_balances
            .lock()
            .ok()
            .and_then(|map| map.get(seed_hash).copied())
            .unwrap_or(0)
    }

    /// Synchronous read of the latest platform-address sync cursor
    /// `(last_sync_timestamp, sync_height)` for `seed_hash`, or `None` when no
    /// coordinator pass has reported a funded address for the wallet yet.
    ///
    /// Read side of the same coordinator-push snapshot as
    /// [`platform_balance_duffs`](Self::platform_balance_duffs); the write side
    /// is `on_platform_address_sync_completed` in [`EventBridge`]. Drives the
    /// "Addresses synced" label. Safe to call from the egui frame loop.
    pub fn platform_sync_info(&self, seed_hash: &WalletSeedHash) -> Option<(u64, u64)> {
        self.platform_sync_cursors
            .lock()
            .ok()
            .and_then(|map| map.get(seed_hash).copied())
    }

    /// Drop the cached sync cursor for `seed_hash` so the "Addresses synced"
    /// label reverts to "never synced" — used by the developer "Clear Platform
    /// Addresses" tool after it wipes the in-memory address pools.
    pub fn clear_platform_sync_info(&self, seed_hash: &WalletSeedHash) {
        if let Ok(mut map) = self.platform_sync_cursors.lock() {
            map.remove(seed_hash);
        }
    }

    /// Populate each wallet's `platform_address_info` from a coordinator-push batch.
    ///
    /// Called by `AppState` when a [`BackendTaskSuccessResult::PlatformAddressSyncPushed`]
    /// result arrives. Converts each raw 20-byte P2PKH hash in `updates` to a
    /// `dashcore::Address` using the active network, then calls
    /// [`Wallet::set_platform_address_info`] for each address.
    ///
    /// This keeps the per-address tab consistent with the coordinator-push total
    /// balance without requiring a manual Refresh on cold start.
    pub fn apply_platform_address_push(&self, updates: PlatformAddressUpdates) {
        use dash_sdk::dpp::key_wallet::PlatformP2PKHAddress;
        let network = self.network;
        if let Ok(wallets) = self.wallets.read() {
            for (seed_hash, entries) in updates {
                // Wallet write lock is held briefly for a pure BTreeMap update —
                // no I/O, no network, no await. Consistent with the codebase's
                // existing frame-loop write pattern (QA-B2-002, intentional).
                if let Some(wallet_arc) = wallets.get(&seed_hash)
                    && let Ok(mut wallet) = wallet_arc.write()
                {
                    for (hash_bytes, balance, nonce) in entries {
                        let addr = PlatformP2PKHAddress::new(hash_bytes).to_address(network);
                        let canonical = Wallet::canonical_address(&addr, network);
                        wallet.set_platform_address_info(canonical, balance, nonce);
                    }
                }
            }
        }
    }

    /// Warm-start the platform-address UI from persisted upstream state.
    ///
    /// Runs once at boot, after the wallet backend is wired and before the
    /// coordinator's first (network) pass. Seeds the per-address tab
    /// (`platform_address_info`), the frame-safe total balance
    /// (`platform_balances`), and the "Addresses synced" cursor
    /// (`platform_sync_cursors`) so the whole platform section renders the
    /// last-synced snapshot immediately — network-independent, with no
    /// "never synced" gap on an offline cold boot. Each map is seeded with
    /// `or_insert`, so a live coordinator push that already landed wins.
    pub(crate) fn warm_start_platform_addresses(&self) {
        use dash_sdk::dpp::balances::credits::CREDITS_PER_DUFF;
        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        let seeds = backend.persisted_platform_address_warm_start();
        if seeds.is_empty() {
            return;
        }

        let updates: PlatformAddressUpdates = seeds
            .iter()
            .filter(|(_, entries, _)| !entries.is_empty())
            .map(|(seed_hash, entries, _)| (*seed_hash, entries.clone()))
            .collect();
        self.apply_platform_address_push(updates);

        if let Ok(mut balances) = self.platform_balances.lock() {
            for (seed_hash, entries, _) in seeds.iter().filter(|(_, e, _)| !e.is_empty()) {
                let total_credits: u64 = entries.iter().map(|(_, credits, _)| credits).sum();
                balances
                    .entry(*seed_hash)
                    .or_insert(total_credits / CREDITS_PER_DUFF);
            }
        }

        if let Ok(mut cursors) = self.platform_sync_cursors.lock() {
            for (seed_hash, _, cursor) in &seeds {
                if let Some(cursor) = cursor {
                    cursors.entry(*seed_hash).or_insert(*cursor);
                }
            }
        }
    }

    /// Get a fee estimator configured with the cached fee multiplier.
    /// Use this instead of `PlatformFeeEstimator::new()` to get accurate fee estimates
    /// that reflect the current network fee multiplier.
    pub fn fee_estimator(&self) -> PlatformFeeEstimator {
        PlatformFeeEstimator::with_fee_multiplier(self.fee_multiplier_permille())
    }

    pub fn is_developer_mode(&self) -> bool {
        self.developer_mode.load(Ordering::Relaxed)
    }

    /// Repaints the UI if animations are enabled.
    ///
    /// Called by UI elements that need to trigger a repaint, such as loading spinners or animated icons.
    pub(super) fn repaint_animation(&self, ctx: &Context) {
        if self.animate.load(Ordering::Relaxed) {
            // Request a repaint after a short delay to allow for animations
            ctx.request_repaint_after(ANIMATION_REFRESH_TIME);
        }
    }

    pub fn platform_version(&self) -> &'static PlatformVersion {
        default_platform_version(&self.network)
    }

    pub fn state_transition_options(&self) -> Option<StateTransitionCreationOptions> {
        if self.is_developer_mode() {
            Some(StateTransitionCreationOptions {
                signing_options: StateTransitionSigningOptions {
                    allow_signing_with_any_security_level: true,
                    allow_signing_with_any_purpose: true,
                },
                batch_feature_version: None,
                method_feature_version: None,
                base_feature_version: None,
            })
        } else {
            None
        }
    }

    /// Rebuild both the Dash RPC `core_client` and the `Sdk` using the
    /// updated `NetworkConfig` from `self.config`.
    pub fn reinit_core_client_and_sdk(self: Arc<Self>) -> Result<(), TaskError> {
        // 1. Grab a fresh snapshot of your NetworkConfig
        let cfg = {
            let cfg_lock = self.config.read()?;
            cfg_lock.clone()
        };

        // Note: developer_mode is now global and managed separately

        // 2. Rebuild the RPC client with the new credentials (cookie auth first, then user/pass).
        let addr = format!("http://{}:{}", cfg.rpc_host(), cfg.rpc_port(self.network));
        let new_client = Self::create_core_rpc_client(&addr, self.network, &cfg.devnet_name, &cfg)?;

        // 3. Parse DAPI addresses from config and rebuild the SDK
        let address_list = match &cfg.dapi_addresses {
            Some(addrs) if !addrs.trim().is_empty() => AddressList::from_str(addrs.trim())
                .map_err(|source| {
                    crate::backend_task::dapi_discovery::DapiDiscoveryError::InvalidAddresses {
                        source,
                    }
                })?,
            _ => {
                return Err(
                    crate::backend_task::dapi_discovery::DapiDiscoveryError::AddressesRequired {
                        network: self.network,
                    }
                    .into(),
                );
            }
        };

        let provider = self.spv_context_provider.read()?.clone();
        let new_sdk = initialize_sdk(address_list, self.network, provider)
            .map_err(|e| TaskError::SdkInitializationFailed { detail: e })?;

        // 4. Swap in the new SDK and client
        {
            let mut client_lock = self.core_client.write()?;
            *client_lock = new_client;
        }
        self.sdk.store(Arc::new(new_sdk));

        // Rebind the provider to hold the new AppContext reference.
        // bind_app_context also registers the provider with the SDK.
        self.spv_context_provider
            .read()?
            .bind_app_context(self.clone())
            .map_err(|e| TaskError::SdkInitializationFailed { detail: e })?;

        Ok(())
    }

    /// Create a Core RPC client for the given URL, trying cookie authentication
    /// first and falling back to user/password credentials.
    fn create_core_rpc_client(
        url: &str,
        network: Network,
        devnet_name: &Option<String>,
        cfg: &NetworkConfig,
    ) -> Result<Client, TaskError> {
        if let Ok(cookie_path) = core_cookie_path(network, devnet_name) {
            if let Ok(client) = Client::new(url, Auth::CookieFile(cookie_path.clone())) {
                return Ok(client);
            }
            tracing::trace!(
                "Cookie auth unavailable at {:?}, using user/pass",
                cookie_path,
            );
        }
        Client::new(
            url,
            Auth::UserPass(
                cfg.core_rpc_user.clone().unwrap_or_default(),
                cfg.core_rpc_password.clone().unwrap_or_default(),
            ),
        )
        .map_err(|e| TaskError::CoreRpc { source: e })
    }

    /// Convert an RPC error to `TaskError`, enriching connection failures with
    /// the configured host:port so the user knows which address was unreachable.
    pub(crate) fn rpc_error_with_url(&self, e: dash_sdk::dashcore_rpc::Error) -> TaskError {
        if is_rpc_connection_error(&e) {
            let url = self
                .config
                .read()
                .ok()
                .map(|c| format!("{}:{}", c.rpc_host(), c.rpc_port(self.network)))
                .unwrap_or_else(|| "unknown".to_string());
            TaskError::CoreRpcConnectionFailed {
                url,
                source: Some(Box::new(e)),
            }
        } else {
            TaskError::from(e)
        }
    }

    /// Convert an SDK error to a [`TaskError`], with special handling for
    /// [`dash_sdk::Error::DriveProofError`]: emits a structured tracing event
    /// at target `proof_log` and returns [`TaskError::ProofError`] with the
    /// SDK error preserved as the source.
    ///
    /// All other SDK errors are converted via [`TaskError::from`].
    pub(crate) fn log_drive_proof_error(
        &self,
        e: dash_sdk::Error,
        request_type: RequestType,
    ) -> TaskError {
        match e {
            dash_sdk::Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                tracing::error!(
                    target: "proof_log",
                    request_type = ?request_type,
                    height = block_info.height,
                    time_ms = block_info.time_ms,
                    proof_bytes_len = proof_bytes.len(),
                    %proof_error,
                    "drive proof verification failed",
                );
                TaskError::ProofError {
                    source_error: Box::new(dash_sdk::Error::DriveProofError(
                        proof_error,
                        proof_bytes,
                        block_info,
                    )),
                }
            }
            e => TaskError::from(e),
        }
    }

    /// Lazily build the wallet seam, idempotently.
    ///
    /// `WalletBackend::new` is async and needs the `TaskResult` sender, which
    /// lives on `AppState` — so construction cannot happen in `Self::new`.
    /// `AppState` calls this once it has both the context and the sender.
    /// Subsequent calls are no-ops (first writer wins).
    pub async fn ensure_wallet_backend(
        self: &Arc<Self>,
        task_result_sender: crate::utils::egui_mpsc::SenderAsync<crate::app::TaskResult>,
    ) -> Result<(), TaskError> {
        // Fast path: already wired, no lock needed.
        if self.wallet_backend.load().is_some() {
            return Ok(());
        }
        // Serialize construction so concurrent first-tasks do not both run the
        // expensive build (which takes an exclusive advisory lock on the
        // persistor file). Re-check under the guard — a racer may have wired it
        // while we waited.
        let _build_guard = self.wallet_backend_build.lock().await;
        if self.wallet_backend.load().is_some() {
            return Ok(());
        }
        let sdk = std::sync::Arc::new(self.sdk.load().as_ref().clone());
        let loader = Arc::new(UpstreamFromPersisted::new());
        let backend = WalletBackend::new(
            self,
            sdk,
            Arc::clone(&self.connection_status),
            task_result_sender,
            loader,
            self.secret_prompt(),
        )
        .await?;
        self.wallet_backend.store(Some(Arc::new(backend)));
        drop(_build_guard);
        self.restore_selected_wallet_from_kv();
        // Render the platform section (per-address tab, total, "Addresses synced"
        // label) from persisted upstream state immediately — network-independent,
        // before the coordinator's first pass, which only fires once a network
        // sync succeeds.
        self.warm_start_platform_addresses();

        // Bootstrap addresses and promote any verified-open seeds into the
        // JIT chokepoint's session cache for the cold-boot path. Signing no
        // longer depends on this — the chokepoint pulls the seed just-in-time
        // from the encrypted vault, and a no-password wallet signs via the
        // unprotected fast-path with no prompt regardless. This runs after the
        // backend is wired and `ctx.wallets` is populated so address bootstrap
        // has the reconstructed wallets to work from. Idempotent.
        self.bootstrap_loaded_wallets().await;
        Ok(())
    }

    /// Populate the in-memory selected-wallet pointers from the wallet
    /// backend's k/v store. Called from [`Self::ensure_wallet_backend`]
    /// once the backend exists; safe to call again later (idempotent
    /// — re-reads kv and re-validates against currently loaded wallets).
    /// Each pointer is kept only if the referenced wallet is still
    /// loaded for this network, matching the pre-C4 validation step.
    fn restore_selected_wallet_from_kv(&self) {
        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        let selected = backend.get_selected_wallet();

        if let Ok(mut guard) = self.selected_wallet_hash.lock() {
            let candidate = selected
                .hd_wallet_hash
                .filter(|h| self.wallets.read().is_ok_and(|w| w.contains_key(h)));
            *guard = candidate;
        }
        if let Ok(mut guard) = self.selected_single_key_hash.lock() {
            let candidate = selected.single_key_hash.filter(|h| {
                self.single_key_wallets
                    .read()
                    .is_ok_and(|w| w.contains_key(h))
            });
            *guard = candidate;
        }
    }

    /// The wallet seam, or `WalletBackendNotYetWired` if not yet built.
    pub fn wallet_backend(&self) -> Result<Arc<WalletBackend>, TaskError> {
        self.wallet_backend
            .load_full()
            .ok_or(TaskError::WalletBackendNotYetWired)
    }

    /// Install the interactive secret-prompt host (the egui host in the GUI).
    ///
    /// Must be called **before** [`Self::ensure_wallet_backend`] builds the
    /// backend, since that is where the prompt is read into the `SecretAccess`
    /// chokepoint. Headless callers (MCP / CLI) skip this and keep the default
    /// [`NullSecretPrompt`], which surfaces `SecretPromptUnavailable` for any
    /// passphrase-protected scope.
    pub fn install_secret_prompt(&self, prompt: Arc<dyn SecretPrompt>) {
        if let Ok(mut guard) = self.secret_prompt.0.lock() {
            *guard = prompt;
        }
    }

    /// The currently-installed secret-prompt host. Falls back to the headless
    /// [`NullSecretPrompt`] if the lock is poisoned (a panicked installer can
    /// never strand the backend without a prompt).
    pub fn secret_prompt(&self) -> Arc<dyn SecretPrompt> {
        self.secret_prompt
            .0
            .lock()
            .map(|g| Arc::clone(&g))
            .unwrap_or_else(|_| Arc::new(NullSecretPrompt) as Arc<dyn SecretPrompt>)
    }

    /// Persist the per-network selected-wallet pointer to the wallet
    /// backend's k/v store. Logs and swallows the write if the backend
    /// is not yet wired or the kv layer errors — wallet selection is
    /// best-effort persistence (the in-memory mutex in `AppContext`
    /// stays authoritative for the running process).
    pub fn persist_selected_wallet_kv(
        &self,
        hd_wallet_hash: Option<WalletSeedHash>,
        single_key_hash: Option<SingleKeyHash>,
    ) {
        let Ok(backend) = self.wallet_backend() else {
            tracing::debug!("Skipping selected-wallet persist; wallet backend not yet wired");
            return;
        };
        let blob = crate::model::selected_wallet::SelectedWallet {
            hd_wallet_hash,
            single_key_hash,
        };
        if let Err(e) = backend.set_selected_wallet(&blob) {
            tracing::warn!(
                network = ?self.network,
                error = ?e,
                "Failed to persist selected wallet to wallet k/v"
            );
        }
    }

    /// Confirmed / unconfirmed / total chain balance for an HD wallet, read
    /// from the display-only `WalletBackend` snapshot (P4a). Pre-first-sync
    /// (or backend not yet wired) yields a zeroed balance, which callers
    /// render as the existing "syncing" state.
    ///
    /// DISPLAY-ONLY: this never participates in coin selection — spending
    /// goes through `WalletBackend::send_payment` /
    /// `create_asset_lock_proof` (A04 fund-safety gate).
    pub fn snapshot_balance(&self, seed_hash: &WalletSeedHash) -> DetWalletBalance {
        self.wallet_backend()
            .map(|wb| wb.wallet_balance(seed_hash))
            .unwrap_or_default()
    }

    /// Number of UTXOs in the wallet's display snapshot. Used to estimate the
    /// Core (L1) transaction fee for a "Max" send, which spends every UTXO.
    ///
    /// DISPLAY-ONLY: like the other snapshot reads, this never drives coin
    /// selection — it only sizes the fee reserved off the displayed balance.
    pub fn snapshot_utxo_count(&self, seed_hash: &WalletSeedHash) -> usize {
        self.wallet_backend()
            .map(|wb| wb.utxos(seed_hash).len())
            .unwrap_or(0)
    }

    /// Whether the wallet's snapshot shows any confirmed or unconfirmed
    /// funds. Replaces the legacy `Wallet::has_balance` predicate.
    pub fn snapshot_has_balance(&self, seed_hash: &WalletSeedHash) -> bool {
        let b = self.snapshot_balance(seed_hash);
        b.confirmed > 0 || b.unconfirmed > 0
    }

    /// UTXO-derived per-address balances from the snapshot (P4a). Replaces
    /// reads of the legacy `Wallet::address_balances` map.
    pub fn snapshot_address_balances(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> std::collections::BTreeMap<dash_sdk::dpp::dashcore::Address, u64> {
        self.wallet_backend()
            .map(|wb| wb.address_balances(seed_hash))
            .unwrap_or_default()
    }
}

/// Test-only accessors for fields that are normally `pub(crate)`.
#[cfg(any(test, feature = "testing"))]
impl AppContext {
    /// Returns a clone of the current SDK instance.
    pub fn sdk(&self) -> Sdk {
        self.sdk.load().as_ref().clone()
    }

    /// Returns a reference to the database.
    pub fn db(&self) -> &Arc<Database> {
        &self.db
    }

    /// Returns a reference to the wallets map.
    pub fn wallets(&self) -> &RwLock<BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>> {
        &self.wallets
    }

    /// Returns the DashPay contract identifier.
    pub fn dashpay_contract_id(&self) -> Identifier {
        self.dashpay_contract.id()
    }
}

/// Returns the default platform version for the given network.
pub(crate) const fn default_platform_version(network: &Network) -> &'static PlatformVersion {
    // TODO: Ideally use sdk.load().version() but this is a free function with no sdk access
    match network {
        Network::Mainnet => &PLATFORM_V11,
        Network::Testnet => &PLATFORM_V11,
        Network::Devnet => &PLATFORM_V11,
        Network::Regtest => &PLATFORM_V11,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wallet_name_with_spaces_is_url_encoded() {
        let base = "http://127.0.0.1:9998";
        let name = "my test wallet";
        let encoded = urlencoding::encode(name);
        let url = format!("{}/wallet/{}", base, encoded);
        assert_eq!(url, "http://127.0.0.1:9998/wallet/my%20test%20wallet");
        assert!(!url.contains(' '));
    }

    /// A fresh data directory (no pre-existing app k/v blob) must resolve
    /// to the SPV backend marker. The marker now lives in
    /// `AppSettings::core_backend_mode` (the upstream k/v store) — the
    /// legacy `settings.core_backend_mode` column was unwired in C3.
    #[test]
    fn fresh_db_resolves_to_spv_backend_mode() {
        let s = crate::model::settings::AppSettings::default();
        assert_eq!(
            s.core_backend_mode, 1,
            "fresh state should default to SPV (=1)"
        );
    }
}
