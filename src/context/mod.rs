pub mod connection_status;
mod contested_names_db;
mod contract_token_db;
pub mod feature_gate;
mod identity_db;
#[cfg(test)]
pub(crate) mod lock_probe;
#[cfg(test)]
pub(crate) use identity_db::test_staging;
pub(crate) mod identity_load_registry;
pub mod migration_status;
mod settings_db;
#[cfg(test)]
pub(crate) mod test_support;
mod wallet_lifecycle;
pub use wallet_lifecycle::PrepareGateGuard;

pub use wallet_lifecycle::WalletUnlockRetention;

use crate::app_dir::core_cookie_path;
use crate::backend_task::error::TaskError;
use crate::config::{Config, NetworkConfig};
use crate::context::feature_gate::ExperimentalFeature;
use crate::context_provider::SpvProvider;
use crate::database::Database;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::model::request_type::RequestType;
use crate::model::wallet::single_key::{SingleKeyHash, SingleKeyWallet};
use crate::model::wallet::{PlatformAddressEntry, PlatformAddressUpdates, Wallet, WalletSeedHash};
use crate::sdk_wrapper::initialize_sdk;
use crate::utils::tasks::TaskManager;
use crate::wallet_backend::{
    DetKv, DetWalletBalance, NullSecretPrompt, SecretPrompt, WalletBackend,
};
use arc_swap::{ArcSwap, ArcSwapOption};
use connection_status::ConnectionStatus;
use dash_sdk::Sdk;
use dash_sdk::dapi_client::AddressList;
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::Network;
#[cfg(any(test, feature = "testing"))]
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::StateTransitionSigningOptions;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::version::v12::PLATFORM_V12;
use dash_sdk::platform::DataContract;
use dash_sdk::platform::Identifier;
use egui::Context;
use migration_status::MigrationStatus;
use platform_wallet_storage::secrets::SecretStore;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard};

use crate::model::settings::AppSettings;
use crate::model::user_role::{UserRole, UserRoleCell};

const ANIMATION_REFRESH_TIME: std::time::Duration = std::time::Duration::from_millis(100);
pub const SDK_THREAD_STACK_SIZE: usize = 4 * 1024 * 1024; // 4 MB stack size for each worker thread

/// A guard that ensures settings cache invalidation happens atomically
///
/// This guard holds a write lock on the cached settings, preventing reads
/// until the k/v update is complete and the cache is properly invalidated.
pub(crate) type SettingsCacheGuard<'a> = RwLockWriteGuard<'a, Option<AppSettings>>;

pub(crate) struct ContactRequestActionClaim<'a> {
    registry: &'a Mutex<HashSet<Identifier>>,
    request_id: Identifier,
}

impl Drop for ContactRequestActionClaim<'_> {
    fn drop(&mut self) {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.request_id);
    }
}

#[derive(Debug)]
pub struct AppContext {
    pub(crate) data_dir: PathBuf,
    pub(crate) network: Network,
    /// App-global user role (the persona / disclosure axis). A single
    /// [`UserRoleCell`] is created once by `AppState` and cloned into every
    /// per-network `AppContext`, so setting it on any context is observed by all
    /// of them (present, and lazily created on a later network switch). Never a
    /// per-context cell — that would let the left-nav feature gate read a stale
    /// value on whichever context the app renders from.
    user_role: UserRoleCell,
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
    /// How far this session's most recent load of each identity got, stamped with
    /// the token of the load it belongs to. The single source of truth for whether
    /// a dispatched load is still outstanding and whether it actually succeeded —
    /// a screen that navigated away cannot learn either from its callbacks. Also
    /// gives a load exclusive use of its identity for its whole
    /// check → fetch → insert → seal span. See [`identity_load_registry`].
    identity_loads: identity_load_registry::SharedLoadRegistry,
    pub(crate) wallets: RwLock<BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>>,
    /// Per-wallet guards covering the complete wallet-meta alias update.
    /// Different wallets remain independent while same-wallet renames serialize.
    hd_wallet_rename_locks: Mutex<HashMap<WalletSeedHash, Arc<Mutex<()>>>>,
    /// Per-identity guards covering every whole-record mutation of one stored
    /// identity. See [`AppContext::identity_record_lock`].
    identity_record_locks: Mutex<HashMap<Identifier, Arc<Mutex<()>>>>,
    pub(crate) single_key_wallets: RwLock<BTreeMap<SingleKeyHash, Arc<RwLock<SingleKeyWallet>>>>,
    /// Hard override that keeps this context's UI still whatever the role — set by
    /// automated tests through [`AppState::with_animations`](crate::app::AppState::with_animations).
    ///
    /// Role-driven suppression is *not* stored here: it is derived on every read by
    /// [`animations_enabled`](Self::animations_enabled). The role cell is shared by
    /// every per-network context but a role change only passes through the active
    /// one, so a mirrored copy would go stale on the others.
    animations_disabled: AtomicBool,
    /// Cached settings to avoid repeated k/v reads + bincode decoding.
    /// Use RwLock to allow multiple readers but exclusive writers for cache invalidation.
    cached_settings: RwLock<Option<AppSettings>>,
    /// Frame-safe pending DPNS names rebuilt after the contest cache changes.
    pending_dpns_usernames:
        RwLock<HashMap<Identifier, crate::model::contested_name::PendingUsername>>,
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
    /// before the wallet backend is wired. Cross-network: a single
    /// imported WIF / HD seed is keyed by its hash, not by the chain prefix.
    secret_store: Arc<SecretStore>,
    // subtasks started by the app context, used for graceful shutdown
    pub(crate) subtasks: Arc<TaskManager>,
    pub(crate) token_balance_refresh_in_flight: AtomicBool,
    /// Tracks the connection status to currently active network
    pub(crate) connection_status: Arc<ConnectionStatus>,
    /// Tracks the legacy-data migration progress. Cheap to read each
    /// frame from the UI. Always present and idle on fresh installs;
    /// driven by [`MigrationTask::FinishUnwire`](crate::backend_task::migration::MigrationTask).
    pub(crate) migration_status: Arc<MigrationStatus>,
    /// Serializes the whole storage-preparation sequence — backend wiring,
    /// hydration, the legacy drain, and the detached DAPI refresh that keeps
    /// running after the drain publishes terminal status. Held by
    /// [`AppContext::prepare_storage`]; also prevents duplicate password waiters
    /// for the same wallet.
    pub(crate) prepare_gate: tokio::sync::Mutex<()>,
    /// Process-local claim shared by every UI surface before a paid DashPay
    /// request action enters its backend flow.
    contact_request_actions_in_flight: Mutex<HashSet<Identifier>>,
    /// Pending wallet selection - set after creating/importing a wallet
    /// so the wallet screen can auto-select the new wallet
    pub(crate) pending_wallet_selection: Mutex<Option<WalletSeedHash>>,
    /// Currently selected HD wallet (persisted across screen navigation)
    pub(crate) selected_wallet_hash: Mutex<Option<WalletSeedHash>>,
    /// Currently selected single key wallet (persisted across screen navigation)
    pub(crate) selected_single_key_hash: Mutex<Option<SingleKeyHash>>,
    /// App-scoped selected identity — the "who am I operating as" choice,
    /// persisted per-network. Primary; the operating wallet is reconciled to it.
    pub(crate) selected_identity_id: Mutex<Option<Identifier>>,
    /// Pending identity selection — set after creating/loading an identity so
    /// the hub adopts it on return (forward-courier, mirrors
    /// `pending_wallet_selection`).
    pub(crate) pending_identity_selection: Mutex<Option<Identifier>>,
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
    /// Frame-safe shielded receive-address snapshot (Bech32m, Orchard account 0).
    ///
    /// The read side of the receive-address bridge: written on the async backend
    /// side by [`Self::cache_shielded_receive_address`] right after the wallet's
    /// Orchard keys are bound, read synchronously in the frame loop via
    /// [`Self::shielded_receive_address`]. Starts empty — the Shielded tab shows
    /// its "not ready yet" copy until a bind completes.
    ///
    /// **Funds safety:** the address is produced by the upstream-owned key slot
    /// (`PlatformWallet::shielded_default_address`), i.e. the same `OrchardKeySet`
    /// that `bind_shielded` registered with the `NetworkShieldedCoordinator` as
    /// the viewing keys it scans with. It is never re-derived DET-side, so a
    /// displayed address is always one the wallet can actually detect notes for.
    /// Evicted on wallet removal so a re-imported seed can never surface a
    /// removed wallet's address. Must never be written from the frame loop
    /// (Nagatha ruling: no `block_in_place`/`block_on` on the UI thread).
    pub(crate) shielded_addresses: Arc<Mutex<std::collections::HashMap<WalletSeedHash, String>>>,
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
    /// **Wallet-removal behaviour (accepted):** removing a wallet does NOT
    /// evict its entry from this map (consistent with `shielded_balances` — the
    /// same known gap). The stale value is never displayed because
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
    /// Whether the preserved legacy `data.db` holds any identity of this
    /// network — the gate on the stranded-key recovery offer, probed at most
    /// once per context by [`Self::has_legacy_identities`]. The legacy file is
    /// a read-only artifact, so one answer holds for the session.
    legacy_identities_present: std::sync::OnceLock<bool>,
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
    pub(crate) fn hd_wallet_rename_lock(&self, seed_hash: WalletSeedHash) -> Arc<Mutex<()>> {
        self.hd_wallet_rename_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(seed_hash)
            .or_default()
            .clone()
    }

    /// The guard serializing every whole-record mutation of `identity_id`:
    /// storing the record, deleting it, and changing its key-protection tier.
    ///
    /// The stored record is written as one whole snapshot, so any two writers
    /// of the same identity are a read-modify-write race — the loser's snapshot
    /// silently reverts the winner's. A multi-step mutation (read → merge →
    /// seal → write) must therefore hold this guard for its whole span; single
    /// writers take it for the write alone, inside
    /// [`insert_local_qualified_identity`](Self::insert_local_qualified_identity),
    /// [`update_local_qualified_identity`](Self::update_local_qualified_identity),
    /// [`set_identity_alias`](Self::set_identity_alias) and
    /// [`delete_local_qualified_identity`](Self::delete_local_qualified_identity),
    /// so coverage does not depend on remembering it at ~20 call sites.
    ///
    /// Lock order is `prepare_gate` → this guard, never the reverse: the
    /// delete and legacy-recovery paths both take the storage-migration mutex
    /// first. Nothing may be held across an `.await`.
    ///
    /// A few UI paths write a record straight from the frame loop (a key add or
    /// remove, an alias rename, removing a node), so they can wait here. That
    /// wait is bounded and cannot deadlock: no holder needs the UI thread to
    /// make progress — the one flow that opens a modal, legacy recovery, takes
    /// this guard only *after* its password prompt has closed — and the longest
    /// holder is a key-protection tier change, whose per-key derivation runs in
    /// the low hundreds of milliseconds. Different identities never contend.
    pub(crate) fn identity_record_lock(&self, identity_id: Identifier) -> Arc<Mutex<()>> {
        // Counted before the caller's blocking acquire, so a test holding the
        // lock learns a background worker reached it rather than merely being
        // slow to start — which elapsed time cannot distinguish. Reported only
        // to the probe the calling thread attached itself to, so a test never
        // sees the suite's other lock traffic.
        #[cfg(test)]
        lock_probe::note_request(lock_probe::LockSite::RecordRequest);
        self.identity_record_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(identity_id)
            .or_default()
            .clone()
    }

    pub(crate) fn try_claim_contact_request_action(
        &self,
        request_id: Identifier,
    ) -> Option<ContactRequestActionClaim<'_>> {
        let mut in_flight = self
            .contact_request_actions_in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !in_flight.insert(request_id) {
            return None;
        }
        Some(ContactRequestActionClaim {
            registry: &self.contact_request_actions_in_flight,
            request_id,
        })
    }

    pub(crate) fn contact_request_action_is_in_flight(&self, request_id: &Identifier) -> bool {
        self.contact_request_actions_in_flight
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(request_id)
    }

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
        user_role: UserRoleCell,
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
        let spv_provider = SpvProvider::new(network);

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

        let load_contract = |contract: SystemDataContract, label: &str| {
            load_system_data_contract(contract, platform_version)
                .inspect_err(|e| tracing::error!(?network, "Failed to load {label} contract: {e}"))
                .ok()
        };

        let dpns_contract = load_contract(SystemDataContract::DPNS, "DPNS")?;
        let withdrawal_contract = load_contract(SystemDataContract::Withdrawals, "Withdrawals")?;
        let token_history_contract =
            load_contract(SystemDataContract::TokenHistory, "TokenHistory")?;
        let keyword_search_contract =
            load_contract(SystemDataContract::KeywordSearch, "KeywordSearch")?;
        let dashpay_contract = load_contract(SystemDataContract::Dashpay, "Dashpay")?;

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
            user_role,
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
            identity_loads: Default::default(),
            wallets: RwLock::new(wallets),
            hd_wallet_rename_locks: Mutex::new(HashMap::new()),
            identity_record_locks: Mutex::new(HashMap::new()),
            single_key_wallets: RwLock::new(single_key_wallets),
            animations_disabled: AtomicBool::new(false),
            cached_settings: RwLock::new(None),
            pending_dpns_usernames: RwLock::new(HashMap::new()),
            app_kv,
            secret_store,
            subtasks,
            token_balance_refresh_in_flight: AtomicBool::new(false),
            connection_status,
            migration_status: Arc::new(MigrationStatus::new_idle()),
            prepare_gate: tokio::sync::Mutex::new(()),
            contact_request_actions_in_flight: Mutex::new(HashSet::new()),
            pending_wallet_selection: Mutex::new(None),
            selected_wallet_hash: Mutex::new(selected_wallet_hash),
            selected_single_key_hash: Mutex::new(selected_single_key_hash),
            selected_identity_id: Mutex::new(None),
            pending_identity_selection: Mutex::new(None),
            fee_multiplier_permille: AtomicU64::new(
                PlatformFeeEstimator::DEFAULT_FEE_MULTIPLIER_PERMILLE,
            ),
            platform_protocol_version: AtomicU32::new(0),
            shielded_balances: Arc::new(Mutex::new(std::collections::HashMap::new())),
            shielded_addresses: Arc::new(Mutex::new(std::collections::HashMap::new())),
            platform_balances: Arc::new(Mutex::new(std::collections::HashMap::new())),
            platform_sync_cursors: Arc::new(Mutex::new(std::collections::HashMap::new())),
            egui_ctx,
            wallet_backend: ArcSwapOption::const_empty(),
            wallet_backend_build: tokio::sync::Mutex::new(()),
            secret_prompt: SecretPromptSlot(Mutex::new(
                Arc::new(NullSecretPrompt) as Arc<dyn SecretPrompt>
            )),
            legacy_identities_present: std::sync::OnceLock::new(),
        };

        let app_context = Arc::new(app_context);
        // Bind the SDK context provider. Chain sync is SPV-only (owned by
        // upstream platform-wallet); the SPV provider is the sole SDK
        // quorum/context provider.
        match app_context.spv_context_provider.read() {
            Ok(provider) => provider.bind_app_context(app_context.clone()),
            Err(e) => {
                tracing::error!("Failed to bind SPV provider: {}", e);
                return None;
            }
        }

        Some(app_context)
    }

    /// Force UI animations off for this context, whatever the role — the switch
    /// behind [`AppState::with_animations`](crate::app::AppState::with_animations),
    /// which automated runs use to keep the frame loop still.
    pub fn set_animations_disabled(&self, disabled: bool) {
        self.animations_disabled.store(disabled, Ordering::Relaxed);
    }

    /// Whether UI elements should animate: not while an override is in force, and
    /// not for Power or Developer, whose animation clutters an operator workflow.
    ///
    /// Derived from the shared role on every read rather than mirrored into a field
    /// when the role changes — see the [`animations_disabled`](Self::animations_disabled)
    /// field docs.
    fn animations_enabled(&self) -> bool {
        !self.animations_disabled.load(Ordering::Relaxed)
            && !self.user_role().at_least(UserRole::Power)
    }

    /// The app-global user role (shared across every per-network context).
    pub fn user_role(&self) -> UserRole {
        self.user_role.get()
    }

    /// Set the app-global user role, publishing it to every context that shares
    /// the cell.
    pub fn set_user_role(&self, role: UserRole) {
        self.user_role.set(role);
    }

    /// Whether an experimental feature is currently exposed.
    ///
    /// Compat shim: during the migration window this tracks the old dev-mode
    /// gating (`>= Power`). Once a feature stabilises, its callsite drops the
    /// [`Check::Experimental`](crate::context::feature_gate::Check) check
    /// entirely and it unlocks for every role.
    pub fn experimental_enabled(&self, _feature: ExperimentalFeature) -> bool {
        self.user_role().at_least(UserRole::Power)
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    /// Shared app-level k/v store. Cheap clone — `Arc<DetKv>` is `Arc`-backed.
    pub fn app_kv(&self) -> Arc<DetKv> {
        Arc::clone(&self.app_kv)
    }

    /// The per-network DET key/value store, or a typed error when the wallet
    /// backend is not yet initialized. Single accessor shared by every
    /// `context/*_db.rs` module.
    pub(crate) fn det_kv(&self) -> Result<DetKv, TaskError> {
        Ok(self.wallet_backend()?.kv())
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
        crate::app_dir::ensure_data_dir_exists(data_dir)?;
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
        crate::app_dir::ensure_data_dir_exists(data_dir)
            .map_err(|source| TaskError::FileSystem { source })?;
        crate::wallet_backend::single_key::open_secret_store(&Self::secret_store_path(data_dir))
            .map(Arc::new)
            .map_err(|source| TaskError::SecretStore {
                source: Box::new(source),
            })
    }

    /// Open the shared seed vault with an explicit `passphrase` — the
    /// funds-safe, non-destructive recovery door for a **legacy vault** an
    /// older build sealed with a passphrase. The keyless [`Self::open_secret_store`]
    /// fails such a vault with
    /// [`SecretStoreError::WrongPassphrase`](platform_wallet_storage::secrets::SecretStoreError::WrongPassphrase);
    /// the GUI boot seam re-opens it in place with this. Same vault file, same
    /// `Arc<SecretStore>` contract — it never deletes, recreates, or rekeys
    /// the vault, so wallet seeds are never at risk.
    pub fn open_secret_store_with_passphrase(
        data_dir: &std::path::Path,
        passphrase: platform_wallet_storage::secrets::SecretString,
    ) -> Result<Arc<SecretStore>, TaskError> {
        crate::app_dir::ensure_data_dir_exists(data_dir)
            .map_err(|source| TaskError::FileSystem { source })?;
        crate::wallet_backend::single_key::open_secret_store_with_passphrase(
            &Self::secret_store_path(data_dir),
            passphrase,
        )
        .map(Arc::new)
        .map_err(|source| TaskError::SecretStore {
            source: Box::new(source),
        })
    }

    /// Absolute path of the shared seed vault: `<data_dir>/secrets/det-secrets.pwsvault`.
    fn secret_store_path(data_dir: &std::path::Path) -> std::path::PathBuf {
        let mut path = data_dir.to_path_buf();
        path.push("secrets");
        path.push("det-secrets.pwsvault");
        path
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

    /// Cache the platform protocol version fetched from the connected network.
    ///
    /// Read by the shielded-operations capability check to decide whether the
    /// network can settle shielded state transitions. `0` means "not fetched
    /// yet", which reads as below any activation version.
    pub fn set_platform_protocol_version(&self, version: u32) {
        self.platform_protocol_version
            .store(version, Ordering::Relaxed);
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

    /// Synchronous, frame-safe reader for the wallet's shielded receive address
    /// (Bech32m, Orchard account 0). Returns `None` until the wallet's shielded
    /// keys are bound — the Shielded tab renders its "not ready yet" copy then.
    ///
    /// The read side of the push snapshot; the write side is
    /// [`Self::cache_shielded_receive_address`], driven from the JIT bootstrap
    /// once `ensure_shielded_bound` succeeds. Safe to call from the egui frame
    /// loop — no blocking I/O, no async.
    pub fn shielded_receive_address(&self, seed_hash: &WalletSeedHash) -> Option<String> {
        self.shielded_addresses
            .lock()
            .ok()
            .and_then(|map| map.get(seed_hash).cloned())
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
    /// `(last_sync_timestamp, sync_height)` for `seed_hash`, or `None` until at
    /// least one platform-address sync pass has completed for the wallet. The
    /// cursor advances on every successful pass regardless of whether it found
    /// any funded addresses, so `Some` means "a sync pass has completed," not
    /// "funds were found."
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

    /// Shared per-address seeding loop for the platform-address maps.
    ///
    /// For each `(seed_hash, entries)` batch, converts every raw 20-byte P2PKH
    /// hash to a canonical address, hands it to `write_info` (the caller picks
    /// overwrite vs seed-if-absent semantics), and registers the address for
    /// signing — the exact DIP-17 index when known, the bounded
    /// reverse-derivation scan otherwise. Without the signer registration a
    /// balance is visible yet unwithdrawable.
    ///
    /// Each wallet write lock is held briefly for a pure `BTreeMap` update — no
    /// I/O, no network, no await — matching the codebase's frame-loop write
    /// pattern.
    fn seed_platform_address_entries<'a, I, W>(&self, batches: I, write_info: W)
    where
        I: IntoIterator<Item = (&'a WalletSeedHash, &'a [PlatformAddressEntry])>,
        W: Fn(&mut Wallet, dash_sdk::dpp::dashcore::Address, u64, u32),
    {
        use dash_sdk::dpp::key_wallet::PlatformP2PKHAddress;
        let network = self.network;
        let Ok(wallets) = self.wallets.read() else {
            return;
        };
        for (seed_hash, entries) in batches {
            if let Some(wallet_arc) = wallets.get(seed_hash)
                && let Ok(mut wallet) = wallet_arc.write()
            {
                for entry in entries {
                    let addr = PlatformP2PKHAddress::new(entry.hash).to_address(network);
                    let canonical = Wallet::canonical_address(&addr, network);
                    write_info(&mut wallet, canonical.clone(), entry.balance, entry.nonce);
                    match entry.index {
                        Some(index) => wallet.register_platform_payment_address(
                            &canonical,
                            entry.account,
                            index,
                            network,
                        ),
                        None => {
                            wallet.reconcile_platform_address(&canonical, network);
                        }
                    }
                }
            }
        }
    }

    /// Populate each wallet's `platform_address_info` from a coordinator-push batch.
    ///
    /// Called by `AppState` when a [`BackendTaskSuccessResult::PlatformAddressSyncPushed`]
    /// result arrives. Overwrites each address's cached info via
    /// [`Wallet::set_platform_address_info`], keeping the per-address tab
    /// consistent with the coordinator-push total balance without requiring a
    /// manual Refresh on cold start.
    pub fn apply_platform_address_push(&self, updates: PlatformAddressUpdates) {
        self.seed_platform_address_entries(
            updates
                .iter()
                .map(|(seed_hash, entries)| (seed_hash, entries.as_slice())),
            |wallet, address, balance, nonce| {
                wallet.set_platform_address_info(address, balance, nonce)
            },
        );
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

        // Seed the per-address tab with an insert-if-absent write so a live
        // push that somehow landed first is never clobbered by staler persisted
        // data (the balance and cursor maps below use the same `or_insert` guard).
        self.seed_platform_address_entries(
            seeds
                .iter()
                .filter(|(_, e, _)| !e.is_empty())
                .map(|(seed_hash, entries, _)| (seed_hash, entries.as_slice())),
            |wallet, address, balance, nonce| {
                wallet.seed_platform_address_info(address, balance, nonce)
            },
        );

        if let Ok(mut balances) = self.platform_balances.lock() {
            for (seed_hash, entries, _) in seeds.iter().filter(|(_, e, _)| !e.is_empty()) {
                let total_credits: u64 = entries.iter().map(|e| e.balance).sum();
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

    /// A handle on the shared app-global role, for wiring a newly-created
    /// per-network context to the same value (see the field docs).
    pub fn user_role_cell(&self) -> UserRoleCell {
        self.user_role.clone()
    }

    /// Repaints the UI if animations are enabled.
    ///
    /// Called by UI elements that need to trigger a repaint, such as loading spinners or animated icons.
    pub(super) fn repaint_animation(&self, ctx: &Context) {
        if self.animations_enabled() {
            // Request a repaint after a short delay to allow for animations
            ctx.request_repaint_after(ANIMATION_REFRESH_TIME);
        }
    }

    pub fn platform_version(&self) -> &'static PlatformVersion {
        default_platform_version(&self.network)
    }

    pub fn state_transition_options(&self) -> Option<StateTransitionCreationOptions> {
        // Signing override: only the Developer role may relax the security-level
        // and purpose checks when signing a state transition.
        if self.user_role().at_least(UserRole::Developer) {
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
            .bind_app_context(self.clone());

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

    /// Recovers from a Drive proof error on a contract put/update.
    ///
    /// Logs the proof failure via [`Self::log_drive_proof_error`], notifies the
    /// UI, then re-fetches the contract by id — the transition may have
    /// committed despite the proof error — and persists it through `persist`.
    /// Returns [`BackendTaskSuccessResult::ContractSavedAfterProofError`] when
    /// the contract is found, otherwise the original proof error.
    pub(crate) async fn recover_contract_after_proof_error(
        &self,
        sdk: &Sdk,
        contract_id: Identifier,
        proof_error: dash_sdk::Error,
        sender: &crate::utils::egui_mpsc::SenderAsync<crate::app::TaskResult>,
        persist: impl FnOnce(&Self, &DataContract),
    ) -> Result<crate::backend_task::BackendTaskSuccessResult, TaskError> {
        use crate::app::TaskResult;
        use crate::backend_task::BackendTaskSuccessResult;
        use dash_sdk::platform::Fetch;

        // Give the network time to propagate the block before re-fetching.
        const REFETCH_DELAY_REGTEST: std::time::Duration = std::time::Duration::from_secs(3);
        const REFETCH_DELAY_DEFAULT: std::time::Duration = std::time::Duration::from_secs(10);

        let task_error =
            self.log_drive_proof_error(proof_error, RequestType::BroadcastStateTransition);

        sender
            .send(TaskResult::unattributed_success(
                BackendTaskSuccessResult::ProofErrorLogged,
            ))
            .await
            .map_err(|_| TaskError::InternalSendError)?;

        let delay = match self.network {
            Network::Regtest => REFETCH_DELAY_REGTEST,
            _ => REFETCH_DELAY_DEFAULT,
        };
        tokio::time::sleep(delay).await;

        if let Ok(Some(contract)) = DataContract::fetch(sdk, contract_id).await {
            persist(self, &contract);
            return Ok(BackendTaskSuccessResult::ContractSavedAfterProofError);
        }

        Err(task_error)
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
        let backend = WalletBackend::new(
            self,
            sdk,
            Arc::clone(&self.connection_status),
            task_result_sender,
            self.secret_prompt(),
        )
        .await?;
        self.wallet_backend.store(Some(Arc::new(backend)));
        drop(_build_guard);
        if let Err(error) = self.refresh_pending_dpns_usernames() {
            tracing::warn!(
                ?error,
                "Pending DPNS username cache could not be warmed from stored contests"
            );
        }
        self.restore_selected_wallet_from_kv();
        self.restore_selected_identity_from_kv();
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
        let mut guard = self
            .secret_prompt
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *guard = prompt;
        self.secret_prompt.0.clear_poison();
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

    /// Whether this context has a host that can render a human password prompt.
    /// The GUI installs that capability during boot; standalone MCP/CLI contexts
    /// retain the non-interactive default.
    pub fn has_interactive_secret_prompt(&self) -> bool {
        self.secret_prompt().is_interactive()
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

    /// The currently selected HD wallet for this network, if any.
    pub fn selected_wallet_hash(&self) -> Option<WalletSeedHash> {
        self.selected_wallet_hash.lock().ok().and_then(|g| *g)
    }

    /// The app-scoped selected identity for this network, if any.
    pub fn selected_identity_id(&self) -> Option<Identifier> {
        self.selected_identity_id.lock().ok().and_then(|g| *g)
    }

    /// Resolve the active identity every operate-as read uses: the selected
    /// identity when still loaded, else the first loaded identity, else `None`.
    ///
    /// FR-6 boundary (resolution layer): the app-global operate-as identity is
    /// **always** a User identity. The candidate set is filtered to
    /// [`IdentityType::User`] before resolving, so neither the keep-if-loaded
    /// check nor the first-loaded fallback can ever resolve a masternode/evonode
    /// — even when a masternode is the only or first loaded identity, or was
    /// persisted as the selection in a prior session. Masternode/evonode
    /// identities are page-scoped (the Masternodes page), never the app-global
    /// identity.
    pub fn resolve_selected_identity(&self) -> Option<QualifiedIdentity> {
        let identities = self.load_local_qualified_identities().ok()?;
        let user_ids: Vec<Identifier> = identities
            .iter()
            .filter(|qi| qi.identity_type == IdentityType::User)
            .map(|qi| qi.identity.id())
            .collect();
        let chosen = crate::model::selected_identity::resolve_selected(
            self.selected_identity_id(),
            &user_ids,
        )?;
        identities.into_iter().find(|qi| qi.identity.id() == chosen)
    }

    /// In-memory single-key selection (read directly for the persist blob).
    fn current_single_key_hash(&self) -> Option<SingleKeyHash> {
        self.selected_single_key_hash.lock().ok().and_then(|g| *g)
    }

    /// Owning HD wallet of an identity, derived from its signing-key
    /// derivation path (the reliable owner — never
    /// `associated_wallets.keys().next()`). Best-effort: `None` when the
    /// identity is unknown or has no wallet-derived signing key.
    fn owning_wallet_hash(&self, id: Identifier) -> Option<WalletSeedHash> {
        let identities = self.load_local_qualified_identities().ok()?;
        let qi = identities.into_iter().find(|qi| qi.identity.id() == id)?;
        let wallet = crate::ui::identities::get_selected_wallet(&qi, Some(self), None).ok()??;
        let hash = wallet.read().ok()?.seed_hash();
        Some(hash)
    }

    /// Set the app-scoped selected identity and reconcile the operating wallet
    /// to its owner. Writes both mutexes directly and persists both blobs once;
    /// never calls a sibling setter (no reconciliation recursion — R5). The
    /// wallet write is cosmetic — signing always re-derives from the identity.
    pub fn set_selected_identity(&self, id: Option<Identifier>) {
        if let Ok(mut g) = self.selected_identity_id.lock() {
            *g = id;
        }
        // Reconcile the derived wallet to the identity's owner. A wallet-less
        // (imported-by-id) identity has NO owning wallet, so the pointer must
        // be cleared to `None` — otherwise the breadcrumb wallet pill keeps
        // showing the previous identity's wallet while Home/operate-as use the
        // wallet-less one (the pill and the active identity disagree). Selecting
        // an identity always writes the owner (`None` included); only clearing
        // the identity (`id == None`) leaves the wallet pointer untouched.
        if let Some(id) = id {
            let owner = self.owning_wallet_hash(id);
            if let Ok(mut g) = self.selected_wallet_hash.lock() {
                *g = owner;
            }
            // Setting an HD owner makes that HD wallet active; clear any
            // single-key selection so the store never holds both (SK-first
            // resolution would otherwise show the single-key wallet).
            if owner.is_some()
                && let Ok(mut g) = self.selected_single_key_hash.lock()
            {
                *g = None;
            }
        }
        self.persist_selected_identity_kv(id);
        self.persist_selected_wallet_kv(
            self.selected_wallet_hash(),
            self.current_single_key_hash(),
        );
    }

    /// Set the selected HD wallet and reconcile the active identity to that
    /// wallet's identities (keep-if-owned, else its first identity, else
    /// `None` → picker). Writes both mutexes directly and persists both blobs
    /// once; never calls a sibling setter (R5).
    pub fn set_selected_hd_wallet(&self, hash: Option<WalletSeedHash>) {
        if let Ok(mut g) = self.selected_wallet_hash.lock() {
            *g = hash;
        }
        // An explicit HD pick makes that HD wallet the sole active wallet: clear
        // any single-key selection so the store never holds both (resolution is
        // single-key-first). A `None` (clear-HD) call leaves single-key intact.
        let single_key = if hash.is_some() {
            if let Ok(mut g) = self.selected_single_key_hash.lock() {
                *g = None;
            }
            None
        } else {
            self.current_single_key_hash()
        };
        let reconciled = match hash {
            Some(h) => {
                // FR-6 boundary: reconcile only over the wallet's User identities
                // so a masternode/evonode is never resolved as the app-global
                // identity via this cross-axis wallet-switch side effect.
                let ids: Vec<Identifier> = self
                    .load_local_qualified_identities_for_wallet(&h)
                    .map(|v| {
                        v.iter()
                            .filter(|qi| qi.identity_type == IdentityType::User)
                            .map(|qi| qi.identity.id())
                            .collect()
                    })
                    .unwrap_or_default();
                crate::model::selected_identity::resolve_selected(self.selected_identity_id(), &ids)
            }
            None => None,
        };
        if let Ok(mut g) = self.selected_identity_id.lock() {
            *g = reconciled;
        }
        self.persist_selected_wallet_kv(hash, single_key);
        self.persist_selected_identity_kv(reconciled);
    }

    /// Set the selected single-key wallet and persist. Single-key wallets own
    /// no identities, so there is no identity to reconcile; this also closes
    /// the pre-existing single-key persist gap (R10).
    pub fn set_selected_single_key_wallet(&self, hash: Option<SingleKeyHash>) {
        if let Ok(mut g) = self.selected_single_key_hash.lock() {
            *g = hash;
        }
        // Mirror of `set_selected_hd_wallet`: an explicit single-key pick clears
        // the HD hash so the store never holds both; clearing (`None`) leaves HD
        // intact. Keeps the invariant self-enforcing regardless of caller.
        let hd = if hash.is_some() {
            if let Ok(mut g) = self.selected_wallet_hash.lock() {
                *g = None;
            }
            None
        } else {
            self.selected_wallet_hash()
        };
        self.persist_selected_wallet_kv(hd, hash);
    }

    /// Stage an identity to become active when the hub is next shown — set
    /// after creating/loading an identity. Forward-courier; mirrors
    /// `pending_wallet_selection`.
    pub fn set_pending_identity_selection(&self, id: Identifier) {
        if let Ok(mut g) = self.pending_identity_selection.lock() {
            *g = Some(id);
        }
    }

    /// Take any staged pending identity selection, clearing it.
    pub fn take_pending_identity_selection(&self) -> Option<Identifier> {
        self.pending_identity_selection
            .lock()
            .ok()
            .and_then(|mut g| g.take())
    }

    /// Persist the per-network selected-identity pointer. Best-effort, mirrors
    /// [`Self::persist_selected_wallet_kv`]; the in-memory mutex stays
    /// authoritative for the running process.
    pub fn persist_selected_identity_kv(&self, id: Option<Identifier>) {
        let Ok(backend) = self.wallet_backend() else {
            tracing::debug!("Skipping selected-identity persist; wallet backend not yet wired");
            return;
        };
        let blob = crate::model::selected_identity::SelectedIdentity { identity_id: id };
        if let Err(e) = backend.set_selected_identity(&blob) {
            tracing::warn!(
                network = ?self.network,
                error = ?e,
                "Failed to persist selected identity to wallet k/v"
            );
        }
    }

    /// Populate the in-memory selected-identity pointer from the wallet
    /// backend's k/v store, keeping it only if the identity is still loaded for
    /// this network. Called from [`Self::ensure_wallet_backend`] after the
    /// wallet pointers are restored; idempotent.
    fn restore_selected_identity_from_kv(&self) {
        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        let stored = backend.get_selected_identity().identity_id;
        // FR-6 one-time sanitization: masternodes were Hub-pickable in prior
        // sessions, so a persisted `selected_identity_id` may point at a
        // masternode/evonode. The app-global identity must always be a User
        // identity, so keep the persisted selection only if it is a still-loaded
        // User identity — otherwise clear it (the count-based hub default takes
        // over). In-memory only, matching the non-destructive restore contract:
        // a transient empty load leaves the KV blob untouched for the next pass.
        let user_ids: Vec<Identifier> = self
            .load_local_qualified_identities()
            .map(|v| {
                v.iter()
                    .filter(|qi| qi.identity_type == IdentityType::User)
                    .map(|qi| qi.identity.id())
                    .collect()
            })
            .unwrap_or_default();
        let kept = crate::model::selected_identity::keep_if_loaded(stored, &user_ids);
        if let Ok(mut g) = self.selected_identity_id.lock() {
            *g = kept;
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

    /// Monotonic generation of the event-pushed snapshot for one wallet.
    pub fn snapshot_generation(&self, seed_hash: &WalletSeedHash) -> u64 {
        self.wallet_backend()
            .map(|wb| wb.wallet_snapshot_generation(seed_hash))
            .unwrap_or_default()
    }

    /// Snapshot generation, exact builder-input composition, and its revision.
    pub fn asset_lock_probe_snapshot(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> (u64, crate::wallet_backend::AssetLockInputState, u64) {
        self.wallet_backend()
            .map(|wallet_backend| wallet_backend.asset_lock_probe_snapshot(seed_hash))
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

    /// Authoritative per-address derivation paths from the snapshot: every
    /// address the upstream wallet generated, across its Core-chain accounts and
    /// the DIP-17 platform-payment pool. The set an address autocomplete may
    /// offer from — it can never surface an address upstream does not watch.
    pub fn snapshot_address_paths(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> std::collections::BTreeMap<
        dash_sdk::dpp::dashcore::Address,
        dash_sdk::dpp::key_wallet::bip32::DerivationPath,
    > {
        self.wallet_backend()
            .map(|wb| wb.address_paths(seed_hash))
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
// TODO(platform#4231): Seeded at v12 (not pinned) so `Sdk`'s protocol-version
// ratchet stays active. See `PlatformInfoTaskRequestType::CurrentEpochInfo` for
// why `ExtendedEpochInfo::fetch_current` can't be used directly right now.
// Revert to `.with_version()` (a hard pin) or otherwise reconsider this once
// https://github.com/dashpay/platform/pull/4231 merges and this repo's platform
// pin advances past it.
pub(crate) const fn default_platform_version(_network: &Network) -> &'static PlatformVersion {
    &PLATFORM_V12
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_workaround_uses_v12_for_every_network() {
        for network in [
            Network::Mainnet,
            Network::Testnet,
            Network::Devnet,
            Network::Regtest,
        ] {
            assert_eq!(
                default_platform_version(&network).protocol_version,
                PLATFORM_V12.protocol_version
            );
        }
    }

    #[test]
    fn install_secret_prompt_recovers_poisoned_slot() {
        use crate::context::test_support::test_app_context;
        use crate::wallet_backend::secret_prompt::test_support::TestPrompt;

        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = ctx.secret_prompt.0.lock().expect("prompt slot lock");
            panic!("poison prompt slot");
        }));
        assert!(poisoned.is_err(), "the prompt slot must be poisoned");

        let prompt: Arc<dyn SecretPrompt> = Arc::new(TestPrompt::never());
        ctx.install_secret_prompt(Arc::clone(&prompt));
        assert!(
            Arc::ptr_eq(&prompt, &ctx.secret_prompt()),
            "install_secret_prompt must replace the host after lock poisoning"
        );
    }

    #[test]
    fn wallet_name_with_spaces_is_url_encoded() {
        let base = "http://127.0.0.1:9998";
        let name = "my test wallet";
        let encoded = urlencoding::encode(name);
        let url = format!("{}/wallet/{}", base, encoded);
        assert_eq!(url, "http://127.0.0.1:9998/wallet/my%20test%20wallet");
        assert!(!url.contains(' '));
    }

    // ── FR-6 resolution-layer boundary (B1) ──────────────────────────────────

    /// Build an offline, wired `AppContext` (no network I/O) so the identity
    /// store is a real, writable DB the accessors read from. Returns the temp
    /// dir (kept alive by the caller) alongside the context.
    async fn offline_ctx() -> (tempfile::TempDir, std::sync::Arc<AppContext>) {
        use crate::app::TaskResult;
        use crate::app_dir::ensure_env_file;
        use crate::context::connection_status::ConnectionStatus;
        use crate::database::test_helpers::create_database_at_path;
        use crate::utils::egui_mpsc::SenderAsync;
        use crate::utils::tasks::TaskManager;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db =
            std::sync::Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            dash_sdk::dpp::dashcore::Network::Testnet,
            db,
            std::sync::Arc::new(TaskManager::new()),
            std::sync::Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("offline testnet AppContext::new");
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        (temp_dir, ctx)
    }

    /// Regression (mn-live-qa Bug 1): the user role is a single app-global value
    /// shared by every per-network `AppContext`. Setting it on the context for
    /// one network must be observable from the context for another — otherwise
    /// the left-nav feature gate (`FeatureGate::Masternodes`) reads a stale value
    /// on whichever per-network context the app renders from, and the
    /// Power-gated Masternodes tab never appears until an app restart re-reads
    /// the persisted role.
    #[test]
    fn user_role_is_shared_across_network_contexts() {
        use crate::app_dir::ensure_env_file;
        use crate::context::connection_status::ConnectionStatus;
        use crate::database::test_helpers::create_database_at_path;
        use crate::utils::tasks::TaskManager;
        use dash_sdk::dpp::dashcore::Network;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db =
            std::sync::Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let subtasks = std::sync::Arc::new(TaskManager::new());
        let connection_status = std::sync::Arc::new(ConnectionStatus::new());
        let egui_ctx = egui::Context::default();
        // A single app-global role cell, owned by `AppState` and shared into
        // every per-network context (mirrors the real construction path).
        let user_role = UserRoleCell::new(UserRole::Everyday);

        // The startup context (Mainnet) and a second context (Testnet) built the
        // way `AppState` builds one on a live network switch — reusing the shared
        // db / kv / secret store / role cell.
        let mainnet = AppContext::new(
            data_dir.clone(),
            Network::Mainnet,
            db.clone(),
            subtasks.clone(),
            connection_status.clone(),
            egui_ctx.clone(),
            app_kv.clone(),
            secret_store.clone(),
            user_role.clone(),
        )
        .expect("mainnet AppContext::new");
        let testnet = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            subtasks,
            connection_status,
            egui_ctx,
            app_kv,
            secret_store,
            user_role,
        )
        .expect("testnet AppContext::new");

        assert_eq!(mainnet.user_role(), UserRole::Everyday);
        assert_eq!(testnet.user_role(), UserRole::Everyday);

        // Raise the role on ONE context, exactly as the Settings selector does.
        mainnet.set_user_role(UserRole::Power);

        assert_eq!(
            testnet.user_role(),
            UserRole::Power,
            "a role set on one network's context must be visible on another \
             network's context"
        );
    }

    /// Animation gating is derived from the shared role, so it cannot go stale on
    /// a context the role change did not pass through.
    ///
    /// The role cell is app-global, but an `AppContext` exists per network. A
    /// mirrored `animate` flag was only refreshed on the context `set_user_role`
    /// was called on, leaving an idle sibling (testnet, while mainnet is active)
    /// to animate — or not — according to a role the user had already left behind,
    /// until the next role change happened to land on it.
    #[test]
    fn animation_gating_follows_the_role_on_every_network_context() {
        use crate::app_dir::ensure_env_file;
        use crate::context::connection_status::ConnectionStatus;
        use crate::database::test_helpers::create_database_at_path;
        use crate::utils::tasks::TaskManager;
        use dash_sdk::dpp::dashcore::Network;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db =
            std::sync::Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let subtasks = std::sync::Arc::new(TaskManager::new());
        let connection_status = std::sync::Arc::new(ConnectionStatus::new());
        let egui_ctx = egui::Context::default();
        let user_role = UserRoleCell::new(UserRole::Everyday);

        let mainnet = AppContext::new(
            data_dir.clone(),
            Network::Mainnet,
            db.clone(),
            subtasks.clone(),
            connection_status.clone(),
            egui_ctx.clone(),
            app_kv.clone(),
            secret_store.clone(),
            user_role.clone(),
        )
        .expect("mainnet AppContext::new");
        let testnet = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            subtasks,
            connection_status,
            egui_ctx,
            app_kv,
            secret_store,
            user_role,
        )
        .expect("testnet AppContext::new");

        assert!(
            mainnet.animations_enabled() && testnet.animations_enabled(),
            "Everyday keeps the animated UI on every context"
        );

        // The role is raised on the ACTIVE context only — the idle sibling never
        // sees the call, exactly as on a live role switch.
        mainnet.set_user_role(UserRole::Power);

        assert!(
            !mainnet.animations_enabled(),
            "Power stills the UI: animation clutters an operator workflow"
        );
        assert!(
            !testnet.animations_enabled(),
            "an idle network's context must observe the role change too, not keep \
             animating from the role the user left behind"
        );

        // …and back down, so the derivation tracks in both directions.
        testnet.set_user_role(UserRole::Everyday);
        assert!(mainnet.animations_enabled() && testnet.animations_enabled());
    }

    /// The test-only override wins over the role: `AppState::with_animations(false)`
    /// keeps the frame loop still for an automated run whatever role the fixture
    /// happens to hold, and a later role change must not silently re-enable it.
    #[test]
    fn disabling_animations_survives_a_role_change() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(tmp.path());

        ctx.set_animations_disabled(true);
        assert!(!ctx.animations_enabled());

        ctx.set_user_role(UserRole::Everyday);
        assert!(
            !ctx.animations_enabled(),
            "an explicit override must not be undone by a role change"
        );
    }

    /// The signing override in `state_transition_options` is gated strictly at
    /// the Developer role: only `Developer` relaxes the security-level and
    /// purpose signing checks. Everyday and Power must receive `None`.
    #[test]
    fn state_transition_options_signing_override_is_developer_only() {
        use crate::app_dir::ensure_env_file;
        use crate::context::connection_status::ConnectionStatus;
        use crate::database::test_helpers::create_database_at_path;
        use crate::utils::tasks::TaskManager;
        use dash_sdk::dpp::dashcore::Network;

        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db =
            std::sync::Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            std::sync::Arc::new(TaskManager::new()),
            std::sync::Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            UserRoleCell::new(UserRole::Everyday),
        )
        .expect("testnet AppContext::new");

        // Below Developer: no signing override.
        for role in [UserRole::Everyday, UserRole::Power] {
            ctx.set_user_role(role);
            assert!(
                ctx.state_transition_options().is_none(),
                "{role:?} must not receive the signing override"
            );
        }

        // Developer: override present with both relaxations enabled.
        ctx.set_user_role(UserRole::Developer);
        let opts = ctx
            .state_transition_options()
            .expect("Developer role must receive the signing override");
        assert!(opts.signing_options.allow_signing_with_any_security_level);
        assert!(opts.signing_options.allow_signing_with_any_purpose);
    }

    /// Seed one wallet-less identity of `identity_type` into the live identity
    /// DB and return its id.
    fn seed_typed(ctx: &AppContext, byte: u8, identity_type: IdentityType) -> Identifier {
        use crate::model::qualified_identity::IdentityStatus;
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        use dash_sdk::dpp::version::PlatformVersion;

        let pv = PlatformVersion::latest();
        let identity = Identity::create_basic_identity(Identifier::from([byte; 32]), pv)
            .expect("basic identity");
        let id = identity.id();
        let qi = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type,
            alias: Some(format!("seed-{byte:02x}")),
            private_keys: KeyStorage::default(),
            dpns_names: vec![],
            associated_wallets: std::collections::BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: std::collections::BTreeMap::new(),
            status: IdentityStatus::PendingCreation,
            network: ctx.network(),
        };
        ctx.insert_local_qualified_identity(&qi, &None)
            .expect("seed identity insert");
        id
    }

    /// TC-FR6-01…06 + TC-NAV-12b — the User-only / masternode accessors split by
    /// type, and `resolve_selected_identity()` never resolves a masternode: not
    /// via keep-if-loaded, and not via the first-loaded fallback even when a
    /// masternode is the only/first loaded identity.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fr6_accessors_and_resolution_filter_exclude_masternodes() {
        let (_dir, ctx) = offline_ctx().await;

        // TC-NAV-12b: only a masternode loaded, nothing selected → resolve is
        // None, never the masternode via the first-loaded fallback.
        let mn = seed_typed(&ctx, 0x91, IdentityType::Masternode);
        assert_eq!(ctx.selected_identity_id(), None, "nothing selected");
        assert!(
            ctx.resolve_selected_identity().is_none(),
            "a lone masternode must never resolve as the app-global identity"
        );
        assert_eq!(
            ctx.load_local_masternode_identities().unwrap().len(),
            1,
            "the masternode accessor lists the masternode"
        );
        assert!(
            ctx.load_local_user_identities().unwrap().is_empty(),
            "the User accessor excludes the masternode"
        );

        // Add an Evonode and a User.
        let evo = seed_typed(&ctx, 0xE2, IdentityType::Evonode);
        let user = seed_typed(&ctx, 0x71, IdentityType::User);

        let user_ids: Vec<Identifier> = ctx
            .load_local_user_identities()
            .unwrap()
            .iter()
            .map(|qi| qi.identity.id())
            .collect();
        assert_eq!(user_ids, vec![user], "User accessor lists only the User id");

        let mn_ids: std::collections::BTreeSet<Identifier> = ctx
            .load_local_masternode_identities()
            .unwrap()
            .iter()
            .map(|qi| qi.identity.id())
            .collect();
        assert_eq!(
            mn_ids,
            [mn, evo].into_iter().collect(),
            "masternode accessor lists MN + Evonode, never the User"
        );

        // The control: the unfiltered accessor still lists all three (the legacy
        // Identities table reads this — masternodes stay visible there).
        assert_eq!(
            ctx.load_local_qualified_identities().unwrap().len(),
            3,
            "the unfiltered accessor keeps MN/Evonode (legacy table control)"
        );

        // With a User present, resolve falls back to the User, never the MN/Evo.
        assert_eq!(
            ctx.resolve_selected_identity().map(|qi| qi.identity.id()),
            Some(user),
            "resolve falls back to the first User, never a masternode",
        );
    }

    /// TC-NAV-12c — a masternode persisted as `selected_identity_id` in a prior
    /// session is sanitized to `None` on context load; a User selection is kept.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fr6_stale_masternode_selection_sanitized_on_restore() {
        let (_dir, ctx) = offline_ctx().await;
        let mn = seed_typed(&ctx, 0xA1, IdentityType::Masternode);
        let user = seed_typed(&ctx, 0x72, IdentityType::User);

        // Persist a masternode as the selection (the pre-FR-6 Hub allowed this).
        ctx.set_selected_identity(Some(mn));
        assert_eq!(ctx.selected_identity_id(), Some(mn), "MN persisted");

        // Even before restore, the resolution filter refuses to resolve the MN.
        assert_eq!(
            ctx.resolve_selected_identity().map(|qi| qi.identity.id()),
            Some(user),
            "resolution filter alone never resolves the persisted masternode",
        );

        // Context-load sanitization clears the stale MN selection in memory.
        ctx.restore_selected_identity_from_kv();
        assert_eq!(
            ctx.selected_identity_id(),
            None,
            "a stale masternode selection is cleared on load",
        );

        // A User selection survives the same sanitization.
        ctx.set_selected_identity(Some(user));
        ctx.restore_selected_identity_from_kv();
        assert_eq!(
            ctx.selected_identity_id(),
            Some(user),
            "a User selection is kept by the sanitizer",
        );
    }
}
