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

mod asset_lock_signer;
mod dashpay;
mod event_bridge;
#[cfg(any(test, feature = "bench"))]
pub mod hydration;
#[cfg(not(any(test, feature = "bench")))]
pub(crate) mod hydration;
mod kv;
mod loader;
mod platform_address;
mod shielded;
#[cfg(any(test, feature = "bench"))]
pub mod single_key;
#[cfg(not(any(test, feature = "bench")))]
pub(crate) mod single_key;
pub mod single_key_entry;
mod snapshot;
mod token_balance;
#[cfg(any(test, feature = "bench"))]
pub mod wallet_meta;
#[cfg(not(any(test, feature = "bench")))]
pub(crate) mod wallet_meta;
#[cfg(any(test, feature = "bench"))]
pub mod wallet_seed_store;
#[cfg(not(any(test, feature = "bench")))]
pub(crate) mod wallet_seed_store;

pub use dashpay::DashpayView;
pub use shielded::{InsertShieldedNote, SHIELDED_SIDECAR_FILE, ShieldedNoteRow, ShieldedView};

pub use asset_lock_signer::AssetLockSignerError;
use asset_lock_signer::WalletAssetLockSigner;

pub use event_bridge::EventBridge;
pub use kv::{
    DetKv, DetScope, KvAdapterError, ObjectKindLite, SCHEMA_VERSION as KV_SCHEMA_VERSION,
};
pub use loader::{PersistedWalletLoader, SeedReregistrationLoader, WalletRegistration};
pub use platform_address::{
    KvCachedPlatformAddresses, PlatformAddressView, UpstreamPlatformAddresses,
};
pub use single_key::SingleKeyView;
use snapshot::SnapshotStore;
pub use snapshot::{DetUtxo, DetWalletBalance, WalletSnapshot};
pub use token_balance::{KvCachedTokenBalances, TokenBalanceView, UpstreamTokenBalances};
pub use wallet_meta::WalletMetaView;
pub use wallet_seed_store::WalletSeedView;

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dash_sdk::Sdk;
use dash_sdk::dash_spv::ClientConfig;
use dash_sdk::dash_spv::client::config::MempoolStrategy;
use dash_sdk::dash_spv::types::ValidationMode;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
use platform_wallet::manager::PlatformWalletManager;
use platform_wallet_storage::secrets::SecretStore;
use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig};

use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::connection_status::ConnectionStatus;
use crate::model::selected_wallet::SelectedWallet;
use crate::model::wallet::WalletSeedHash;
use crate::utils::egui_mpsc::SenderAsync;

/// The upstream persister DET consumes. Authored upstream (PR #3625) — DET
/// does not write its own persister (removal-inventory: consume, don't
/// reimplement).
type DetPersister = SqlitePersister;

/// One-shot latch guarding chain-sync startup. The upstream
/// `SpvRuntime::spawn_in_background` unconditionally spawns a fresh run loop
/// per call, so [`WalletBackend::start`] uses this to spawn exactly once even
/// when invoked repeatedly (Connect clicked twice, eager-init plus a manual
/// click).
#[derive(Debug, Default)]
struct StartLatch(AtomicBool);

impl StartLatch {
    /// Returns `true` exactly once — on the first call. Every later call
    /// returns `false`. Atomic swap, so concurrent callers race to a single
    /// winner.
    fn try_begin(&self) -> bool {
        !self.0.swap(true, Ordering::SeqCst)
    }

    /// Whether the latch has been triggered.
    fn is_started(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Default BIP-44 account index for wallet receive/send operations. DET has
/// always operated account 0; multi-account support is out of P2 scope.
const DEFAULT_BIP44_ACCOUNT: u32 = 0;

/// Upstream `WalletId` = `SHA256(root_xpub || root_chain_code)`, distinct
/// from DET's `WalletSeedHash` = `SHA256(seed_bytes)`. The map is the bridge:
/// populated once per wallet at registration, read by every DET-keyed call.
type WalletId = [u8; 32];

struct Inner {
    pwm: PlatformWalletManager<DetPersister>,
    /// Shared handle to the same persister `pwm` consumes. Kept so the
    /// typed key/value adapter ([`DetKv`]) can read/write app data
    /// alongside wallet state without opening a second connection.
    persister: Arc<DetPersister>,
    loader: Arc<dyn PersistedWalletLoader>,
    /// Display-only snapshot store (balance/tx/utxo), pushed by the
    /// `EventBridge`. See [`snapshot`]. DISPLAY-ONLY — never feeds coin
    /// selection (A04 fund-safety gate).
    snapshots: Arc<SnapshotStore>,
    /// `WalletSeedHash` → upstream `WalletId`. See [`WalletId`].
    id_map: std::sync::RwLock<std::collections::BTreeMap<WalletSeedHash, WalletId>>,
    /// Sync-accessible cache of `Arc<PlatformWallet>` keyed by `WalletId`,
    /// populated at registration. Lets synchronous UI code (egui frame)
    /// reach the upstream wallet without an async hop or a tokio
    /// `block_on`. Tracked-asset-lock pickers use this path —
    /// see [`Self::list_tracked_asset_locks_blocking`].
    wallets: std::sync::RwLock<
        std::collections::BTreeMap<WalletId, Arc<platform_wallet::PlatformWallet>>,
    >,
    /// `WalletSeedHash` → BIP-39 seed snapshot. Stored once at registration so
    /// the upstream signer-driven asset-lock / payment builders can derive
    /// secp256k1 keys without re-reading DET's wallet store on every call.
    /// Zeroized on drop with the backend.
    seeds:
        std::sync::RwLock<std::collections::BTreeMap<WalletSeedHash, zeroize::Zeroizing<[u8; 64]>>>,
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
    /// BIP-39 seeds (`seed.v1` labels under `WalletId(seed_hash)`, see
    /// [`wallet_seed_store`]). [`Self::seeds`] caches plaintext seeds
    /// for the duration of the process so signers don't re-open the
    /// vault on every call.
    secret_store: Arc<SecretStore>,
    /// Per-network shielded-notes sidecar. Lazy-materialised on first
    /// write at `<spv_storage_dir>/det-shielded.sqlite`. See
    /// [`shielded`] (T-SH-01).
    shielded: ShieldedView,
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
    /// In-memory cache of decrypted single-key bytes for the duration
    /// of the process. Populated by
    /// [`SingleKeyView::unlock_with_passphrase`] and consulted by
    /// [`SingleKeyView::sign_with`] so a single passphrase prompt
    /// unlocks every subsequent sign for the same key. Dropped on
    /// shutdown — never persisted, never serialised.
    single_key_unlocked: std::sync::RwLock<std::collections::BTreeMap<String, [u8; 32]>>,
    /// Guards [`WalletBackend::start`] so chain sync spawns exactly once.
    /// See [`StartLatch`].
    start_latch: StartLatch,
}

/// The single wallet entry point. See module docs.
#[derive(Clone)]
pub struct WalletBackend {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for WalletBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WalletBackend")
            .field("network", &self.inner.network)
            .finish_non_exhaustive()
    }
}

impl WalletBackend {
    /// Construct the backend: open the upstream SQLite persister, build the
    /// `PlatformWalletManager` with the DET `EventBridge`, then register every
    /// wallet the loader yields (per registration, upstream
    /// `create_wallet_from_seed_bytes` also rehydrates persisted
    /// identity/address state — see g2-mock-boundary.md §G2.1 and the
    /// upstream-reality note in the P2 recommendation).
    ///
    /// Does NOT start chain sync — call [`Self::start`] after construction.
    pub async fn new(
        ctx: &Arc<AppContext>,
        sdk: Arc<Sdk>,
        connection_status: Arc<ConnectionStatus>,
        task_result_sender: SenderAsync<TaskResult>,
        loader: Arc<dyn PersistedWalletLoader>,
    ) -> Result<Self, TaskError> {
        let network = ctx.network;
        let spv_storage_dir = Self::resolve_spv_storage_dir(ctx.data_dir(), network)?;

        let persister_config =
            SqlitePersisterConfig::new(spv_storage_dir.join("platform-wallet.sqlite"));
        let persister = Arc::new(
            SqlitePersister::open(persister_config)
                .map_err(TaskError::from_wallet_storage_open_error)?,
        );

        let secret_store_path = Self::resolve_secret_store_path(ctx.data_dir());
        let secret_store = Arc::new(single_key::open_secret_store(&secret_store_path).map_err(
            |source| TaskError::SecretStore {
                source: Box::new(source),
            },
        )?);

        let snapshots = Arc::new(SnapshotStore::new());

        let bridge = Arc::new(EventBridge::new(
            connection_status,
            task_result_sender,
            Arc::clone(&snapshots),
        ));

        let pwm = PlatformWalletManager::new(sdk, Arc::clone(&persister), bridge);

        let peer = Self::spv_primary_peer_socket(ctx, network);

        let app_kv = ctx.app_kv();

        let backend = Self {
            inner: Arc::new(Inner {
                pwm,
                persister,
                loader,
                snapshots,
                id_map: std::sync::RwLock::new(std::collections::BTreeMap::new()),
                wallets: std::sync::RwLock::new(std::collections::BTreeMap::new()),
                seeds: std::sync::RwLock::new(std::collections::BTreeMap::new()),
                peer,
                network,
                shielded: ShieldedView::new(&spv_storage_dir),
                spv_storage_dir,
                dashpay_address_index_lock: std::sync::Mutex::new(()),
                secret_store,
                single_key_index: std::sync::RwLock::new(std::collections::BTreeMap::new()),
                single_key_unlocked: std::sync::RwLock::new(std::collections::BTreeMap::new()),
                app_kv,
                start_latch: StartLatch::default(),
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
    fn hydrate_context_wallets(&self, ctx: &Arc<AppContext>) -> Result<(), TaskError> {
        let view = self.single_key();
        view.rehydrate_index()?;
        let single_key_wallets = view.hydrate_wallets();
        let reconstructed = self.hydrate_wallets_for_network(ctx.network)?;
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

    /// Run the loader and register each wallet with the upstream manager.
    async fn register_persisted_wallets(&self, ctx: &Arc<AppContext>) -> Result<(), TaskError> {
        let registrations = self.inner.loader.wallets_to_register(ctx)?;
        tracing::info!(
            count = registrations.len(),
            "Registering persisted wallets with the wallet backend"
        );

        for reg in registrations {
            // Snapshot the seed for the asset-lock / payment signer adapter.
            // Idempotent: a re-registration on the same backend just rewrites
            // the same bytes for the same hash.
            self.inner
                .seeds
                .write()?
                .insert(reg.seed_hash, reg.seed_bytes.clone());
            let already_this_process = self.inner.id_map.read()?.contains_key(&reg.seed_hash);
            if !already_this_process {
                // `create_wallet_from_seed_bytes` also loads persisted
                // identity/address deltas and runs identity discovery
                // upstream (see upstream `manager/wallet_lifecycle.rs`).
                match self
                    .inner
                    .pwm
                    .create_wallet_from_seed_bytes(
                        reg.network,
                        *reg.seed_bytes,
                        WalletAccountCreationOptions::Default,
                        None,
                    )
                    .await
                {
                    Ok(pw) => {
                        let wallet_id = pw.wallet_id();
                        self.inner.id_map.write()?.insert(reg.seed_hash, wallet_id);
                        self.inner
                            .wallets
                            .write()?
                            .insert(wallet_id, Arc::clone(&pw));
                        self.inner
                            .snapshots
                            .register_wallet(reg.seed_hash, wallet_id, pw);
                        tracing::debug!(
                            wallet = %hex::encode(reg.seed_hash),
                            "Wallet registered with backend"
                        );
                    }
                    Err(platform_wallet::error::PlatformWalletError::WalletAlreadyExists(_)) => {
                        // Already present in the upstream manager (e.g. a
                        // prior Stage-B run before this process). Resolve its
                        // id by re-deriving deterministically from the seed
                        // (NOT by parsing the error string — CLAUDE.md), so
                        // the DET-keyed map and the snapshot store stay
                        // consistent and the whole step is idempotent.
                        if let Some(wallet_id) =
                            Self::wallet_id_from_seed(reg.network, &reg.seed_bytes)
                        {
                            self.inner.id_map.write()?.insert(reg.seed_hash, wallet_id);
                            if let Some(pw) = self.inner.pwm.get_wallet(&wallet_id).await {
                                self.inner
                                    .wallets
                                    .write()?
                                    .insert(wallet_id, Arc::clone(&pw));
                                self.inner
                                    .snapshots
                                    .register_wallet(reg.seed_hash, wallet_id, pw);
                            }
                        }
                        tracing::debug!(
                            wallet = %hex::encode(reg.seed_hash),
                            "Wallet already registered upstream — idempotent"
                        );
                    }
                    Err(e) => {
                        return Err(TaskError::WalletBackend {
                            source: Box::new(e),
                        });
                    }
                }
            }

            // Recurrence trap (a5538dc8): the upstream persister `load()`
            // does NOT reconstruct identity funding HD accounts, so they
            // must be re-provisioned for every persisted identity on every
            // (re-)registration — including the idempotent re-run path,
            // which is exactly an app relaunch. Idempotent per account.
            let identity_indices: Vec<u32> = {
                let wallets = ctx.wallets.read()?;
                match wallets.get(&reg.seed_hash) {
                    Some(w) => w.read()?.identities.keys().copied().collect::<Vec<u32>>(),
                    None => Vec::new(),
                }
            };
            for idx in identity_indices {
                self.ensure_identity_funding_accounts(&reg.seed_hash, idx)
                    .await?;
            }
        }
        Ok(())
    }

    /// Start chain sync and the periodic upstream coordinators.
    ///
    /// Upstream has no single `PlatformWalletManager::start()`; this
    /// orchestrates the parts: `SpvRuntime::spawn_in_background(ClientConfig)`
    /// plus the platform-address / identity / shielded sync coordinators.
    ///
    /// `SpvRuntime::start()` only *constructs* the client; the network and
    /// sync loop runs inside `SpvRuntime::run()`, which itself calls
    /// `start()` and which `spawn_in_background()` drives on the tokio
    /// runtime. Sync failures surface asynchronously via the upstream run
    /// task and the `EventBridge` `on_error` callback, not from this call.
    ///
    /// Idempotent: the first call latches a started flag and spawns the run
    /// loop; subsequent calls return `Ok(())` without spawning a second loop.
    pub async fn start(&self) -> Result<(), TaskError> {
        if !self.inner.start_latch.try_begin() {
            tracing::debug!("Wallet backend chain sync already started; ignoring");
            return Ok(());
        }

        let config = self.build_client_config();

        self.inner.pwm.spv_arc().spawn_in_background(config);

        self.inner.pwm.platform_address_sync_arc().start();
        self.inner.pwm.identity_sync_arc().start();
        // The upstream shielded sync coordinator only exists when
        // `platform-wallet`'s `shielded` feature is enabled; DET enables only
        // `serde`, so there is no `shielded_sync_arc()` to start here.

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
    pub async fn shutdown(&self) {
        self.inner.pwm.shutdown().await;
    }

    /// Number of wallets currently registered with the backend.
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

    /// Per-address Platform funds + sync-cursor view (T5 seam). Returns
    /// the ACTIVE k/v-cached impl; [`UpstreamPlatformAddresses`] is the
    /// reserved swap target. See [`platform_address`] for why the cache
    /// stays active on 08b0ed9 (upstream lacks a public nonce reader).
    pub fn platform_addresses(&self) -> KvCachedPlatformAddresses {
        KvCachedPlatformAddresses::new(self.kv())
    }

    /// Per-`(identity, token)` balance view (T6 seam). Returns the ACTIVE
    /// k/v-cached impl; [`UpstreamTokenBalances`] is the reserved swap
    /// target. See [`token_balance`] for why the cache stays active on
    /// 08b0ed9 (no public per-token balance reader).
    pub fn token_balances(&self) -> KvCachedTokenBalances {
        KvCachedTokenBalances::new(self.kv())
    }

    /// Shared handle to the encrypted secret store backing imported-key
    /// material. Most callers should reach for [`Self::single_key`]
    /// instead — this accessor exists for the migration engine
    /// (T-SK-02), which writes legacy WIFs back into the vault.
    pub fn secret_store(&self) -> &Arc<SecretStore> {
        &self.inner.secret_store
    }

    /// Per-network shielded sidecar (T-SH-01). The file at
    /// `<spv_storage_dir>/det-shielded.sqlite` is created lazily on the
    /// first write; a wallet with no shielded activity gets no sidecar
    /// on disk (FR-3.3). T-SH-03 will rewire callers off the legacy
    /// `database::shielded` API onto this view.
    pub fn shielded(&self) -> &ShieldedView {
        &self.inner.shielded
    }

    /// View over the single-key (imported WIF) operations. The view
    /// borrows the secret store, the in-memory address index, the
    /// cross-network app k/v sidecar that persists imported-key
    /// metadata, and the in-process unlock cache; all four are cheap to
    /// construct, so callers can build one per operation.
    ///
    /// TODO(SEC-002 follow-up): wire the sign-time passphrase prompt
    /// flow across every backend task that ends up calling
    /// `single_key().sign_with(...)` (identity register, send funds,
    /// asset-lock signer, ...). The storage + unlock-cache API ships in
    /// the same commit as this view; the per-task prompt UX is a
    /// separate change.
    pub fn single_key(&self) -> SingleKeyView<'_> {
        SingleKeyView {
            secret_store: &self.inner.secret_store,
            index: &self.inner.single_key_index,
            network: self.inner.network,
            app_kv: Some(&self.inner.app_kv),
            unlocked: Some(&self.inner.single_key_unlocked),
        }
    }

    /// View over the DET-owned wallet-metadata sidecar (alias /
    /// `is_main` / `core_wallet_name`). Backed by the cross-network
    /// app-level k/v store; see [`WalletMetaView`] (T-W-00) for the
    /// key schema. The view borrows a shared `Arc<DetKv>` handle, so
    /// callers may build one per operation rather than threading it.
    pub fn wallet_meta(&self) -> WalletMetaView<'_> {
        WalletMetaView::new(&self.inner.app_kv)
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
        match self
            .kv()
            .get::<SelectedWallet>(DetScope::Global, SelectedWallet::KV_KEY)
        {
            Ok(Some(s)) => s,
            Ok(None) => SelectedWallet::default(),
            Err(e) => {
                tracing::warn!(
                    network = ?self.inner.network,
                    error = ?e,
                    "Failed to load SelectedWallet from wallet k/v; using default"
                );
                SelectedWallet::default()
            }
        }
    }

    /// Persist the [`SelectedWallet`] pointer to this network's wallet
    /// k/v store.
    pub fn set_selected_wallet(&self, selected: &SelectedWallet) -> Result<(), KvAdapterError> {
        self.kv()
            .put(DetScope::Global, SelectedWallet::KV_KEY, selected)
    }

    /// Broadcast a raw transaction over the network via the upstream
    /// `SpvRuntime`. Network-level (not tied to a specific wallet); used for
    /// asset-lock transactions built outside the per-wallet send path.
    pub async fn broadcast_transaction(
        &self,
        tx: &dash_sdk::dpp::dashcore::Transaction,
    ) -> Result<dash_sdk::dpp::dashcore::Txid, TaskError> {
        use platform_wallet::broadcaster::{SpvBroadcaster, TransactionBroadcaster};
        let broadcaster = SpvBroadcaster::new(self.inner.pwm.spv_arc());
        broadcaster
            .broadcast(tx)
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })
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
                source: Box::new(e),
            })
    }

    /// Whether chain sync has not yet reached the tip.
    pub async fn is_syncing(&self) -> bool {
        match self.inner.pwm.spv().sync_progress().await {
            Some(p) => !p.is_synced(),
            None => false,
        }
    }

    /// Map a DET `WalletSeedHash` to the upstream wallet handle.
    async fn resolve_wallet(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<Arc<platform_wallet::PlatformWallet>, TaskError> {
        let wallet_id = *self
            .inner
            .id_map
            .read()?
            .get(seed_hash)
            .ok_or(TaskError::WalletBackendNotYetWired)?;
        self.inner
            .pwm
            .get_wallet(&wallet_id)
            .await
            .ok_or(TaskError::WalletBackendNotYetWired)
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
                source: Box::new(e),
            })?;
        Ok(addr.to_string())
    }

    /// Re-establish a DashPay contact on UPSTREAM derivation only.
    ///
    /// Derives the `DashpayReceivingFunds` account via the upstream engine
    /// and registers it so the SPV adapter monitors incoming payments. No DET
    /// re-derivation, no comparison — upstream is authoritative
    /// (Decision #6, back-compat dropped). Idempotent: upstream no-ops if the
    /// contact account already exists.
    pub async fn register_dashpay_contact(
        &self,
        seed_hash: &WalletSeedHash,
        owner_identity_id: &dash_sdk::platform::Identifier,
        contact_identity_id: &dash_sdk::platform::Identifier,
        account_index: u32,
    ) -> Result<(), TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        wallet
            .identity()
            .register_contact_account(owner_identity_id, contact_identity_id, account_index)
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })
    }

    /// Durably flush every registered wallet's buffered changesets to the
    /// upstream persister. Called before the one-time migration's
    /// strictly-last legacy-table DROP so the new persister is durable
    /// before any legacy data is destroyed.
    pub async fn flush_persister(&self) -> Result<(), TaskError> {
        let ids = self.inner.pwm.wallet_ids().await;
        for id in ids {
            if let Some(w) = self.inner.pwm.get_wallet(&id).await {
                w.flush_persist()
                    .map_err(|source| TaskError::WalletPersistenceFlushFailed { source })?;
            }
        }
        Ok(())
    }

    /// Re-register every persisted wallet with the upstream manager
    /// (idempotent). Exposed for the one-time migration engine; the upstream
    /// `create_wallet_from_seed_bytes` also runs identity discovery from
    /// chain, repopulating `IdentityManager` (data-model-and-migration.md
    /// conversion surface — identities "repopulated on first sync").
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
    /// Async variant: prefer this from backend tasks. For
    /// synchronous UI code (egui frame loop), use
    /// [`Self::list_tracked_asset_locks_blocking`].
    pub async fn list_tracked_asset_locks(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<Vec<platform_wallet::wallet::asset_lock::tracked::TrackedAssetLock>, TaskError>
    {
        let wallet = self.resolve_wallet(seed_hash).await?;
        Ok(wallet.asset_locks().list_tracked_locks().await)
    }

    /// Blocking variant of [`Self::list_tracked_asset_locks`] for synchronous
    /// UI contexts. Reads from WalletBackend's sync wallet cache so it
    /// does not enter the upstream tokio-async lock — safe to call from
    /// the egui frame loop.
    pub fn list_tracked_asset_locks_blocking(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Vec<platform_wallet::wallet::asset_lock::tracked::TrackedAssetLock> {
        let wallet_id = match self.inner.id_map.read() {
            Ok(map) => match map.get(seed_hash) {
                Some(id) => *id,
                None => return Vec::new(),
            },
            Err(_) => return Vec::new(),
        };
        let wallet = match self.inner.wallets.read() {
            Ok(map) => match map.get(&wallet_id) {
                Some(w) => Arc::clone(w),
                None => return Vec::new(),
            },
            Err(_) => return Vec::new(),
        };
        wallet.asset_locks().list_tracked_locks_blocking()
    }

    /// Deterministically derive the upstream `WalletId` from seed bytes
    /// without touching the manager. Used only to recover the id on the
    /// idempotent `WalletAlreadyExists` re-registration path (avoids
    /// parsing the upstream error string — CLAUDE.md).
    fn wallet_id_from_seed(network: Network, seed_bytes: &[u8; 64]) -> Option<WalletId> {
        use dash_sdk::dpp::key_wallet::wallet::Wallet;
        Wallet::from_seed_bytes(*seed_bytes, network, WalletAccountCreationOptions::Default)
            .ok()
            .map(|w| w.wallet_id)
    }

    /// Snapshot the cached seed and wrap it in a soft-wallet signer for the
    /// upstream signer-driven asset-lock / payment builders. Snapshot is
    /// cloned (and zeroized when the signer drops) so derivation can run
    /// without contention on the upstream wallet-manager lock.
    fn signer_for(&self, seed_hash: &WalletSeedHash) -> Result<WalletAssetLockSigner, TaskError> {
        let seed = self
            .inner
            .seeds
            .read()?
            .get(seed_hash)
            .cloned()
            .ok_or(TaskError::WalletBackendNotYetWired)?;
        Ok(WalletAssetLockSigner::new(seed, self.inner.network))
    }

    /// Derive the secp256k1 [`PrivateKey`] at `path` from the cached seed.
    /// Used after `create_asset_lock_proof` to obtain the one-time
    /// credit-output key needed to sign DET-retained non-identity state
    /// transitions (Platform-address top-up, shielded deposit).
    fn derive_private_key(
        &self,
        seed_hash: &WalletSeedHash,
        path: &dash_sdk::dpp::key_wallet::bip32::DerivationPath,
    ) -> Result<dash_sdk::dpp::dashcore::PrivateKey, TaskError> {
        let seed = self
            .inner
            .seeds
            .read()?
            .get(seed_hash)
            .cloned()
            .ok_or(TaskError::WalletBackendNotYetWired)?;
        let xprv = path
            .derive_priv_ecdsa_for_master_seed(seed.as_ref(), self.inner.network)
            .map_err(|source| TaskError::WalletBackend {
                source: Box::new(platform_wallet::error::PlatformWalletError::KeyDerivation(
                    source.to_string(),
                )),
            })?;
        Ok(xprv.to_priv())
    }

    /// Build, sign, and broadcast a payment from the wallet's default BIP-44
    /// account to `recipients` (`(address, duffs)`). Returns the txid.
    pub async fn send_payment(
        &self,
        seed_hash: &WalletSeedHash,
        recipients: Vec<(dash_sdk::dpp::dashcore::Address, u64)>,
    ) -> Result<dash_sdk::dpp::dashcore::Txid, TaskError> {
        use dash_sdk::dpp::key_wallet::account::account_type::StandardAccountType;
        let signer = self.signer_for(seed_hash)?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let tx = wallet
            .core()
            .send_to_addresses(
                StandardAccountType::BIP44Account,
                DEFAULT_BIP44_ACCOUNT,
                recipients,
                &signer,
            )
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })?;
        Ok(tx.txid())
    }

    /// Build, track, and broadcast a **non-identity** asset lock via the
    /// upstream `AssetLockManager`. `funding_type` selects the funding
    /// derivation; `identity_index` is the funding-account derivation index
    /// (ignored for non-identity funding types). Returns the finalized
    /// asset-lock proof, its one-time credit-output private key (derived
    /// locally from the wallet seed at the path upstream selected), and the
    /// txid.
    ///
    /// For identity-funded asset locks
    /// (`AssetLockFundingType::IdentityRegistration` /
    /// `AssetLockFundingType::IdentityTopUp`) the upstream
    /// `IdentityWallet::*_with_funding` orchestrators submit the
    /// Platform-side state transition themselves and never expose a
    /// credit-output `PrivateKey` — use [`Self::register_identity`] /
    /// [`Self::top_up_identity`] instead.
    pub(crate) async fn create_asset_lock_proof(
        &self,
        seed_hash: &WalletSeedHash,
        amount_duffs: u64,
        funding_type: platform_wallet::AssetLockFundingType,
        identity_index: u32,
    ) -> Result<
        (
            dash_sdk::dpp::prelude::AssetLockProof,
            dash_sdk::dpp::dashcore::PrivateKey,
            dash_sdk::dpp::dashcore::Txid,
        ),
        TaskError,
    > {
        use platform_wallet::AssetLockFundingType;

        // Identity asset locks fund from the IdentityRegistration /
        // IdentityTopUp HD accounts, which the upstream persister never
        // reconstructs (a5538dc8). Provision them here — the single
        // chokepoint every asset-lock caller funnels through — so no call
        // site can bypass it. Idempotent. Non-identity funding types are
        // no-ops. Exhaustive — a new upstream variant must force a
        // review here instead of silently falling through.
        match funding_type {
            AssetLockFundingType::IdentityRegistration | AssetLockFundingType::IdentityTopUp => {
                self.ensure_identity_funding_accounts(seed_hash, identity_index)
                    .await?;
            }
            AssetLockFundingType::IdentityTopUpNotBound
            | AssetLockFundingType::IdentityInvitation
            | AssetLockFundingType::AssetLockAddressTopUp
            | AssetLockFundingType::AssetLockShieldedAddressTopUp => {}
        }

        let signer = self.signer_for(seed_hash)?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let (proof, credit_output_path, out_point) = wallet
            .asset_locks()
            .create_funded_asset_lock_proof(
                amount_duffs,
                DEFAULT_BIP44_ACCOUNT,
                funding_type,
                identity_index,
                &signer,
            )
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })?;
        let private_key = self.derive_private_key(seed_hash, &credit_output_path)?;
        Ok((proof, private_key, out_point.txid))
    }

    /// Register a new identity on Platform funded by an asset lock built and
    /// tracked-to-finality by the upstream `AssetLockManager`. Returns the
    /// persisted [`Identity`].
    ///
    /// Wraps upstream `IdentityWallet::register_identity_with_funding` —
    /// upstream handles asset-lock build/broadcast, IS→CL fallback with the
    /// CL-height-too-low retry, the actual `PutIdentity` submission, and the
    /// asset-lock cleanup. The DET retry loop around `UnknownVersionError`
    /// and the manual IS-proof-invalid fallback are no longer needed at the
    /// caller — upstream owns both paths.
    ///
    /// `funding` is the upstream funding selector — `FromWalletBalance`
    /// builds a fresh asset lock, `FromExistingAssetLock` resumes from a
    /// tracked outpoint (the wallet-backend tracker is the single source of
    /// asset-lock state).
    pub async fn register_identity(
        &self,
        seed_hash: &WalletSeedHash,
        identity_index: u32,
        funding: platform_wallet::wallet::asset_lock::AssetLockFunding,
        keys_map: std::collections::BTreeMap<u32, dash_sdk::dpp::identity::IdentityPublicKey>,
        identity_signer: &crate::model::qualified_identity::QualifiedIdentity,
        settings: Option<dash_sdk::platform::transition::put_settings::PutSettings>,
    ) -> Result<dash_sdk::platform::Identity, TaskError> {
        // Re-provisioning idempotent. Run here so the chokepoint protection
        // applies to upstream's signer-driven flow too.
        self.ensure_identity_funding_accounts(seed_hash, identity_index)
            .await?;

        let asset_lock_signer = self.signer_for(seed_hash)?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        wallet
            .identity()
            .register_identity_with_funding(
                funding,
                identity_index,
                keys_map,
                identity_signer,
                &asset_lock_signer,
                settings,
            )
            .await
            .map_err(map_identity_register_error)
    }

    /// Top up an existing identity's credit balance from this wallet's
    /// UTXOs. Returns the post-top-up identity balance (credits).
    ///
    /// Wraps upstream `IdentityWallet::top_up_identity_with_funding` —
    /// upstream handles asset-lock build/broadcast, IS→CL fallback, the
    /// `TopUpIdentity` submission, and the asset-lock cleanup. The
    /// caller-side IS-proof-invalid fallback and `UnknownVersionError`
    /// retry are no longer needed.
    ///
    /// `funding` is the upstream funding selector — `FromWalletBalance`
    /// builds a fresh asset lock, `FromExistingAssetLock` resumes from a
    /// tracked outpoint.
    pub async fn top_up_identity(
        &self,
        seed_hash: &WalletSeedHash,
        identity_id: &dash_sdk::platform::Identifier,
        funding: platform_wallet::wallet::asset_lock::AssetLockFunding,
        identity_index: u32,
        settings: Option<dash_sdk::platform::transition::put_settings::PutSettings>,
    ) -> Result<u64, TaskError> {
        self.ensure_identity_funding_accounts(seed_hash, identity_index)
            .await?;

        let asset_lock_signer = self.signer_for(seed_hash)?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        wallet
            .identity()
            .top_up_identity_with_funding(identity_id, funding, &asset_lock_signer, settings)
            .await
            .map_err(|e| map_identity_top_up_error(*identity_id, e))
    }

    // UPSTREAM GAP: rs-platform-wallet has no identity-funding-account
    // registrar (sibling to register_contact_account). Contained exception —
    // key_wallet plumbing lives ONLY here, never leaks past WalletBackend.
    // Do not replicate; do not collapse the dual-insert; do not use the
    // funds-bearing API. Tracked: upstream-contribution TODO 9cdcfb25.
    //
    // `peek_next_funding_address` reads BOTH `wallet.accounts.*` (xpub
    // source) AND `wallet_info.accounts.*` (mutable managed account), so the
    // account must exist in both collections. The upstream persister
    // `load()` reconstructs neither, hence the reload re-provision.
    // Idempotent: probes both collections and no-ops if present (no error-
    // string parsing — direct membership checks).
    async fn provision_identity_funding_account(
        &self,
        seed_hash: &WalletSeedHash,
        account_type: dash_sdk::dpp::key_wallet::AccountType,
    ) -> Result<(), TaskError> {
        use dash_sdk::dpp::key_wallet::AccountType;
        use dash_sdk::dpp::key_wallet::managed_account::ManagedCoreKeysAccount;

        // Restrict to the two identity-funding flavours; everything else is a
        // misuse — keeping the match exhaustive forces a review if a new
        // upstream identity-funding variant appears.
        match account_type {
            AccountType::IdentityRegistration | AccountType::IdentityTopUp { .. } => {}
            _ => return Err(TaskError::WalletBackendNotYetWired),
        }

        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let mut wm = wallet.wallet_manager().write().await;
        let (kw, info) = wm
            .get_wallet_mut_and_info_mut(&wallet_id)
            .ok_or(TaskError::WalletBackendNotYetWired)?;

        let in_wallet = match account_type {
            AccountType::IdentityRegistration => kw.accounts.identity_registration.is_some(),
            AccountType::IdentityTopUp { registration_index } => {
                kw.accounts.identity_topup.contains_key(&registration_index)
            }
            _ => unreachable!("checked above"),
        };
        let in_managed = match account_type {
            AccountType::IdentityRegistration => {
                info.core_wallet.accounts.identity_registration.is_some()
            }
            AccountType::IdentityTopUp { registration_index } => info
                .core_wallet
                .accounts
                .identity_topup
                .contains_key(&registration_index),
            _ => unreachable!("checked above"),
        };
        if in_wallet && in_managed {
            return Ok(());
        }

        if !in_wallet {
            kw.add_account(account_type, None)
                .map_err(|e| TaskError::WalletBackend {
                    source: Box::new(
                        platform_wallet::error::PlatformWalletError::AssetLockTransaction(
                            e.to_string(),
                        ),
                    ),
                })?;
        }

        let derived = match account_type {
            AccountType::IdentityRegistration => kw.accounts.identity_registration.as_ref(),
            AccountType::IdentityTopUp { registration_index } => {
                kw.accounts.identity_topup.get(&registration_index)
            }
            _ => unreachable!("checked above"),
        }
        .ok_or(TaskError::WalletBackendNotYetWired)?;

        let managed = ManagedCoreKeysAccount::from_account(derived);
        info.core_wallet
            .accounts
            .insert_keys_bearing_account(managed)
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(
                    platform_wallet::error::PlatformWalletError::AssetLockTransaction(
                        e.to_string(),
                    ),
                ),
            })?;
        Ok(())
    }

    /// Provision the identity-registration funding account and the per-
    /// identity top-up funding account for the given wallet identity index.
    /// Idempotent; safe to call before every asset-lock and on every reload.
    pub async fn ensure_identity_funding_accounts(
        &self,
        seed_hash: &WalletSeedHash,
        registration_index: u32,
    ) -> Result<(), TaskError> {
        use dash_sdk::dpp::key_wallet::AccountType;
        self.provision_identity_funding_account(seed_hash, AccountType::IdentityRegistration)
            .await?;
        self.provision_identity_funding_account(
            seed_hash,
            AccountType::IdentityTopUp { registration_index },
        )
        .await
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
        if !matches!(network, Network::Devnet | Network::Regtest) {
            return None;
        }
        let cfg = ctx.config.read().ok()?;
        let host = cfg.core_host.as_deref()?;
        let port = match network {
            Network::Mainnet => 9999,
            Network::Testnet => 19999,
            Network::Devnet => 20001,
            Network::Regtest => 19899,
        };
        format!("{host}:{port}").to_socket_addrs().ok()?.next()
    }

    /// Per-process file path of the encrypted secret store vault. Shared
    /// across networks: the secret store is not per-network (a single
    /// imported WIF is a P2PKH key whose network prefix lives in the
    /// derived address). The parent directory is created lazily by
    /// [`single_key::open_secret_store`].
    fn resolve_secret_store_path(app_data_dir: &Path) -> std::path::PathBuf {
        let mut path = app_data_dir.to_path_buf();
        path.push("secrets");
        path.push("det-secrets.pwsvault");
        path
    }

    fn resolve_spv_storage_dir(
        app_data_dir: &Path,
        network: Network,
    ) -> Result<std::path::PathBuf, TaskError> {
        let mut dir = app_data_dir.to_path_buf();
        dir.push("spv");
        dir.push(match network {
            Network::Mainnet => "mainnet",
            Network::Testnet => "testnet",
            Network::Devnet => "devnet",
            Network::Regtest => "regtest",
        });
        std::fs::create_dir_all(&dir).map_err(|source| TaskError::FileSystem { source })?;
        Ok(dir)
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
        IdentityOpErrorKind::Other => TaskError::WalletBackend {
            source: Box::new(e),
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
        IdentityOpErrorKind::Other => TaskError::WalletBackend {
            source: Box::new(e),
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

        // Everything else — preconditions, wallet state, builder errors.
        P::WalletCreation(_)
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
        | P::SpvNotRunning
        | P::SpvError(_)
        | P::TokenError(_)
        | P::ShieldedNoUnspentNotes
        | P::ShieldedInsufficientBalance { .. }
        | P::ShieldedBuildError(_)
        | P::ShieldedBroadcastFailed(_)
        | P::ShieldedSyncFailed(_)
        | P::ShieldedTreeUpdateFailed(_)
        | P::ShieldedStoreError(_)
        | P::ShieldedNullifierSyncFailed(_)
        | P::ShieldedMerkleWitnessUnavailable(_)
        | P::ShieldedKeyDerivation(_)
        | P::ShieldedNotBound => IdentityOpErrorKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The start latch is one-shot: `try_begin` returns `true` only on the
    /// first call, so `WalletBackend::start` spawns the SPV run loop exactly
    /// once even when called repeatedly (Connect clicked twice, eager-init plus
    /// a manual click). This guards against a double-SPV-spawn regression.
    #[test]
    fn start_latch_fires_once() {
        let latch = StartLatch::default();
        assert!(!latch.is_started(), "fresh latch must not be started");
        assert!(latch.try_begin(), "first try_begin must win");
        assert!(
            latch.is_started(),
            "latch must report started after winning"
        );
        assert!(!latch.try_begin(), "second try_begin must lose");
        assert!(!latch.try_begin(), "third try_begin must lose");
        assert!(latch.is_started(), "latch stays started");
    }

    /// Concurrent callers race to a single winner — exactly one thread sees
    /// `try_begin() == true`. Pins the atomic-swap contract that prevents two
    /// SPV run loops from racing against the same data directory.
    #[test]
    fn start_latch_single_winner_under_contention() {
        use std::sync::Arc as StdArc;
        use std::sync::atomic::AtomicUsize;

        let latch = StdArc::new(StartLatch::default());
        let winners = StdArc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..16)
            .map(|_| {
                let latch = StdArc::clone(&latch);
                let winners = StdArc::clone(&winners);
                std::thread::spawn(move || {
                    if latch.try_begin() {
                        winners.fetch_add(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread panicked");
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
}
