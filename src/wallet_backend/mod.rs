//! The single wallet seam.
//!
//! `WalletBackend` wraps the upstream `PlatformWalletManager` and is the only
//! place `platform-wallet` types are allowed to live. Nothing upstream
//! (`PlatformWalletManager`, `PlatformWallet`, `WalletId`, `SqlitePersister`,
//! `WalletManager`) escapes this module — callers see DET-shaped methods and
//! DET-shaped results only (rust-best-practices M-DONT-LEAK-TYPES,
//! C-NEWTYPE-HIDE). `Clone` is `O(1)` via `Arc<Inner>` (M-SERVICES-CLONE);
//! the type is `Send + Sync`.
//!
//! P1 scope: skeleton + construction/event plumbing. It is NOT yet wired into
//! `AppContext` and the `BackendTask` dispatch still runs the P0.5 stubs —
//! P2 points the task arms here. See
//! `docs/ai-design/2026-05-18-platform-wallet-migration/backend-architecture.md`.

mod asset_lock_signer;
mod event_bridge;
mod loader;
mod snapshot;

pub use asset_lock_signer::AssetLockSignerError;
use asset_lock_signer::WalletAssetLockSigner;

pub use event_bridge::EventBridge;
pub use loader::{PersistedWalletLoader, SeedReregistrationLoader, WalletRegistration};
use snapshot::SnapshotStore;
pub use snapshot::{DetUtxo, DetWalletBalance, WalletSnapshot};

use std::path::Path;
use std::sync::Arc;

use dash_sdk::Sdk;
use dash_sdk::dash_spv::ClientConfig;
use dash_sdk::dash_spv::client::config::MempoolStrategy;
use dash_sdk::dash_spv::types::ValidationMode;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
use platform_wallet::manager::PlatformWalletManager;
use platform_wallet_storage::{SqlitePersister, SqlitePersisterConfig};

use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::connection_status::ConnectionStatus;
use crate::model::wallet::WalletSeedHash;
use crate::utils::egui_mpsc::SenderAsync;

/// The upstream persister DET consumes. Authored upstream (PR #3625) — DET
/// does not write its own persister (removal-inventory: consume, don't
/// reimplement).
type DetPersister = SqlitePersister;

/// Default BIP-44 account index for wallet receive/send operations. DET has
/// always operated account 0; multi-account support is out of P2 scope.
const DEFAULT_BIP44_ACCOUNT: u32 = 0;

/// DET-facing asset-lock funding selector. Hides the upstream
/// `AssetLockFundingType` (M-DONT-LEAK-TYPES).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetLockKind {
    /// Funds a new identity registration.
    IdentityRegistration,
    /// Tops up an existing identity.
    IdentityTopUp,
    /// Funds a Platform (DIP-17) address directly.
    PlatformAddressTopUp,
    /// Funds a shielded-pool deposit (ShieldFromAssetLock).
    Shielded,
}

impl AssetLockKind {
    /// Map to the upstream funding-account selector. All variants — including
    /// `Shielded` — resolve to an upstream `AssetLockFundingType`, so coin
    /// selection always runs against the upstream authoritative live UTXO set
    /// (no DET-side selection from a snapshot or legacy `Wallet.utxos`).
    fn funding_type(self) -> platform_wallet::AssetLockFundingType {
        use platform_wallet::AssetLockFundingType;
        match self {
            AssetLockKind::IdentityRegistration => AssetLockFundingType::IdentityRegistration,
            AssetLockKind::IdentityTopUp => AssetLockFundingType::IdentityTopUp,
            AssetLockKind::PlatformAddressTopUp => AssetLockFundingType::AssetLockAddressTopUp,
            AssetLockKind::Shielded => AssetLockFundingType::AssetLockShieldedAddressTopUp,
        }
    }
}

/// DET-facing selector for an identity funding HD account. Hides the upstream
/// `key_wallet::AccountType` (M-DONT-LEAK-TYPES).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityFundingAccount {
    /// The wallet-wide identity-registration funding account (singular).
    Registration,
    /// The per-identity top-up funding account, keyed by the identity's
    /// wallet HD registration index.
    TopUp { registration_index: u32 },
}

/// Upstream `WalletId` = `SHA256(root_xpub || root_chain_code)`, distinct
/// from DET's `WalletSeedHash` = `SHA256(seed_bytes)`. The map is the bridge:
/// populated once per wallet at registration, read by every DET-keyed call.
type WalletId = [u8; 32];

struct Inner {
    pwm: PlatformWalletManager<DetPersister>,
    loader: Arc<dyn PersistedWalletLoader>,
    /// Display-only snapshot store (balance/tx/utxo), pushed by the
    /// `EventBridge`. See [`snapshot`]. DISPLAY-ONLY — never feeds coin
    /// selection (A04 fund-safety gate).
    snapshots: Arc<SnapshotStore>,
    /// `WalletSeedHash` → upstream `WalletId`. See [`WalletId`].
    id_map: std::sync::RwLock<std::collections::BTreeMap<WalletSeedHash, WalletId>>,
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
        let spv_storage_dir = Self::spv_storage_dir(ctx.data_dir(), network)?;

        let persister_config =
            SqlitePersisterConfig::new(spv_storage_dir.join("platform-wallet.sqlite"));
        let persister = Arc::new(
            SqlitePersister::open(persister_config)
                .map_err(|source| TaskError::WalletStorage { source })?,
        );

        let snapshots = Arc::new(SnapshotStore::new());

        let bridge = Arc::new(EventBridge::new(
            connection_status,
            task_result_sender,
            Arc::clone(&snapshots),
        ));

        let pwm = PlatformWalletManager::new(sdk, persister, bridge);

        let peer = Self::spv_primary_peer_socket(ctx, network);

        let backend = Self {
            inner: Arc::new(Inner {
                pwm,
                loader,
                snapshots,
                id_map: std::sync::RwLock::new(std::collections::BTreeMap::new()),
                seeds: std::sync::RwLock::new(std::collections::BTreeMap::new()),
                peer,
                network,
                spv_storage_dir,
            }),
        };

        backend.register_persisted_wallets(ctx).await?;

        Ok(backend)
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
    pub async fn start(&self) -> Result<(), TaskError> {
        let config = self.build_client_config();

        self.inner.pwm.spv_arc().spawn_in_background(config);

        self.inner.pwm.platform_address_sync_arc().start();
        self.inner.pwm.identity_sync_arc().start();
        // The upstream shielded sync coordinator only exists when
        // `platform-wallet`'s `shielded` feature is enabled; DET enables only
        // `serde`, so there is no `shielded_sync_arc()` to start here.

        Ok(())
    }

    /// Stop all upstream background tasks. Idempotent.
    pub async fn shutdown(&self) {
        self.inner.pwm.shutdown().await;
    }

    /// Number of wallets currently registered with the backend.
    pub async fn wallet_count(&self) -> usize {
        self.inner.pwm.wallet_ids().await.len()
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
    /// upstream `AssetLockManager`. `kind` selects the funding derivation;
    /// `identity_index` is the funding-account derivation index (ignored for
    /// non-identity kinds). Returns the finalized asset-lock proof, its
    /// one-time credit-output private key (derived locally from the wallet
    /// seed at the path upstream selected), and the txid.
    ///
    /// For identity-funded asset locks (`IdentityRegistration` /
    /// `IdentityTopUp`) the upstream `IdentityWallet::*_with_funding`
    /// orchestrators submit the Platform-side state transition themselves
    /// and never expose a credit-output `PrivateKey` — use
    /// [`Self::register_identity`] / [`Self::top_up_identity`] instead.
    pub(crate) async fn create_asset_lock_proof(
        &self,
        seed_hash: &WalletSeedHash,
        amount_duffs: u64,
        kind: AssetLockKind,
        identity_index: u32,
    ) -> Result<
        (
            dash_sdk::dpp::prelude::AssetLockProof,
            dash_sdk::dpp::dashcore::PrivateKey,
            dash_sdk::dpp::dashcore::Txid,
        ),
        TaskError,
    > {
        // Identity asset locks fund from the IdentityRegistration /
        // IdentityTopUp HD accounts, which the upstream persister never
        // reconstructs (a5538dc8). Provision them here — the single
        // chokepoint every asset-lock caller funnels through — so no call
        // site can bypass it. Idempotent. Non-identity kinds are no-ops.
        match kind {
            AssetLockKind::IdentityRegistration | AssetLockKind::IdentityTopUp => {
                self.ensure_identity_funding_accounts(seed_hash, identity_index)
                    .await?;
            }
            AssetLockKind::PlatformAddressTopUp | AssetLockKind::Shielded => {}
        }

        let signer = self.signer_for(seed_hash)?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let funding_type = kind.funding_type();
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
    pub async fn register_identity(
        &self,
        seed_hash: &WalletSeedHash,
        identity_index: u32,
        amount_duffs: u64,
        keys_map: std::collections::BTreeMap<u32, dash_sdk::dpp::identity::IdentityPublicKey>,
        identity_signer: &crate::model::qualified_identity::QualifiedIdentity,
        settings: Option<dash_sdk::platform::transition::put_settings::PutSettings>,
    ) -> Result<dash_sdk::platform::Identity, TaskError> {
        use platform_wallet::wallet::asset_lock::AssetLockFunding;

        // Re-provisioning idempotent. Run here so the chokepoint protection
        // applies to upstream's signer-driven flow too.
        self.ensure_identity_funding_accounts(seed_hash, identity_index)
            .await?;

        let asset_lock_signer = self.signer_for(seed_hash)?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let funding = AssetLockFunding::FromWalletBalance {
            amount_duffs,
            account_index: DEFAULT_BIP44_ACCOUNT,
        };
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
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })
    }

    /// Top up an existing identity's credit balance from this wallet's
    /// UTXOs. Returns the post-top-up identity balance (credits).
    ///
    /// Wraps upstream `IdentityWallet::top_up_identity_with_funding` —
    /// upstream handles asset-lock build/broadcast, IS→CL fallback, the
    /// `TopUpIdentity` submission, and the asset-lock cleanup. The
    /// caller-side IS-proof-invalid fallback and `UnknownVersionError`
    /// retry are no longer needed.
    pub async fn top_up_identity(
        &self,
        seed_hash: &WalletSeedHash,
        identity_id: &dash_sdk::platform::Identifier,
        amount_duffs: u64,
        identity_index: u32,
        settings: Option<dash_sdk::platform::transition::put_settings::PutSettings>,
    ) -> Result<u64, TaskError> {
        use platform_wallet::wallet::asset_lock::AssetLockFunding;

        self.ensure_identity_funding_accounts(seed_hash, identity_index)
            .await?;

        let asset_lock_signer = self.signer_for(seed_hash)?;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let funding = AssetLockFunding::FromWalletBalance {
            amount_duffs,
            account_index: DEFAULT_BIP44_ACCOUNT,
        };
        wallet
            .identity()
            .top_up_identity_with_funding(identity_id, funding, &asset_lock_signer, settings)
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })
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
        account: IdentityFundingAccount,
    ) -> Result<(), TaskError> {
        use dash_sdk::dpp::key_wallet::AccountType;
        use dash_sdk::dpp::key_wallet::managed_account::ManagedCoreKeysAccount;

        let account_type = match account {
            IdentityFundingAccount::Registration => AccountType::IdentityRegistration,
            IdentityFundingAccount::TopUp { registration_index } => {
                AccountType::IdentityTopUp { registration_index }
            }
        };

        let wallet = self.resolve_wallet(seed_hash).await?;
        let wallet_id = wallet.wallet_id();
        let mut wm = wallet.wallet_manager().write().await;
        let (kw, info) = wm
            .get_wallet_mut_and_info_mut(&wallet_id)
            .ok_or(TaskError::WalletBackendNotYetWired)?;

        let in_wallet = match account {
            IdentityFundingAccount::Registration => kw.accounts.identity_registration.is_some(),
            IdentityFundingAccount::TopUp { registration_index } => {
                kw.accounts.identity_topup.contains_key(&registration_index)
            }
        };
        let in_managed = match account {
            IdentityFundingAccount::Registration => {
                info.core_wallet.accounts.identity_registration.is_some()
            }
            IdentityFundingAccount::TopUp { registration_index } => info
                .core_wallet
                .accounts
                .identity_topup
                .contains_key(&registration_index),
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

        let derived = match account {
            IdentityFundingAccount::Registration => kw.accounts.identity_registration.as_ref(),
            IdentityFundingAccount::TopUp { registration_index } => {
                kw.accounts.identity_topup.get(&registration_index)
            }
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
        self.provision_identity_funding_account(seed_hash, IdentityFundingAccount::Registration)
            .await?;
        self.provision_identity_funding_account(
            seed_hash,
            IdentityFundingAccount::TopUp { registration_index },
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

    fn spv_storage_dir(
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

#[cfg(test)]
mod tests {
    use super::*;
    use platform_wallet::AssetLockFundingType;

    /// I1: every asset-lock kind, including `Shielded`, resolves to an
    /// upstream funding type — coin selection is therefore always upstream-
    /// authoritative, never DET-side from a snapshot or legacy `Wallet.utxos`.
    #[test]
    fn asset_lock_kind_maps_to_upstream_funding_type() {
        assert_eq!(
            AssetLockKind::IdentityRegistration.funding_type(),
            AssetLockFundingType::IdentityRegistration
        );
        assert_eq!(
            AssetLockKind::IdentityTopUp.funding_type(),
            AssetLockFundingType::IdentityTopUp
        );
        assert_eq!(
            AssetLockKind::PlatformAddressTopUp.funding_type(),
            AssetLockFundingType::AssetLockAddressTopUp
        );
        assert_eq!(
            AssetLockKind::Shielded.funding_type(),
            AssetLockFundingType::AssetLockShieldedAddressTopUp
        );
    }
}
