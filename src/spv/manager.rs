use crate::app_dir::app_user_data_dir_path;
use crate::config::NetworkConfig;
use crate::utils::tasks::TaskManager;
use dash_sdk::dash_spv::network::MultiPeerNetworkManager;
use dash_sdk::dash_spv::storage::DiskStorageManager;
use dash_sdk::dash_spv::types::{DetailedSyncProgress, SpvEvent, SyncProgress, ValidationMode};
use dash_sdk::dash_spv::{ClientConfig, DashSpvClient};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::managed_account_operations::ManagedAccountOperations;
use dash_sdk::dpp::key_wallet_manager::wallet_manager::{WalletError, WalletId, WalletManager};
// use dash_sdk::dpp::key_wallet::bip32::ExtendedPubKey; // not needed directly here
use crate::spv::wallet_bridge::{WatchOnlyAccount, WatchOnlyWalletAttachment};
use dash_sdk::dpp::key_wallet;
use dash_sdk::dpp::key_wallet::account::{AccountType, StandardAccountType};
use std::fmt;
use std::fs;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;
use tokio::sync::RwLock as AsyncRwLock;
use tokio::runtime::Runtime as TokioRuntime;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Preferred backend for Core-level operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreBackendMode {
    Rpc = 0,
    Spv = 1,
}

impl CoreBackendMode {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

impl Default for CoreBackendMode {
    fn default() -> Self {
        CoreBackendMode::Rpc
    }
}

impl From<u8> for CoreBackendMode {
    fn from(value: u8) -> Self {
        match value {
            1 => CoreBackendMode::Spv,
            _ => CoreBackendMode::Rpc,
        }
    }
}

/// High-level status of the SPV client runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpvStatus {
    Idle,
    Starting,
    Syncing,
    Running,
    Stopping,
    Stopped,
    Error,
}

impl SpvStatus {
    pub fn is_active(self) -> bool {
        matches!(
            self,
            SpvStatus::Starting | SpvStatus::Syncing | SpvStatus::Running | SpvStatus::Stopping
        )
    }
}

impl Default for SpvStatus {
    fn default() -> Self {
        SpvStatus::Idle
    }
}

/// Progress update emitted over events. This is a lightweight view used to
/// avoid borrowing the client while monitoring the network.
#[derive(Debug, Clone, Copy)]
pub struct EventSyncProgress {
    pub current_height: u32,
    pub target_height: u32,
    pub percentage: f32,
}

/// Snapshot of the SPV runtime state for UI consumption.
#[derive(Debug, Clone, Default)]
pub struct SpvStatusSnapshot {
    pub status: SpvStatus,
    pub sync_progress: Option<SyncProgress>,
    pub detailed_progress: Option<DetailedSyncProgress>,
    pub event_progress: Option<EventSyncProgress>,
    pub last_error: Option<String>,
    pub started_at: Option<SystemTime>,
    pub last_updated: Option<SystemTime>,
}

#[derive(Debug, Clone)]
struct InternalState {
    status: SpvStatus,
    sync_progress: Option<SyncProgress>,
    detailed_progress: Option<DetailedSyncProgress>,
    event_progress: Option<EventSyncProgress>,
    last_error: Option<String>,
    started_at: Option<SystemTime>,
    last_updated: Option<SystemTime>,
}

impl Default for InternalState {
    fn default() -> Self {
        Self {
            status: SpvStatus::Idle,
            sync_progress: None,
            detailed_progress: None,
            event_progress: None,
            last_error: None,
            started_at: None,
            last_updated: None,
        }
    }
}

struct RuntimeHandle {
    stop: CancellationToken,
}

/// Manages SPV client lifecycle and exposes status updates.
pub struct SpvManager {
    network: Network,
    data_dir: PathBuf,
    config: Arc<RwLock<NetworkConfig>>,
    subtasks: Arc<TaskManager>,
    wallet: Arc<AsyncRwLock<WalletManager<ManagedWalletInfo>>>,
    state: Arc<RwLock<InternalState>>,
    runtime: Mutex<Option<RuntimeHandle>>,
    // mapping DET wallet seed_hash -> SPV wallet identifier (if created)
    det_wallets: Arc<RwLock<std::collections::BTreeMap<[u8; 32], WalletId>>>,
    // signal channel to trigger external reconcile on wallet-related events
    reconcile_tx: Mutex<Option<mpsc::Sender<()>>>,
    // Dedicated Tokio runtime for SPV network loop (isolated from UI/runtime contention)
    spv_runtime: Mutex<Option<TokioRuntime>>,
}

impl SpvManager {
    pub fn new(
        network: Network,
        config: Arc<RwLock<NetworkConfig>>,
        subtasks: Arc<TaskManager>,
    ) -> Result<Arc<Self>, String> {
        let cfg = config.read().map_err(|e| e.to_string())?;
        let data_dir = build_spv_data_dir(network, &cfg)?;
        drop(cfg);
        fs::create_dir_all(&data_dir).map_err(|e| format!("Failed to create SPV data dir: {e}"))?;

        let manager = Arc::new(Self {
            network,
            data_dir,
            config,
            subtasks,
            wallet: Arc::new(AsyncRwLock::new(WalletManager::<ManagedWalletInfo>::new())),
            state: Arc::new(RwLock::new(InternalState::default())),
            runtime: Mutex::new(None),
            det_wallets: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
            reconcile_tx: Mutex::new(None),
            spv_runtime: Mutex::new(None),
        });

        Ok(manager)
    }

    pub fn status(&self) -> SpvStatusSnapshot {
        let state = self.state.read().expect("SPV state lock poisoned");
        SpvStatusSnapshot {
            status: state.status,
            sync_progress: state.sync_progress.clone(),
            detailed_progress: state.detailed_progress.clone(),
            event_progress: state.event_progress,
            last_error: state.last_error.clone(),
            started_at: state.started_at,
            last_updated: state.last_updated,
        }
    }

    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        let mut runtime = self.runtime.lock().expect("SPV runtime lock poisoned");
        if runtime.is_some() {
            return Ok(());
        }

        self.update_state(|state| {
            state.status = SpvStatus::Starting;
            state.last_error = None;
            state.started_at = Some(SystemTime::now());
            state.last_updated = state.started_at;
        });

        let stop_token = CancellationToken::new();
        *runtime = Some(RuntimeHandle { stop: stop_token.clone() });

        let manager = Arc::clone(self);
        let runtime_manager = Arc::clone(&manager);
        let global_cancel = self.subtasks.cancellation_token.clone();

        // Spawn a dedicated OS thread with a multi-thread Tokio runtime
        std::thread::Builder::new()
            .name("spv".to_string())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(4)
                    .enable_all()
                    .thread_name("spv-rt")
                    .build()
                    .expect("Failed to create SPV runtime");

                rt.block_on(async move {
                    if let Err(err) = runtime_manager.run_loop(stop_token, global_cancel).await {
                        tracing::error!(error = %err, network = ?manager.network, "SPV runtime failed");
                        manager.update_state(|state| {
                            state.status = SpvStatus::Error;
                            state.last_error = Some(err.clone());
                            state.last_updated = Some(SystemTime::now());
                        });
                    }

                    let mut runtime = manager.runtime.lock().expect("SPV runtime lock poisoned");
                    *runtime = None;
                });
            })
            .map_err(|e| format!("Failed to spawn SPV thread: {e}"))?;

        Ok(())
    }

    pub fn stop(&self) {
        let runtime = self.runtime.lock().expect("SPV runtime lock poisoned");
        if let Some(handle) = runtime.as_ref() {
            self.update_state(|state| {
                state.status = SpvStatus::Stopping;
                state.last_updated = Some(SystemTime::now());
            });
            handle.stop.cancel();
        }
    }

    pub fn wallet(&self) -> Arc<AsyncRwLock<WalletManager<ManagedWalletInfo>>> {
        Arc::clone(&self.wallet)
    }

    pub fn det_wallets_snapshot(&self) -> std::collections::BTreeMap<[u8; 32], WalletId> {
        self.det_wallets
            .read()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// Create a reconciliation signal channel for external listeners.
    /// Returns a receiver that will get a signal when SPV wallet state likely changed.
    pub fn register_reconcile_channel(&self) -> mpsc::Receiver<()> {
        let (tx, rx) = mpsc::channel(64);
        let mut guard = self.reconcile_tx.lock().expect("reconcile_tx poisoned");
        *guard = Some(tx);
        rx
    }

    /// Attach watch-only accounts (e.g., BIP44 account 0 xpubs) from DET into the SPV wallet manager.
    /// This stores a mapping locally and attempts to prepare the SPV wallet manager state when possible.
    pub async fn attach_watch_only_accounts(
        &self,
        attachment: WatchOnlyWalletAttachment,
    ) -> Result<(), String> {
        // Map dashcore::Network to key_wallet::Network (they are identical variants).
        fn to_wallet_network(n: Network) -> key_wallet::Network {
            match n {
                Network::Dash => key_wallet::Network::Dash,
                Network::Testnet => key_wallet::Network::Testnet,
                Network::Devnet => key_wallet::Network::Devnet,
                Network::Regtest => key_wallet::Network::Regtest,
                other => {
                    // Fallback: treat unknown as Dash (shouldn't happen for standard nets)
                    tracing::warn!(
                        ?other,
                        "Unknown dashcore::Network; defaulting to Dash for wallet network mapping"
                    );
                    key_wallet::Network::Dash
                }
            }
        }

        let net_wallet = to_wallet_network(attachment.network);

        let mut wm = self.wallet.write().await;
        let mut map = self.det_wallets.write().map_err(|e| e.to_string())?;

        for WatchOnlyAccount {
            seed_hash,
            xpub,
            account_index,
        } in attachment.accounts
        {
            let xpub_str = xpub.to_string();

            // 1) Ensure a watch-only wallet exists for this xpub
            let wallet_id = match wm.import_wallet_from_xpub(&xpub_str, net_wallet, false) {
                Ok(id) => id,
                Err(WalletError::WalletExists(id)) => id,
                Err(e) => {
                    tracing::error!(?e, seed = %hex::encode(seed_hash), "import_wallet_from_xpub failed");
                    return Err(format!("import_wallet_from_xpub failed: {e}"));
                }
            };

            // 2) Ensure BIP44 account exists on the Wallet (backed by xpub)
            let acct_type = AccountType::Standard {
                index: account_index,
                standard_account_type: StandardAccountType::BIP44Account,
            };
            if let Err(e) = wm.create_account(&wallet_id, acct_type, net_wallet, Some(xpub)) {
                match e {
                    WalletError::AccountCreation(msg) if msg.contains("already exists") => {
                        // Ignore duplicates
                    }
                    other => {
                        tracing::error!(?other, seed = %hex::encode(seed_hash), "create_account failed");
                        return Err(format!("create_account failed: {other}"));
                    }
                }
            }

            // 3) Add ManagedAccount so filters have monitored addresses (idempotent)
            if let Some(info) = wm.get_wallet_info_mut(&wallet_id) {
                let _ = info.add_managed_account_from_xpub(acct_type, net_wallet, xpub);
            } else {
                return Err(format!("wallet_info not found for wallet_id {wallet_id:?}"));
            }

            // Store mapping of DET seed_hash to WalletId
            map.insert(seed_hash, wallet_id);
        }

        // Optional: log monitored addresses count for debugging
        let addr_count = wm.monitored_addresses(attachment.network).len();
        tracing::info!(addresses = addr_count, network = ?attachment.network, "SPV now watching addresses");

        Ok(())
    }

    fn update_state<F>(&self, mut f: F)
    where
        F: FnMut(&mut InternalState),
    {
        let mut state = self.state.write().expect("SPV state lock poisoned");
        f(&mut state);
    }

    async fn run_loop(
        self: Arc<Self>,
        stop_token: CancellationToken,
        global_cancel: CancellationToken,
    ) -> Result<(), String> {
        let mut client = self.build_client().await?;

        self.update_state(|state| {
            state.status = SpvStatus::Syncing;
            state.last_error = None;
            state.last_updated = Some(SystemTime::now());
        });

        client
            .start()
            .await
            .map_err(|e| format!("SPV start failed: {e}"))?;

        // Wait for at least one peer to connect before attempting sync
        // This mirrors the CLI flow and reduces startup churn
        {
            let mut waited_ms: u64 = 0;
            loop {
                // Respect stop/cancel while waiting for peers
                if stop_token.is_cancelled() || global_cancel.is_cancelled() {
                    return Ok(());
                }
                let peers = client.get_peer_count().await;
                if peers > 0 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                waited_ms = waited_ms.saturating_add(200);
                if waited_ms % 5000 == 0 {
                    tracing::info!("SPV waiting for peers... {}s elapsed", waited_ms / 1000);
                }
            }
        }

        match client.sync_to_tip().await {
            Ok(progress) => {
                println!("Sync progress: {:?}", progress);
                self.update_state(|state| {
                    state.sync_progress = Some(progress.clone());
                    state.last_updated = Some(SystemTime::now());
                    if progress.headers_synced && progress.filter_headers_synced {
                        state.status = SpvStatus::Running;
                    }
                });
            }
            Err(err) => {
                let _ = client.stop().await;
                self.update_state(|state| {
                    state.status = SpvStatus::Error;
                    state.last_error = Some(format!("Initial sync failed: {err}"));
                    state.last_updated = Some(SystemTime::now());
                });
                return Err(format!("Initial sync failed: {err}"));
            }
        }

        // NOTE: To minimize contention during catch-up, we defer wiring progress/event consumers
        // until after monitor_network completes or the client reports Running status via state.
        // This mirrors the CLI's prioritization of the monitor loop.

        enum MonitorOutcome {
            Completed(Result<(), dash_sdk::dash_spv::SpvError>),
            StopRequested,
            GlobalCancelled,
        }

        let outcome = {
            #[allow(unused_mut)]
            let mut monitor_future = client.monitor_network();
            tokio::pin!(monitor_future);

            tokio::select! {
                result = &mut monitor_future => MonitorOutcome::Completed(result),
                _ = stop_token.cancelled() => MonitorOutcome::StopRequested,
                _ = global_cancel.cancelled() => MonitorOutcome::GlobalCancelled,
            }
        };

        match outcome {
            MonitorOutcome::Completed(Ok(())) => {
                let _ = client.stop().await;
                self.update_state(|state| {
                    state.status = SpvStatus::Stopped;
                    state.last_updated = Some(SystemTime::now());
                });
                Ok(())
            }
            MonitorOutcome::Completed(Err(err)) => {
                let _ = client.stop().await;
                let message = format!("monitor_network failed: {err}");
                self.update_state(|state| {
                    state.status = SpvStatus::Error;
                    state.last_error = Some(message.clone());
                    state.last_updated = Some(SystemTime::now());
                });
                Err(message)
            }
            MonitorOutcome::StopRequested | MonitorOutcome::GlobalCancelled => {
                let _ = client.stop().await;
                self.update_state(|state| {
                    state.status = SpvStatus::Stopped;
                    state.last_updated = Some(SystemTime::now());
                });
                Ok(())
            }
        }
    }

    async fn build_client(
        &self,
    ) -> Result<
        DashSpvClient<
            WalletManager<ManagedWalletInfo>,
            MultiPeerNetworkManager,
            DiskStorageManager,
        >,
        String,
    > {
        let mut config = ClientConfig::new(self.network)
            .with_storage_path(self.data_dir.clone())
            .with_validation_mode(ValidationMode::Full)
            // Start from the latest built-in checkpoint instead of genesis
            // (effective only when storage is empty / first initialization)
            .with_start_height(u32::MAX);

        // Pin peers when running against local nodes to avoid random peers.
        if self.network == Network::Devnet || self.network == Network::Regtest {
            if let Some(peer) = self.primary_peer_socket() {
                config.add_peer(peer);
            }
        } else if self.network == Network::Testnet {
            // For testnet testing, connect only to local Dash Core at 127.0.0.1:19999
            if let Ok(mut it) = "127.0.0.1:19999".to_socket_addrs() {
                if let Some(peer) = it.next() {
                    config.add_peer(peer);
                }
            }
        }

        let network_manager = MultiPeerNetworkManager::new(&config)
            .await
            .map_err(|e| format!("Failed to initialize SPV network manager: {e}"))?;

        let storage_manager = DiskStorageManager::new(self.data_dir.clone())
            .await
            .map_err(|e| format!("Failed to initialize SPV storage: {e}"))?;

        DashSpvClient::new(
            config,
            network_manager,
            storage_manager,
            Arc::clone(&self.wallet),
        )
        .await
        .map_err(|e| format!("Failed to create SPV client: {e}"))
    }

    fn primary_peer_socket(&self) -> Option<std::net::SocketAddr> {
        let config = self.config.read().ok()?;

        let host = config.core_host.as_str();
        let port = match self.network {
            Network::Dash => 9999,
            Network::Testnet => 19999,
            Network::Devnet => 20001,
            Network::Regtest => 19899,
            _ => 9999,
        };

        let addr = format!("{}:{}", host, port);
        addr.to_socket_addrs().ok()?.next()
    }
}

fn build_spv_data_dir(network: Network, config: &NetworkConfig) -> Result<PathBuf, String> {
    let mut base = app_user_data_dir_path().map_err(|e| e.to_string())?;
    base.push("spv");
    fs::create_dir_all(&base).map_err(|e| format!("Failed to create SPV base dir: {e}"))?;

    let network_dir = match network {
        Network::Dash => "mainnet".to_string(),
        Network::Testnet => "testnet".to_string(),
        Network::Devnet => config
            .devnet_name
            .clone()
            .unwrap_or_else(|| "devnet".to_string()),
        Network::Regtest => "regtest".to_string(),
        other => format!("{other:?}"),
    };

    Ok(base.join(network_dir))
}

impl From<&InternalState> for SpvStatusSnapshot {
    fn from(value: &InternalState) -> Self {
        Self {
            status: value.status,
            sync_progress: value.sync_progress.clone(),
            detailed_progress: value.detailed_progress.clone(),
            event_progress: value.event_progress,
            last_error: value.last_error.clone(),
            started_at: value.started_at,
            last_updated: value.last_updated,
        }
    }
}

impl fmt::Debug for SpvManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpvManager")
            .field("network", &self.network)
            .field("data_dir", &self.data_dir)
            .finish()
    }
}
