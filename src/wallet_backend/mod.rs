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
mod kv;
mod loader;
mod platform_address;
pub mod secret_access;
pub mod secret_prompt;
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
pub(crate) use dashpay::{derive_contact_info_encryption_keys, derive_contact_xpub_material};

pub(crate) use det_platform_signer::{DetPlatformSigner, PlatformPathIndex};
pub(crate) use det_signer::DetSigner;
pub use secret_access::{SecretAccess, SecretPlaintext, SecretSession, WalletPromptMeta};
pub use secret_prompt::{
    NullSecretPrompt, RememberPolicy, SecretPrompt, SecretPromptCancelled, SecretPromptReply,
    SecretPromptRequest, SecretPromptRetry, SecretScope,
};

use coordinator_gate::CoordinatorGate;

pub use auth_pubkey_cache::AuthPubkeyCacheView;
pub use avatar_cache::AvatarCacheView;
pub use contact_profile_cache::{CachedContactProfile, ContactProfileCacheView};
pub use event_bridge::EventBridge;
pub use kv::{DetKv, DetScope, KvAdapterError, SCHEMA_VERSION as KV_SCHEMA_VERSION};
pub use loader::{LoadedWallets, PersistedLoadSkip, PersistedWalletLoader, UpstreamFromPersisted};
pub use platform_address::{
    KvCachedPlatformAddresses, PlatformAddressView, UpstreamPlatformAddresses,
};
pub use single_key::SingleKeyView;
use snapshot::SnapshotStore;
pub use snapshot::{DetUtxo, DetWalletBalance, WalletSnapshot};
use token_balance::TokenBalanceStore;
pub use token_balance::{TokenBalanceSnapshot, TokenBalanceView, UpstreamTokenBalances};
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
use platform_wallet::manager::PlatformWalletManager;
use platform_wallet_storage::secrets::SecretStore;
use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig};

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
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
    /// Lock-free per-`(identity, token)` balance snapshot, refreshed off the
    /// UI thread from the upstream `IdentitySyncManager` and read on the
    /// frame thread via [`Self::token_balances`]. See [`token_balance`].
    token_balances: Arc<TokenBalanceStore>,
    /// `WalletSeedHash` → upstream `WalletId`. See [`WalletId`].
    id_map: std::sync::RwLock<std::collections::BTreeMap<WalletSeedHash, WalletId>>,
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
    /// BIP-39 seeds (`envelope.v1` labels under `WalletId(seed_hash)`, see
    /// [`wallet_seed_store`]). [`Self::secret_access`] decrypts seeds
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
        // the same handle before the backend is wired (PROJ-010).
        let secret_store = ctx.secret_store();

        let snapshots = Arc::new(SnapshotStore::new());

        let coordinator_gate = Arc::new(CoordinatorGate::default());

        let bridge = Arc::new(EventBridge::new(
            connection_status,
            task_result_sender.clone(),
            Arc::clone(&snapshots),
            Arc::clone(&coordinator_gate),
            // Phase E push writer: the shielded sync-completed callback writes
            // per-wallet balances into AppContext's frame-safe snapshot.
            Arc::clone(&ctx.shielded_balances),
            // Platform-address push writer: the platform address sync-completed callback
            // writes per-wallet owned-only balances into AppContext's frame-safe snapshot.
            Arc::clone(&ctx.platform_balances),
        ));

        let pwm = PlatformWalletManager::new(sdk, Arc::clone(&persister), bridge);

        // Wire the upstream shielded coordinator into the manager.
        //
        // Uses a dedicated SQLite file (`platform-wallet-shielded.sqlite`) owned
        // entirely by the upstream coordinator — it is the single source of
        // truth for all Orchard state now that DET's home-grown subsystem was
        // retired (Phase D). The coordinator starts empty — no wallets are bound
        // until `ensure_shielded_bound` runs (on wallet unlock). Subsequent
        // calls with the same path are idempotent (upstream no-ops).
        pwm.configure_shielded(spv_storage_dir.join("platform-wallet-shielded.sqlite"))
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
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
                loader,
                snapshots,
                token_balances: Arc::new(TokenBalanceStore::new()),
                id_map: std::sync::RwLock::new(std::collections::BTreeMap::new()),
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

    /// Run the configured loader to bring back persisted wallets watch-only.
    /// Identity-funding re-provision is deferred to the asset-lock chokepoint
    /// (which obtains the seed just-in-time), so this pass only loads and logs.
    async fn register_persisted_wallets(&self, ctx: &Arc<AppContext>) -> Result<(), TaskError> {
        let loader = Arc::clone(&self.inner.loader);
        let outcome = loader.load(self, ctx).await?;
        tracing::info!(
            loaded = outcome.loaded.len(),
            skipped = outcome.skipped.len(),
            "Persisted-wallet load pass complete"
        );
        for (seed_hash, reason) in &outcome.skipped {
            tracing::warn!(
                wallet = ?seed_hash.map(hex::encode),
                ?reason,
                "Skipped a corrupt persisted wallet row on load"
            );
        }

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

        // 2. One persister load pass, only when the manager is empty.
        //    A non-empty `skipped` is success; `Err` is reserved for
        //    whole-load failures. On a re-run the manager already holds
        //    the wallets, so the upstream load (which rejects duplicates)
        //    is skipped and only the resolution below runs.
        let skipped = if self.inner.pwm.wallet_ids().await.is_empty() {
            let outcome = self.inner.pwm.load_from_persistor().await.map_err(|e| {
                TaskError::WalletBackend {
                    source: Box::new(e),
                }
            })?;
            outcome
                .skipped
                .iter()
                .map(|(_wallet_id, reason)| (None, persisted_load_skip_from_upstream(reason)))
                .collect()
        } else {
            Vec::new()
        };

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

            self.inner.id_map.write()?.insert(seed_hash, wallet_id);
            self.inner
                .wallets
                .write()?
                .insert(wallet_id, Arc::clone(&pw));
            self.inner
                .snapshots
                .register_wallet(seed_hash, wallet_id, pw);
            tracing::debug!(
                wallet = %hex::encode(seed_hash),
                "Watch-only wallet registered with backend (seedless)"
            );
            loaded.push(seed_hash);
        }

        Ok(LoadedWallets { loaded, skipped })
    }

    /// `ExtendedPubKey::encode()` bytes of the loaded watch-only wallet's
    /// BIP44 account 0 xpub, or `None` when the wallet has no BIP44
    /// account. Read seedlessly off the rebuilt watch-only manifest.
    async fn bip44_account_xpub_encoded(
        &self,
        pw: &platform_wallet::PlatformWallet,
    ) -> Option<Vec<u8>> {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        let guard = pw.state().await;
        guard
            .wallet()
            .accounts
            .all_accounts()
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
            .map(|a| a.account_xpub.encode().to_vec())
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
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
        use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
        let wallet = UpstreamWallet::from_seed_bytes(
            *seed,
            self.inner.network,
            WalletAccountCreationOptions::Default,
        )
        .map_err(|e| TaskError::WalletBackend {
            source: Box::new(platform_wallet::error::PlatformWalletError::WalletCreation(
                e.to_string(),
            )),
        })?;
        let account_xpub = wallet
            .accounts
            .all_accounts()
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
            .map(|a| a.account_xpub.encode().to_vec())
            .ok_or(TaskError::WalletRegistrationXpubMismatch)?;
        Ok((wallet.wallet_id, account_xpub))
    }

    // TODO(PROJ-015): TC-012 receive-address reuse unverified — see if dashpay/platform#3770
    //   addresses it; if not, escalate.
    /// Register a wallet with the upstream SPV backend from its seed, so the
    /// upstream persistor is populated and the wallet's addresses are watched
    /// (W1 — create/import write path; PROJ-010 regression fix).
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
                *seed,
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
                    source: Box::new(e),
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
    /// DET sidecars but absent from the upstream persistor (W2; PROJ-010).
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
        self.register_wallet_from_seed(
            seed_hash,
            seed,
            registration_birth_height(WalletOrigin::Imported),
        )
        .await
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
        let Some(pw) = self.inner.pwm.get_wallet(&wallet_id).await else {
            return Err(TaskError::WalletBackend {
                source: Box::new(platform_wallet::error::PlatformWalletError::WalletNotFound(
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

        self.inner.id_map.write()?.insert(seed_hash, wallet_id);
        self.inner
            .wallets
            .write()?
            .insert(wallet_id, Arc::clone(&pw));
        self.inner
            .snapshots
            .register_wallet(seed_hash, wallet_id, pw);
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
    /// run. Idempotent — forgetting an unknown wallet is a no-op success.
    pub(crate) fn forget_wallet_local_state(
        &self,
        seed_hash: &WalletSeedHash,
        wallet_id: Option<WalletId>,
    ) -> Result<(), TaskError> {
        // Encrypted seed-envelope vault (the JIT decrypt source).
        if let Err(e) = self.wallet_seeds().delete(seed_hash) {
            tracing::warn!(
                wallet = %hex::encode(seed_hash),
                error = ?e,
                "Failed to delete seed envelope from vault"
            );
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
        }

        // Plaintext Orchard state (notes + nullifier cursor) now lives in the
        // upstream coordinator store; `remove_upstream_wallet` detaches it.

        // DET-side avatar cache (PROJ-040). Avatars live in the cross-network
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
        }

        // In-memory maps + snapshot registration.
        if let Some(wallet_id) = wallet_id {
            self.inner.id_map.write()?.remove(seed_hash);
            self.inner.wallets.write()?.remove(&wallet_id);
            self.inner.snapshots.forget_wallet(seed_hash, &wallet_id);
        }

        Ok(())
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
        self.inner
            .pwm
            .clear_shielded()
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
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
                source: Box::new(e),
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
    /// maps) with no runtime. Returns the upstream `WalletId`s whose watch-only
    /// persistor rows still need the async [`Self::remove_upstream_wallet`]
    /// removal — the caller drives those off-thread. Resilient to partial
    /// failure.
    pub(crate) fn forget_all_wallets_local(&self) -> Vec<WalletId> {
        let network = self.inner.network;

        // HD wallets: enumerate from the persisted wallet-meta sidecar so a
        // never-loaded wallet is still wiped.
        let mut upstream_ids = Vec::new();
        for (seed_hash, _meta) in self.wallet_meta().list(network) {
            let wallet_id = self.registered_wallet_id(&seed_hash);
            if let Some(id) = wallet_id {
                upstream_ids.push(id);
            }
            if let Err(e) = self.forget_wallet_local_state(&seed_hash, wallet_id) {
                tracing::warn!(
                    wallet = %hex::encode(seed_hash),
                    error = ?e,
                    "Failed to wipe local HD wallet state during clear-all"
                );
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
            }
        }

        // Belt-and-suspenders: drop any remaining session-cached secrets
        // (single-key forget does not clear the session cache).
        self.forget_all_secrets();

        upstream_ids
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
    ///
    /// SPV is spawned immediately, but the Platform/identity sync coordinators
    /// are gated on masternode-list readiness via [`CoordinatorGate`]: starting
    /// them before quorums exist fires proof-verifying DAPI calls that fail
    /// locally and get every node banned by the SDK. The gate fires them once,
    /// either now (if masternodes already synced) or when the `EventBridge`
    /// reports the masternode list reached `Synced`.
    pub async fn start(&self) -> Result<(), TaskError> {
        if !self.inner.start_latch.try_begin() {
            tracing::debug!("Wallet backend chain sync already started; ignoring");
            return Ok(());
        }

        let config = self.build_client_config();

        self.inner.pwm.spv_arc().spawn_in_background(config);

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
            let _ = task_result_sender.try_send(TaskResult::Success(Box::new(
                BackendTaskSuccessResult::PlatformReadyDiscoverIdentities,
            )));
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
        let wallet_meta: std::collections::BTreeMap<WalletSeedHash, WalletPromptMeta> =
            reconstructed
                .iter()
                .map(|(seed_hash, wallet)| {
                    (
                        *seed_hash,
                        WalletPromptMeta {
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
                    .map_err(|_| TaskError::SingleKeyCryptoFailure)
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

    /// View over the DET-owned identity-authentication public-key cache
    /// (D4b). Backed by the same cross-network app-level k/v store as
    /// [`Self::wallet_meta`], keyed per wallet under
    /// `DetScope::Wallet(seed_hash)`; see [`AuthPubkeyCacheView`] for the
    /// key schema. The cache memoises the hardened-path identity-auth
    /// pubkeys so the steady-state read is seed-free.
    pub fn auth_pubkey_cache(&self) -> AuthPubkeyCacheView<'_> {
        AuthPubkeyCacheView::new(&self.inner.app_kv)
    }

    /// View over the DET-owned avatar image cache (PROJ-040). Backed by the
    /// same cross-network app-level k/v store as [`Self::wallet_meta`], keyed
    /// by avatar URL under [`DetScope::Global`]. Upstream persists only the
    /// avatar hash and fingerprint, never the bytes, so this is the only place
    /// a contact's avatar image survives offline / between views.
    pub fn avatar_cache(&self) -> AvatarCacheView<'_> {
        AvatarCacheView::new(&self.inner.app_kv)
    }

    /// View over the DET-owned contact-profile cache (PROJ-040). A contact's
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
            .ok_or(TaskError::WalletNotLoaded)?;
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
                source: Box::new(e),
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

        let wallet_id = *self
            .inner
            .id_map
            .read()?
            .get(seed_hash)
            .ok_or(TaskError::WalletNotLoaded)?;
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
        use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
        use platform_wallet::error::PlatformWalletError;

        if contacts.is_empty() {
            return Ok(0);
        }
        let network = self.inner.network;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();

        // The DIP-15 receiving path is hardened, so derive the account xpubs
        // from a signable seed-built wallet — the live wallet is watch-only and
        // cannot derive hardened paths. Built once, reused for every contact.
        let seed_wallet =
            UpstreamWallet::from_seed_bytes(*seed, network, WalletAccountCreationOptions::Default)
                .map_err(|e| TaskError::WalletBackend {
                    source: Box::new(PlatformWalletError::WalletCreation(e.to_string())),
                })?;

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
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let persister = wallet.persister().clone();
        let mut wm = wallet.wallet_manager().write().await;
        let info = wm
            .get_wallet_info_mut(&wallet_id)
            .ok_or(TaskError::WalletStateInconsistent)?;
        match info.identity_manager.managed_identity_mut(owner_id) {
            Some(managed) => {
                managed.add_sent_contact_request(contact_request, &persister);
            }
            None => {
                tracing::warn!(
                    owner_id = %owner_id,
                    "record_sent_contact_request: managed identity not \
                     found; state transition committed but local manager \
                     not updated",
                );
            }
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
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let persister = wallet.persister().clone();
        let mut wm = wallet.wallet_manager().write().await;
        let info = wm
            .get_wallet_info_mut(&wallet_id)
            .ok_or(TaskError::WalletStateInconsistent)?;
        match info.identity_manager.managed_identity_mut(owner_id) {
            Some(managed) => {
                managed.add_incoming_contact_request(contact_request, &persister);
            }
            None => {
                tracing::warn!(
                    owner_id = %owner_id,
                    "record_incoming_contact_request: managed identity not \
                     found; auto-establishment will depend on dashpay_sync",
                );
            }
        }
        Ok(())
    }

    /// Durably flush every registered wallet's buffered changesets to the
    /// upstream persister. Called before the one-time migration's
    /// strictly-last legacy-table DROP so the new persister is durable
    /// before any legacy data is destroyed.
    //
    // TODO(PROJ-007 T7): remove the legacy `single_key_wallet` table ONLY
    // via
    // `crate::backend_task::migration::finish_unwire::drop_legacy_single_key_table_when_safe`,
    // which runs the data-loss gate internally and aborts on error. A
    // password-protected legacy single-key row is encrypted under the
    // user's OLD password and has no other copy until T-SK-03 restores it —
    // dropping the table early is permanent key loss.
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

    /// Re-run the seedless watch-only load pass (idempotent). Exposed for
    /// the one-time migration engine and the reload-survival tests; the
    /// upstream `load_from_persistor` rebuilds each wallet watch-only and
    /// re-provisions identity funding accounts per loaded wallet.
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
    /// DET's snapshot. Use after registering watched tokens so a user-initiated
    /// "Refresh" reflects the latest balances without waiting for the
    /// background loop's next tick. A no-op pass (already syncing) still
    /// refreshes the snapshot from whatever state is current.
    pub async fn sync_token_balances_now(&self) {
        self.inner.pwm.identity_sync().sync_now().await;
        self.refresh_token_balances().await;
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

    /// Derive the secp256k1 [`PrivateKey`] at `path` from a held HD seed.
    /// Used after `create_asset_lock_proof` to obtain the one-time
    /// credit-output key needed to sign DET-retained non-identity state
    /// transitions (Platform-address top-up, shielded deposit). The seed is
    /// the one already held open by the surrounding `with_secret_session`
    /// scope, so this never re-prompts.
    fn derive_private_key_from_held(
        &self,
        plaintext: SecretPlaintext<'_>,
        path: &dash_sdk::dpp::key_wallet::bip32::DerivationPath,
    ) -> Result<dash_sdk::dpp::dashcore::PrivateKey, TaskError> {
        let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
        let xprv = path
            .derive_priv_ecdsa_for_master_seed(seed, self.inner.network)
            .map_err(|source| TaskError::WalletBackend {
                source: Box::new(platform_wallet::error::PlatformWalletError::KeyDerivation(
                    source.to_string(),
                )),
            })?;
        Ok(xprv.to_priv())
    }

    /// Test-only probe that the chokepoint can decrypt the seed for
    /// `seed_hash` without a prompt (the no-password / unprotected fast-path)
    /// AND that the resulting [`DetSigner`] actually produces a signature.
    /// Mirrors the production signing precondition so a regression on the
    /// no-password cold-boot path — decrypt or sign — is caught. The
    /// unprotected seed resolves with no interaction.
    #[cfg(test)]
    pub(crate) async fn assert_can_sign(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<(), TaskError> {
        use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
        use dash_sdk::dpp::key_wallet::signer::Signer;

        let scope = Self::hd_scope(seed_hash);
        let path: DerivationPath = "m/44'/1'/0'/0/0"
            .parse()
            .expect("static derivation path parses");
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let signer = DetSigner::from_held(session.plaintext(), self.inner.network);
                // Drive a real sign, not just signer construction: a derive or
                // sign regression must fail here.
                signer
                    .sign_ecdsa(&path, [0x11u8; 32])
                    .await
                    .map_err(|_| TaskError::SingleKeyCryptoFailure)?;
                Ok(())
            })
            .await
    }

    /// Build, sign, and broadcast a payment from the wallet's default BIP-44
    /// account to `recipients` (`(address, duffs)`). Returns the txid.
    ///
    /// One [`SecretAccess::with_secret_session`] scope wraps the whole build:
    /// the seed is decrypted just-in-time (one prompt for a passphrase-
    /// protected wallet, none for a no-password wallet), borrowed by the
    /// [`DetSigner`] for every input sign, and zeroized when the scope ends.
    pub async fn send_payment(
        &self,
        seed_hash: &WalletSeedHash,
        recipients: Vec<(dash_sdk::dpp::dashcore::Address, u64)>,
    ) -> Result<dash_sdk::dpp::dashcore::Txid, TaskError> {
        use dash_sdk::dpp::key_wallet::account::account_type::StandardAccountType;
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let signer = DetSigner::from_held(session.plaintext(), self.inner.network);
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
            })
            .await
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

        // One held-seed scope covers account provisioning, the funding-input
        // signer, and the credit-output key derivation, so the whole operation
        // prompts at most once and the seed zeroizes when the scope ends.
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                // Identity asset locks fund from the IdentityRegistration /
                // IdentityTopUp HD accounts, which the upstream persister never
                // reconstructs (a5538dc8). Provision them here — the single
                // chokepoint every asset-lock caller funnels through — so no
                // call site can bypass it. Idempotent. Non-identity funding
                // types are no-ops. Exhaustive — a new upstream variant must
                // force a review here instead of silently falling through.
                // Must run inside the session so the seed is available for
                // hardened xpub derivation (the live wallet is watch-only).
                match funding_type {
                    AssetLockFundingType::IdentityRegistration
                    | AssetLockFundingType::IdentityTopUp => {
                        let plaintext = session.plaintext();
                        let seed = plaintext
                            .expose_hd_seed()
                            .ok_or(TaskError::WalletStateInconsistent)?;
                        self.ensure_identity_funding_accounts(seed_hash, seed, identity_index)
                            .await?;
                    }
                    AssetLockFundingType::IdentityTopUpNotBound
                    | AssetLockFundingType::IdentityInvitation
                    | AssetLockFundingType::AssetLockAddressTopUp
                    | AssetLockFundingType::AssetLockShieldedAddressTopUp => {}
                }
                let signer = DetSigner::from_held(session.plaintext(), self.inner.network);
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
                let private_key =
                    self.derive_private_key_from_held(session.plaintext(), &credit_output_path)?;
                Ok((proof, private_key, out_point.txid))
            })
            .await
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
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                // Re-provisioning idempotent. Run inside the session so the
                // seed is available for hardened xpub derivation (the live
                // wallet is watch-only).
                let plaintext = session.plaintext();
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::WalletStateInconsistent)?;
                self.ensure_identity_funding_accounts(seed_hash, seed, identity_index)
                    .await?;
                let asset_lock_signer =
                    DetSigner::from_held(session.plaintext(), self.inner.network);
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
            })
            .await
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
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                // Run inside the session so the seed is available for
                // hardened xpub derivation (the live wallet is watch-only).
                let plaintext = session.plaintext();
                let seed = plaintext
                    .expose_hd_seed()
                    .ok_or(TaskError::WalletStateInconsistent)?;
                self.ensure_identity_funding_accounts(seed_hash, seed, identity_index)
                    .await?;
                let asset_lock_signer =
                    DetSigner::from_held(session.plaintext(), self.inner.network);
                let wallet = self.resolve_wallet(seed_hash).await?;
                wallet
                    .identity()
                    .top_up_identity_with_funding(
                        identity_id,
                        funding,
                        &asset_lock_signer,
                        settings,
                    )
                    .await
                    .map_err(|e| map_identity_top_up_error(*identity_id, e))
            })
            .await
    }

    /// Whether `address` is already revealed in this wallet's upstream
    /// platform-payment pool (account 0). This is the exact membership query the
    /// orchestrated `fund_from_asset_lock` pre-flight runs, so a `true` here
    /// means the orchestrator will accept the recipient. The pool holds only the
    /// wallet's own revealed platform addresses, so a `true` also implies
    /// ownership. No reveal side effect: an owned-but-unrevealed address reads
    /// `false`, and the caller falls back to the manual path.
    pub(crate) async fn platform_address_in_pool(
        &self,
        seed_hash: &WalletSeedHash,
        address: &dash_sdk::dpp::address_funds::PlatformAddress,
    ) -> Result<bool, TaskError> {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        use dash_sdk::dpp::key_wallet::PlatformP2PKHAddress;

        let PlatformAddress::P2pkh(hash) = address else {
            return Ok(false);
        };
        let p2pkh = PlatformP2PKHAddress::new(*hash);

        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let manager = wallet.wallet_manager().read().await;
        Ok(manager
            .get_wallet_info(&wallet_id)
            .and_then(|info| {
                info.core_wallet
                    .platform_payment_managed_account_at_index(0)
            })
            .is_some_and(|account| account.contains_platform_address(&p2pkh)))
    }

    /// Fund wallet-owned platform addresses from a Core asset lock through the
    /// upstream orchestration pipeline.
    ///
    /// Wraps `PlatformAddressWallet::fund_from_asset_lock` — upstream owns the
    /// full recovery pipeline: asset-lock build/broadcast (for
    /// `FromWalletBalance`), `submit_with_cl_height_retry`, the InstantSend →
    /// ChainLock fallback, and `consume_asset_lock` on acceptance (so the lock
    /// is never left reusable on an ambiguous failure).
    ///
    /// Callers must pass only destination addresses already revealed in this
    /// wallet's upstream platform-payment pool (gate with
    /// [`Self::platform_address_in_pool`]) — the orchestrator's pre-flight
    /// rejects any other recipient. The `address_signer` authorises per-output
    /// witnesses; the `asset_lock_signer` signs the outer state transition
    /// against the lock's credit-output key. Neither signer copies the seed —
    /// both borrow it from the held session.
    ///
    /// `platform_account_index` selects the platform-payment account (DET uses
    /// 0). The `addresses` map must contain exactly one `None`-amount entry (the
    /// remainder recipient); the lock is consumed in full.
    // Mirrors the upstream `fund_from_asset_lock` surface; each argument is a
    // distinct, required input to that orchestrator.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn fund_platform_address(
        &self,
        seed_hash: &WalletSeedHash,
        funding: platform_wallet::wallet::asset_lock::AssetLockFunding,
        platform_account_index: u32,
        addresses: std::collections::BTreeMap<
            dash_sdk::dpp::address_funds::PlatformAddress,
            Option<dash_sdk::dpp::balances::credits::Credits>,
        >,
        fee_strategy: dash_sdk::dpp::address_funds::AddressFundsFeeStrategy,
        path_index: &PlatformPathIndex,
        settings: Option<dash_sdk::platform::transition::put_settings::PutSettings>,
    ) -> Result<(), TaskError> {
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                let address_signer =
                    DetPlatformSigner::from_held(seed, self.inner.network, path_index);
                let asset_lock_signer =
                    DetSigner::from_held(session.plaintext(), self.inner.network);
                let wallet = self.resolve_wallet(seed_hash).await?;
                wallet
                    .platform()
                    .fund_from_asset_lock(
                        funding,
                        platform_account_index,
                        addresses,
                        fee_strategy,
                        &address_signer,
                        &asset_lock_signer,
                        settings,
                    )
                    .await
                    .map(|_changeset| ())
                    .map_err(map_platform_address_fund_error)
            })
            .await
    }

    // UPSTREAM GAP: rs-platform-wallet has no identity-funding-account
    // registrar (sibling to register_contact_account). Contained exception —
    // key_wallet plumbing lives ONLY here, never leaks past WalletBackend.
    // Do not replicate; do not collapse the dual-insert; do not use the
    // funds-bearing API. Tracked: upstream-contribution TODO 9cdcfb25.
    //
    // `peek_next_funding_address` reads BOTH `wallet.accounts.*` (xpub
    // source) AND `wallet_info.accounts.*` (mutable managed account), so the
    // account must exist in both collections. `load()` rebuilds
    // `IdentityRegistration` from the manifest; per-index
    // `IdentityTopUp{registration_index}` enters the manifest only after a
    // register/top-up, so a reloaded already-registered identity needs this
    // re-provision. Idempotent: probes both collections and no-ops if present
    // (no error-string parsing — direct membership checks).
    //
    // `seed` must be the wallet's HD seed so the hardened account xpub can be
    // derived — the live wallet is watch-only and cannot derive hardened paths
    // itself. Mirrors the pattern in `register_contact_receiving_accounts`.
    async fn provision_identity_funding_account(
        &self,
        seed_hash: &WalletSeedHash,
        seed: &[u8; 64],
        account_type: dash_sdk::dpp::key_wallet::AccountType,
    ) -> Result<(), TaskError> {
        use dash_sdk::dpp::key_wallet::AccountType;
        use dash_sdk::dpp::key_wallet::managed_account::ManagedCoreKeysAccount;
        use dash_sdk::dpp::key_wallet::wallet::Wallet as UpstreamWallet;
        use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
        use platform_wallet::error::PlatformWalletError;

        // Restrict to the two identity-funding flavours; everything else is a
        // misuse — keeping the match exhaustive forces a review if a new
        // upstream identity-funding variant appears.
        match account_type {
            AccountType::IdentityRegistration | AccountType::IdentityTopUp { .. } => {}
            _ => return Err(TaskError::UnsupportedIdentityFundingAccount),
        }

        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let mut wm = wallet.wallet_manager().write().await;
        let (kw, info) = wm
            .get_wallet_mut_and_info_mut(&wallet_id)
            .ok_or(TaskError::WalletStateInconsistent)?;

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
            // The live wallet is watch-only: calling `add_account(…, None)` would
            // try to derive a hardened path from an absent private key and fail
            // with "Watch-only wallet has no private key". Derive the xpub from a
            // short-lived seed wallet instead and pass it as `Some(xpub)`.
            let seed_wallet = UpstreamWallet::from_seed_bytes(
                *seed,
                self.inner.network,
                WalletAccountCreationOptions::Default,
            )
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(PlatformWalletError::WalletCreation(e.to_string())),
            })?;

            let path = account_type
                .derivation_path(self.inner.network)
                .map_err(|e| TaskError::WalletBackend {
                    source: Box::new(PlatformWalletError::WalletCreation(e.to_string())),
                })?;

            let account_xpub = seed_wallet.derive_extended_public_key(&path).map_err(|e| {
                TaskError::WalletBackend {
                    source: Box::new(PlatformWalletError::WalletCreation(e.to_string())),
                }
            })?;

            kw.add_account(account_type, Some(account_xpub))
                .map_err(|e| TaskError::WalletBackend {
                    source: Box::new(PlatformWalletError::AssetLockTransaction(e.to_string())),
                })?;
        }

        let derived = match account_type {
            AccountType::IdentityRegistration => kw.accounts.identity_registration.as_ref(),
            AccountType::IdentityTopUp { registration_index } => {
                kw.accounts.identity_topup.get(&registration_index)
            }
            _ => unreachable!("checked above"),
        }
        .ok_or(TaskError::WalletStateInconsistent)?;

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
    ///
    /// `seed` must be held for the duration of this call (obtained from
    /// `SecretPlaintext::expose_hd_seed` inside a `with_secret_session` scope).
    pub async fn ensure_identity_funding_accounts(
        &self,
        seed_hash: &WalletSeedHash,
        seed: &[u8; 64],
        registration_index: u32,
    ) -> Result<(), TaskError> {
        use dash_sdk::dpp::key_wallet::AccountType;
        self.provision_identity_funding_account(seed_hash, seed, AccountType::IdentityRegistration)
            .await?;
        self.provision_identity_funding_account(
            seed_hash,
            seed,
            AccountType::IdentityTopUp { registration_index },
        )
        .await
    }

    // ── Shielded-pool methods ──────────────────────────────────────────────────
    //
    // `platform-wallet` is always built with the `shielded` Cargo feature in
    // DET (added in Phase A).  No DET-level opt-out exists, so these methods
    // are unconditionally available.  The fund-moving ops and reads consumed by
    // `run_shielded_task` / `ensure_shielded_bound` are wired (Phase D); the
    // activity/notes list reads remain `#[allow(dead_code)]` until the Phase-F
    // upstream-coordinator read path lands.

    /// Resolve the network-scoped shielded coordinator.
    ///
    /// Returns `ShieldedNotConfigured` when `configure_shielded` was not called
    /// during backend construction (should never happen in practice — it is
    /// called unconditionally in `WalletBackend::new`).
    async fn shielded_coordinator_arc(
        &self,
    ) -> Result<
        std::sync::Arc<platform_wallet::wallet::shielded::NetworkShieldedCoordinator>,
        TaskError,
    > {
        self.inner
            .pwm
            .shielded_coordinator()
            .await
            .ok_or(TaskError::ShieldedNotConfigured)
    }

    /// Idempotently bind Orchard ZIP-32 keys for `seed_hash` to the shielded
    /// coordinator.  A no-op when the wallet is already bound; one call needed
    /// per wallet per process lifetime.
    ///
    /// Called from `bootstrap_wallet_addresses_jit` (Phase C-bind) inside the
    /// existing `with_secret_session` scope; also callable directly for the
    /// MCP headless path.
    pub(crate) async fn ensure_shielded_bound(
        &self,
        seed_hash: &WalletSeedHash,
        seed: &[u8],
    ) -> Result<(), TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        if wallet.is_shielded_bound().await {
            return Ok(());
        }
        let coordinator = self.shielded_coordinator_arc().await?;
        wallet
            .bind_shielded(seed, &[0], &coordinator)
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })
    }

    /// Bind Orchard keys for `seed_hash` by resolving its HD seed just-in-time
    /// through the [`SecretAccess`] chokepoint, then delegating to
    /// [`Self::ensure_shielded_bound`].
    ///
    /// The headless sibling of the Phase-C-bind call in
    /// `bootstrap_wallet_addresses_jit`: an unprotected wallet resolves
    /// prompt-free via the no-passphrase fast-path; a protected one needs its
    /// seed already promoted to the session cache. Idempotent — exits
    /// immediately when the wallet is already bound. The Phase-G `shielded_init`
    /// MCP tool uses this so an agent can bind a wallet without a GUI prompt.
    #[cfg(any(feature = "mcp", feature = "cli"))]
    pub(crate) async fn ensure_shielded_bound_jit(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<(), TaskError> {
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                self.ensure_shielded_bound(seed_hash, seed).await
            })
            .await
    }

    /// Warm the Orchard proving key so the first shielded spend does not block.
    ///
    /// The Halo2 proving key takes ~30 s to build on first use; this primes the
    /// process-global cache ahead of time. Runs on a blocking thread so the
    /// async runtime keeps serving other work during the build, and is
    /// idempotent — a second call returns immediately. Returns whether the key
    /// is ready afterwards (a `spawn_blocking` panic leaves it unbuilt → `false`).
    /// The Phase-G `shielded_init` MCP tool calls this during headless setup.
    #[cfg(any(feature = "mcp", feature = "cli"))]
    pub(crate) async fn warm_shielded_prover(&self) -> bool {
        tokio::task::spawn_blocking(|| {
            let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
            prover.warm_up();
            prover.is_ready()
        })
        .await
        .unwrap_or(false)
    }

    /// Fund the shielded pool from a Core asset lock through the upstream
    /// orchestrator pipeline.
    ///
    /// Mirrors `fund_platform_address` exactly: the `asset_lock_signer` is a
    /// `DetSigner` borrowed from the held JIT session, and the upstream method
    /// owns the full IS→CL fallback + `consume_asset_lock` path.  A single
    /// `(recipient, None)` entry passes the whole lock value (minus the flat
    /// shielded fee) to `recipient`.
    ///
    /// The Orchard prover is created internally via
    /// `CachedOrchardProver::new()` — callers do not supply a prover.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn shield_from_asset_lock(
        &self,
        seed_hash: &WalletSeedHash,
        funding: platform_wallet::wallet::asset_lock::AssetLockFunding,
        recipient: dash_sdk::dpp::address_funds::OrchardAddress,
        dummy_outputs: usize,
        settings: Option<dash_sdk::platform::transition::put_settings::PutSettings>,
    ) -> Result<(), TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let asset_lock_signer =
                    DetSigner::from_held(session.plaintext(), self.inner.network);
                let wallet = self.resolve_wallet(seed_hash).await?;
                let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
                wallet
                    .shielded_fund_from_asset_lock(
                        &coordinator,
                        funding,
                        vec![(recipient, None)],
                        &asset_lock_signer,
                        &prover,
                        None,
                        dummy_outputs,
                        settings,
                    )
                    .await
                    .map_err(map_shielded_op_error)
            })
            .await
    }

    /// Shield platform-address credits (Type 15) into the Orchard pool.
    ///
    /// The `signer` authorises the per-address `AddressWitness`; it is a
    /// `DetPlatformSigner` built from the held JIT seed and `path_index`.
    /// Build `path_index` via `PlatformPathIndex::from_wallet` before calling —
    /// the same pattern as `fund_platform_address`.
    ///
    /// The Orchard prover is created internally via
    /// `CachedOrchardProver::new()` — callers do not supply a prover.
    pub(crate) async fn shield_from_balance(
        &self,
        seed_hash: &WalletSeedHash,
        path_index: &PlatformPathIndex,
        shielded_account: u32,
        payment_account: u32,
        amount: u64,
    ) -> Result<(), TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let scope = Self::hd_scope(seed_hash);
        self.inner
            .secret_access
            .with_secret_session(&scope, async |session| {
                let plaintext = session.plaintext();
                let seed = plaintext.expose_hd_seed().ok_or(TaskError::WalletLocked)?;
                let signer = DetPlatformSigner::from_held(seed, self.inner.network, path_index);
                let wallet = self.resolve_wallet(seed_hash).await?;
                let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
                wallet
                    .shielded_shield_from_account(
                        &coordinator,
                        shielded_account,
                        payment_account,
                        amount,
                        &signer,
                        &prover,
                    )
                    .await
                    .map_err(map_shielded_op_error)
            })
            .await
    }

    /// Shielded → shielded transfer from `account`'s notes to `recipient`.
    ///
    /// No seed scope needed — the Orchard ASK is already resident in the
    /// wallet's bound `shielded_keys` slot from `ensure_shielded_bound`.
    ///
    /// The Orchard prover is created internally via
    /// `CachedOrchardProver::new()` — callers do not supply a prover.
    pub(crate) async fn shielded_transfer(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
        recipient_raw_43: &[u8; 43],
        amount: u64,
        memo: [u8; 36],
    ) -> Result<(), TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
        wallet
            .shielded_transfer_to(
                &coordinator,
                account,
                recipient_raw_43,
                amount,
                memo,
                &prover,
            )
            .await
            .map_err(map_shielded_op_error)
    }

    /// Unshield from `account`'s notes to a transparent platform address
    /// (bech32m `"dash1…"` / `"tdash1…"` string).
    ///
    /// No seed scope needed — keys are already bound.
    ///
    /// The Orchard prover is created internally via
    /// `CachedOrchardProver::new()` — callers do not supply a prover.
    pub(crate) async fn shielded_unshield(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
        to_platform_addr_bech32m: &str,
        amount: u64,
    ) -> Result<(), TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
        wallet
            .shielded_unshield_to(
                &coordinator,
                account,
                to_platform_addr_bech32m,
                amount,
                &prover,
            )
            .await
            .map_err(map_shielded_op_error)
    }

    /// Withdraw from `account`'s notes to a Core L1 address (Base58Check).
    ///
    /// No seed scope needed — keys are already bound.
    ///
    /// The Orchard prover is created internally via
    /// `CachedOrchardProver::new()` — callers do not supply a prover.
    pub(crate) async fn shielded_withdraw(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
        to_core_address: &str,
        amount: u64,
        core_fee_per_byte: u32,
    ) -> Result<(), TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let prover = platform_wallet::wallet::shielded::CachedOrchardProver::new();
        wallet
            .shielded_withdraw_to(
                &coordinator,
                account,
                to_core_address,
                amount,
                core_fee_per_byte,
                &prover,
            )
            .await
            .map_err(map_shielded_op_error)
    }

    /// Per-account unspent shielded balance for `seed_hash`'s wallet.
    ///
    /// Returns an empty map when the wallet is not bound or has no shielded
    /// balance.  This is the push-snapshot producer (Phase E): the result is
    /// written into `AppContext::shielded_balances` by `on_shielded_sync_completed`.
    pub(crate) async fn shielded_balances(
        &self,
        seed_hash: &WalletSeedHash,
    ) -> Result<std::collections::BTreeMap<u32, u64>, TaskError> {
        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        wallet
            .shielded_balances(&coordinator)
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })
    }

    /// The default Orchard payment address for `account` on `seed_hash`'s wallet
    /// (raw 43-byte representation).  Returns `None` if the wallet is not bound
    /// or `account` is not registered.
    pub async fn shielded_default_address(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
    ) -> Result<Option<[u8; 43]>, TaskError> {
        let wallet = self.resolve_wallet(seed_hash).await?;
        Ok(wallet.shielded_default_address(account).await)
    }

    /// A page of shielded activity for `account` on `seed_hash`'s wallet,
    /// sorted for display (pending first, then descending block height).
    ///
    /// `offset` / `limit` mirror the coordinator store's pagination contract.
    #[allow(dead_code)]
    pub(crate) async fn shielded_activity(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<platform_wallet::wallet::shielded::ShieldedActivityEntry>, TaskError> {
        use platform_wallet::wallet::shielded::{ShieldedStore, SubwalletId};

        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let subwallet = SubwalletId::new(wallet_id, account);

        coordinator
            .store()
            .read()
            .await
            .get_activity(subwallet, offset, limit)
            .map_err(|source| TaskError::ShieldedStoreReadFailed { source })
    }

    /// Unspent shielded notes for `account` on `seed_hash`'s wallet.
    ///
    /// Note: for spendability checks, prefer `shielded_balances`; this method
    /// exposes the raw note list for diagnostic and display purposes.
    #[allow(dead_code)]
    pub(crate) async fn shielded_notes(
        &self,
        seed_hash: &WalletSeedHash,
        account: u32,
    ) -> Result<Vec<platform_wallet::wallet::shielded::ShieldedNote>, TaskError> {
        use platform_wallet::wallet::shielded::{ShieldedStore, SubwalletId};

        let coordinator = self.shielded_coordinator_arc().await?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let subwallet = SubwalletId::new(wallet_id, account);

        coordinator
            .store()
            .read()
            .await
            .get_unspent_notes(subwallet)
            .map_err(|source| TaskError::ShieldedStoreReadFailed { source })
    }

    /// Force an immediate shielded sync pass (network-wide across every bound
    /// wallet on the coordinator).
    ///
    /// `sync_now` fires `on_shielded_sync_completed` synchronously before it
    /// returns, so the [`EventBridge`] has already written the post-sync
    /// per-wallet balances into `AppContext::shielded_balances` (Phase E) by the
    /// time this resolves — a subsequent `shielded_balance_credits` read sees
    /// the fresh figure. The 60-second background loop is the normal driver;
    /// this is the explicit-refresh primitive for the backend-e2e lifecycle test
    /// and the Phase-G `shielded_sync` MCP tool. A no-op when shielded support
    /// was never configured (empty coordinator → empty pass).
    pub async fn sync_shielded_now(&self, force: bool) {
        self.inner.pwm.shielded_sync_arc().sync_now(force).await;
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
                source: Box::new(e),
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

        // ShieldedSpendUnconfirmed handled by the pre-flight above — unreachable.
        P::ShieldedSpendUnconfirmed { .. } => unreachable!("handled by pre-flight"),

        // Every remaining variant → generic WalletBackend wrapper.
        other @ (P::WalletCreation(_)
        | P::RehydrationTopologyUnsupported { .. }
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
        | P::ShieldedBroadcastUnconfirmed { .. }
        | P::ShieldedSyncFailed(_)
        | P::ShieldedTreeUpdateFailed(_)
        | P::ShieldedStoreError(_)
        | P::ShieldedMerkleWitnessUnavailable(_)
        | P::ShieldedKeyDerivation(_)) => TaskError::WalletBackend {
            source: Box::new(other),
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
        IdentityOpErrorKind::Other => TaskError::WalletBackend {
            source: Box::new(e),
        },
    }
}

/// Map an upstream load-skip reason to its DET-opaque equivalent,
/// dropping any row-derived string so no upstream detail crosses the
/// seam.
fn persisted_load_skip_from_upstream(
    reason: &platform_wallet::manager::load_outcome::SkipReason,
) -> PersistedLoadSkip {
    use platform_wallet::manager::load_outcome::{CorruptKind, SkipReason};
    match reason {
        SkipReason::CorruptPersistedRow { kind } => match kind {
            CorruptKind::MissingManifest => PersistedLoadSkip::MissingManifest,
            CorruptKind::MalformedXpub => PersistedLoadSkip::MalformedXpub,
            CorruptKind::DecodeError(_) => PersistedLoadSkip::DecodeError,
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
        | P::ShieldedMerkleWitnessUnavailable(_)
        | P::ShieldedKeyDerivation(_)
        | P::ShieldedNotBound
        // Broadcast was accepted but its execution result is unconfirmed — the
        // op may already be on chain, so it is neither a rejection nor a
        // finality timeout. Bucket as Other; the upstream contract says the
        // caller must not re-submit (the next sync reconciles).
        | P::ShieldedBroadcastUnconfirmed { .. }
        | P::ShieldedSpendUnconfirmed { .. }
        | P::RehydrationTopologyUnsupported { .. } => IdentityOpErrorKind::Other,
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

    /// The upstream load-skip families map 1:1 onto the DET-opaque
    /// [`PersistedLoadSkip`] and drop the row-derived string so no
    /// upstream detail crosses the seam.
    #[test]
    fn skip_reason_maps_to_det_opaque_variants() {
        use platform_wallet::manager::load_outcome::{CorruptKind, SkipReason};
        let cases = [
            (
                CorruptKind::MissingManifest,
                PersistedLoadSkip::MissingManifest,
            ),
            (CorruptKind::MalformedXpub, PersistedLoadSkip::MalformedXpub),
            (
                CorruptKind::DecodeError("secret-ish detail".into()),
                PersistedLoadSkip::DecodeError,
            ),
        ];
        for (kind, expected) in cases {
            let reason = SkipReason::CorruptPersistedRow { kind };
            assert_eq!(persisted_load_skip_from_upstream(&reason), expected);
        }
    }

    /// The seedless bridge keys off the BIP44 account xpub. The DET
    /// account xpub (`Wallet::new_from_seed`) must equal the upstream
    /// account xpub for the SAME seed, so the watch-only wallet rebuilt
    /// from the persisted manifest resolves back to DET's seed hash.
    /// Locks the cryptographic invariant the
    /// [`WalletBackend::load_from_persistor_seedless`] gate relies on.
    #[test]
    fn bridge_account_xpub_matches_upstream_for_same_seed() {
        use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
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
        let up_xpub = up
            .accounts
            .all_accounts()
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
            .map(|a| a.account_xpub.encode().to_vec())
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
            w.accounts
                .all_accounts()
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
