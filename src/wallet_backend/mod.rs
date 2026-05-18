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
use crate::utils::egui_mpsc::SenderAsync;

/// The upstream persister DET consumes. Authored upstream (PR #3625) — DET
/// does not write its own persister (removal-inventory: consume, don't
/// reimplement).
type DetPersister = SqlitePersister;

struct Inner {
    pwm: PlatformWalletManager<DetPersister>,
    loader: Arc<dyn PersistedWalletLoader>,
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
            // `create_wallet_from_seed_bytes` also loads persisted
            // identity/address deltas and runs identity discovery upstream
            // (see upstream `manager/wallet_lifecycle.rs`).
            self.inner
                .pwm
                .create_wallet_from_seed_bytes(
                    reg.network,
                    *reg.seed_bytes,
                    WalletAccountCreationOptions::Default,
                )
                .await
                .map_err(|e| TaskError::WalletBackend {
                    source: Box::new(e),
                })?;
            tracing::debug!(
                wallet = %hex::encode(reg.seed_hash),
                "Wallet registered with backend"
            );
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

    /// Whether chain sync has not yet reached the tip.
    pub async fn is_syncing(&self) -> bool {
        match self.inner.pwm.spv().sync_progress().await {
            Some(p) => !p.is_synced(),
            None => false,
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
