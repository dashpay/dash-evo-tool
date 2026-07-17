//! The wallet orchestration seam.
//!
//! `WalletBackend` wraps the upstream `PlatformWalletManager` and is the
//! orchestration layer for everything wallet-related — seed-snapshot
//! ownership, the identity-funding-account chokepoint, and signer
//! construction. It is NOT a type-translation layer: project invariant
//! **M-PLATFORM-WALLET-FIRST-PARTY** allows `platform_wallet`,
//! `key_wallet`, and `platform_wallet_storage` types to appear freely on
//! its public surface. Callers route through `WalletBackend` for the
//! orchestration value, not because the upstream types are hidden.
//!
//! Boundaries that still hold (responsibility, not type leak):
//! 1. No upstream types in DET's SQLite schema (`database/`).
//! 2. No upstream types in MCP tool schemas (`src/mcp/tools/**`).
//! 3. No raw upstream `Display` in user-facing strings — upstream errors
//!    go to `BannerHandle::with_details(e)` only.
//!
//! `Clone` is `O(1)` via `Arc<Inner>` (M-SERVICES-CLONE); the type is
//! `Send + Sync`. See
//! `docs/ai-design/2026-05-18-platform-wallet-migration/backend-architecture.md`.

#[cfg(any(test, feature = "bench"))]
pub mod auth_pubkey_cache;
#[cfg(not(any(test, feature = "bench")))]
pub(crate) mod auth_pubkey_cache;
mod avatar_cache;
mod contact_profile_cache;
mod coordinator_gate;
mod dashpay;
mod det_platform_signer;
mod det_signer;
mod event_bridge;
#[cfg(any(test, feature = "bench"))]
pub mod hydration;
#[cfg(not(any(test, feature = "bench")))]
pub(crate) mod hydration;
pub mod identity_key_store;
#[cfg(any(test, feature = "bench"))]
pub mod identity_meta;
#[cfg(not(any(test, feature = "bench")))]
pub(crate) mod identity_meta;
mod identity_ops;
mod kv;
#[cfg(test)]
pub(crate) mod kv_test_support;
#[cfg(test)]
pub(crate) mod leak_test_support;
mod loader;
mod payments;
pub(crate) mod poison;
pub mod secret_access;
pub mod secret_prompt;
pub mod secret_seam;
mod shielded;
mod sidecar;
#[cfg(any(test, feature = "bench"))]
pub mod single_key;
#[cfg(not(any(test, feature = "bench")))]
pub(crate) mod single_key;
pub mod single_key_entry;
mod snapshot;
mod token_balance;
mod versioned_bincode;
#[cfg(any(test, feature = "bench"))]
pub mod wallet_meta;
#[cfg(not(any(test, feature = "bench")))]
pub(crate) mod wallet_meta;
#[cfg(any(test, feature = "bench"))]
pub mod wallet_seed_store;
#[cfg(not(any(test, feature = "bench")))]
pub(crate) mod wallet_seed_store;

pub use dashpay::DashpayView;
pub(crate) use dashpay::{
    ContactRequestActionKind, ContactRequestActionPhase, derive_contact_info_encryption_keys,
    derive_contact_xpub_material,
};

pub(crate) use det_platform_signer::{DetPlatformSigner, PlatformPathIndex};
pub(crate) use det_signer::{DetSigner, DetSignerError};
pub use identity_key_store::IdentityKeyView;
pub use identity_meta::IdentityMetaView;
pub use secret_access::{
    PromptMeta, SecretAccess, SecretLease, SecretPlaintext, SecretSession, VerifiedIdentityPassword,
};
pub use secret_prompt::{
    NullSecretPrompt, RememberPolicy, SecretPrompt, SecretPromptCancelled, SecretPromptReply,
    SecretPromptRequest, SecretPromptRetry, SecretScope,
};
pub use secret_seam::SecretSeam;

use coordinator_gate::CoordinatorGate;

pub use auth_pubkey_cache::AuthPubkeyCacheView;
pub use avatar_cache::AvatarCacheView;
pub use contact_profile_cache::{CachedContactProfile, ContactProfileCacheView};
pub use event_bridge::EventBridge;
pub(crate) use kv::network_prefix;
pub use kv::{DetKv, DetScope, KvAdapterError, SCHEMA_VERSION as KV_SCHEMA_VERSION};
pub use loader::LoadedWallets;
pub use single_key::SingleKeyView;
use snapshot::SnapshotStore;
pub use snapshot::{DetUtxo, DetWalletBalance, WalletSnapshot};
use token_balance::TokenBalanceStore;
pub use token_balance::UpstreamTokenBalances;
pub use wallet_meta::WalletMetaView;
pub use wallet_seed_store::WalletSeedView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenBalanceSyncOutcome {
    Performed,
    AlreadyInFlight,
}

async fn run_token_balance_sync_if_idle<IsSyncing, SyncNow, SyncFuture>(
    mut is_syncing: IsSyncing,
    sync_now: SyncNow,
) -> TokenBalanceSyncOutcome
where
    IsSyncing: FnMut() -> bool,
    SyncNow: FnOnce() -> SyncFuture,
    SyncFuture: std::future::Future<Output = ()>,
{
    if is_syncing() {
        return TokenBalanceSyncOutcome::AlreadyInFlight;
    }
    sync_now().await;
    if is_syncing() {
        TokenBalanceSyncOutcome::AlreadyInFlight
    } else {
        TokenBalanceSyncOutcome::Performed
    }
}

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dash_sdk::Sdk;
use dash_sdk::dash_spv::ClientConfig;
use dash_sdk::dash_spv::client::config::MempoolStrategy;
use dash_sdk::dash_spv::types::ValidationMode;
use dash_sdk::dpp::dashcore::Network;
use platform_wallet::error::PlatformWalletError;
use platform_wallet::manager::PlatformWalletManager;
use platform_wallet_storage::secrets::SecretStore;
use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig};

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::connection_status::ConnectionStatus;
use crate::model::selected_identity::SelectedIdentity;
use crate::model::selected_wallet::SelectedWallet;
use crate::model::wallet::meta::wallet_label;
use crate::model::wallet::{PlatformAddressEntry, WalletSeedHash};
use crate::utils::egui_mpsc::SenderAsync;

/// The upstream persister DET consumes. Authored upstream (PR #3625) — DET
/// does not write its own persister (removal-inventory: consume, don't
/// reimplement).
type DetPersister = SqlitePersister;

/// Which side of a contact relationship
/// [`WalletBackend::record_contact_request`] writes into the local
/// wallet-manager. Selects the upstream `add_*_contact_request` call and the
/// warning wording for the missing-managed-identity case.
#[derive(Clone, Copy)]
enum ContactRequestRecord {
    /// Our outgoing request — recorded into `sent_contact_requests`.
    Sent,
    /// A peer's incoming request — recorded into `incoming_contact_requests`.
    Incoming,
}

#[derive(Debug)]
enum StartFlightError {
    Failed(Arc<PlatformWalletError>),
    Superseded,
}

type StartFlightOutcome = Result<(), StartFlightError>;

#[derive(Debug, Default)]
struct StartFlight {
    begun: AtomicBool,
    outcome: tokio::sync::OnceCell<StartFlightOutcome>,
}

/// Shared-result latch guarding chain-sync startup. The upstream
/// `SpvRuntime::spawn_run_loop` unconditionally spawns a fresh run loop per
/// call, so [`WalletBackend::start`] joins concurrent callers onto one flight
/// and reuses its outcome without spawning a second loop.
#[derive(Debug)]
struct StartLatch {
    current: std::sync::Mutex<Arc<StartFlight>>,
    lifecycle: tokio::sync::Mutex<()>,
}

impl Default for StartLatch {
    fn default() -> Self {
        Self {
            current: std::sync::Mutex::new(Arc::new(StartFlight::default())),
            lifecycle: tokio::sync::Mutex::new(()),
        }
    }
}

impl StartLatch {
    fn flight(&self) -> Arc<StartFlight> {
        Arc::clone(
            &self
                .current
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    fn is_current(&self, flight: &Arc<StartFlight>) -> bool {
        Arc::ptr_eq(
            &self
                .current
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            flight,
        )
    }

    async fn claim(&self, flight: &Arc<StartFlight>) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        let lifecycle = self.lifecycle.lock().await;
        self.is_current(flight).then_some(lifecycle)
    }

    /// Whether the latch has been triggered.
    fn is_started(&self) -> bool {
        self.flight().begun.load(Ordering::SeqCst)
    }

    /// Re-arm the latch with a fresh flight for the next start attempt.
    fn reset(&self) {
        *self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(StartFlight::default());
    }

    /// Re-arm only if `flight` is still current, preserving a newer reset.
    fn reset_if_current(&self, flight: &Arc<StartFlight>) {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if Arc::ptr_eq(&current, flight) {
            *current = Arc::new(StartFlight::default());
        }
    }
}

/// Default BIP-44 account index for wallet receive/send operations. DET has
/// always operated account 0; multi-account support is out of P2 scope.
const DEFAULT_BIP44_ACCOUNT: u32 = 0;

/// Number of times [`WalletBackend::resolve_registered_wallet`] re-probes the
/// upstream wallet manager before concluding a wallet is genuinely absent.
/// Tolerates the brief window where a concurrent registration has created the
/// wallet upstream but the manager has not finished exposing it via
/// `get_wallet` — the loser of that race must not spuriously fail.
const REGISTRATION_RESOLVE_RETRIES: u32 = 50;

/// Delay between the re-probes counted by [`REGISTRATION_RESOLVE_RETRIES`].
/// Fifty tries at 20ms bound the wait below one second in the (rare)
/// genuinely-absent case while covering slower concurrent registrations.
const REGISTRATION_RESOLVE_BACKOFF: std::time::Duration = std::time::Duration::from_millis(20);

/// Upstream `WalletId` = `SHA256(root_xpub || root_chain_code)`, distinct
/// from DET's `WalletSeedHash` = `SHA256(seed_bytes)`. The map is the bridge:
/// populated once per wallet at registration, read by every DET-keyed call.
type WalletId = [u8; 32];

/// Per-wallet platform-address warm-start seed: `(seed_hash, owned
/// [`PlatformAddressEntry`] list, optional (timestamp, height) cursor)`.
type PlatformWarmStartSeed = Vec<(
    WalletSeedHash,
    Vec<PlatformAddressEntry>,
    Option<(u64, u64)>,
)>;

type RegistrationFlightOutcome = Result<(), Arc<TaskError>>;

struct RegistrationFlight {
    outcome: tokio::sync::OnceCell<RegistrationFlightOutcome>,
}

impl RegistrationFlight {
    fn new() -> Self {
        Self {
            outcome: tokio::sync::OnceCell::new(),
        }
    }
}

struct Inner {
    pwm: PlatformWalletManager<DetPersister>,
    /// Shared handle to the same persister `pwm` consumes. Kept so the
    /// typed key/value adapter ([`DetKv`]) can read/write app data
    /// alongside wallet state without opening a second connection.
    persister: Arc<DetPersister>,
    /// Display-only snapshot store (balance/tx/utxo), pushed by the
    /// `EventBridge`. See [`snapshot`]. DISPLAY-ONLY — never feeds coin
    /// selection (A04 fund-safety gate).
    snapshots: Arc<SnapshotStore>,
    /// Lock-free per-`(identity, token)` balance snapshot, refreshed off the
    /// UI thread from the upstream `IdentitySyncManager` and read on the
    /// frame thread via [`Self::token_balances`]. See [`token_balance`].
    token_balances: Arc<TokenBalanceStore>,
    /// `WalletSeedHash` → upstream `WalletId`. See [`WalletId`].
    id_map: std::sync::RwLock<std::collections::BTreeMap<WalletSeedHash, WalletId>>,
    #[cfg(test)]
    registration_attempts: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    registration_test_barrier: std::sync::Mutex<Option<Arc<tokio::sync::Barrier>>>,
    #[cfg(test)]
    registration_test_failure: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    clear_shielded_test_failure: std::sync::atomic::AtomicBool,
    /// Per-wallet shared-result flights for upstream registration. Every caller
    /// that joins an active flight awaits the same success or typed error.
    registration_flights:
        std::sync::Mutex<std::collections::BTreeMap<WalletSeedHash, Arc<RegistrationFlight>>>,
    /// Request-wide async locks for paid DashPay actions. The Hub and legacy
    /// DashPay screens have separate UI state, so backend serialization is the
    /// final guard against two callers paying for the same request concurrently.
    dashpay_request_action_locks: dashpay::ContactRequestActionLocks,
    /// Cache of `Arc<PlatformWallet>` keyed by `WalletId`, populated at
    /// registration. Lets sync code reach an upstream wallet handle without an
    /// async hop (e.g. DashPay address-pool scanning).
    wallets: std::sync::RwLock<
        std::collections::BTreeMap<WalletId, Arc<platform_wallet::PlatformWallet>>,
    >,
    /// Optional peer `host:port` for Devnet/Regtest or a user-selected local
    /// node. `None` ⇒ DNS-seed discovery (Mainnet/Testnet default).
    peer: Option<std::net::SocketAddr>,
    network: Network,
    spv_storage_dir: std::path::PathBuf,
    /// Serializes DashPay address-index increments across the process. The
    /// `DetKv` adapter has no atomic read-modify-write primitive, so the
    /// `dashpay_increment_send_index` path takes this mutex around its
    /// get-then-put cycle. Contention is negligible — outgoing-payment
    /// dispatch is user-initiated and rare relative to lock acquisition
    /// cost.
    dashpay_address_index_lock: std::sync::Mutex<()>,
    /// Encrypted secret vault. Holds imported single-key WIFs
    /// (`single_key_priv.*` labels, see [`single_key`]) and HD-wallet
    /// BIP-39 seeds (`seed.raw.v1`, with `envelope.v1` only during migration,
    /// under `WalletId(seed_hash)`; see [`wallet_seed_store`]).
    /// [`Self::secret_access`] decrypts seeds
    /// just-in-time from this vault for each signing operation; no
    /// long-lived plaintext seed cache exists.
    secret_store: Arc<SecretStore>,
    /// Cross-network app-level k/v store at `<data_dir>/det-app.sqlite`.
    /// Backs the DET-owned wallet-metadata sidecar (alias / `is_main` /
    /// `core_wallet_name`) — see [`wallet_meta`] (T-W-00). Shared with
    /// `AppContext::app_kv` so settings and wallet meta both write into
    /// the same persister.
    app_kv: Arc<DetKv>,
    /// In-memory index of imported single-key entries, keyed by their
    /// P2PKH address. Drives `SingleKeyView::list` without enumerating
    /// the (non-enumerable) secret store. Seeded on cold boot from the
    /// k/v sidecar by `hydrate_context_wallets` (T-W-01b) and kept in
    /// sync by `SingleKeyView::import_wif` / `forget`.
    single_key_index: std::sync::RwLock<
        std::collections::BTreeMap<String, crate::model::single_key::ImportedKey>,
    >,
    /// The just-in-time secret chokepoint. Constructed over the same
    /// [`Self::secret_store`] with the host-chosen [`SecretPrompt`]; seeded
    /// with prompt-copy metadata at hydration. Every signing / derivation
    /// consumer obtains plaintext through this — HD seeds via
    /// [`SecretScope::HdSeed`], imported keys via [`SecretScope::SingleKey`]
    /// — so no long-lived plaintext seed or single-key cache exists.
    secret_access: SecretAccess,
    /// Guards [`WalletBackend::start`] so chain sync spawns exactly once.
    /// See [`StartLatch`].
    start_latch: StartLatch,
    /// Quorum-readiness gate for the Platform/identity sync coordinators.
    /// Shared with the `EventBridge`: `start` arms it, the bridge fires it when
    /// the masternode list reaches `Synced`. See [`CoordinatorGate`].
    coordinator_gate: Arc<CoordinatorGate>,
    /// Frame-loop result channel, used by [`WalletBackend::start`] to nudge
    /// `AppState` to run the all-wallets identity sweep once Platform is ready.
    /// A cheap owned clone of the same sender the `EventBridge` holds.
    task_result_sender: SenderAsync<TaskResult>,
}

/// The single wallet entry point. See module docs.
#[derive(Clone)]
pub struct WalletBackend {
    inner: Arc<Inner>,
}

/// Outcome of the [`WalletBackend::forget_all_wallets_local`] "delete all local
/// data" sweep: the upstream wallet ids whose watch-only persistor rows still
/// need async removal, plus every delete failure so the caller reports a
/// partial wipe instead of a false success.
pub(crate) struct ClearAllOutcome {
    pub(crate) upstream_ids: Vec<WalletId>,
    pub(crate) failures: Vec<TaskError>,
}

impl std::fmt::Debug for WalletBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalletBackend")
            .field("network", &self.inner.network)
            .finish_non_exhaustive()
    }
}

impl WalletBackend {
    pub(crate) fn sdk(&self) -> &Sdk {
        self.inner.pwm.sdk()
    }

    /// Construct the backend: open the upstream SQLite persister, build the
    /// `PlatformWalletManager` with the DET `EventBridge`, then register every
    /// persisted wallet via [`Self::load_from_persistor_seedless`] (per
    /// registration, upstream `create_wallet_from_seed_bytes` also rehydrates
    /// persisted identity/address state — see g2-mock-boundary.md §G2.1 and the
    /// upstream-reality note in the P2 recommendation).
    ///
    /// Does NOT start chain sync — call [`Self::start`] after construction.
    pub async fn new(
        ctx: &Arc<AppContext>,
        sdk: Arc<Sdk>,
        connection_status: Arc<ConnectionStatus>,
        task_result_sender: SenderAsync<TaskResult>,
        prompt: Arc<dyn SecretPrompt>,
    ) -> Result<Self, TaskError> {
        let network = ctx.network;
        let spv_storage_dir = Self::resolve_spv_storage_dir(ctx.data_dir(), network)?;

        let persister_config =
            SqlitePersisterConfig::new(spv_storage_dir.join("platform-wallet.sqlite"));
        let persister = Arc::new(
            SqlitePersister::open(persister_config)
                .map_err(TaskError::from_wallet_storage_open_error)?,
        );

        // Reuse the vault handle `AppContext` already opened at boot. The file
        // backend holds an exclusive advisory lock for the handle's lifetime,
        // so opening a second handle here would fail with `AlreadyLocked` — and
        // `register_wallet` must be able to write seed-envelope sidecars through
        // the same handle before the backend is wired.
        let secret_store = ctx.secret_store();

        let snapshots = Arc::new(SnapshotStore::new());

        let coordinator_gate = Arc::new(CoordinatorGate::default());

        let bridge = Arc::new(EventBridge::new(
            connection_status,
            task_result_sender.clone(),
            Arc::clone(&snapshots),
            Arc::clone(&coordinator_gate),
            // The shielded sync-completed callback writes per-wallet balances
            // into AppContext's frame-safe snapshot.
            Arc::clone(&ctx.shielded_balances),
            // Platform-address push writer: the platform address sync-completed callback
            // writes per-wallet owned-only balances into AppContext's frame-safe snapshot.
            Arc::clone(&ctx.platform_balances),
            // ...and the matching `(timestamp, height)` sync-cursor snapshot that
            // drives the "Addresses synced" status label.
            Arc::clone(&ctx.platform_sync_cursors),
        ));

        let pwm = PlatformWalletManager::new(sdk, Arc::clone(&persister), bridge);

        // Wire the upstream shielded coordinator into the manager.
        //
        // Uses a dedicated SQLite file (`platform-wallet-shielded.sqlite`) owned
        // entirely by the upstream coordinator — the single source of truth for
        // all Orchard state. The coordinator starts empty — no wallets are bound
        // until `ensure_shielded_bound` runs (on wallet unlock). Subsequent
        // calls with the same path are idempotent (upstream no-ops).
        pwm.configure_shielded(spv_storage_dir.join("platform-wallet-shielded.sqlite"))
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Arc::new(e),
            })?;

        let peer = Self::spv_primary_peer_socket(ctx, network);

        let app_kv = ctx.app_kv();

        // The JIT chokepoint shares the same encrypted vault and is given the
        // host-chosen prompt (egui host in the GUI, `NullSecretPrompt`
        // headless). Wave C migrates consumers onto it; constructed now so
        // the prompt round-trips and the seam is live.
        let secret_access = SecretAccess::new(Arc::clone(&secret_store), prompt, network);

        let backend = Self {
            inner: Arc::new(Inner {
                pwm,
                persister,
                snapshots,
                token_balances: Arc::new(TokenBalanceStore::new()),
                id_map: std::sync::RwLock::new(std::collections::BTreeMap::new()),
                #[cfg(test)]
                registration_attempts: std::sync::atomic::AtomicUsize::new(0),
                #[cfg(test)]
                registration_test_barrier: std::sync::Mutex::new(None),
                #[cfg(test)]
                registration_test_failure: std::sync::atomic::AtomicBool::new(false),
                #[cfg(test)]
                clear_shielded_test_failure: std::sync::atomic::AtomicBool::new(false),
                registration_flights: std::sync::Mutex::new(std::collections::BTreeMap::new()),
                dashpay_request_action_locks: dashpay::ContactRequestActionLocks::default(),
                wallets: std::sync::RwLock::new(std::collections::BTreeMap::new()),
                peer,
                network,
                spv_storage_dir,
                dashpay_address_index_lock: std::sync::Mutex::new(()),
                secret_store,
                single_key_index: std::sync::RwLock::new(std::collections::BTreeMap::new()),
                app_kv,
                secret_access,
                start_latch: StartLatch::default(),
                coordinator_gate,
                task_result_sender,
            }),
        };

        // T-W-01 cold-boot: rebuild `ctx.wallets` from the wallet-meta +
        // seed-envelope sidecars before the loader runs. The legacy
        // `db.get_wallets` row → `Wallet` mapping moved here once the
        // sidecars became the authoritative source. `register_persisted_wallets`
        // expects `ctx.wallets` to be populated so it can re-provision
        // identity funding accounts (a5538dc8) for every persisted
        // identity, so hydration must precede registration.
        backend.hydrate_context_wallets(ctx)?;

        backend.register_persisted_wallets(ctx).await?;

        Ok(backend)
    }

    /// Refill `ctx.wallets` and `ctx.single_key_wallets` from the
    /// sidecars for the active network. Idempotent: a re-run overwrites
    /// with the same reconstructed wallets keyed by `seed_hash` /
    /// `key_hash`. Entries already present in the maps (e.g. created
    /// during the current process before the backend was wired) are
    /// preserved — sidecar entries only fill gaps so freshly-created
    /// wallets are never clobbered.
    ///
    /// Called once during [`Self::new`] (cold boot) and again by the
    /// `finish_unwire` migration after it populates the sidecars on first boot
    /// (F140) — at `new` time the sidecars are still empty, so without the
    /// post-migration re-run migrated wallets stay invisible until the second
    /// restart.
    pub(crate) fn hydrate_context_wallets(&self, ctx: &Arc<AppContext>) -> Result<(), TaskError> {
        let view = self.single_key();
        view.rehydrate_index()?;
        let single_key_wallets = view.hydrate_wallets();
        let reconstructed = self.hydrate_wallets_for_network(ctx.network)?;

        // Seed the JIT chokepoint's prompt-copy metadata so a passphrase
        // prompt can show the wallet alias / password hint and the key
        // nickname / hint. Absent metadata degrades to a generic label, so
        // this is best-effort and runs even when no wallets reconstruct.
        self.seed_secret_access_meta(&reconstructed);

        if reconstructed.is_empty() && single_key_wallets.is_empty() {
            return Ok(());
        }
        {
            let mut wallets = ctx.wallets.write()?;
            for (seed_hash, wallet) in reconstructed {
                wallets
                    .entry(seed_hash)
                    .or_insert_with(|| Arc::new(std::sync::RwLock::new(wallet)));
            }
            if !wallets.is_empty() {
                ctx.has_wallet
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if !single_key_wallets.is_empty() {
            let mut sk = ctx.single_key_wallets.write()?;
            for (key_hash, wallet) in single_key_wallets {
                sk.entry(key_hash)
                    .or_insert_with(|| Arc::new(std::sync::RwLock::new(wallet)));
            }
            if !sk.is_empty() {
                ctx.has_wallet
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        Ok(())
    }

    /// Run the seedless load to bring back persisted wallets watch-only.
    /// Identity-funding re-provision is deferred to the asset-lock chokepoint
    /// (which obtains the seed just-in-time), so this pass loads, logs, and
    /// raises a warning banner for any skipped wallet.
    async fn register_persisted_wallets(&self, ctx: &Arc<AppContext>) -> Result<(), TaskError> {
        let outcome = self.load_from_persistor_seedless(ctx).await?;
        tracing::info!(
            loaded = outcome.loaded.len(),
            "Persisted-wallet load pass complete"
        );

        // `load()` rebuilds `IdentityRegistration` from the manifest, but
        // per-index `IdentityTopUp{registration_index}` enters the manifest
        // only after a register/top-up, so a reloaded already-registered
        // identity lacks it. Re-deriving needs the seed (absent under seedless
        // load), so every identity asset lock re-provisions at the
        // `create_asset_lock_proof` chokepoint. Nothing to do at load time.
        for seed_hash in &outcome.loaded {
            tracing::debug!(
                wallet = %hex::encode(seed_hash),
                "Deferring identity-funding provision to the asset-lock chokepoint (seed obtained just-in-time)"
            );
        }
        Ok(())
    }

    /// Seedless watch-only load over the upstream PR #3692 rehydration
    /// API. One `load_from_persistor()` call rebuilds every wallet **the
    /// persistor already holds** as a watch-only entry (no seed touched);
    /// each registered upstream `WalletId` is then resolved to its DET
    /// [`WalletSeedHash`] by matching the loaded wallet's BIP44 account
    /// xpub against the `xpub_encoded` DET persisted in its sidecar.
    ///
    /// READ-ONLY: this does not register wallets. A wallet absent from the
    /// persistor (never created/imported under the W1/W2 writers, or
    /// post-reset) is not loaded here — it is registered by
    /// [`Self::register_wallet_from_seed`] (W1) or
    /// [`Self::ensure_upstream_registered`] (W2) the next time its seed is
    /// in hand, after which this path rebuilds it on subsequent boots.
    ///
    /// Fund-routing gate (HIGH): a loaded wallet whose BIP44 account
    /// xpub matches **no** sidecar entry is rejected — never registered,
    /// never displayed. The match is the published-xpub == scanned-xpub
    /// invariant by construction: the watch-only wallet is rebuilt from
    /// the persisted account manifest, and DET keys off the same
    /// account xpub it published at create time.
    ///
    /// Idempotent: the upstream `load_from_persistor()` rejects a wallet
    /// that is already registered, so it runs only when the manager is
    /// empty. A re-run (relaunch simulation, migration replay) re-resolves
    /// the already-registered wallets without a second upstream load.
    ///
    /// No upstream type escapes this method: `WalletId` / `PlatformWallet`
    /// are mapped to [`LoadedWallets`] (DET [`WalletSeedHash`]) before
    /// returning.
    pub(super) async fn load_from_persistor_seedless(
        &self,
        ctx: &Arc<AppContext>,
    ) -> Result<LoadedWallets, TaskError> {
        // 1. Build the account-xpub -> WalletSeedHash bridge from DET's
        //    sidecars. Seedless: `xpub_encoded` is the persisted
        //    `m/44'/coin'/0'` account xpub (see model/wallet/meta.rs).
        let bridge: std::collections::HashMap<Vec<u8>, WalletSeedHash> = self
            .wallet_meta()
            .list(ctx.network)
            .into_iter()
            .filter(|(_, meta)| !meta.xpub_encoded.is_empty())
            .map(|(seed_hash, meta)| (meta.xpub_encoded, seed_hash))
            .collect();

        // 2. One persister load pass, only when the manager is empty. Any
        //    load failure is fatal: the upstream API returns `Result<(), _>`,
        //    so `Err` is the only failure signal. On a re-run the manager
        //    already holds the wallets, so the upstream load (which rejects
        //    duplicates) is skipped and only the resolution below runs.
        if self.inner.pwm.wallet_ids().await.is_empty() {
            self.inner
                .pwm
                .load_from_persistor()
                .await
                .map_err(|e| TaskError::WalletBackend {
                    source: Arc::new(e),
                })?;
        }

        // 3. Resolve every currently-registered wallet to its DET seed
        //    hash via the fund-routing gate, registering it in the
        //    DET-keyed maps. Driven off the manager's live wallet set so
        //    the path is identical on first load and re-run.
        let mut loaded = Vec::new();
        for wallet_id in self.inner.pwm.wallet_ids().await {
            let Some(pw) = self.inner.pwm.get_wallet(&wallet_id).await else {
                tracing::warn!(
                    wallet_id = %hex::encode(wallet_id),
                    "Registered wallet not retrievable from manager; skipping"
                );
                continue;
            };

            // Fund-routing gate: resolve the DET seed hash by matching
            // the loaded wallet's BIP44 account xpub against the bridge.
            // An unmatched wallet is rejected.
            let account_xpub = self.bip44_account_xpub_encoded(&pw).await;
            let Some(seed_hash) = account_xpub.as_ref().and_then(|x| bridge.get(x).copied()) else {
                tracing::warn!(
                    wallet_id = %hex::encode(wallet_id),
                    "Loaded wallet xpub matches no persisted DET wallet; rejecting (fund-routing gate)"
                );
                continue;
            };

            self.hydrate_persisted_transactions(&wallet_id)?;
            self.inner.id_map.write()?.insert(seed_hash, wallet_id);
            self.inner
                .wallets
                .write()?
                .insert(wallet_id, Arc::clone(&pw));
            self.inner
                .snapshots
                .register_wallet(seed_hash, wallet_id, pw);
            self.inner.snapshots.recompute(&wallet_id);
            tracing::debug!(
                wallet = %hex::encode(seed_hash),
                "Watch-only wallet registered with backend (seedless)"
            );
            loaded.push(seed_hash);
        }

        Ok(LoadedWallets { loaded })
    }

    /// `ExtendedPubKey::encode()` bytes of the loaded watch-only wallet's
    /// BIP44 account 0 xpub, or `None` when the wallet has no BIP44
    /// account. Read seedlessly off the rebuilt watch-only manifest.
    async fn bip44_account_xpub_encoded(
        &self,
        pw: &platform_wallet::PlatformWallet,
    ) -> Option<Vec<u8>> {
        let guard = pw.state().await;
        bip44_account0_xpub(guard.wallet().accounts.all_accounts()).map(|x| x.encode().to_vec())
    }

    /// Build a short-lived signable wallet from the raw HD `seed`, with the
    /// default account set, so hardened account xpubs and the `WalletId` can be
    /// derived — the live wallet is watch-only and cannot derive hardened paths
    /// itself. Callers read only the derived public material; the seed is
    /// borrowed from an open secret session and never retained here.
    fn seed_wallet(
        &self,
        seed: &[u8; 64],
    ) -> Result<dash_sdk::dpp::key_wallet::wallet::Wallet, TaskError> {
        use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
        UpstreamWallet::from_seed_bytes(
            *seed,
            self.inner.network,
            WalletAccountCreationOptions::Default,
        )
        .map_err(|source| TaskError::SeedWalletBuildFailed { source })
    }

    /// The upstream `WalletId = SHA256(root_xpub ‖ chaincode)` and BIP44
    /// account-0 xpub bytes for the given seed, computed WITHOUT registering.
    ///
    /// Builds the same `key_wallet::Wallet` the upstream
    /// `create_wallet_from_seed_bytes` builds internally and reads its
    /// already-computed `wallet_id` (the idempotency probe key) and account
    /// xpub (the fund-routing gate's expected value). DET cannot derive the
    /// `WalletId` from its sidecar account xpub — BIP44 hardens every level
    /// above the account — so the seed is required; this is only ever called
    /// where the seed is already in hand.
    fn upstream_identity_from_seed(
        &self,
        seed: &[u8; 64],
    ) -> Result<(WalletId, Vec<u8>), TaskError> {
        let wallet = self.seed_wallet(seed)?;
        let account_xpub = bip44_account0_xpub(wallet.accounts.all_accounts())
            .map(|x| x.encode().to_vec())
            // A freshly-built default wallet always has its BIP44 account 0;
            // its absence is an internal inconsistency, not an xpub mismatch.
            .ok_or(TaskError::WalletStateInconsistent)?;
        Ok((wallet.wallet_id, account_xpub))
    }

    // TODO: receive-address reuse (tracked by TC-012). Two consecutive
    //   `next_receive_address()` calls return the SAME address: upstream
    //   `next_unused` returns the lowest UNUSED receive address until it is
    //   actually used on-chain — funds-safe BIP-44 keypool behavior, but not the
    //   "fresh address each call" UX the Receive flow wants. The fix is a
    //   reserve-on-hand-out API that must propagate three layers before DET can
    //   adopt it:
    //     1. dashpay/rust-dashcore#818 "feat(key-wallet): reserve receive
    //        addresses on hand-out" — adds `next_unused_and_reserve`
    //        (+ reserve/release/sweep); ready-for-review, NOT yet merged.
    //     2. dashpay/platform — surface it as
    //        `CoreWallet::next_receive_address_and_reserve_for_account` (the
    //        pinned rev still calls the old non-reserving path).
    //     3. DET — bump the platform dep, then switch
    //        `next_receive_address()` to the reserving variant.
    //   Until all three land, `next_receive_address` stays on `next_unused`
    //   (funds-safe) and tc_012's "advances each call" assertion is pinned
    //   PENDING; tc_012b's gap-window funds-safety assertion stays active.
    /// Register a wallet with the upstream SPV backend from its seed, so the
    /// upstream persistor is populated and the wallet's addresses are watched
    /// (W1 — create/import write path; regression fix).
    ///
    /// The upstream `create_wallet_from_seed_bytes` is the only writer to the
    /// `platform-wallet.sqlite` persistor; the seedless cold-boot loader only
    /// reads it. Without this call nothing ever populates the persistor, so a
    /// fresh / reset / migrated install never watches the wallet and received
    /// funds stay invisible at 100% sync.
    ///
    /// `birth_height_override` is the SPV scan-window floor — `None` scans from
    /// the current tip (fresh wallet), `Some(0)` from genesis (imported/
    /// recovered wallet that may already hold funds). See
    /// [`crate::model::wallet::birth_height`].
    ///
    /// Idempotent: a wallet already in the upstream manager is a no-op, and an
    /// upstream `WalletAlreadyExists` is mapped to `Ok` so a W1/W2 race never
    /// double-watches. The `seed` is borrowed for the call only and never
    /// parked.
    pub(crate) async fn register_wallet_from_seed(
        &self,
        seed_hash: &WalletSeedHash,
        seed: &[u8; 64],
        birth_height_override: Option<u32>,
    ) -> Result<(), TaskError> {
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
        use platform_wallet::error::PlatformWalletError;

        // Idempotency probe: if this seed's wallet is already registered in the
        // DET maps, there is nothing to do — never re-register, never
        // double-watch. The cheap `seed_hash` lookup runs FIRST so the common
        // already-registered case (W1 and W2 can both fire per boot) skips the
        // expensive seed derivation in `upstream_identity_from_seed`.
        if self.inner.id_map.read()?.contains_key(seed_hash) {
            tracing::debug!(
                wallet = %hex::encode(seed_hash),
                "Wallet already registered with backend; skipping upstream register"
            );
            return Ok(());
        }

        // Not registered: derive the upstream identity (BIP32 from the seed)
        // now, since the resolve / create paths below both need it.
        let (wallet_id, expected_account_xpub) = self.upstream_identity_from_seed(seed)?;

        if self.inner.pwm.get_wallet(&wallet_id).await.is_some() {
            // Present upstream but absent from the DET maps (e.g. a prior
            // partial run): resolve it into the maps without a second create.
            return self
                .resolve_registered_wallet(*seed_hash, wallet_id, &expected_account_xpub)
                .await;
        }

        // Write the persistor via the sole upstream writer. A concurrent
        // registration that wins the race surfaces as `WalletAlreadyExists`,
        // which is success for our purposes (the wallet IS registered).
        match self
            .inner
            .pwm
            .create_wallet_from_seed_bytes(
                self.inner.network,
                seed,
                WalletAccountCreationOptions::Default,
                birth_height_override,
            )
            .await
        {
            Ok(_pw) => {}
            Err(PlatformWalletError::WalletAlreadyExists(_)) => {
                tracing::debug!(
                    wallet = %hex::encode(seed_hash),
                    "Upstream reports wallet already exists; treating as registered"
                );
            }
            Err(e) => {
                return Err(TaskError::WalletBackend {
                    source: Arc::new(e),
                });
            }
        }

        self.resolve_registered_wallet(*seed_hash, wallet_id, &expected_account_xpub)
            .await?;
        tracing::info!(
            wallet = %hex::encode(seed_hash),
            birth_height = ?birth_height_override,
            "Registered wallet with upstream SPV backend"
        );
        Ok(())
    }

    /// Cold-boot / first-unlock reconciliation: register a wallet present in
    /// DET sidecars but absent from the upstream persistor (W2).
    ///
    /// Migrated installs, wallets created before this fix, and post-reset
    /// states land with an empty upstream persistor, so the seedless cold-boot
    /// loader has nothing to read. This re-populates the persistor the first
    /// time the seed becomes available — prompt-free for unprotected wallets at
    /// boot, on the unlock gesture for protected ones. The caller already holds
    /// the plaintext seed inside a JIT secret scope, so this introduces no new
    /// password prompt (preserves the watch-only-at-boot contract).
    ///
    /// Imported/recovered/migrated wallets may hold deposits made before
    /// registration, so this always uses `Some(0)` (genesis) — the only floor
    /// that guarantees a pre-existing deposit is found. Idempotent: a wallet
    /// already registered is a no-op.
    pub(crate) async fn ensure_upstream_registered(
        &self,
        seed_hash: &WalletSeedHash,
        seed: &[u8; 64],
    ) -> Result<(), TaskError> {
        use crate::model::wallet::birth_height::{WalletOrigin, registration_birth_height};

        if self.inner.id_map.read()?.contains_key(seed_hash) {
            return Ok(());
        }
        let flight = {
            let mut flights = self
                .inner
                .registration_flights
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            Arc::clone(
                flights
                    .entry(*seed_hash)
                    .or_insert_with(|| Arc::new(RegistrationFlight::new())),
            )
        };
        #[cfg(test)]
        let registration_test_barrier = {
            self.inner
                .registration_test_barrier
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        };
        #[cfg(test)]
        if let Some(barrier) = registration_test_barrier {
            barrier.wait().await;
        }
        let outcome = flight
            .outcome
            .get_or_init(|| async {
                #[cfg(test)]
                self.inner
                    .registration_attempts
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                #[cfg(test)]
                if self
                    .inner
                    .registration_test_failure
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    return Err(Arc::new(TaskError::WalletRegistrationXpubMismatch));
                }
                self.register_wallet_from_seed(
                    seed_hash,
                    seed,
                    registration_birth_height(WalletOrigin::Imported),
                )
                .await
                .map_err(Arc::new)
            })
            .await;

        {
            let mut flights = self
                .inner
                .registration_flights
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if flights
                .get(seed_hash)
                .is_some_and(|active| Arc::ptr_eq(active, &flight))
            {
                flights.remove(seed_hash);
            }
        }

        match outcome {
            Ok(()) => Ok(()),
            Err(source) => Err(TaskError::WalletRegistrationFlightFailed {
                source: Arc::clone(source),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn registration_attempt_count(&self) -> usize {
        self.inner
            .registration_attempts
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn set_registration_test_barrier(&self, parties: usize) {
        *self
            .inner
            .registration_test_barrier
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) =
            Some(Arc::new(tokio::sync::Barrier::new(parties)));
    }

    #[cfg(test)]
    pub(crate) fn set_registration_test_failure(&self, fail: bool) {
        self.inner
            .registration_test_failure
            .store(fail, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn set_clear_shielded_test_failure(&self, fail: bool) {
        self.inner
            .clear_shielded_test_failure
            .store(fail, std::sync::atomic::Ordering::Relaxed);
    }

    /// Resolve one just-registered upstream wallet into the DET-keyed maps via
    /// the account-xpub fund-routing gate, identical in spirit to the gate the
    /// seedless loader applies per wallet.
    ///
    /// Fund-routing gate (HIGH): the registered wallet's BIP44 account xpub
    /// MUST equal `expected_account_xpub` — DET's published xpub for this
    /// seed. A mismatch means the upstream wallet would route funds for a
    /// different seed than DET believes it holds, so it is rejected (never
    /// inserted into the maps, never displayed) rather than silently trusted.
    async fn resolve_registered_wallet(
        &self,
        seed_hash: WalletSeedHash,
        wallet_id: WalletId,
        expected_account_xpub: &[u8],
    ) -> Result<(), TaskError> {
        // A concurrent registration that won the create race may sit between
        // inserting the wallet upstream and exposing it through `get_wallet`, so
        // a single probe can read `None` even though the wallet IS being
        // registered. Re-poll a few times before declaring it missing — this is
        // the TOCTOU tolerance for the A→B window the loser can land in
        // (CWE-362/367). The fund-routing xpub gate below is unchanged.
        let mut pw = None;
        for attempt in 0..REGISTRATION_RESOLVE_RETRIES {
            if let Some(found) = self.inner.pwm.get_wallet(&wallet_id).await {
                pw = Some(found);
                break;
            }
            if attempt + 1 < REGISTRATION_RESOLVE_RETRIES {
                tokio::time::sleep(REGISTRATION_RESOLVE_BACKOFF).await;
            }
        }
        let Some(pw) = pw else {
            return Err(TaskError::WalletBackend {
                source: Arc::new(platform_wallet::error::PlatformWalletError::WalletNotFound(
                    hex::encode(wallet_id),
                )),
            });
        };

        let registered_xpub = self.bip44_account_xpub_encoded(&pw).await;
        if registered_xpub.as_deref() != Some(expected_account_xpub) {
            tracing::warn!(
                wallet = %hex::encode(seed_hash),
                "Registered wallet xpub does not match DET's published xpub; rejecting (fund-routing gate)"
            );
            return Err(TaskError::WalletRegistrationXpubMismatch);
        }

        self.hydrate_persisted_transactions(&wallet_id)?;
        self.inner.id_map.write()?.insert(seed_hash, wallet_id);
        self.inner
            .wallets
            .write()?
            .insert(wallet_id, Arc::clone(&pw));
        self.inner
            .snapshots
            .register_wallet(seed_hash, wallet_id, pw);
        self.inner.snapshots.recompute(&wallet_id);
        Ok(())
    }

    /// Restore persisted public transaction records before publishing a
    /// wallet's first display snapshot. This path never touches wallet secrets.
    fn hydrate_persisted_transactions(&self, wallet_id: &WalletId) -> Result<(), TaskError> {
        use crate::backend_task::error::WalletTransactionHistoryError;
        use dash_sdk::dpp::dashcore::hashes::Hash;
        use platform_wallet::changeset::PlatformWalletPersistence;
        use platform_wallet_storage::WalletStorageError;

        let storage_error = |source: rusqlite::Error| TaskError::WalletTransactionHistoryLoad {
            source: WalletTransactionHistoryError::Persistence {
                source: WalletStorageError::Sqlite(source).into(),
            },
        };
        let database_path = self.inner.spv_storage_dir.join("platform-wallet.sqlite");
        let connection = rusqlite::Connection::open_with_flags(
            database_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(&storage_error)?;
        let txid_bytes = {
            // Upstream's public API decodes full records one txid at a time.
            // Enumerate only its keys here, then delegate every record read.
            // Project invalid widths to an empty blob before materialization;
            // `Txid::from_slice` below then returns the typed hash error.
            let mut statement = connection
                .prepare(
                    "SELECT CASE WHEN length(txid) = 32 THEN txid ELSE X'' END \
                     FROM core_transactions WHERE wallet_id = ?1 ORDER BY txid",
                )
                .map_err(&storage_error)?;
            let rows = statement
                .query_map([wallet_id.as_slice()], |row| row.get::<_, Vec<u8>>(0))
                .map_err(&storage_error)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(&storage_error)?
        };
        drop(connection);

        let mut records = Vec::with_capacity(txid_bytes.len());
        for bytes in txid_bytes {
            let txid = dash_sdk::dpp::dashcore::Txid::from_slice(&bytes).map_err(|source| {
                TaskError::WalletTransactionHistoryLoad {
                    source: WalletTransactionHistoryError::Persistence {
                        source: WalletStorageError::HashDecode { source }.into(),
                    },
                }
            })?;
            let record = self
                .inner
                .persister
                .get_core_tx_record(*wallet_id, &txid)
                .map_err(|source| TaskError::WalletTransactionHistoryLoad {
                    source: WalletTransactionHistoryError::Persistence { source },
                })?
                .ok_or(TaskError::WalletTransactionHistoryLoad {
                    source: WalletTransactionHistoryError::RecordMissing { txid },
                })?;
            records.push(record);
        }

        self.inner
            .snapshots
            .hydrate_transactions(wallet_id, records.iter());
        Ok(())
    }

    /// Wipe every piece of DET-local state for a forgotten wallet — the
    /// encrypted seed-envelope vault, the session secret cache, the wallet-meta
    /// sidecar, and the in-memory `id_map`/`wallets`/snapshot registration.
    /// (Orchard state lives in the upstream coordinator now and is detached by
    /// [`Self::remove_upstream_wallet`].)
    ///
    /// This is the synchronous secret-bearing cleanup. The upstream
    /// (watch-only, seedless) persistor removal is the sole async step and is
    /// driven separately via [`Self::remove_upstream_wallet`]. Persisted secret
    /// state is removed before the in-memory handle, so a mid-failure crash
    /// never leaves a recoverable seed behind a forgotten in-memory entry.
    /// Resilient to partial failure: each step is logged and the rest still
    /// run. Idempotent — forgetting an unknown wallet is a no-op success. If any
    /// delete fails, the first failure is returned so the caller never reports a
    /// clean wipe when a recoverable secret may survive on disk.
    pub(crate) fn forget_wallet_local_state(
        &self,
        seed_hash: &WalletSeedHash,
        wallet_id: Option<WalletId>,
    ) -> Result<(), TaskError> {
        let mut first_error: Option<TaskError> = None;

        // Seed vault — delete BOTH the raw `seed.raw.v1` (the current form) and
        // the legacy `envelope.v1`. Idempotent on both; a wallet may be in
        // either form (raw post-migration, legacy pre-migration), so removal
        // must clear whichever is present to leave no recoverable seed.
        if let Err(e) = self.wallet_seeds().delete_raw(seed_hash) {
            tracing::warn!(
                wallet = %hex::encode(seed_hash),
                error = ?e,
                "Failed to delete raw seed from vault"
            );
            first_error.get_or_insert(e);
        }
        if let Err(e) = self.wallet_seeds().delete(seed_hash) {
            tracing::warn!(
                wallet = %hex::encode(seed_hash),
                error = ?e,
                "Failed to delete seed envelope from vault"
            );
            first_error.get_or_insert(e);
        }

        // Session secret cache (any remembered plaintext seed).
        self.inner.secret_access.forget(&Self::hd_scope(seed_hash));

        // DET wallet-meta sidecar.
        if let Err(e) = self.wallet_meta().delete(self.inner.network, seed_hash) {
            tracing::warn!(
                wallet = %hex::encode(seed_hash),
                error = ?e,
                "Failed to delete wallet-meta sidecar"
            );
            first_error.get_or_insert(e);
        }

        // Plaintext Orchard state (notes + nullifier cursor) now lives in the
        // upstream coordinator store; `remove_upstream_wallet` detaches it.

        // DET-side avatar cache. Avatars live in the cross-network
        // Global scope keyed by URL, not partitioned per wallet, so a forgotten
        // wallet's contact avatars would otherwise survive deletion forever.
        // It is a rebuild-on-view cache, so clearing the whole thing on any
        // wallet removal is correct and cheap.
        if let Err(e) = self.avatar_cache().clear() {
            tracing::warn!(
                wallet = %hex::encode(seed_hash),
                error = ?e,
                "Failed to clear avatar cache during wallet removal"
            );
            first_error.get_or_insert(e);
        }

        // In-memory maps + snapshot registration.
        if let Some(wallet_id) = wallet_id {
            self.inner.id_map.write()?.remove(seed_hash);
            self.inner.wallets.write()?.remove(&wallet_id);
            self.inner.snapshots.forget_wallet(seed_hash, &wallet_id);
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// The upstream `WalletId` DET has registered for `seed_hash`, if any.
    /// Sync, lock-free-ish (one read lock). Used by the sync wallet-removal
    /// path to drive the async upstream persistor removal off the main thread.
    pub(crate) fn registered_wallet_id(&self, seed_hash: &WalletSeedHash) -> Option<WalletId> {
        self.inner.id_map.read().ok()?.get(seed_hash).copied()
    }

    /// Reset the upstream shielded coordinator for this network: quiesces the
    /// 60-second sync loop and empties the coordinator's per-subwallet store so
    /// the next bind cold-resyncs from index 0. Idempotent and a no-op when
    /// shielded support was never configured. Used by the "delete all local
    /// data" sweep.
    pub(crate) async fn clear_shielded(&self) -> Result<(), TaskError> {
        #[cfg(test)]
        if self
            .inner
            .clear_shielded_test_failure
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(TaskError::WalletDataClearUnavailable);
        }

        self.inner
            .pwm
            .clear_shielded()
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Arc::new(e),
            })
    }

    /// Remove a wallet from the upstream `platform-wallet.sqlite` persistor
    /// (also detaches the shielded coordinator). The watch-only persistor row
    /// carries no seed, so this is safe to drive asynchronously after the sync
    /// secret-bearing cleanup has already run. A `WalletNotFound` race is
    /// success.
    pub(crate) async fn remove_upstream_wallet(
        &self,
        wallet_id: &WalletId,
    ) -> Result<(), TaskError> {
        use platform_wallet::error::PlatformWalletError;
        match self.inner.pwm.remove_wallet(wallet_id).await {
            Ok(_) | Err(PlatformWalletError::WalletNotFound(_)) => Ok(()),
            Err(e) => Err(TaskError::WalletBackend {
                source: Arc::new(e),
            }),
        }
    }

    /// Permanently wipe the DET-local state of EVERY wallet on this network —
    /// the "delete all local data" sweep (F60). Enumerates every persisted HD
    /// wallet (the wallet-meta sidecar) and every imported single key (the
    /// single-key sidecar), so it reaches wallets that were never loaded into
    /// memory this session, not just the in-memory set.
    ///
    /// Synchronous: it wipes the secret-bearing state (seed-envelope vault,
    /// single-key vault, sidecars, shielded notes, session cache, in-memory
    /// maps) with no runtime. Returns a [`ClearAllOutcome`] carrying the
    /// upstream `WalletId`s whose watch-only persistor rows still need the async
    /// [`Self::remove_upstream_wallet`] removal — the caller drives those
    /// off-thread — plus every delete failure. Resilient to partial failure:
    /// every wallet is attempted even after one fails.
    pub(crate) fn forget_all_wallets_local(&self) -> ClearAllOutcome {
        let network = self.inner.network;

        // HD wallets: enumerate from the persisted wallet-meta sidecar so a
        // never-loaded wallet is still wiped.
        let mut upstream_ids = Vec::new();
        let mut failures: Vec<TaskError> = Vec::new();
        for (seed_hash, _meta) in self.wallet_meta().list(network) {
            let wallet_id = self.registered_wallet_id(&seed_hash);
            if let Some(id) = wallet_id {
                upstream_ids.push(id);
            }
            // `forget_wallet_local_state` logs each failed step; keep only the
            // returned first failure so the caller can report a partial wipe.
            if let Err(e) = self.forget_wallet_local_state(&seed_hash, wallet_id) {
                failures.push(e);
            }
        }

        // Single-key wallets: enumerate from the persisted single-key sidecar
        // and forget each (vault row + sidecar meta + index entry).
        let single_key = self.single_key();
        for key in single_key.list_persisted() {
            if let Err(e) = single_key.forget(&key.address) {
                tracing::warn!(
                    address = %key.address,
                    error = ?e,
                    "Failed to forget single-key wallet during clear-all"
                );
                failures.push(e);
            }
        }

        // Belt-and-suspenders: drop any remaining session-cached secrets
        // (single-key forget does not clear the session cache).
        self.forget_all_secrets();

        ClearAllOutcome {
            upstream_ids,
            failures,
        }
    }

    /// Start chain sync and the periodic upstream coordinators.
    ///
    /// Upstream has no single `PlatformWalletManager::start()`; this
    /// orchestrates the parts: `SpvRuntime::start(ClientConfig)` followed by
    /// `spawn_run_loop()`, plus the platform-address / identity / shielded sync
    /// coordinators.
    ///
    /// `SpvRuntime::start()` initializes the network manager, disk storage,
    /// and SPV client. The sync loop runs separately on the tokio runtime;
    /// later sync failures surface asynchronously through the upstream run
    /// task and the `EventBridge` `on_error` callback.
    ///
    /// Idempotent: concurrent calls join one shared-result flight, and calls
    /// after a successful start reuse its `Ok(())` without spawning a second
    /// run loop.
    ///
    /// SPV is spawned immediately, but the Platform/identity sync coordinators
    /// are gated on masternode-list readiness via [`CoordinatorGate`]: starting
    /// them before quorums exist fires proof-verifying DAPI calls that fail
    /// locally and get every node banned by the SDK. The gate fires them once,
    /// either now (if masternodes already synced) or when the `EventBridge`
    /// reports the masternode list reached `Synced`.
    pub async fn start(&self) -> Result<(), TaskError> {
        loop {
            let flight = self.inner.start_latch.flight();
            let outcome = flight
                .outcome
                .get_or_init(|| async {
                    let Some(_lifecycle) = self.inner.start_latch.claim(&flight).await else {
                        return Err(StartFlightError::Superseded);
                    };
                    flight.begun.store(true, Ordering::SeqCst);
                    match self.start_once().await {
                        Ok(()) => Ok(()),
                        Err(source) => {
                            self.inner.start_latch.reset_if_current(&flight);
                            Err(StartFlightError::Failed(Arc::new(source)))
                        }
                    }
                })
                .await;

            match outcome {
                Ok(()) => return Ok(()),
                Err(StartFlightError::Failed(source)) => {
                    return Err(TaskError::WalletBackend {
                        source: Arc::clone(source),
                    });
                }
                Err(StartFlightError::Superseded) => {}
            }
        }
    }

    async fn start_once(&self) -> Result<(), PlatformWalletError> {
        let config = self.build_client_config();

        // New API: `start(config)` (async, initializes the SPV client) is called first,
        // then `spawn_run_loop()` (sync, spawns the background run-loop task).
        // The old `spawn_in_background(config)` combined both steps.
        let spv = self.inner.pwm.spv_arc();
        spv.start(config).await?;
        spv.spawn_run_loop();

        // Defer the coordinator starts behind the quorum-readiness gate. The
        // gate is reachable from the `EventBridge`, which the long-lived SPV
        // run loop holds, so the action captures WEAK handles: a strong capture
        // would pin the coordinators (and through them the persister's advisory
        // lock) for as long as the SPV task lives, surviving backend teardown
        // and blocking the next reconnect's persister open. At fire time the
        // backend is live, so the upgrade always succeeds.
        let platform_address_sync = Arc::downgrade(&self.inner.pwm.platform_address_sync_arc());
        let identity_sync = Arc::downgrade(&self.inner.pwm.identity_sync_arc());
        // Shielded sync coordinator (Orchard note scanning). Runs the
        // background `ShieldedSyncManager` loop once masternodes are ready.
        // `configure_shielded` (called in `new()`) opens the coordinator's
        // SQLite file up front; `start()` here just fires the scan loop.
        // If no wallets have called `bind_shielded` yet, each pass produces
        // an empty summary and returns immediately — safe no-op.
        let shielded_sync = Arc::downgrade(&self.inner.pwm.shielded_sync_arc());
        // Owned clone of the frame-loop sender: the gate closure fires exactly
        // once per session (single-winner `fired`), so this nudges `AppState` to
        // run the all-wallets identity sweep at most once, right when Platform
        // is provably reachable. Cloning the sender avoids capturing any
        // `Weak<AppContext>` in this run-loop closure.
        let task_result_sender = self.inner.task_result_sender.clone();
        self.inner.coordinator_gate.arm(Box::new(move || {
            match platform_address_sync.upgrade() {
                Some(coordinator) => coordinator.start(),
                None => tracing::warn!(
                    coordinator = "platform-address-sync",
                    "Coordinator start skipped: backend torn down before the quorum gate fired"
                ),
            }
            match identity_sync.upgrade() {
                Some(coordinator) => coordinator.start(),
                None => tracing::warn!(
                    coordinator = "identity-sync",
                    "Coordinator start skipped: backend torn down before the quorum gate fired"
                ),
            }
            match shielded_sync.upgrade() {
                Some(coordinator) => coordinator.start(),
                None => tracing::warn!(
                    coordinator = "shielded-sync",
                    "Coordinator start skipped: backend torn down before the quorum gate fired"
                ),
            }
            // Platform is reachable now — ask the frame loop to start the
            // all-wallets identity discovery sweep. Non-blocking and fired once
            // (single-winner gate): a full 256-deep channel would drop this and
            // the sweep would not run until a reconnect re-arms the gate, but the
            // user can always run discovery manually, so the drop is tolerated.
            let _ = task_result_sender.try_send(TaskResult::unattributed_success(
                BackendTaskSuccessResult::PlatformReadyDiscoverIdentities,
            ));
        }));

        Ok(())
    }

    /// Whether chain sync has been started on this backend.
    ///
    /// Reflects the latch set by [`Self::start`]; used by tests and by callers
    /// that want to avoid a redundant spawn attempt.
    pub fn is_started(&self) -> bool {
        self.inner.start_latch.is_started()
    }

    /// Stop all upstream background tasks. Idempotent.
    ///
    /// Stops the SPV run-loop task **first**, then quiesces the sync-manager
    /// coordinators.  Ordering matters for the platform-open registry:
    ///
    /// The spawned SPV background task holds `Arc<SpvRuntime>`, which
    /// transitively keeps `Arc<SqlitePersister>` alive via the chain
    /// `SpvRuntime → event_manager → BalanceUpdateHandler → wallets →
    /// Arc<PlatformWallet> → WalletPersister::inner`.  The upstream
    /// `platform-wallet-storage` crate uses a process-global `REGISTRY`
    /// (`OnceLock<Mutex<HashSet<PathBuf>>>`) to enforce a single open per
    /// path; a path is removed from the registry only in
    /// `Drop<SqlitePersister>`.  A still-running SPV task prevents the last
    /// `Arc<SqlitePersister>` from dropping, so the path stays registered and
    /// the next `WalletBackend::new` (reconnect) fails with
    /// `WalletStorageError::AlreadyOpen`.
    ///
    /// Calling `SpvRuntime::stop()` here joins the background task
    /// (abort-with-15 s timeout), releasing those transitive refs.  Once the
    /// task is gone, the remaining refs are all structural (inside
    /// `PlatformWalletManager`) and are released synchronously when the
    /// `WalletBackend` itself is dropped by the caller.
    ///
    /// Coordinator ordering: stopping the SPV *producer* before the
    /// *consumers* is safe — no new events can arrive, and any in-flight
    /// `sync_now` pass will complete before the subsequent `quiesce()` returns.
    pub async fn shutdown(&self) {
        // Stop the SPV run-loop first: joins/aborts the background task so the
        // transitive Arc<SqlitePersister> it holds is released before the
        // manager tears down its coordinators.  Errors here are non-fatal —
        // teardown must proceed regardless.
        if let Err(e) = self.inner.pwm.spv().stop().await {
            tracing::warn!(
                error = ?e,
                "SPV run loop did not stop cleanly during shutdown; continuing teardown"
            );
        }
        // `pwm.shutdown()` quiesces the periodic coordinators — draining any
        // in-flight pass and its persister / host-callback fan-out — then drains
        // the wallet-event adapter task. Best-effort: a non-clean report used to
        // flag a still-live worker or orphan, which teardown proceeds past
        // regardless — log it rather than surface it.
        //
        // TODO(platform-pr3968): `shutdown()` returns `()` at this rev (no
        // clean-shutdown report type yet) — the report check below is
        // commented out rather than dropped outright; restore once platform
        // re-adds the report type. User-confirmed removal of the
        // shutdown-failure warning log for this rev.
        self.inner.pwm.shutdown().await;
        // if !report.all_clean() {
        //     tracing::warn!(
        //         ?report,
        //         "Wallet manager shutdown did not complete cleanly; continuing teardown"
        //     );
        // }
    }

    /// Stop chain sync **in place**, keeping this backend (and its
    /// `Arc<SqlitePersister>`) alive so a same-network reconnect can restart
    /// on the SAME instance via [`Self::start`] — the persister DB is never
    /// closed/reopened, so the reconnect cannot hit
    /// `WalletStorageError::AlreadyOpen` (the root of the B-2 bug) by
    /// construction.
    ///
    /// Unlike [`Self::shutdown`], this deliberately does **not** call
    /// `pwm.shutdown()`: that cancels and joins the wallet-event adapter task,
    /// which has no re-create path, so a subsequent restart would lose event
    /// processing. Instead it stops the restartable pieces only:
    ///
    /// 1. `pwm.spv().stop()` — stops/joins the SPV run loop (releasing the SPV
    ///    storage advisory lock) while leaving the `SpvRuntime` and its
    ///    `PlatformEventManager` in place for the next `spawn_in_background`.
    /// 2. `quiesce()` the three sync coordinators (cancel + drain the in-flight
    ///    pass) via their `Arc` accessors — NOT `pwm.shutdown()` — so the event
    ///    adapter keeps running.
    /// 3. Re-arm the DET start gates ([`StartLatch::reset`] +
    ///    [`CoordinatorGate::reset`]) so the reconnect's `start()` spawns the
    ///    run loop again and re-fires the coordinators once masternodes re-sync.
    ///
    /// SPV is stopped before the coordinators (producer before consumers),
    /// mirroring [`Self::shutdown`]'s ordering.
    ///
    /// This is the live same-network disconnect path
    /// ([`AppContext::stop_spv`](crate::context::AppContext::stop_spv) calls it).
    ///
    /// SAFETY (restart-in-place vs the upstream coordinators): restarting the
    /// SAME coordinator instance is race-free because every coordinator clears
    /// its cancel slot under a `background_generation` guard in the pinned
    /// platform rev — `identity_sync`, `shielded_sync`, and (since b4506492)
    /// `platform_address_sync`. Without that guard a lagging old thread could
    /// clobber a freshly-installed cancel token, leaking an uncancellable /
    /// duplicate loop; the guard makes the stale thread observe the bumped
    /// generation and stand down.
    pub async fn stop_in_place(&self) {
        let _lifecycle = self.inner.start_latch.lifecycle.lock().await;
        // 1. Stop the SPV run loop first (producer), keeping the SpvRuntime.
        if let Err(e) = self.inner.pwm.spv().stop().await {
            tracing::warn!(
                error = ?e,
                "SPV run loop did not stop cleanly during stop_in_place; continuing"
            );
        }
        // 2. Quiesce the coordinators (consumers) directly — do NOT call
        //    `pwm.shutdown()`, which would also tear down the non-restartable
        //    wallet-event adapter.
        self.inner.pwm.platform_address_sync_arc().quiesce().await;
        self.inner.pwm.identity_sync_arc().quiesce().await;
        self.inner.pwm.shielded_sync_arc().quiesce().await;
        // 3. Re-arm the DET start gates for the next start() on this backend.
        self.inner.start_latch.reset();
        self.inner.coordinator_gate.reset();
    }

    /// Number of wallets currently registered with the backend.
    #[cfg(test)]
    pub async fn wallet_count(&self) -> usize {
        self.inner.pwm.wallet_ids().await.len()
    }

    /// Typed key/value adapter backed by the same upstream persister as
    /// wallet state. Used for DET-owned application data (settings,
    /// scheduled votes, DashPay overlays, etc.) that has no upstream
    /// schema of its own. See [`DetKv`] for namespacing conventions and
    /// the schema-version envelope.
    pub fn kv(&self) -> DetKv {
        DetKv::new(Arc::clone(&self.inner.persister))
    }

    /// Persisted platform-address warm-start data, read straight from the
    /// persister so the per-address tab, total balance, and "Addresses synced"
    /// label can render the last-synced snapshot on cold boot —
    /// network-independent, before the coordinator's first (network) pass.
    ///
    /// Per wallet: owned [`PlatformAddressEntry`] values (each carrying its
    /// DIP-17 `account`/`index` recovered from the persisted provider state)
    /// plus the persisted `(timestamp, height)` cursor when a prior sync
    /// completed. One full persister read; on failure returns empty and the
    /// first coordinator push warms the UI once the network is reachable.
    pub(crate) fn persisted_platform_address_warm_start(&self) -> PlatformWarmStartSeed {
        use platform_wallet::changeset::PlatformWalletPersistence;
        let start = match self.inner.persister.load() {
            Ok(start) => start,
            Err(e) => {
                tracing::debug!(error = ?e, "platform-address warm-start: persister load failed");
                return Vec::new();
            }
        };
        let per_wallet: Vec<(WalletId, Vec<PlatformAddressEntry>, u64, u64)> = start
            .platform_addresses
            .into_iter()
            .map(|(wallet_id, state)| {
                let entries: Vec<PlatformAddressEntry> = state
                    .per_account
                    .iter()
                    .flat_map(|(&account, account_state)| {
                        account_state.found().iter().map(move |(p2pkh, funds)| {
                            PlatformAddressEntry {
                                hash: p2pkh.to_bytes(),
                                balance: funds.balance,
                                nonce: funds.nonce,
                                account,
                                // Recover the DIP-17 index from the account's
                                // index↔address bijection so the address is
                                // registered exactly on warm-start; `None` (a
                                // found address absent from the bimap should not
                                // happen) falls back to reverse-derivation.
                                index: account_state.addresses().get_by_right(p2pkh).copied(),
                            }
                        })
                    })
                    .collect();
                (wallet_id, entries, state.sync_timestamp, state.sync_height)
            })
            .collect();
        platform_warm_start_seed(per_wallet, |wallet_id| {
            self.inner.snapshots.seed_hash_for(wallet_id)
        })
    }

    /// Per-`(identity, token)` balance view (T6 seam). Reads the lock-free
    /// snapshot last published by [`Self::refresh_token_balances`] off the
    /// upstream `IdentitySyncManager`. Infallible, frame-safe. See
    /// [`token_balance`] for the syncing-vs-zero contract.
    pub fn token_balances(&self) -> UpstreamTokenBalances {
        UpstreamTokenBalances::new(&self.inner.token_balances)
    }

    /// Reflect a proof-derived post-transaction balance in the token-balance
    /// snapshot immediately, before the next upstream sync pass confirms it.
    /// Synchronous and lock-free. Used by token mutation tasks (mint /
    /// transfer / burn / purchase / claim) for instant UI feedback.
    pub fn apply_known_token_balance(
        &self,
        identity_id: dash_sdk::platform::Identifier,
        token_id: dash_sdk::platform::Identifier,
        balance: u64,
    ) {
        self.inner
            .token_balances
            .apply(identity_id, token_id, balance);
    }

    /// Shared handle to the encrypted secret store backing imported-key
    /// material. Most callers should reach for [`Self::single_key`]
    /// instead — this accessor exists for the migration engine
    /// (T-SK-02), which writes legacy WIFs back into the vault.
    pub fn secret_store(&self) -> &Arc<SecretStore> {
        &self.inner.secret_store
    }

    /// The just-in-time secret chokepoint. O(1)-clone handle; signing,
    /// shielded bind, and DashPay derivation reach for this to obtain
    /// plaintext through [`SecretAccess::with_secret`] /
    /// [`SecretAccess::with_secret_session`] rather than any long-lived
    /// seed cache.
    pub fn secret_access(&self) -> SecretAccess {
        self.inner.secret_access.clone()
    }

    /// Clear every session-cached secret in the JIT chokepoint, zeroizing
    /// them. Called on network switch and teardown (the `AppContext` drops
    /// the per-network backend on switch, but this is the explicit, eager
    /// belt-and-suspenders path the design mandates).
    pub fn forget_all_secrets(&self) {
        self.inner.secret_access.forget_all();
    }

    /// Seed the JIT chokepoint's prompt-copy metadata from the reconstructed
    /// HD wallets and the rehydrated single-key index. Best-effort: missing
    /// metadata degrades to a generic prompt label, never an error.
    fn seed_secret_access_meta(
        &self,
        reconstructed: &[(WalletSeedHash, crate::model::wallet::Wallet)],
    ) {
        let wallet_meta: std::collections::BTreeMap<WalletSeedHash, PromptMeta> = reconstructed
            .iter()
            .map(|(seed_hash, wallet)| {
                (
                    *seed_hash,
                    PromptMeta {
                        alias: wallet.alias.clone(),
                        password_hint: wallet.password_hint().clone(),
                    },
                )
            })
            .collect();
        self.inner.secret_access.set_wallet_meta(wallet_meta);

        if let Ok(index) = self.inner.single_key_index.read() {
            self.inner.secret_access.set_single_key_index(index.clone());
        }
    }

    /// View over the single-key (imported WIF) operations. The view
    /// borrows the secret store, the in-memory address index, and the
    /// cross-network app k/v sidecar that persists imported-key metadata;
    /// all three are cheap to construct, so callers can build one per
    /// operation. Passphrase-protected signing goes through
    /// [`Self::sign_single_key`] (the JIT chokepoint), not this view.
    pub fn single_key(&self) -> SingleKeyView<'_> {
        SingleKeyView {
            secret_store: &self.inner.secret_store,
            index: &self.inner.single_key_index,
            network: self.inner.network,
            app_kv: Some(&self.inner.app_kv),
        }
    }

    /// Sign a 32-byte digest with the imported key at `address`, obtaining
    /// the key just-in-time through the JIT chokepoint. A passphrase-
    /// protected key prompts once; an unprotected key signs with no prompt
    /// (the chokepoint's unprotected fast-path). The decrypted key is
    /// borrowed by a [`DetSigner`] for the single sign and zeroized when the
    /// scope ends.
    ///
    /// Has no production caller yet — single-key *send* is still stubbed
    /// upstream — but is the documented signing chokepoint for that flow and
    /// is exercised by unit tests until it is un-gated.
    pub async fn sign_single_key(
        &self,
        address: &str,
        msg: &[u8; 32],
    ) -> Result<dash_sdk::dpp::dashcore::secp256k1::ecdsa::Signature, TaskError> {
        let scope = SecretScope::SingleKey {
            address: address.to_string(),
        };
        self.inner
            .secret_access
            .with_secret(&scope, |plaintext| {
                let signer = DetSigner::from_held(plaintext, self.inner.network);
                signer
                    .sign_single_key_ecdsa(msg)
                    .map_err(|source| TaskError::SingleKeySignFailed { source })
            })
            .await
    }

    /// View over the DET-owned wallet-metadata sidecar (alias /
    /// `is_main` / `core_wallet_name`). Backed by the cross-network
    /// app-level k/v store; see [`WalletMetaView`] (T-W-00) for the
    /// key schema. The view borrows a shared `Arc<DetKv>` handle, so
    /// callers may build one per operation rather than threading it.
    pub fn wallet_meta(&self) -> WalletMetaView<'_> {
        WalletMetaView::new(&self.inner.app_kv)
    }

    /// View over the DET-owned identity-metadata sidecar (the password hint for
    /// an identity whose keys are password-protected). Backed by the
    /// same cross-network app-level k/v store as [`Self::wallet_meta`]; see
    /// [`IdentityMetaView`] for the key schema. Display-only — it never gates
    /// whether a sign-time prompt fires (the vault scheme does).
    pub fn identity_meta(&self) -> IdentityMetaView<'_> {
        IdentityMetaView::new(&self.inner.app_kv)
    }

    /// Replace the JIT chokepoint's identity prompt-copy index from the loaded
    /// identities (alias) and their persisted hints ([`Self::identity_meta`]).
    /// Display-only: it never decides whether to prompt (the vault scheme
    /// does). Best-effort — a missing hint degrades to "no hint", never an
    /// error. Called whenever identities are (re)loaded so the sign-time prompt
    /// for an opted-in identity shows its label and hint.
    pub fn seed_identity_prompt_index(
        &self,
        identities: &[crate::model::qualified_identity::QualifiedIdentity],
    ) {
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        let network = self.inner.network;
        let meta_view = self.identity_meta();
        let index: std::collections::BTreeMap<[u8; 32], secret_access::PromptMeta> = identities
            .iter()
            .map(|qi| {
                let id = qi.identity.id().to_buffer();
                let password_hint = meta_view.get(network, &id).and_then(|m| m.password_hint);
                (
                    id,
                    secret_access::PromptMeta {
                        alias: Some(qi.to_string()),
                        password_hint,
                    },
                )
            })
            .collect();
        self.inner.secret_access.set_identity_prompt_index(index);
    }

    /// View over the DET-owned identity-authentication public-key cache
    /// (D4b). Backed by the same cross-network app-level k/v store as
    /// [`Self::wallet_meta`], keyed per wallet under
    /// `DetScope::Wallet(seed_hash)`; see [`AuthPubkeyCacheView`] for the
    /// key schema. The cache memoises the hardened-path identity-auth
    /// pubkeys so the steady-state read is seed-free.
    pub fn auth_pubkey_cache(&self) -> AuthPubkeyCacheView<'_> {
        AuthPubkeyCacheView::new(&self.inner.app_kv)
    }

    /// View over the DET-owned avatar image cache. Backed by the
    /// same cross-network app-level k/v store as [`Self::wallet_meta`], keyed
    /// by avatar URL under [`DetScope::Global`]. Upstream persists only the
    /// avatar hash and fingerprint, never the bytes, so this is the only place
    /// a contact's avatar image survives offline / between views.
    pub fn avatar_cache(&self) -> AvatarCacheView<'_> {
        AvatarCacheView::new(&self.inner.app_kv)
    }

    /// View over the DET-owned contact-profile cache. A contact's
    /// profile belongs to an out-of-wallet identity and is never rehydrated
    /// upstream, so this cache is the only offline source of a contact's
    /// display name, avatar URL, bio, and DPNS username between network reads.
    pub fn contact_profile_cache(&self) -> ContactProfileCacheView<'_> {
        ContactProfileCacheView::new(&self.inner.app_kv)
    }

    /// View over the encrypted HD wallet seed vault (T-W-00.5-v2).
    /// Each wallet's full seed envelope (ciphertext + salt + nonce +
    /// `uses_password` + hint + master xpub) lives behind one upstream
    /// `SecretStore` entry keyed by `WalletId(seed_hash)`. The vault's
    /// Argon2id + XChaCha20-Poly1305 layer protects the envelope at
    /// rest; DET's own AES-GCM envelope is preserved inside it so the
    /// per-wallet password UX is unchanged.
    pub fn wallet_seeds(&self) -> WalletSeedView<'_> {
        WalletSeedView::new(&self.inner.secret_store)
    }

    /// Per-network storage directory under `<data_dir>/spv/<network>/`.
    ///
    /// Hosts the upstream `platform-wallet.sqlite` persister file and any
    /// other per-network sidecar databases DET maintains (e.g. the shielded
    /// commitment tree at `shielded-commitment-tree.sqlite`).
    pub fn spv_storage_dir(&self) -> &std::path::Path {
        &self.inner.spv_storage_dir
    }

    /// Read the persisted [`SelectedWallet`] pointer for this network.
    ///
    /// Returns [`SelectedWallet::default`] (both fields `None`) when the
    /// blob is absent (fresh install, never selected) or the stored
    /// value fails to decode (corrupt / future schema). Backed by the
    /// same per-network [`SqlitePersister`] as wallet state — selection
    /// is per-network by construction, no key prefix needed.
    pub fn get_selected_wallet(&self) -> SelectedWallet {
        kv::kv_get_or_default(
            &self.kv(),
            DetScope::Global,
            SelectedWallet::KV_KEY,
            "selected_wallet",
        )
    }

    /// Persist the [`SelectedWallet`] pointer to this network's wallet
    /// k/v store.
    pub fn set_selected_wallet(&self, selected: &SelectedWallet) -> Result<(), KvAdapterError> {
        self.kv()
            .put(DetScope::Global, SelectedWallet::KV_KEY, selected)
    }

    /// Read the persisted [`SelectedIdentity`] pointer for this network.
    ///
    /// Returns [`SelectedIdentity::default`] (`None`) when the blob is absent
    /// (fresh install, never selected) or fails to decode. Backed by the same
    /// per-network persister as wallet state — selection is per-network by
    /// construction.
    pub fn get_selected_identity(&self) -> SelectedIdentity {
        kv::kv_get_or_default(
            &self.kv(),
            DetScope::Global,
            SelectedIdentity::KV_KEY,
            "selected_identity",
        )
    }

    /// Persist the [`SelectedIdentity`] pointer to this network's wallet
    /// k/v store.
    pub fn set_selected_identity(&self, selected: &SelectedIdentity) -> Result<(), KvAdapterError> {
        self.kv()
            .put(DetScope::Global, SelectedIdentity::KV_KEY, selected)
    }

    /// Resolve a quorum public key from the upstream SPV masternode state.
    /// This is the SDK proof-verification path (fed by `SpvProvider`).
    pub async fn get_quorum_public_key(
        &self,
        quorum_type: u32,
        quorum_hash: [u8; 32],
        core_chain_locked_height: u32,
    ) -> Result<[u8; 48], TaskError> {
        self.inner
            .pwm
            .spv()
            .get_quorum_public_key(quorum_type, quorum_hash, core_chain_locked_height)
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Arc::new(e),
            })
    }

    /// Whether chain sync has not yet reached the tip.
    pub async fn is_syncing(&self) -> bool {
        match self.inner.pwm.spv().sync_progress().await {
            Some(p) => !p.is_synced(),
            None => false,
        }
    }

    /// [`TaskError::WalletNotLoaded`] naming the wallet the caller asked for,
    /// so a user with several wallets open knows which one to wait for. Reads
    /// the alias from the meta sidecar — the wallet is by definition absent
    /// from `id_map` here, so there is no live handle to ask.
    fn wallet_not_loaded(&self, seed_hash: &WalletSeedHash) -> TaskError {
        let alias = self
            .wallet_meta()
            .get(self.inner.network, seed_hash)
            .map(|meta| meta.alias)
            .unwrap_or_default();
        TaskError::WalletNotLoaded {
            wallet_label: wallet_label(&alias, seed_hash),
        }
    }

    /// Map a DET `WalletSeedHash` to the upstream wallet handle.
    async fn resolve_wallet(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<Arc<platform_wallet::PlatformWallet>, TaskError> {
        // The guard drops before the error path reads the meta sidecar.
        let wallet_id = self.inner.id_map.read()?.get(seed_hash).copied();
        let wallet_id = wallet_id.ok_or_else(|| self.wallet_not_loaded(seed_hash))?;
        self.inner
            .pwm
            .get_wallet(&wallet_id)
            .await
            .ok_or(TaskError::WalletStateInconsistent)
    }

    /// Derive the next unused receive address for the wallet's default BIP-44
    /// account, as a DET address string.
    pub async fn next_receive_address(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<String, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let addr = wallet
            .core()
            .next_receive_address_for_account(DEFAULT_BIP44_ACCOUNT)
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Arc::new(e),
            })?;
        Ok(addr.to_string())
    }

    /// The BIP-44 external (receive) addresses SPV currently watches for the
    /// wallet's default account, as DET address strings.
    ///
    /// This is the SPV-monitored gap-limit window: only deposits to one of
    /// these addresses are seen by the wallet. The Receive flow must only ever
    /// hand out an address from this set — see the funds-safety regression in
    /// `tests/backend-e2e/wallet_tasks.rs`.
    ///
    /// Takes the upstream manager's blocking read lock, so this is for sync
    /// (UI-thread) callers only — never call it from inside a tokio task. From
    /// async, wrap it in `tokio::task::block_in_place`.
    pub fn monitored_receive_addresses(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<Vec<String>, TaskError> {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};

        let wallet_id = self.inner.id_map.read()?.get(seed_hash).copied();
        let wallet_id = wallet_id.ok_or_else(|| self.wallet_not_loaded(seed_hash))?;
        let standard = AccountType::Standard {
            index: DEFAULT_BIP44_ACCOUNT,
            standard_account_type: StandardAccountType::BIP44Account,
        };
        let addresses = self
            .inner
            .pwm
            .account_address_pools_blocking(&wallet_id, &standard)
            .into_iter()
            // pool_type 0 == External (receive); change addresses are pool 1.
            .filter(|pool| pool.pool_type == 0)
            .flat_map(|pool| pool.addresses.into_iter().map(|info| info.address))
            .collect();
        Ok(addresses)
    }

    /// Register every established contact's DIP-15 receiving account so the SPV
    /// layer watches the addresses each contact pays us at.
    ///
    /// The receiving-account path `m/9'/coin'/15'/0'/owner/friend` is hardened,
    /// so the upstream `IdentityWallet::register_contact_account` — which derives
    /// from the live wallet — returns a watch-only error on the wallets DET
    /// rehydrates at boot (they cannot do hardened derivation). This derives the
    /// account xpub from a seed-built (signable) wallet instead and inserts the
    /// managed `DashpayReceivingFunds` account directly: the contained
    /// seed-bearing dual-insert exception, sibling to
    /// [`Self::provision_identity_funding_account`].
    ///
    /// Upstream keeps the managed account in runtime state only, so this re-runs
    /// on every cold boot / unlock and is idempotent (the account-map insert
    /// overwrites in place). Only **newly-added** accounts trigger a
    /// `bump_monitor_revision`: the `dash-spv` mempool sync manager (shared via
    /// one `Arc`) checks the aggregate revision on each 100ms tick and rebuilds
    /// the peer bloom filter when it changes. The tick only runs when the mempool
    /// manager is in `SyncState::Synced`; if registration happens before SPV sync
    /// completes, the accounts are already in the wallet when `activate_all_peers`
    /// sends the initial `FilterLoad` at `SyncEvent::FiltersSyncComplete`, so
    /// the addresses are watched regardless of sync phase.
    ///
    /// The caller must already hold the wallet's JIT seed (this runs inside
    /// `bootstrap_wallet_addresses_jit`'s secret scope), so a locked wallet is
    /// never reached and never prompted. `contacts` are `(owner, contact)`
    /// identity-id pairs. Returns the number of **newly-inserted** accounts
    /// (0 on idempotent re-registration).
    pub(crate) async fn register_contact_receiving_accounts(
        &self,
        seed_hash: &WalletSeedHash,
        seed: &[u8; 64],
        contacts: &[(
            dash_sdk::platform::Identifier,
            dash_sdk::platform::Identifier,
        )],
    ) -> Result<usize, TaskError> {
        use dash_sdk::dpp::key_wallet::Account;
        use dash_sdk::dpp::key_wallet::AccountType;
        use dash_sdk::dpp::key_wallet::managed_account::ManagedCoreFundsAccount;
        use dash_sdk::dpp::key_wallet::managed_account::managed_account_trait::ManagedAccountTrait;
        use dash_sdk::dpp::key_wallet::managed_account::managed_account_type::ManagedAccountType;
        if contacts.is_empty() {
            return Ok(0);
        }
        let network = self.inner.network;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();

        // The DIP-15 receiving path is hardened, so derive the account xpubs
        // from a signable seed-built wallet — the live wallet is watch-only and
        // cannot derive hardened paths. Built once, reused for every contact.
        let seed_wallet = self.seed_wallet(seed)?;

        let mut accounts = Vec::with_capacity(contacts.len());
        for (owner, contact) in contacts {
            // Account 0': upstream `DashpayReceivingFunds` hardcodes account 0'
            // and every DET caller has only ever used account 0.
            let account_type = AccountType::DashpayReceivingFunds {
                index: 0,
                user_identity_id: owner.to_buffer(),
                friend_identity_id: contact.to_buffer(),
            };
            let path = match account_type.derivation_path(network) {
                Ok(path) => path,
                Err(error) => {
                    tracing::debug!(%error, "Skipping contact account: derivation path failed");
                    continue;
                }
            };
            match seed_wallet.derive_extended_public_key(&path) {
                Ok(account_xpub) => accounts.push(Account {
                    parent_wallet_id: Some(wallet_id),
                    account_type,
                    network,
                    account_xpub,
                    is_watch_only: false,
                }),
                Err(error) => {
                    tracing::debug!(%error, "Skipping contact account: xpub derivation failed");
                }
            }
        }
        if accounts.is_empty() {
            return Ok(0);
        }

        // Insert under the manager write lock — purely synchronous, no await
        // held. Track accounts that are genuinely new (map len growth) so we
        // only bump monitor_revision — and thus trigger a bloom-filter rebuild
        // — when the set actually changes, not on every idempotent re-run.
        let mut wm = wallet.wallet_manager().write().await;
        let info = wm
            .get_wallet_info_mut(&wallet_id)
            .ok_or(TaskError::WalletStateInconsistent)?;
        let before = info.core_wallet.accounts.dashpay_receival_accounts.len();
        for account in &accounts {
            let managed = ManagedCoreFundsAccount::from_account(account);
            if let Err(error) = info
                .core_wallet
                .accounts
                .insert_funds_bearing_account(managed)
            {
                tracing::debug!(%error, "Skipping contact account: managed insert failed");
            }
        }
        let newly_inserted = info
            .core_wallet
            .accounts
            .dashpay_receival_accounts
            .len()
            .saturating_sub(before);
        // Only bump the monitor revision when new accounts were added: a
        // revision bump on every unlock would cause a spurious bloom-filter
        // rebuild on each boot even when the contact set is unchanged.
        if newly_inserted > 0 {
            for account in info.core_wallet.accounts.all_funding_accounts_mut() {
                if matches!(
                    account.managed_account_type(),
                    ManagedAccountType::DashpayReceivingFunds { .. }
                ) {
                    account.bump_monitor_revision();
                }
            }
        }
        Ok(newly_inserted)
    }

    /// Count `DashpayReceivingFunds` accounts currently registered in the
    /// wallet-manager for `seed_hash`. Used in integration tests to assert that
    /// [`Self::register_contact_receiving_accounts`] actually wired contacts
    /// into the live wallet-manager state.
    pub async fn dashpay_receiving_account_count(&self, seed_hash: &WalletSeedHash) -> usize {
        let Ok(wallet) = self.resolve_wallet(seed_hash).await else {
            return 0;
        };
        let wallet_id = wallet.wallet_id();
        let wm = wallet.wallet_manager().read().await;
        wm.get_wallet_info(&wallet_id)
            .map(|info| info.core_wallet.accounts.dashpay_receival_accounts.len())
            .unwrap_or(0)
    }

    /// Record a successfully-sent contact request in the upstream
    /// wallet-manager's in-memory `sent_contact_requests` map.
    ///
    /// After a contact-request state transition is accepted by Platform,
    /// the local `ManagedIdentity` must be updated so that `dashpay_sync`
    /// can later auto-establish the contact when the peer's reciprocal
    /// request arrives.  The upstream auto-establishment gate in
    /// `add_incoming_contact_request` only promotes an identity to
    /// `established_contacts` when `sent_contact_requests[peer]` already
    /// exists locally.  DET's custom `send_contact_request_with_proof`
    /// bypasses `IdentityWallet::send_contact_request_with_external_signer`
    /// and therefore never writes to that map without this explicit call.
    ///
    /// Non-fatal when the managed identity is not yet in the manager —
    /// logs a warning and returns `Ok(())` since the state transition was
    /// already committed to Platform.
    pub(crate) async fn record_sent_contact_request(
        &self,
        seed_hash: &WalletSeedHash,
        owner_id: &dash_sdk::platform::Identifier,
        contact_request: platform_wallet::ContactRequest,
    ) -> Result<(), TaskError> {
        self.record_contact_request(
            seed_hash,
            owner_id,
            contact_request,
            ContactRequestRecord::Sent,
        )
        .await
    }

    /// Shared body for [`Self::record_sent_contact_request`] and
    /// [`Self::record_incoming_contact_request`]: record `contact_request` on
    /// the given `direction` into `owner_id`'s local wallet-manager, persisting
    /// the resulting changeset.
    ///
    /// Non-fatal when the managed identity is not yet in the manager — logs a
    /// direction-specific warning and returns `Ok(())` since the state
    /// transition was already committed to Platform.
    async fn record_contact_request(
        &self,
        seed_hash: &WalletSeedHash,
        owner_id: &dash_sdk::platform::Identifier,
        contact_request: platform_wallet::ContactRequest,
        direction: ContactRequestRecord,
    ) -> Result<(), TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let persister = wallet.persister().clone();
        let mut wm = wallet.wallet_manager().write().await;
        let info = wm
            .get_wallet_info_mut(&wallet_id)
            .ok_or(TaskError::WalletStateInconsistent)?;
        match info.identity_manager.managed_identity_mut(owner_id) {
            Some(managed) => {
                let recorded = match direction {
                    ContactRequestRecord::Sent => {
                        managed.add_sent_contact_request(contact_request, &persister)
                    }
                    ContactRequestRecord::Incoming => {
                        managed.add_incoming_contact_request(contact_request, &persister)
                    }
                };
                recorded.map_err(|e| TaskError::WalletBackend {
                    source: Arc::new(e.into()),
                })?;
            }
            None => match direction {
                ContactRequestRecord::Sent => tracing::warn!(
                    owner_id = %owner_id,
                    "record_sent_contact_request: managed identity not \
                     found; state transition committed but local manager \
                     not updated",
                ),
                ContactRequestRecord::Incoming => tracing::warn!(
                    owner_id = %owner_id,
                    "record_incoming_contact_request: managed identity not \
                     found; auto-establishment will depend on dashpay_sync",
                ),
            },
        }
        Ok(())
    }

    /// Record a peer's incoming contact request in the accepter's local
    /// wallet-manager **before** sending the reciprocal request.
    ///
    /// Called by `accept_contact_request` with the sender's CR document
    /// that was just fetched from Platform.  Pre-populating
    /// `incoming_contact_requests[sender]` means that when
    /// `record_sent_contact_request` fires for the accepter's outgoing
    /// CR immediately afterwards, `add_sent_contact_request` finds the
    /// matching incoming entry and auto-establishes the contact
    /// in-process — no `dashpay_sync` round-trip required.
    ///
    /// Without this call the accept path has a dead-end: after
    /// `record_sent_contact_request` populates `sent[A]`,
    /// `sync_contact_requests` sees `sent[A]` and skips A's incoming
    /// document (its skip guard is `sent || incoming || established`),
    /// so `add_incoming_contact_request` is never called and
    /// `established_contacts` stays empty.
    ///
    /// Non-fatal when the managed identity is absent — logs a warning
    /// and returns `Ok(())`.
    pub(crate) async fn record_incoming_contact_request(
        &self,
        seed_hash: &WalletSeedHash,
        owner_id: &dash_sdk::platform::Identifier,
        contact_request: platform_wallet::ContactRequest,
    ) -> Result<(), TaskError> {
        self.record_contact_request(
            seed_hash,
            owner_id,
            contact_request,
            ContactRequestRecord::Incoming,
        )
        .await
    }

    /// Re-run the seedless watch-only load pass (idempotent): the upstream
    /// `load_from_persistor` rebuilds each wallet watch-only and re-provisions
    /// identity funding accounts per loaded wallet.
    pub async fn ensure_wallets_registered(&self, ctx: &Arc<AppContext>) -> Result<(), TaskError> {
        self.register_persisted_wallets(ctx).await
    }

    // -----------------------------------------------------------------
    // Read accessors — DET-typed display surface (P4a).
    //
    // These read the EventBridge-pushed `WalletSnapshot`. They are
    // synchronous, lock-free, and infallible: an absent (pre-first-sync)
    // snapshot yields empties, which the UI renders as "syncing". The
    // snapshot is DISPLAY-ONLY — coin selection / tx construction MUST go
    // through `send_payment` / `create_asset_lock_proof` (A04 gate).
    // -----------------------------------------------------------------

    /// Confirmed / unconfirmed / total balance for the wallet.
    pub fn wallet_balance(&self, seed_hash: &WalletSeedHash) -> DetWalletBalance {
        self.inner.snapshots.snapshot(seed_hash).balance
    }

    /// Full transaction history for the wallet (event-sourced).
    pub fn transaction_history(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Vec<crate::model::wallet::WalletTransaction> {
        self.inner
            .snapshots
            .snapshot(seed_hash)
            .transactions
            .clone()
    }

    /// Current unspent outputs for the wallet. DISPLAY-ONLY — never feed
    /// these into coin selection (A04 fund-safety gate).
    pub fn utxos(&self, seed_hash: &WalletSeedHash) -> Vec<DetUtxo> {
        self.inner.snapshots.snapshot(seed_hash).utxos.clone()
    }

    /// UTXO-derived per-address balances for the wallet.
    pub fn address_balances(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> std::collections::BTreeMap<dash_sdk::dpp::dashcore::Address, u64> {
        self.inner
            .snapshots
            .snapshot(seed_hash)
            .address_balances
            .clone()
    }

    /// Authoritative derivation path for every generated address of the wallet,
    /// from the lock-free display snapshot. Lets the account-summary view
    /// categorize funded addresses DET's `watched_addresses` bookkeeping has not
    /// indexed yet, so none are dropped from the per-category tab totals.
    pub fn address_paths(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> std::collections::BTreeMap<
        dash_sdk::dpp::dashcore::Address,
        dash_sdk::dpp::key_wallet::bip32::DerivationPath,
    > {
        self.inner
            .snapshots
            .snapshot(seed_hash)
            .address_paths
            .clone()
    }

    /// The SPV-watched BIP-44 external (receive) addresses from the lock-free
    /// display snapshot, as strings.
    ///
    /// UI-thread safe (no blocking lock), unlike
    /// [`Self::monitored_receive_addresses`]. The Receive list reads this so it
    /// only ever shows watched addresses. Empty before the first sync publishes
    /// a snapshot — the UI then asks the backend to derive one.
    pub fn snapshot_monitored_receive_addresses(&self, seed_hash: &WalletSeedHash) -> Vec<String> {
        self.inner
            .snapshots
            .snapshot(seed_hash)
            .monitored_receive_addresses
            .clone()
    }

    /// Whether a (post-first-sync) snapshot has been published for the
    /// wallet. `false` ⇒ render the "syncing" state, not a zero balance.
    pub fn has_snapshot(&self, seed_hash: &WalletSeedHash) -> bool {
        self.inner.snapshots.has_snapshot(seed_hash)
    }

    /// Whether the wallet is registered with the upstream manager (its
    /// addresses are being watched by the `SpvRuntime`). This is the
    /// pre-sync registration signal — distinct from [`Self::has_snapshot`],
    /// which only flips after the first wallet event arrives.
    pub fn is_wallet_registered(&self, seed_hash: &WalletSeedHash) -> bool {
        self.inner
            .id_map
            .read()
            .map(|m| m.contains_key(seed_hash))
            .unwrap_or(false)
    }

    /// List the wallet's tracked asset locks (built, broadcast,
    /// instant-locked, chain-locked, or consumed). The upstream
    /// `AssetLockManager` is the single source of truth — the DET-side
    /// `Wallet.unused_asset_locks` mirror was removed.
    ///
    /// Async only: UI screens fetch this off the frame loop via
    /// [`WalletTask::ListTrackedAssetLocks`](crate::backend_task::wallet::WalletTask::ListTrackedAssetLocks).
    pub async fn list_tracked_asset_locks(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<Vec<platform_wallet::wallet::asset_lock::tracked::TrackedAssetLock>, TaskError>
    {
        let wallet = self.resolve_wallet(seed_hash).await?;
        Ok(wallet.asset_locks().list_tracked_locks().await)
    }

    /// Register (or update) an identity's watched-token list with the upstream
    /// `IdentitySyncManager` so its background loop fetches their balances.
    ///
    /// Idempotent and ordering-stable: a not-yet-registered identity is added
    /// fresh; an already-registered one has its watch list replaced via
    /// `update_watched_tokens`, which preserves the identity's
    /// `last_sync_unix` and per-token balances rather than resetting them to
    /// "syncing". Pass the full set of tokens DET tracks for the identity;
    /// passing an empty set clears the watch list.
    pub async fn register_identity_tokens(
        &self,
        identity_id: dash_sdk::platform::Identifier,
        token_ids: Vec<dash_sdk::platform::Identifier>,
    ) {
        let identity_sync = self.inner.pwm.identity_sync();
        if identity_sync
            .state_for_identity(&identity_id)
            .await
            .is_some()
        {
            identity_sync
                .update_watched_tokens(identity_id, token_ids)
                .await;
        } else {
            identity_sync
                .register_identity(identity_id, token_ids)
                .await;
        }
    }

    /// Stop the upstream `IdentitySyncManager` from watching a single
    /// `(identity, token)` pair so its background loop no longer fetches that
    /// token's balance and the pair drops out of the next published snapshot.
    ///
    /// Reads the identity's current watched-token set, removes `token_id`, and
    /// replaces the set via `update_watched_tokens` — which preserves the
    /// remaining tokens' cached balances. A no-op if the identity isn't
    /// registered or wasn't watching the token. Upstream `Identifier` types
    /// stay inside this seam; callers pass and receive DET-side identifiers.
    pub async fn unwatch_identity_token(
        &self,
        identity_id: dash_sdk::platform::Identifier,
        token_id: dash_sdk::platform::Identifier,
    ) {
        let identity_sync = self.inner.pwm.identity_sync();
        let Some(state) = identity_sync.state_for_identity(&identity_id).await else {
            return;
        };
        let remaining: Vec<dash_sdk::platform::Identifier> = state
            .tokens
            .iter()
            .map(|info| info.token_id)
            .filter(|t| *t != token_id)
            .collect();
        if remaining.len() == state.tokens.len() {
            return;
        }
        identity_sync
            .update_watched_tokens(identity_id, remaining)
            .await;
        self.refresh_token_balances().await;
    }

    /// Force one immediate upstream token-balance sync pass, then republish
    /// DET's snapshot. Reports when upstream skipped the request because
    /// another pass already owns its single-flight slot.
    pub(crate) async fn sync_token_balances_now(&self) -> TokenBalanceSyncOutcome {
        let identity_sync = self.inner.pwm.identity_sync();
        let outcome = run_token_balance_sync_if_idle(
            || identity_sync.is_syncing(),
            || identity_sync.sync_now(),
        )
        .await;
        if outcome == TokenBalanceSyncOutcome::Performed {
            self.refresh_token_balances().await;
        }
        outcome
    }

    /// Republish the lock-free token-balance snapshot from the upstream
    /// `IdentitySyncManager`'s current state. Async — call from a backend
    /// task, never the egui frame; the frame reads the published snapshot via
    /// [`Self::token_balances`].
    ///
    /// Only identities that have completed at least one sync pass
    /// (`last_sync_unix != 0`) contribute rows; an unsynced identity is
    /// omitted so the UI renders it as "syncing", not a zero balance. Upstream
    /// `IdentityTokenSyncState` / `IdentityTokenSyncInfo` / `TokenAmount` are
    /// converted to DET types here so they never cross the seam.
    pub async fn refresh_token_balances(&self) {
        let all_state = self.inner.pwm.identity_sync().all_state().await;
        let synced = all_state.into_values().filter_map(|state| {
            if state.last_sync_unix == 0 {
                return None;
            }
            let balances = state
                .tokens
                .into_iter()
                .map(|info| (info.token_id, info.balance))
                .collect::<std::collections::BTreeMap<_, _>>();
            Some((state.identity_id, balances))
        });
        self.inner
            .token_balances
            .publish(TokenBalanceStore::snapshot_from(synced));
    }

    /// The [`SecretScope`] that addresses the HD seed for `seed_hash`.
    fn hd_scope(seed_hash: &WalletSeedHash) -> SecretScope {
        SecretScope::HdSeed {
            seed_hash: *seed_hash,
        }
    }

    fn build_client_config(&self) -> ClientConfig {
        // Scan from genesis so historical wallet transactions are found via
        // compact block filters.
        let mut config = ClientConfig::new(self.inner.network)
            .with_storage_path(self.inner.spv_storage_dir.clone())
            .with_validation_mode(ValidationMode::Full)
            .with_start_height(0)
            .with_mempool_tracking(MempoolStrategy::BloomFilter);
        if let Some(peer) = self.inner.peer {
            config.add_peer(peer);
        }
        config
    }

    /// Resolve an explicit SPV peer for local networks. Devnet/Regtest have
    /// no DNS seeds, so a `core_host` peer is required there; Mainnet/Testnet
    /// fall back to DNS-seed discovery (`None`).
    fn spv_primary_peer_socket(
        ctx: &Arc<AppContext>,
        network: Network,
    ) -> Option<std::net::SocketAddr> {
        use std::net::ToSocketAddrs;
        /// Default Core P2P port on Devnet.
        const DEVNET_P2P_PORT: u16 = 20001;
        /// Default Core P2P port on Regtest.
        const REGTEST_P2P_PORT: u16 = 19899;

        let port = match network {
            Network::Devnet => DEVNET_P2P_PORT,
            Network::Regtest => REGTEST_P2P_PORT,
            _ => return None,
        };
        let cfg = ctx.config.read().ok()?;
        let host = cfg.core_host.as_deref()?;
        format!("{host}:{port}").to_socket_addrs().ok()?.next()
    }

    fn resolve_spv_storage_dir(
        app_data_dir: &Path,
        network: Network,
    ) -> Result<std::path::PathBuf, TaskError> {
        let mut dir = app_data_dir.to_path_buf();
        dir.push("spv");
        dir.push(kv::network_prefix(network));
        std::fs::create_dir_all(&dir).map_err(|source| TaskError::FileSystem { source })?;
        Ok(dir)
    }
}

/// The BIP44 account-0 extended public key among `accounts`, or `None` when
/// there is no BIP44 account-0.
///
/// The single definition of the fund-routing gate's account predicate: DET
/// resolves a watch-only wallet to its seed by matching this exact account
/// xpub, so the predicate must not drift between the seedless-load path and the
/// pre-registration probe.
fn bip44_account0_xpub<'a>(
    accounts: impl IntoIterator<Item = &'a dash_sdk::dpp::key_wallet::Account>,
) -> Option<dash_sdk::dpp::key_wallet::bip32::ExtendedPubKey> {
    use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
    accounts
        .into_iter()
        .find(|a| {
            matches!(
                a.account_type,
                AccountType::Standard {
                    index: 0,
                    standard_account_type: StandardAccountType::BIP44Account,
                }
            )
        })
        .map(|a| a.account_xpub)
}

/// Map a [`PlatformWalletError`] from any shielded operation to the correct
/// [`TaskError`] variant.
///
/// **Exhaustive — no `_` arm** on the outer match so a future upstream variant
/// addition forces a review here instead of silently falling through to
/// [`TaskError::WalletBackend`].
///
/// [`ShieldedSpendUnconfirmed`](platform_wallet::error::PlatformWalletError::ShieldedSpendUnconfirmed)
/// is pre-flighted before the exhaustive match because `operation` is
/// `&'static str` (not an enum): we copy it out without consuming `e`, then
/// route to the correct per-op `*ConfirmationUnknown` variant.  Unknown
/// `operation` values (future upstream ops) fall through to `WalletBackend`.
fn map_shielded_op_error(e: platform_wallet::error::PlatformWalletError) -> TaskError {
    use platform_wallet::error::PlatformWalletError as P;

    // Pre-flight: route ShieldedSpendUnconfirmed by operation name.
    // `operation` is `&'static str` (Copy) — extract without consuming `e`.
    let maybe_op = match &e {
        P::ShieldedSpendUnconfirmed { operation, .. } => Some(*operation),
        _ => None,
    };
    if let Some(op) = maybe_op {
        return match op {
            "shield" => TaskError::ShieldCreditsConfirmationUnknown {
                source: Box::new(e),
            },
            "transfer" => TaskError::ShieldedTransferConfirmationUnknown {
                source: Box::new(e),
            },
            "unshield" => TaskError::UnshieldConfirmationUnknown {
                source: Box::new(e),
            },
            "withdraw" => TaskError::ShieldedWithdrawalConfirmationUnknown {
                source: Box::new(e),
            },
            // Future operation name from a newer upstream — fall through to
            // WalletBackend rather than silently discarding the error.
            _ => TaskError::WalletBackend {
                source: Arc::new(e),
            },
        };
    }

    // Exhaustive match — no `_` arm.
    match e {
        P::ShieldedNotBound => TaskError::ShieldedNotBound,

        // Asset-lock finality failures (IS deadline / IS-expired / CL fallback).
        other @ (P::FinalityTimeout(_)
        | P::AssetLockProofWait(_)
        | P::AssetLockExpired(_)
        | P::AssetLockNotChainLocked(_)) => TaskError::AssetLockFinalityTimeout {
            source: Box::new(other),
        },

        // Handled by the pre-flight above. Kept as a defensive fallthrough
        // rather than `unreachable!`: this is a funds-safety path, so if the
        // pre-flight ever stops covering a case it must degrade to the generic
        // wrapper, never panic mid-operation.
        other @ P::ShieldedSpendUnconfirmed { .. } => TaskError::WalletBackend {
            source: Arc::new(other),
        },

        // Every remaining variant → generic WalletBackend wrapper.
        //
        // TODO(platform-pr3968): `ShieldedShutdownIncomplete` doesn't exist on
        // `PlatformWalletError` at this rev; it belongs in this bucket once
        // platform re-adds it.
        other @ (P::WalletCreation(_)
        | P::PersisterLoad(_)
        | P::AddressNonceMismatch { .. }
        | P::WalletNotFound(_)
        | P::WalletAlreadyExists(_)
        | P::IdentityAlreadyExists(_)
        | P::IdentityNotFound(_)
        | P::NoPrimaryIdentity
        | P::InvalidIdentityData(_)
        | P::ContactRequestNotFound(_)
        | P::IdentityIndexNotSet(_)
        | P::DashpayReceivingAccountAlreadyExists { .. }
        | P::DashpayExternalAccountAlreadyExists { .. }
        | P::AssetLockTransaction(_)
        | P::TransactionBroadcast(_)
        | P::TransactionBroadcastUnconfirmed(_)
        | P::TransactionBuild(_)
        | P::NoSpendableInputs { .. }
        | P::Sdk(_)
        | P::AddressSync(_)
        | P::AddressOperation(_)
        | P::OnlyOutputAddressesFunded { .. }
        | P::OnlyDustInputs { .. }
        | P::ChangeBelowMinimumOutput { .. }
        | P::InputSumOverflow
        | P::AddressNotFound(_)
        | P::KeyDerivation(_)
        | P::Persistence(_)
        | P::SeedMismatch { .. }
        | P::WalletLocked
        | P::SpvAlreadyRunning
        | P::NoWalletsConfigured
        | P::SpvError(_)
        | P::TokenError(_)
        | P::ShieldedNoUnspentNotes
        | P::ShieldedInsufficientBalance { .. }
        | P::ShieldedBuildError(_)
        | P::ShieldedBroadcastFailed(_)
        | P::ShieldedBroadcastUnconfirmed { .. }
        | P::ShieldedNoRecordedAnchor(_)
        | P::ShieldedSyncFailed(_)
        | P::ShieldedTreeUpdateFailed(_)
        | P::ShieldedStoreError(_)
        | P::ShieldedMerkleWitnessUnavailable(_)
        | P::ShieldedKeyDerivation(_)) => TaskError::WalletBackend {
            source: Arc::new(other),
        },
    }
}

/// Classify a `PlatformWalletError` returned from
/// `register_identity_with_funding` into a typed `TaskError`. Network /
/// broadcast rejections become `IdentityCreateRejected`; asset-lock
/// finality failures become `AssetLockFinalityTimeout`; everything else
/// falls through to the generic `WalletBackend` wrapper. Structural match
/// — never parses error strings.
fn map_identity_register_error(e: platform_wallet::error::PlatformWalletError) -> TaskError {
    match identity_op_error_kind(&e) {
        IdentityOpErrorKind::Rejected => TaskError::IdentityCreateRejected {
            source: Box::new(e),
        },
        IdentityOpErrorKind::FinalityTimeout => TaskError::AssetLockFinalityTimeout {
            source: Box::new(e),
        },
        // Registration creates the identity, so it cannot legitimately raise a
        // "not managed" lookup error — fold into the generic envelope.
        IdentityOpErrorKind::NotManaged | IdentityOpErrorKind::Other => TaskError::WalletBackend {
            source: Arc::new(e),
        },
    }
}

/// Same as [`map_identity_register_error`] but for the top-up façade —
/// the `identity_id` is carried into the rejection variant so the user-
/// facing message can reference the affected identity.
fn map_identity_top_up_error(
    identity_id: dash_sdk::platform::Identifier,
    e: platform_wallet::error::PlatformWalletError,
) -> TaskError {
    match identity_op_error_kind(&e) {
        IdentityOpErrorKind::Rejected => TaskError::IdentityTopUpRejected {
            identity_id,
            source: Box::new(e),
        },
        IdentityOpErrorKind::FinalityTimeout => TaskError::AssetLockFinalityTimeout {
            source: Box::new(e),
        },
        IdentityOpErrorKind::NotManaged => TaskError::IdentityNotManaged {
            identity_id,
            source: Box::new(e),
        },
        IdentityOpErrorKind::Other => TaskError::WalletBackend {
            source: Arc::new(e),
        },
    }
}

/// Shape persisted platform-address state into per-wallet warm-start seed data.
///
/// Resolves each upstream wallet id to its DET [`WalletSeedHash`] via `resolve`
/// and derives the `(timestamp, height)` cursor — present only when a prior sync
/// completed (`sync_timestamp > 0`), so a never-synced wallet stays "never
/// synced". Wallets that resolve to no DET seed, or that carry neither owned
/// addresses nor a cursor, are dropped. Pure — no I/O, no upstream types — so it
/// is unit-testable without a persister.
fn platform_warm_start_seed(
    per_wallet: Vec<(WalletId, Vec<PlatformAddressEntry>, u64, u64)>,
    resolve: impl Fn(&WalletId) -> Option<WalletSeedHash>,
) -> PlatformWarmStartSeed {
    per_wallet
        .into_iter()
        .filter_map(|(wallet_id, entries, sync_timestamp, sync_height)| {
            let seed_hash = resolve(&wallet_id)?;
            let cursor = (sync_timestamp > 0).then_some((sync_timestamp, sync_height));
            (!entries.is_empty() || cursor.is_some()).then_some((seed_hash, entries, cursor))
        })
        .collect()
}

/// Map an orchestrated platform-address funding error to a typed `TaskError`.
/// Shares the identity-flow bucketing: an asset-lock finality timeout reuses
/// [`TaskError::AssetLockFinalityTimeout`], a network/broadcast rejection lands
/// in [`TaskError::PlatformAddressFundRejected`], and everything else falls
/// through to the generic [`TaskError::WalletBackend`] envelope.
fn map_platform_address_fund_error(e: platform_wallet::error::PlatformWalletError) -> TaskError {
    match identity_op_error_kind(&e) {
        IdentityOpErrorKind::Rejected => TaskError::PlatformAddressFundRejected {
            source: Box::new(e),
        },
        IdentityOpErrorKind::FinalityTimeout => TaskError::AssetLockFinalityTimeout {
            source: Box::new(e),
        },
        // Platform-address funding does not consult the identity manager, so a
        // "not managed" classification is not meaningful here — fold into the
        // generic envelope alongside the other preconditions.
        IdentityOpErrorKind::NotManaged | IdentityOpErrorKind::Other => TaskError::WalletBackend {
            source: Arc::new(e),
        },
    }
}

/// Bucket for `PlatformWalletError`s coming out of identity register / top-up.
enum IdentityOpErrorKind {
    /// Network or broadcast rejected the submission (SDK error or asset-lock
    /// transaction broadcast failure).
    Rejected,
    /// Asset-lock proof finalization (IS → CL fallback) failed to produce a
    /// usable proof — IS deadline elapsed, IS expired with no CL fallback, or
    /// the wait helper itself failed.
    FinalityTimeout,
    /// The identity is not registered in the wallet's active set, so a lookup
    /// op (top-up) cannot find it — retrying the same op cannot help; the
    /// identity must be reloaded.
    NotManaged,
    /// Anything else — preconditions, wallet state, builder failures.
    Other,
}

/// Map `PlatformWalletError` variants to coarse buckets. Exhaustive on the
/// upstream enum — no `_` arm — so a future variant addition forces a
/// review here instead of silently falling through.
fn identity_op_error_kind(e: &platform_wallet::error::PlatformWalletError) -> IdentityOpErrorKind {
    use platform_wallet::error::PlatformWalletError as P;
    match e {
        // Network / broadcast rejections.
        P::Sdk(_) | P::TransactionBroadcast(_) => IdentityOpErrorKind::Rejected,

        // Asset-lock finality failures (IS deadline / IS-expired / CL fallback).
        P::FinalityTimeout(_)
        | P::AssetLockProofWait(_)
        | P::AssetLockExpired(_)
        | P::AssetLockNotChainLocked(_) => IdentityOpErrorKind::FinalityTimeout,

        // The identity is absent from the wallet's active set — a missing
        // manager registration, not a transient fault.
        P::IdentityNotFound(_) | P::IdentityIndexNotSet(_) => IdentityOpErrorKind::NotManaged,

        // Everything else — preconditions, wallet state, builder errors.
        P::WalletCreation(_)
        | P::WalletNotFound(_)
        | P::WalletAlreadyExists(_)
        | P::IdentityAlreadyExists(_)
        | P::NoPrimaryIdentity
        | P::InvalidIdentityData(_)
        | P::ContactRequestNotFound(_)
        | P::DashpayReceivingAccountAlreadyExists { .. }
        | P::DashpayExternalAccountAlreadyExists { .. }
        | P::AssetLockTransaction(_)
        | P::TransactionBuild(_)
        | P::NoSpendableInputs { .. }
        | P::AddressSync(_)
        | P::AddressOperation(_)
        | P::OnlyOutputAddressesFunded { .. }
        | P::OnlyDustInputs { .. }
        | P::ChangeBelowMinimumOutput { .. }
        | P::InputSumOverflow
        | P::AddressNotFound(_)
        | P::KeyDerivation(_)
        | P::WalletLocked
        | P::SpvAlreadyRunning
        | P::NoWalletsConfigured
        | P::SpvError(_)
        | P::TokenError(_)
        | P::ShieldedNoUnspentNotes
        | P::ShieldedInsufficientBalance { .. }
        | P::ShieldedBuildError(_)
        | P::ShieldedBroadcastFailed(_)
        | P::ShieldedSyncFailed(_)
        | P::ShieldedTreeUpdateFailed(_)
        | P::ShieldedStoreError(_)
        | P::ShieldedMerkleWitnessUnavailable(_)
        | P::ShieldedKeyDerivation(_)
        | P::ShieldedNoRecordedAnchor(_)
        | P::ShieldedNotBound
        | P::PersisterLoad(_)
        | P::Persistence(_)
        | P::SeedMismatch { .. }
        // Address nonce desync is a precondition/state fault unrelated to
        // identity registration; bucket as Other.
        | P::AddressNonceMismatch { .. }
        // Broadcast was accepted but its execution result is unconfirmed — the
        // op may already be on chain, so it is neither a rejection nor a
        // finality timeout. Bucket as Other; the upstream contract says the
        // caller must not re-submit (the next sync reconciles).
        | P::TransactionBroadcastUnconfirmed(_)
        | P::ShieldedBroadcastUnconfirmed { .. }
        | P::ShieldedSpendUnconfirmed { .. } => IdentityOpErrorKind::Other,
        // TODO(platform-pr3968): `ShieldedShutdownIncomplete` doesn't exist on
        // `PlatformWalletError` at this rev; it belongs in the `Other` bucket
        // once platform re-adds it.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_balance_sync_reports_an_already_running_upstream_pass() {
        let sync_called = std::cell::Cell::new(false);

        let outcome = run_token_balance_sync_if_idle(
            || true,
            || async {
                sync_called.set(true);
            },
        )
        .await;

        assert_eq!(outcome, TokenBalanceSyncOutcome::AlreadyInFlight);
        assert!(!sync_called.get());
    }

    /// A completed start flight remains current, so repeated calls reuse its
    /// result instead of spawning another SPV run loop.
    #[test]
    fn start_latch_reuses_completed_flight() {
        let latch = StartLatch::default();
        assert!(!latch.is_started(), "fresh latch must not be started");

        let first = latch.flight();
        first.begun.store(true, Ordering::SeqCst);
        first.outcome.set(Ok(())).expect("set flight outcome");

        let repeated = latch.flight();
        assert!(Arc::ptr_eq(&first, &repeated));
        assert!(matches!(repeated.outcome.get(), Some(Ok(()))));
        assert!(latch.is_started());
    }

    /// Restart-in-place replaces the completed flight, and a stale failed
    /// flight cannot reset the replacement.
    #[test]
    fn start_latch_reset_allows_restart() {
        let latch = StartLatch::default();
        let first = latch.flight();
        first.begun.store(true, Ordering::SeqCst);
        assert!(latch.is_started());

        latch.reset();
        let replacement = latch.flight();
        assert!(!Arc::ptr_eq(&first, &replacement));
        assert!(!latch.is_started(), "reset must clear the latch");

        latch.reset_if_current(&first);
        assert!(
            Arc::ptr_eq(&replacement, &latch.flight()),
            "a stale flight must not replace the current flight"
        );
    }

    /// A caller that captured a flight before reset cannot initialize that
    /// stale generation after the replacement becomes current.
    #[tokio::test]
    async fn start_latch_stale_flight_cannot_initialize_after_reset() {
        let latch = StartLatch::default();
        let stale = latch.flight();

        latch.reset();

        assert!(latch.claim(&stale).await.is_none());
        let current = latch.flight();
        assert!(latch.claim(&current).await.is_some());
    }

    /// Concurrent callers share one `OnceCell` initializer, preserving the
    /// single-spawn guarantee while every caller receives the stored outcome.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn start_latch_single_winner_under_contention() {
        use std::sync::atomic::AtomicUsize;

        let latch = Arc::new(StartLatch::default());
        let winners = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(tokio::sync::Barrier::new(16));

        let handles: Vec<_> = (0..16)
            .map(|_| {
                let latch = Arc::clone(&latch);
                let winners = Arc::clone(&winners);
                let barrier = Arc::clone(&barrier);
                tokio::spawn(async move {
                    barrier.wait().await;
                    let flight = latch.flight();
                    let outcome = flight
                        .outcome
                        .get_or_init(|| async {
                            flight.begun.store(true, Ordering::SeqCst);
                            winners.fetch_add(1, Ordering::SeqCst);
                            Ok(())
                        })
                        .await;
                    assert!(outcome.is_ok());
                })
            })
            .collect();
        for h in handles {
            h.await.expect("task panicked");
        }

        assert_eq!(
            winners.load(Ordering::SeqCst),
            1,
            "exactly one caller may win the start latch"
        );
        assert!(latch.is_started());
    }

    /// I2: a network/broadcast rejection from `register_identity_with_funding`
    /// maps to the dedicated `IdentityCreateRejected` envelope (not the generic
    /// `WalletBackend` fallback). Structural — no string parsing.
    #[test]
    fn map_identity_register_error_classifies_rejection() {
        let inner = platform_wallet::error::PlatformWalletError::TransactionBroadcast(
            "rejected".to_string(),
        );
        let mapped = map_identity_register_error(inner);
        assert!(
            matches!(mapped, TaskError::IdentityCreateRejected { .. }),
            "Expected IdentityCreateRejected, got: {mapped:?}"
        );
    }

    /// I3: an asset-lock finality failure surfaced during identity register
    /// maps to `AssetLockFinalityTimeout`, regardless of which finality
    /// sub-variant fired upstream.
    #[test]
    fn map_identity_register_error_classifies_finality_timeout() {
        use dash_sdk::dpp::dashcore::hashes::Hash;
        let outpoint = dash_sdk::dpp::dashcore::OutPoint::new(
            dash_sdk::dpp::dashcore::Txid::from_byte_array([0u8; 32]),
            0,
        );
        let inner = platform_wallet::error::PlatformWalletError::FinalityTimeout(outpoint);
        let mapped = map_identity_register_error(inner);
        assert!(
            matches!(mapped, TaskError::AssetLockFinalityTimeout { .. }),
            "Expected AssetLockFinalityTimeout, got: {mapped:?}"
        );
    }

    /// I4: precondition / wallet-state failures fall through to the generic
    /// `WalletBackend` envelope — they are neither rejections nor finality
    /// timeouts.
    #[test]
    fn map_identity_register_error_falls_through_for_other() {
        let inner = platform_wallet::error::PlatformWalletError::WalletLocked;
        let mapped = map_identity_register_error(inner);
        assert!(
            matches!(mapped, TaskError::WalletBackend { .. }),
            "Expected WalletBackend fallthrough, got: {mapped:?}"
        );
    }

    /// I5: the top-up façade carries the identity_id into the rejection
    /// variant so the user-facing message references the affected identity.
    #[test]
    fn map_identity_top_up_error_carries_identity_id() {
        let identity_id = dash_sdk::platform::Identifier::random();
        let inner = platform_wallet::error::PlatformWalletError::TransactionBroadcast(
            "rejected".to_string(),
        );
        let mapped = map_identity_top_up_error(identity_id, inner);
        match mapped {
            TaskError::IdentityTopUpRejected {
                identity_id: got, ..
            } => assert_eq!(got, identity_id, "identity_id must be preserved"),
            other => panic!("Expected IdentityTopUpRejected, got: {other:?}"),
        }
    }

    /// A top-up against an identity the wallet has not registered
    /// (`IdentityNotFound` / `IdentityIndexNotSet`) maps to the dedicated
    /// `IdentityNotManaged` envelope — not the "retry in a moment" fallback —
    /// carrying the id, and its message names the id and tells the user to
    /// reload the identity.
    #[test]
    fn map_identity_top_up_error_not_managed_is_actionable() {
        use platform_wallet::error::PlatformWalletError as P;
        for inner in [
            P::IdentityNotFound(dash_sdk::platform::Identifier::random()),
            P::IdentityIndexNotSet(dash_sdk::platform::Identifier::random()),
        ] {
            let identity_id = dash_sdk::platform::Identifier::random();
            let mapped = map_identity_top_up_error(identity_id, inner);
            let TaskError::IdentityNotManaged {
                identity_id: got, ..
            } = &mapped
            else {
                panic!("Expected IdentityNotManaged, got: {mapped:?}");
            };
            assert_eq!(*got, identity_id, "identity_id must be preserved");
            let msg = mapped.to_string();
            assert!(
                msg.contains(&format!("{identity_id}")),
                "message must name the identity id: {msg}"
            );
            let lower = msg.to_lowercase();
            assert!(
                lower.contains("reload"),
                "message must tell the user to reload the identity: {msg}"
            );
            assert!(
                !lower.contains("retry in a moment"),
                "must not be the transient-retry fallback: {msg}"
            );
        }
    }

    /// Registration creates the identity, so a `not-managed` classification on
    /// the register path folds into the generic `WalletBackend` envelope rather
    /// than the top-up-only `IdentityNotManaged` variant.
    #[test]
    fn map_identity_register_error_not_managed_folds_to_generic() {
        let inner = platform_wallet::error::PlatformWalletError::IdentityNotFound(
            dash_sdk::platform::Identifier::random(),
        );
        assert!(
            matches!(
                map_identity_register_error(inner),
                TaskError::WalletBackend { .. }
            ),
            "register path must not produce IdentityNotManaged"
        );
    }

    /// Cold-boot warm-start shaping: a funded wallet seeds its addresses + a
    /// cursor; a successfully-synced-but-empty wallet seeds the cursor alone (so
    /// the label reads "synced", not "never synced"); a wallet that never
    /// completed a sync (`sync_timestamp == 0`, no addresses) is dropped; and an
    /// unresolvable wallet id is dropped.
    #[test]
    fn platform_warm_start_seed_shapes_entries_and_gates_cursor() {
        let funded: WalletId = [1u8; 32];
        let synced_empty: WalletId = [2u8; 32];
        let never_synced: WalletId = [3u8; 32];
        let unresolved: WalletId = [4u8; 32];

        let entry = |hash: [u8; 20], balance: u64, nonce: u32| PlatformAddressEntry {
            hash,
            balance,
            nonce,
            account: 0,
            index: Some(0),
        };
        let per_wallet: Vec<(WalletId, Vec<PlatformAddressEntry>, u64, u64)> = vec![
            (funded, vec![entry([0xAAu8; 20], 500, 7)], 1_700, 900),
            (synced_empty, vec![], 1_700, 900),
            (never_synced, vec![], 0, 0),
            (unresolved, vec![entry([0xBBu8; 20], 1, 1)], 1_700, 900),
        ];

        // Identity resolver, except `unresolved` maps to no DET seed hash.
        let out: std::collections::BTreeMap<_, _> =
            platform_warm_start_seed(per_wallet, |w| (*w != unresolved).then_some(*w))
                .into_iter()
                .map(|(seed_hash, entries, cursor)| (seed_hash, (entries, cursor)))
                .collect();

        assert_eq!(
            out.len(),
            2,
            "never-synced and unresolvable wallets are dropped"
        );

        let funded_out = out.get(&funded).expect("funded wallet seeds");
        assert_eq!(funded_out.0, vec![entry([0xAAu8; 20], 500, 7)]);
        assert_eq!(funded_out.1, Some((1_700, 900)));

        let empty_out = out
            .get(&synced_empty)
            .expect("synced-empty wallet seeds a cursor");
        assert!(empty_out.0.is_empty(), "no addresses to seed");
        assert_eq!(
            empty_out.1,
            Some((1_700, 900)),
            "a completed sync warm-starts the cursor even with no funds"
        );

        assert!(
            !out.contains_key(&never_synced),
            "no timestamp and no addresses leaves the wallet never-synced"
        );
        assert!(
            !out.contains_key(&unresolved),
            "an unresolvable wallet id is dropped"
        );
    }

    /// A network/broadcast rejection from the orchestrated platform-address
    /// funding maps to the dedicated `PlatformAddressFundRejected` envelope
    /// (not the generic `WalletBackend` fallback). Structural — no string
    /// parsing.
    #[test]
    fn map_platform_address_fund_error_classifies_rejection() {
        let inner = platform_wallet::error::PlatformWalletError::TransactionBroadcast(
            "rejected".to_string(),
        );
        let mapped = map_platform_address_fund_error(inner);
        assert!(
            matches!(mapped, TaskError::PlatformAddressFundRejected { .. }),
            "Expected PlatformAddressFundRejected, got: {mapped:?}"
        );
    }

    /// An asset-lock finality failure surfaced during orchestrated platform
    /// funding reuses the shared `AssetLockFinalityTimeout` envelope.
    #[test]
    fn map_platform_address_fund_error_classifies_finality_timeout() {
        use dash_sdk::dpp::dashcore::hashes::Hash;
        let outpoint = dash_sdk::dpp::dashcore::OutPoint::new(
            dash_sdk::dpp::dashcore::Txid::from_byte_array([0u8; 32]),
            0,
        );
        let inner = platform_wallet::error::PlatformWalletError::FinalityTimeout(outpoint);
        let mapped = map_platform_address_fund_error(inner);
        assert!(
            matches!(mapped, TaskError::AssetLockFinalityTimeout { .. }),
            "Expected AssetLockFinalityTimeout, got: {mapped:?}"
        );
    }

    /// Precondition / wallet-state failures fall through to the generic
    /// `WalletBackend` envelope.
    #[test]
    fn map_platform_address_fund_error_falls_through_for_other() {
        let inner = platform_wallet::error::PlatformWalletError::WalletLocked;
        let mapped = map_platform_address_fund_error(inner);
        assert!(
            matches!(mapped, TaskError::WalletBackend { .. }),
            "Expected WalletBackend fallthrough, got: {mapped:?}"
        );
    }

    /// The orchestrator-vs-manual gate is decided by upstream's own pool
    /// membership: `WalletBackend::platform_address_in_pool` converts a
    /// `PlatformAddress` to the upstream `PlatformP2PKHAddress` and asks
    /// `ManagedPlatformAccount::contains_platform_address`. This pins that exact
    /// semantic offline — building a real upstream `ManagedPlatformAccount` from
    /// a known platform-payment account xpub and asserting an address inside the
    /// pre-generated pool is recognised while a foreign one is not. (The full
    /// helper needs a resolved `PlatformWallet`, which isn't constructible
    /// offline; this covers the membership logic the helper delegates to.)
    #[test]
    fn upstream_pool_membership_distinguishes_in_pool_from_foreign() {
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
        use dash_sdk::dpp::key_wallet::managed_account::address_pool::AddressPoolType;
        use dash_sdk::dpp::key_wallet::{
            AddressPool, KeySource, ManagedPlatformAccount, PlatformP2PKHAddress,
        };

        let network = Network::Testnet;
        let seed = [7u8; 64];

        // DIP-17 platform-payment account path: m/9'/coin'/17'/0'/0' (coin 1' on
        // testnet). The pool appends the non-hardened leaf, matching DET's
        // `platform_payment_path`.
        let account_path = DerivationPath::from(vec![
            ChildNumber::Hardened { index: 9 },
            ChildNumber::Hardened { index: 1 },
            ChildNumber::Hardened { index: 17 },
            ChildNumber::Hardened { index: 0 },
            ChildNumber::Hardened { index: 0 },
        ]);
        let account_xpub = account_path
            .derive_pub_ecdsa_for_master_seed(&seed, network)
            .expect("derive account xpub");

        let pool = AddressPool::new(
            account_path,
            AddressPoolType::Absent,
            20,
            network,
            &KeySource::Public(account_xpub),
        )
        .expect("build pool");
        let account = ManagedPlatformAccount::new(0, 0, pool, false);

        // An address the pool actually generated is recognised as in-pool.
        let in_pool = *account
            .all_platform_addresses()
            .first()
            .expect("pre-generated pool is non-empty");
        assert!(
            account.contains_platform_address(&in_pool),
            "a pre-generated pool address must be recognised"
        );

        // A foreign address (not derived from this account) is not in the pool.
        let foreign = PlatformP2PKHAddress::new([0xAB; 20]);
        assert!(
            !account.contains_platform_address(&foreign),
            "a foreign address must not be recognised as in-pool"
        );
    }

    /// The seedless bridge keys off the BIP44 account xpub. The DET
    /// account xpub (`Wallet::new_from_seed`) must equal the upstream
    /// account xpub for the SAME seed, so the watch-only wallet rebuilt
    /// from the persisted manifest resolves back to DET's seed hash.
    /// Locks the cryptographic invariant the
    /// [`WalletBackend::load_from_persistor_seedless`] gate relies on.
    #[test]
    fn bridge_account_xpub_matches_upstream_for_same_seed() {
        use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;

        let seed = [0x42u8; 64];
        let network = Network::Testnet;

        // DET side: what DET persists as `xpub_encoded`.
        let det = crate::model::wallet::Wallet::new_from_seed(seed, network, None, None)
            .expect("DET wallet");
        let det_xpub = det.master_bip44_ecdsa_extended_public_key.encode().to_vec();

        // Upstream side: the watch-only manifest carries the same account
        // xpub on the BIP44 account.
        let up =
            UpstreamWallet::from_seed_bytes(seed, network, WalletAccountCreationOptions::Default)
                .expect("upstream wallet");
        let up_xpub = bip44_account0_xpub(up.accounts.all_accounts())
            .map(|x| x.encode().to_vec())
            .expect("upstream BIP44 account");

        assert_eq!(
            det_xpub, up_xpub,
            "DET and upstream must agree on the BIP44 account xpub for the same seed"
        );

        // A bridge built from DET's sidecar resolves the matching xpub to
        // the DET seed hash, and rejects a non-matching xpub.
        let seed_hash = det.seed_hash();
        let bridge: std::collections::HashMap<Vec<u8>, WalletSeedHash> =
            std::iter::once((det_xpub.clone(), seed_hash)).collect();
        assert_eq!(
            bridge.get(&up_xpub).copied(),
            Some(seed_hash),
            "matching account xpub must resolve to the DET seed hash"
        );

        let other = crate::model::wallet::Wallet::new_from_seed([0x99u8; 64], network, None, None)
            .expect("other wallet");
        let other_xpub = other
            .master_bip44_ecdsa_extended_public_key
            .encode()
            .to_vec();
        assert!(
            !bridge.contains_key(&other_xpub),
            "a non-matching account xpub must be rejected by the gate"
        );
    }

    /// `WalletId` is independent of the account-xpub depth: the same seed yields
    /// the same `WalletId` whether or not BIP44 accounts are created. An upstream
    /// `get_wallet(wallet_id)` hit therefore proves the entry shares this seed's
    /// root, so the fund-routing gate's account-xpub comparison is the only thing
    /// that can vary — the invariant the gate relies on.
    #[test]
    fn wallet_id_is_independent_of_account_creation() {
        use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;

        let seed = [0x42u8; 64];
        let network = Network::Testnet;

        let with_accounts =
            UpstreamWallet::from_seed_bytes(seed, network, WalletAccountCreationOptions::Default)
                .expect("wallet with accounts");
        let without_accounts =
            UpstreamWallet::from_seed_bytes(seed, network, WalletAccountCreationOptions::None)
                .expect("wallet without accounts");

        assert_eq!(
            with_accounts.wallet_id, without_accounts.wallet_id,
            "WalletId must not depend on which accounts were created"
        );
    }

    /// Issue #7 regression guard: on a FRESH wallet the fund-routing gate's two
    /// BIP44 account-0 xpubs must agree byte-for-byte. Two independent
    /// `from_seed_bytes(Default)` builds (the gate's expected vs just-created
    /// sides), DET's published `master_bip44_ecdsa_extended_public_key`, and a
    /// bincode persist round-trip (the watch-only reload path) must ALL encode
    /// identically — same depth/parent_fingerprint/child_number, not just
    /// pubkey+chaincode. If this ever diverges, the gate rejects fresh wallets
    /// and every wallet dead-ends in `WalletNotLoaded`. Empirically the layers
    /// agree (issue #7 is not a pure-derivation/persistence defect).
    #[test]
    fn fresh_bip44_account0_xpub_is_stable_across_gate_sides() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;

        let seed = [0x42u8; 64];
        let network = Network::Testnet;

        let find_bip44_0 = |w: &UpstreamWallet| {
            bip44_account0_xpub(w.accounts.all_accounts())
                .expect("a fresh Default wallet must contain a BIP44 account-0")
        };

        let a = find_bip44_0(
            &UpstreamWallet::from_seed_bytes(seed, network, WalletAccountCreationOptions::Default)
                .expect("wallet a"),
        );
        let b = find_bip44_0(
            &UpstreamWallet::from_seed_bytes(seed, network, WalletAccountCreationOptions::Default)
                .expect("wallet b"),
        );
        assert_eq!(
            a.encode(),
            b.encode(),
            "two fresh from_seed_bytes BIP44 account-0 xpubs must encode identically"
        );

        let det = crate::model::wallet::Wallet::new_from_seed(seed, network, None, None)
            .expect("DET wallet");
        assert_eq!(
            det.master_bip44_ecdsa_extended_public_key.encode(),
            a.encode(),
            "DET's published xpub must match the upstream-derived one (the bridge invariant)"
        );

        // Watch-only reload path: the persister stores the xpub as a bincode
        // blob and the loader reads it back. That round-trip must not change the
        // encoding, or a freshly-registered wallet would fail the gate on the
        // next boot.
        use platform_wallet::changeset::AccountRegistrationEntry;
        let entry = AccountRegistrationEntry {
            account_type: AccountType::Standard {
                index: 0,
                standard_account_type: StandardAccountType::BIP44Account,
            },
            account_xpub: a,
        };
        let cfg = bincode::config::standard();
        let bytes = bincode::serde::encode_to_vec(&entry, cfg).expect("encode entry");
        let (decoded, _): (AccountRegistrationEntry, usize) =
            bincode::serde::decode_from_slice(&bytes, cfg).expect("decode entry");
        assert_eq!(
            decoded.account_xpub.encode(),
            a.encode(),
            "bincode round-trip must preserve the account xpub encoding"
        );
    }

    /// `map_shielded_op_error` must route an ambiguous post-broadcast
    /// `ShieldedSpendUnconfirmed` to the per-operation `*ConfirmationUnknown`
    /// variant keyed off `operation`, so the UI surfaces the correct
    /// "do not re-submit" message and never falsely reports success. A wrong
    /// route here is a funds-safety bug (the user re-spends thinking it failed),
    /// so this guards the exact string keys upstream emits.
    #[test]
    fn map_shielded_op_error_routes_spend_unconfirmed_by_operation() {
        use platform_wallet::error::PlatformWalletError as P;
        let unconfirmed = |op: &'static str| P::ShieldedSpendUnconfirmed {
            operation: op,
            reason: "ambiguous broadcast".to_string(),
        };

        assert!(matches!(
            map_shielded_op_error(unconfirmed("shield")),
            TaskError::ShieldCreditsConfirmationUnknown { .. }
        ));
        assert!(matches!(
            map_shielded_op_error(unconfirmed("transfer")),
            TaskError::ShieldedTransferConfirmationUnknown { .. }
        ));
        assert!(matches!(
            map_shielded_op_error(unconfirmed("unshield")),
            TaskError::UnshieldConfirmationUnknown { .. }
        ));
        assert!(matches!(
            map_shielded_op_error(unconfirmed("withdraw")),
            TaskError::ShieldedWithdrawalConfirmationUnknown { .. }
        ));
        // An operation name from a newer upstream must fall through to the
        // generic wrapper rather than silently mis-routing.
        assert!(matches!(
            map_shielded_op_error(unconfirmed("future-op")),
            TaskError::WalletBackend { .. }
        ));
    }

    /// The not-bound / not-configured shielded preconditions map to their
    /// dedicated typed variants (not the generic `WalletBackend` wrapper) so
    /// callers can react and the UI can guide the user to unlock / restart.
    #[test]
    fn map_shielded_op_error_maps_bind_preconditions() {
        use platform_wallet::error::PlatformWalletError as P;
        assert!(matches!(
            map_shielded_op_error(P::ShieldedNotBound),
            TaskError::ShieldedNotBound
        ));
    }
}
