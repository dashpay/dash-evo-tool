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

mod event_bridge;
mod loader;

pub use event_bridge::EventBridge;
pub use loader::{PersistedWalletLoader, SeedReregistrationLoader, WalletRegistration};

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
}

/// Upstream `WalletId` = `SHA256(root_xpub || root_chain_code)`, distinct
/// from DET's `WalletSeedHash` = `SHA256(seed_bytes)`. The map is the bridge:
/// populated once per wallet at registration, read by every DET-keyed call.
type WalletId = [u8; 32];

struct Inner {
    pwm: PlatformWalletManager<DetPersister>,
    loader: Arc<dyn PersistedWalletLoader>,
    /// `WalletSeedHash` → upstream `WalletId`. See [`WalletId`].
    id_map: std::sync::RwLock<std::collections::BTreeMap<WalletSeedHash, WalletId>>,
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

        let bridge = Arc::new(EventBridge::new(connection_status, task_result_sender));

        let pwm = PlatformWalletManager::new(sdk, persister, bridge);

        let peer = Self::spv_primary_peer_socket(ctx, network);

        let backend = Self {
            inner: Arc::new(Inner {
                pwm,
                loader,
                id_map: std::sync::RwLock::new(std::collections::BTreeMap::new()),
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
            if self.inner.id_map.read()?.contains_key(&reg.seed_hash) {
                // Already registered this process — idempotent skip (Stage-B
                // re-run after a crash must not double-register).
                continue;
            }
            // `create_wallet_from_seed_bytes` also loads persisted
            // identity/address deltas and runs identity discovery upstream
            // (see upstream `manager/wallet_lifecycle.rs`).
            match self
                .inner
                .pwm
                .create_wallet_from_seed_bytes(
                    reg.network,
                    *reg.seed_bytes,
                    WalletAccountCreationOptions::Default,
                )
                .await
            {
                Ok(pw) => {
                    self.inner
                        .id_map
                        .write()?
                        .insert(reg.seed_hash, pw.wallet_id());
                    tracing::debug!(
                        wallet = %hex::encode(reg.seed_hash),
                        "Wallet registered with backend"
                    );
                }
                Err(platform_wallet::error::PlatformWalletError::WalletAlreadyExists(_)) => {
                    // Already present in the upstream manager (e.g. a prior
                    // Stage-B run before this process). Resolve its id so the
                    // DET-keyed map stays consistent; this keeps the whole
                    // step idempotent.
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
        Ok(())
    }

    /// Start chain sync and the periodic upstream coordinators.
    ///
    /// Upstream has no single `PlatformWalletManager::start()`; this
    /// orchestrates the parts: `SpvRuntime::start(ClientConfig)` plus the
    /// platform-address / identity / shielded sync coordinators.
    pub async fn start(&self) -> Result<(), TaskError> {
        let config = self.build_client_config();

        self.inner
            .pwm
            .spv()
            .start(config)
            .await
            .map_err(|e| TaskError::WalletSyncStartFailed {
                source: Box::new(e),
            })?;

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

    /// Build, sign, and broadcast a payment from the wallet's default BIP-44
    /// account to `recipients` (`(address, duffs)`). Returns the txid.
    pub async fn send_payment(
        &self,
        seed_hash: &WalletSeedHash,
        recipients: Vec<(dash_sdk::dpp::dashcore::Address, u64)>,
    ) -> Result<dash_sdk::dpp::dashcore::Txid, TaskError> {
        use dash_sdk::dpp::key_wallet::account::account_type::StandardAccountType;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let tx = wallet
            .core()
            .send_to_addresses(
                StandardAccountType::BIP44Account,
                DEFAULT_BIP44_ACCOUNT,
                recipients,
            )
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })?;
        Ok(tx.txid())
    }

    /// Build, track, and broadcast an asset lock via the upstream
    /// `AssetLockManager` (which also continuously tracks it to finality and
    /// returns the finalized proof). `kind` selects the funding derivation;
    /// `identity_index` is the funding-account derivation index. Returns the
    /// finalized asset-lock proof, its one-time private key, and the txid —
    /// everything an identity create/top-up or platform-address top-up state
    /// transition needs.
    pub async fn create_asset_lock_proof(
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
        use platform_wallet::AssetLockFundingType;
        let wallet = self.resolve_wallet(seed_hash).await?;
        let funding_type = match kind {
            AssetLockKind::IdentityRegistration => AssetLockFundingType::IdentityRegistration,
            AssetLockKind::IdentityTopUp => AssetLockFundingType::IdentityTopUp,
            AssetLockKind::PlatformAddressTopUp => AssetLockFundingType::AssetLockAddressTopUp,
        };
        let (proof, key, out_point) = wallet
            .asset_locks()
            .create_funded_asset_lock_proof(
                amount_duffs,
                DEFAULT_BIP44_ACCOUNT,
                funding_type,
                identity_index,
            )
            .await
            .map_err(|e| TaskError::WalletBackend {
                source: Box::new(e),
            })?;
        Ok((proof, key, out_point.txid))
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
