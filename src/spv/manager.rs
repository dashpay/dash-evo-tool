use crate::app_dir::app_user_data_dir_path;
use crate::config::NetworkConfig;
use crate::utils::tasks::TaskManager;
use dash_sdk::dash_spv::network::MultiPeerNetworkManager;
use dash_sdk::dash_spv::storage::DiskStorageManager;
use dash_sdk::dash_spv::types::{DetailedSyncProgress, SyncProgress, ValidationMode, SpvEvent};
use dash_sdk::dash_spv::{ClientConfig, DashSpvClient};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::ManagedWalletInfo;
use dash_sdk::dpp::key_wallet_manager::wallet_manager::WalletManager;
use std::fmt;
use std::fs;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;
use tokio::sync::RwLock as AsyncRwLock;
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
        *runtime = Some(RuntimeHandle {
            stop: stop_token.clone(),
        });

        let manager = Arc::clone(self);
        let runtime_manager = Arc::clone(&manager);
        let global_cancel = self.subtasks.cancellation_token.clone();

        self.subtasks.spawn_sync(async move {
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

        // Subscribe to SPV detailed progress for live header sync updates
        if let Some(mut progress_rx) = client.take_progress_receiver() {
            let manager = Arc::clone(&self);
            let stop = stop_token.clone();
            let cancel = global_cancel.clone();
            self.subtasks.spawn_sync(async move {
                let mut seen_progress = false;
                loop {
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        _ = cancel.cancelled() => break,
                        msg = progress_rx.recv() => {
                            match msg {
                                Some(detailed) => {
                                    if !seen_progress { seen_progress = true; tracing::debug!("SPV progress: first DetailedSyncProgress received"); }
                                    manager.update_state(|state| {
                                        state.detailed_progress = Some(detailed.clone());
                                        state.last_updated = Some(SystemTime::now());
                                        // Status based on stage
                                        // Consider Running when complete; otherwise Syncing
                                        // If peer_best_height == 0, keep Syncing
                                        if detailed.percentage >= 100.0 || detailed.current_height >= detailed.peer_best_height {
                                            state.status = SpvStatus::Running;
                                        } else if matches!(state.status, SpvStatus::Starting | SpvStatus::Idle | SpvStatus::Stopped) {
                                            state.status = SpvStatus::Syncing;
                                        }
                                    });
                                }
                                None => break,
                            }
                        }
                    }
                }
            });
        } else {
            tracing::debug!("SPV progress channel not available; headers progress will not stream");
        }

        // Subscribe to SPV events for filters and other signals
        if let Some(mut events) = client.take_event_receiver() {
            let manager = Arc::clone(&self);
            let stop = stop_token.clone();
            let cancel = global_cancel.clone();
            self.subtasks.spawn_sync(async move {
                let mut seen_any = false;
                loop {
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        _ = cancel.cancelled() => break,
                        evt = events.recv() => {
                            match evt {
                                Some(other) => {
                                    // Log any other events at debug level to help diagnose
                                    tracing::debug!(?other, "SPV event observed");
                                }
                                None => break,
                            }
                        }
                    }
                }
            });
        } else {
            tracing::debug!("SPV events channel not available; UI will not receive event-driven progress");
        }

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
            .with_validation_mode(ValidationMode::Basic)
            .with_log_level("info");

        if self.network == Network::Devnet || self.network == Network::Regtest {
            if let Some(peer) = self.primary_peer_socket() {
                config.add_peer(peer);
                config = config.with_restrict_to_configured_peers(true);
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
