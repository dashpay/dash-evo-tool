use super::error::{SpvError, SpvResult};
use crate::app_dir::app_user_data_dir_path;
use crate::config::NetworkConfig;
use crate::model::wallet::WalletSeedHash;
use crate::utils::tasks::TaskManager;
use dash_sdk::dash_spv::client::interface::{DashSpvClientCommand, DashSpvClientInterface};
use dash_sdk::dash_spv::network::NetworkEvent;
use dash_sdk::dash_spv::network::PeerNetworkManager;
use dash_sdk::dash_spv::storage::DiskStorageManager;
use dash_sdk::dash_spv::sync::SyncEvent;
use dash_sdk::dash_spv::sync::SyncProgress as WatchSyncProgress;
use dash_sdk::dash_spv::sync::SyncState as WatchSyncState;
use dash_sdk::dash_spv::types::{DetailedSyncProgress, SyncProgress, SyncStage, ValidationMode};
use dash_sdk::dash_spv::{ClientConfig, DashSpvClient, Hash, LLMQType, QuorumHash};
use dash_sdk::dpp::dashcore::{Address, InstantLock, Network, Transaction, Txid};
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, ExtendedPrivKey};
use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::{
    ManagedWalletInfo, transaction_building::AccountTypePreference,
    wallet_info_interface::WalletInfoInterface,
};
use dash_sdk::dpp::key_wallet_manager::WalletEvent;
use dash_sdk::dpp::key_wallet_manager::wallet_interface::WalletInterface;
use dash_sdk::dpp::key_wallet_manager::wallet_manager::{WalletError, WalletId, WalletManager};
// use dash_sdk::dpp::key_wallet::bip32::ExtendedPubKey; // not needed directly here
use std::fmt;
use std::fs;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;
use tokio::sync::RwLock as AsyncRwLock;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

/// Preferred backend for Core-level operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoreBackendMode {
    #[default]
    Rpc = 0,
    Spv = 1,
}

impl CoreBackendMode {
    pub fn as_u8(self) -> u8 {
        self as u8
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpvStatus {
    #[default]
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

/// Snapshot of the SPV runtime state for UI consumption.
/// Uses dash-spv's built-in progress types directly instead of duplicating.
#[derive(Debug, Clone, Default)]
pub struct SpvStatusSnapshot {
    pub status: SpvStatus,
    pub sync_progress: Option<SyncProgress>,
    pub detailed_progress: Option<DetailedSyncProgress>,
    pub last_error: Option<String>,
    pub started_at: Option<SystemTime>,
    pub last_updated: Option<SystemTime>,
    pub connected_peers: usize,
}

/// Type alias for the SPV client with our specific configuration
type SpvClient =
    DashSpvClient<WalletManager<ManagedWalletInfo>, PeerNetworkManager, DiskStorageManager>;

/// Events forwarded from SPV to AppContext for asset lock proof construction.
pub(crate) enum AssetLockFinalityEvent {
    InstantLock {
        txid: Txid,
        instant_lock: Box<InstantLock>,
    },
    ChainLock {
        height: u32,
    },
}

/// Manages SPV client lifecycle and exposes status updates.
/// Uses dash-spv's built-in state management while maintaining a dedicated runtime for performance.
///
/// The client itself is owned by the background runtime thread and accessed through
/// its internally-shared components (wallet, storage, etc.) rather than through additional locking.
pub struct SpvManager {
    network: Network,
    data_dir: PathBuf,
    config: Arc<RwLock<NetworkConfig>>,
    subtasks: Arc<TaskManager>,
    wallet: Arc<AsyncRwLock<WalletManager<ManagedWalletInfo>>>,
    // Storage manager for direct access to SPV data (shared component from client)
    storage: Arc<Mutex<Option<Arc<tokio::sync::Mutex<DiskStorageManager>>>>>,
    // Interface for sending commands to the running SPV client (quorum lookups, etc.)
    client_interface: Arc<RwLock<Option<DashSpvClientInterface>>>,
    status: Arc<RwLock<SpvStatus>>,
    last_error: Arc<RwLock<Option<String>>>,
    started_at: Arc<RwLock<Option<SystemTime>>>,
    sync_progress_state: Arc<RwLock<Option<SyncProgress>>>,
    detailed_progress_state: Arc<RwLock<Option<DetailedSyncProgress>>>,
    progress_updated_at: Arc<RwLock<Option<SystemTime>>>,
    // mapping DET wallet seed_hash -> SPV wallet identifier (if created)
    det_wallets: Arc<RwLock<std::collections::BTreeMap<[u8; 32], WalletId>>>,
    // signal channel to trigger external reconcile on wallet-related events
    reconcile_tx: Mutex<Option<mpsc::Sender<()>>>,
    // signal channel to forward instant lock / chain lock events for asset lock proof construction
    finality_tx: Mutex<Option<mpsc::Sender<AssetLockFinalityEvent>>>,
    // Whether to use local Dash Core node instead of DNS seed discovery
    use_local_node: Arc<std::sync::atomic::AtomicBool>,
    // Cancellation token for clean shutdown
    stop_token: Mutex<Option<CancellationToken>>,
    // Channel to send requests to the SPV runtime thread
    request_tx: Mutex<Option<mpsc::Sender<SpvRequest>>>,
    // Network manager clone for broadcasting transactions (set when client is running)
    network_manager: Arc<AsyncRwLock<Option<PeerNetworkManager>>>,
    // Number of currently connected SPV peers
    connected_peers: Arc<RwLock<usize>>,
}

/// Requests that can be sent to the SPV runtime thread
///
/// Note: These requests are handled in the same async context where the client lives,
/// allowing direct access to client methods without additional locking overhead.
enum SpvRequest {
    BroadcastTransaction {
        tx: Box<Transaction>,
        response_tx: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
}

#[derive(Debug, Clone)]
pub struct SpvDerivedAddress {
    pub address: Address,
    pub derivation_path: DerivationPath,
}

impl SpvManager {
    // ==================== Lock Helper Methods ====================
    // These methods provide safe access to locks with proper error handling
    // instead of panicking on lock poisoning.

    fn read_status(&self) -> SpvResult<SpvStatus> {
        self.status
            .read()
            .map(|g| *g)
            .map_err(|_| SpvError::LockPoisoned("status".into()))
    }

    fn write_status(&self, value: SpvStatus) -> SpvResult<()> {
        let mut guard = self
            .status
            .write()
            .map_err(|_| SpvError::LockPoisoned("status".into()))?;
        *guard = value;
        Ok(())
    }

    fn read_last_error(&self) -> SpvResult<Option<String>> {
        self.last_error
            .read()
            .map(|g| g.clone())
            .map_err(|_| SpvError::LockPoisoned("last_error".into()))
    }

    fn write_last_error(&self, value: Option<String>) -> SpvResult<()> {
        let mut guard = self
            .last_error
            .write()
            .map_err(|_| SpvError::LockPoisoned("last_error".into()))?;
        *guard = value;
        Ok(())
    }

    fn read_started_at(&self) -> SpvResult<Option<SystemTime>> {
        self.started_at
            .read()
            .map(|g| *g)
            .map_err(|_| SpvError::LockPoisoned("started_at".into()))
    }

    fn write_started_at(&self, value: Option<SystemTime>) -> SpvResult<()> {
        let mut guard = self
            .started_at
            .write()
            .map_err(|_| SpvError::LockPoisoned("started_at".into()))?;
        *guard = value;
        Ok(())
    }

    fn read_sync_progress(&self) -> SpvResult<Option<SyncProgress>> {
        self.sync_progress_state
            .read()
            .map(|g| g.clone())
            .map_err(|_| SpvError::LockPoisoned("sync_progress".into()))
    }

    fn write_sync_progress(&self, value: Option<SyncProgress>) -> SpvResult<()> {
        let mut guard = self
            .sync_progress_state
            .write()
            .map_err(|_| SpvError::LockPoisoned("sync_progress".into()))?;
        *guard = value;
        Ok(())
    }

    fn read_detailed_progress(&self) -> SpvResult<Option<DetailedSyncProgress>> {
        self.detailed_progress_state
            .read()
            .map(|g| g.clone())
            .map_err(|_| SpvError::LockPoisoned("detailed_progress".into()))
    }

    fn write_detailed_progress(&self, value: Option<DetailedSyncProgress>) -> SpvResult<()> {
        let mut guard = self
            .detailed_progress_state
            .write()
            .map_err(|_| SpvError::LockPoisoned("detailed_progress".into()))?;
        *guard = value;
        Ok(())
    }

    fn read_progress_updated_at(&self) -> SpvResult<Option<SystemTime>> {
        self.progress_updated_at
            .read()
            .map(|g| *g)
            .map_err(|_| SpvError::LockPoisoned("progress_updated_at".into()))
    }

    fn write_progress_updated_at(&self, value: Option<SystemTime>) -> SpvResult<()> {
        let mut guard = self
            .progress_updated_at
            .write()
            .map_err(|_| SpvError::LockPoisoned("progress_updated_at".into()))?;
        *guard = value;
        Ok(())
    }

    // ==================== Public API ====================

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
            wallet: Arc::new(AsyncRwLock::new(WalletManager::<ManagedWalletInfo>::new(
                network,
            ))),
            storage: Arc::new(Mutex::new(None)),
            client_interface: Arc::new(RwLock::new(None)),
            status: Arc::new(RwLock::new(SpvStatus::Idle)),
            last_error: Arc::new(RwLock::new(None)),
            started_at: Arc::new(RwLock::new(None)),
            sync_progress_state: Arc::new(RwLock::new(None)),
            detailed_progress_state: Arc::new(RwLock::new(None)),
            progress_updated_at: Arc::new(RwLock::new(None)),
            det_wallets: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
            reconcile_tx: Mutex::new(None),
            finality_tx: Mutex::new(None),
            use_local_node: Arc::new(AtomicBool::new(false)),
            stop_token: Mutex::new(None),
            request_tx: Mutex::new(None),
            network_manager: Arc::new(AsyncRwLock::new(None)),
            connected_peers: Arc::new(RwLock::new(0)),
        });

        Ok(manager)
    }

    /// Set whether to use local Dash Core node for SPV sync instead of DNS seed discovery.
    /// Note: This only takes effect when starting a new SPV sync session.
    pub fn set_use_local_node(&self, use_local: bool) {
        self.use_local_node.store(use_local, Ordering::SeqCst);
    }

    /// Get whether to use local Dash Core node for SPV sync.
    pub fn use_local_node(&self) -> bool {
        self.use_local_node.load(Ordering::SeqCst)
    }

    /// Async status method for getting full details including progress.
    /// Returns default snapshot on lock errors to avoid panics.
    pub async fn status_async(&self) -> SpvStatusSnapshot {
        let status = self.read_status().unwrap_or(SpvStatus::Idle);
        let last_error = self.read_last_error().unwrap_or(None);
        let started_at = self.read_started_at().unwrap_or(None);
        let sync_progress = self.read_sync_progress().unwrap_or(None);
        let detailed_progress = self.read_detailed_progress().unwrap_or(None);
        let last_updated = self
            .read_progress_updated_at()
            .unwrap_or(None)
            .or(Some(SystemTime::now()));
        let connected_peers = self.connected_peers.read().map(|g| *g).unwrap_or(0);

        SpvStatusSnapshot {
            status,
            sync_progress,
            detailed_progress,
            last_error,
            started_at,
            last_updated,
            connected_peers,
        }
    }

    /// Sync status method for UI updates (doesn't fetch detailed progress).
    /// Returns default snapshot on lock errors to avoid panics.
    pub fn status(&self) -> SpvStatusSnapshot {
        let status = self.read_status().unwrap_or(SpvStatus::Idle);
        let last_error = self.read_last_error().unwrap_or(None);
        let started_at = self.read_started_at().unwrap_or(None);
        let sync_progress = self.read_sync_progress().unwrap_or(None);
        let detailed_progress = self.read_detailed_progress().unwrap_or(None);
        let last_updated = self
            .read_progress_updated_at()
            .unwrap_or(None)
            .or(Some(SystemTime::now()));
        let connected_peers = self.connected_peers.read().map(|g| *g).unwrap_or(0);

        SpvStatusSnapshot {
            status,
            sync_progress,
            detailed_progress,
            last_error,
            started_at,
            last_updated,
            connected_peers,
        }
    }

    pub fn start(self: &Arc<Self>, expected_wallet_count: usize) -> Result<(), String> {
        // Check if already running
        {
            let stop_token_guard = self
                .stop_token
                .lock()
                .map_err(|_| "SPV stop_token lock poisoned")?;
            if stop_token_guard.is_some() {
                return Ok(());
            }
        }

        self.write_status(SpvStatus::Starting)
            .map_err(|e| e.to_string())?;
        self.write_last_error(None).map_err(|e| e.to_string())?;
        self.write_started_at(Some(SystemTime::now()))
            .map_err(|e| e.to_string())?;
        self.write_sync_progress(None).map_err(|e| e.to_string())?;
        self.write_detailed_progress(None)
            .map_err(|e| e.to_string())?;
        self.write_progress_updated_at(None)
            .map_err(|e| e.to_string())?;

        let stop_token = CancellationToken::new();
        {
            let mut guard = self
                .stop_token
                .lock()
                .map_err(|_| "SPV stop_token lock poisoned")?;
            *guard = Some(stop_token.clone());
        }

        let manager = Arc::clone(self);
        let global_cancel = self.subtasks.cancellation_token.clone();

        // Spawn a dedicated OS thread with a multi-thread Tokio runtime for SPV operations
        // This ensures SPV sync doesn't compete with UI thread resources
        std::thread::Builder::new()
            .name("spv".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(4)
                    .enable_all()
                    .thread_name("spv-rt")
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let err_msg = format!("Failed to create SPV runtime: {e}");
                        tracing::error!("{}", err_msg);
                        let _ = manager.write_last_error(Some(err_msg));
                        let _ = manager.write_status(SpvStatus::Error);
                        return;
                    }
                };

                rt.block_on(async move {
                    let manager_for_loop = Arc::clone(&manager);
                    if let Err(err) = manager_for_loop.run_spv_loop(stop_token, global_cancel, expected_wallet_count).await {
                        tracing::error!(error = %err, network = ?manager.network, "SPV runtime failed");
                        if let Err(e) = manager.write_last_error(Some(err.clone())) {
                            tracing::error!("Failed to write SPV error: {}", e);
                        }
                        if let Err(e) = manager.write_status(SpvStatus::Error) {
                            tracing::error!("Failed to write SPV status: {}", e);
                        }
                    }

                    // Clean up on exit
                    if let Ok(mut guard) = manager.stop_token.lock() {
                        *guard = None;
                    }
                });
            })
            .map_err(|e| format!("Failed to spawn SPV thread: {e}"))?;

        Ok(())
    }

    pub fn stop(&self) {
        let maybe_token = self.stop_token.lock().ok().and_then(|g| g.clone());

        if let Some(token) = maybe_token {
            let _ = self.write_status(SpvStatus::Stopping);
            token.cancel();
        } else {
            let _ = self.write_status(SpvStatus::Stopped);
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

    pub fn wallet_id_for_seed(&self, seed_hash: WalletSeedHash) -> Option<WalletId> {
        self.det_wallets
            .read()
            .ok()
            .and_then(|map| map.get(&seed_hash).copied())
    }

    pub async fn unload_wallet(&self, seed_hash: WalletSeedHash) -> Result<(), String> {
        let wallet_id = {
            let map = self.det_wallets.read().map_err(|e| e.to_string())?;
            map.get(&seed_hash).copied()
        };

        let Some(wallet_id) = wallet_id else {
            return Ok(());
        };

        let mut wm = self.wallet.write().await;
        match wm.remove_wallet(&wallet_id) {
            Ok((_wallet, _info)) => {
                drop(wm);
                let mut map = self.det_wallets.write().map_err(|e| e.to_string())?;
                map.remove(&seed_hash);
                Ok(())
            }
            Err(WalletError::WalletNotFound(_)) => Ok(()),
            Err(err) => Err(format!("Failed to unload SPV wallet: {err}")),
        }
    }

    pub async fn broadcast_transaction(&self, tx: &Transaction) -> Result<(), String> {
        let request_tx = self
            .request_tx
            .lock()
            .map_err(|_| "SPV request_tx lock poisoned")?
            .clone()
            .ok_or_else(|| "SPV client not running".to_string())?;

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();

        request_tx
            .send(SpvRequest::BroadcastTransaction {
                tx: Box::new(tx.clone()),
                response_tx,
            })
            .await
            .map_err(|_| "SPV runtime channel closed".to_string())?;

        response_rx
            .await
            .map_err(|_| "SPV request cancelled".to_string())?
    }

    /// Create a reconciliation signal channel for external listeners.
    /// Returns a receiver that will get a signal when SPV wallet state likely changed.
    pub fn register_reconcile_channel(&self) -> mpsc::Receiver<()> {
        let (tx, rx) = mpsc::channel(64);
        if let Ok(mut guard) = self.reconcile_tx.lock() {
            *guard = Some(tx);
        }
        rx
    }

    /// Create a finality event channel for external listeners.
    /// Returns a receiver that will get events when SPV detects instant locks or chain locks.
    pub(crate) fn register_finality_channel(&self) -> mpsc::Receiver<AssetLockFinalityEvent> {
        let (tx, rx) = mpsc::channel(64);
        if let Ok(mut guard) = self.finality_tx.lock() {
            *guard = Some(tx);
        }
        rx
    }

    /// Remove all cached SPV data on disk for the current network.
    ///
    /// This requires the SPV runtime to be stopped first; otherwise the
    /// on-disk files could be re-created immediately by the running client.
    pub fn clear_data_dir(&self) -> Result<(), String> {
        let status = self.read_status().map_err(|e| e.to_string())?;
        if status.is_active() {
            return Err("Stop the SPV client before clearing its data".to_string());
        }

        if let Ok(mut storage_guard) = self.storage.lock() {
            *storage_guard = None;
        }

        if let Ok(mut interface_guard) = self.client_interface.write() {
            *interface_guard = None;
        }

        if let Ok(mut request_guard) = self.request_tx.lock() {
            *request_guard = None;
        }

        if let Ok(mut wallet_map) = self.det_wallets.write() {
            wallet_map.clear();
        }

        // Reset the in-memory WalletManager's synced_height so the next SPV session
        // scans filters from genesis instead of the stale height from the previous run.
        match self.wallet.try_write() {
            Ok(mut wm) => {
                wm.update_synced_height(0);
            }
            Err(_) => {
                tracing::warn!("Failed to reset WalletManager synced_height during SPV data clear");
            }
        }

        self.write_sync_progress(None).map_err(|e| e.to_string())?;
        self.write_detailed_progress(None)
            .map_err(|e| e.to_string())?;
        self.write_progress_updated_at(None)
            .map_err(|e| e.to_string())?;
        self.write_started_at(None).map_err(|e| e.to_string())?;
        self.write_last_error(None).map_err(|e| e.to_string())?;
        self.write_status(SpvStatus::Idle)
            .map_err(|e| e.to_string())?;
        if let Ok(mut guard) = self.connected_peers.write() {
            *guard = 0;
        }

        if self.data_dir.exists() {
            fs::remove_dir_all(&self.data_dir).map_err(|e| {
                format!(
                    "Failed to clear SPV data directory {}: {e}",
                    self.data_dir.display()
                )
            })?;
        }

        fs::create_dir_all(&self.data_dir).map_err(|e| {
            format!(
                "Failed to re-create SPV data directory {}: {e}",
                self.data_dir.display()
            )
        })?;

        Ok(())
    }

    /// Attempt to resolve a quorum public key via the SPV client's masternode/quorum state.
    ///
    /// This method sends a request through the DashSpvClientInterface to query the running
    /// SPV client. If SPV is not running or the key is not known, an error is returned.
    pub fn get_quorum_public_key(
        &self,
        quorum_type: u32,
        quorum_hash: [u8; 32],
        core_chain_locked_height: u32,
    ) -> Result<[u8; 48], String> {
        tracing::debug!(
            "get_quorum_public_key called: type={}, hash={}, height={}",
            quorum_type,
            hex::encode(quorum_hash),
            core_chain_locked_height
        );

        let interface = {
            let guard = self
                .client_interface
                .read()
                .map_err(|e| format!("client_interface lock poisoned: {e}"))?;
            guard
                .clone()
                .ok_or_else(|| "SPV client not initialized".to_string())?
        };

        let llmq_type = LLMQType::from(quorum_type as u8);
        let qh = QuorumHash::from_byte_array(quorum_hash).reverse();

        tracing::debug!(
            "SPV quorum public key lookup in progress: type={}, hash={}, height={}",
            quorum_type,
            hex::encode(quorum_hash),
            core_chain_locked_height
        );

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    interface.get_quorum_by_height(core_chain_locked_height, llmq_type, qh),
                )
                .await
                .map_err(|_| {
                    let msg = format!(
                        "Quorum lookup timed out after 30s at height {} for llmq_type={} hash=0x{}",
                        core_chain_locked_height,
                        quorum_type,
                        hex::encode(quorum_hash),
                    );
                    tracing::warn!("{}", msg);
                    msg
                })?
                .map(|q| {
                    tracing::debug!(
                        "Quorum public key found: type={}, hash={}, height={}",
                        quorum_type,
                        hex::encode(quorum_hash),
                        core_chain_locked_height
                    );
                    *q.quorum_entry.quorum_public_key.as_ref()
                })
                .map_err(|e| {
                    tracing::warn!(
                        "Quorum lookup failed at height {} for llmq_type={} hash=0x{}: {}",
                        core_chain_locked_height,
                        quorum_type,
                        hex::encode(quorum_hash),
                        e
                    );
                    e.to_string()
                })
            })
        })
    }

    pub async fn load_wallet_from_seed(
        &self,
        seed_hash: WalletSeedHash,
        mut seed_bytes: [u8; 64],
    ) -> Result<WalletId, String> {
        let existing_wallet_id = {
            let map = self.det_wallets.read().map_err(|e| e.to_string())?;
            map.get(&seed_hash).copied()
        };

        let mut wm = self.wallet.write().await;

        if let Some(wallet_id) = existing_wallet_id {
            if let Some(wallet) = wm.get_wallet(&wallet_id)
                && wallet.can_sign()
            {
                seed_bytes.zeroize();
                return Ok(wallet_id);
            }

            if let Err(err) = wm.remove_wallet(&wallet_id) {
                tracing::warn!(wallet = %hex::encode(wallet_id), ?err, "Failed to remove existing SPV wallet before upgrade");
            } else {
                tracing::info!(wallet = %hex::encode(wallet_id), "Upgrading SPV wallet from watch-only to full access");
            }
        }

        let xprv = ExtendedPrivKey::new_master(self.network, &seed_bytes).map_err(|e| {
            seed_bytes.zeroize();
            format!("ExtendedPrivKey::new_master failed: {e}")
        })?;
        seed_bytes.zeroize();
        let xprv_str = xprv.to_string();

        let account_options = Self::default_account_creation_options();

        let wallet_id = match wm.import_wallet_from_extended_priv_key(&xprv_str, account_options) {
            Ok(id) => id,
            Err(WalletError::WalletExists(id)) => id,
            Err(err) => {
                return Err(format!(
                    "import_wallet_from_extended_priv_key failed: {err}"
                ));
            }
        };

        drop(wm);

        let mut map = self.det_wallets.write().map_err(|e| e.to_string())?;
        map.insert(seed_hash, wallet_id);

        Ok(wallet_id)
    }

    pub async fn next_bip44_receive_address(
        &self,
        seed_hash: WalletSeedHash,
        account_index: u32,
    ) -> Result<SpvDerivedAddress, String> {
        let wallet_id = {
            let map = self.det_wallets.read().map_err(|e| e.to_string())?;
            map.get(&seed_hash)
                .copied()
                .ok_or_else(|| "Wallet seed not loaded into SPV".to_string())?
        };

        let mut wm = self.wallet.write().await;

        let result = wm
            .get_receive_address(
                &wallet_id,
                account_index,
                AccountTypePreference::BIP44,
                true,
            )
            .map_err(|e| format!("get_receive_address failed: {e}"))?;

        let address = result
            .address
            .ok_or_else(|| "Wallet manager did not return an address".to_string())?;

        let derivation_path = {
            let info = wm
                .get_wallet_info(&wallet_id)
                .ok_or_else(|| "wallet info missing".to_string())?;
            let collection = info.accounts();
            let account = collection
                .standard_bip44_accounts
                .get(&account_index)
                .ok_or_else(|| "BIP44 account missing".to_string())?;
            let metadata = account
                .get_address_info(&address)
                .ok_or_else(|| "Address metadata unavailable".to_string())?;
            metadata.path
        };

        Ok(SpvDerivedAddress {
            address,
            derivation_path,
        })
    }

    fn default_account_creation_options() -> WalletAccountCreationOptions {
        WalletAccountCreationOptions::Default
    }

    async fn run_spv_loop(
        self: Arc<Self>,
        stop_token: CancellationToken,
        global_cancel: CancellationToken,
        expected_wallet_count: usize,
    ) -> Result<(), String> {
        // Wait for all expected wallets to be fully loaded into the WalletManager
        // before building the client.  Wallet loading happens via queue_spv_wallet_load
        // which calls import_wallet_from_extended_priv_key (derives all accounts and
        // addresses) under a write lock.  Once wallet_count() reaches the expected
        // value, all addresses are derived and monitored_addresses() is populated.
        if expected_wallet_count > 0 {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                {
                    let wm = self.wallet.read().await;
                    let loaded = wm.wallet_count();
                    if loaded >= expected_wallet_count {
                        let addr_count = wm.monitored_addresses().len();
                        tracing::info!(
                            expected = expected_wallet_count,
                            loaded,
                            addresses = addr_count,
                            "SPV: all wallets loaded with monitored addresses, proceeding"
                        );
                        break;
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    let wm = self.wallet.read().await;
                    tracing::warn!(
                        expected = expected_wallet_count,
                        loaded = wm.wallet_count(),
                        "SPV: timed out waiting for all wallets to load, proceeding anyway"
                    );
                    break;
                }
                if stop_token.is_cancelled() || global_cancel.is_cancelled() {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }

        // Build and start the client
        let has_wallets = expected_wallet_count > 0;
        let mut client = self.build_client(has_wallets).await?;
        client
            .start()
            .await
            .map_err(|e| format!("SPV start failed: {e}"))?;

        // Store the shared storage reference for later access
        {
            let storage = client.storage();
            if let Ok(mut storage_guard) = self.storage.lock() {
                *storage_guard = Some(storage);
            }
        }

        // Subscribe to sync events (broadcast)
        let sync_rx = client.subscribe_sync_events();
        self.spawn_sync_event_handler(sync_rx);

        // Subscribe to wallet events (broadcast from WalletManager)
        {
            let wm = self.wallet.read().await;
            let wallet_rx = wm.subscribe_events();
            self.spawn_wallet_event_handler(wallet_rx);
        }

        // Subscribe to network events (broadcast)
        let net_rx = client.subscribe_network_events();
        self.spawn_network_event_handler(net_rx);

        // Set up progress handler using watch channel
        let progress_rx = client.subscribe_progress();
        self.spawn_progress_watcher(progress_rx);

        // Set up request handler with access to shared components
        let (request_tx, request_rx) = mpsc::channel(32);
        {
            if let Ok(mut guard) = self.request_tx.lock() {
                *guard = Some(request_tx);
            }
        }

        // Spawn request handler in a separate task
        self.spawn_request_handler(request_rx, stop_token.clone());

        // Create command channel for the DashSpvClientInterface
        // Note: Unbounded channel is required by SDK's DashSpvClientInterface API.
        // Memory usage is bounded in practice by SPV command processing speed.
        let (command_tx, command_receiver) = tokio::sync::mpsc::unbounded_channel();

        // Store the interface for external queries (quorum lookups, etc.)
        {
            let interface = DashSpvClientInterface::new(command_tx);
            let mut guard = self
                .client_interface
                .write()
                .map_err(|e| format!("client_interface lock poisoned: {e}"))?;
            *guard = Some(interface);
        }

        let _ = self.write_status(SpvStatus::Syncing);

        // Run sync and monitor with the client owned in this scope
        let result = self
            .clone()
            .run_sync_and_monitor(client, command_receiver, stop_token, global_cancel)
            .await;

        // Clear the interface and network manager since the client is done
        {
            if let Ok(mut guard) = self.client_interface.write() {
                *guard = None;
            }
        }
        {
            let mut nm_guard = self.network_manager.write().await;
            *nm_guard = None;
        }
        if let Ok(mut guard) = self.connected_peers.write() {
            *guard = 0;
        }
        {
            // Drop shared storage/request handles so the disk lock is released before restart.
            if let Ok(mut storage_guard) = self.storage.lock() {
                *storage_guard = None;
            }
            if let Ok(mut guard) = self.request_tx.lock() {
                *guard = None;
            }
        }

        result
    }

    async fn run_sync_and_monitor(
        self: Arc<Self>,
        mut client: SpvClient,
        command_receiver: mpsc::UnboundedReceiver<DashSpvClientCommand>,
        stop_token: CancellationToken,
        global_cancel: CancellationToken,
    ) -> Result<(), String> {
        // Monitor network continuously - this handles initial sync and ongoing monitoring
        // Requests are handled through the DashSpvClientInterface command channel
        enum Outcome {
            MonitorCompleted(Result<(), dash_sdk::dash_spv::SpvError>),
            StopRequested,
            GlobalCancelled,
        }

        let outcome = {
            let monitor_cancel = CancellationToken::new();
            let monitor_future = client.monitor_network(command_receiver, monitor_cancel.clone());
            tokio::pin!(monitor_future);

            tokio::select! {
                result = &mut monitor_future => Outcome::MonitorCompleted(result),
                _ = stop_token.cancelled() => {
                    monitor_cancel.cancel();
                    Outcome::StopRequested
                },
                _ = global_cancel.cancelled() => {
                    monitor_cancel.cancel();
                    Outcome::GlobalCancelled
                },
            }
        }; // monitor_future is dropped here, releasing the mutable borrow

        // Stop the client after monitoring completes or is cancelled
        let _ = client.stop().await;

        match outcome {
            Outcome::MonitorCompleted(Ok(())) => {
                let _ = self.write_status(SpvStatus::Stopped);
                Ok(())
            }
            Outcome::MonitorCompleted(Err(err)) => {
                let message = format!("monitor_network failed: {err}");
                let _ = self.write_last_error(Some(message.clone()));
                let _ = self.write_status(SpvStatus::Error);
                Err(message)
            }
            Outcome::StopRequested | Outcome::GlobalCancelled => {
                let _ = self.write_status(SpvStatus::Stopped);
                Ok(())
            }
        }
    }

    fn spawn_request_handler(
        &self,
        mut request_rx: mpsc::Receiver<SpvRequest>,
        cancel: CancellationToken,
    ) {
        tracing::info!("SPV request handler started");
        let network_manager = Arc::clone(&self.network_manager);
        self.subtasks.spawn_sync(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        tracing::info!("SPV request handler cancelled");
                        break;
                    }
                    request = request_rx.recv() => {
                        match request {
                            Some(SpvRequest::BroadcastTransaction { tx, response_tx }) => {
                                tracing::debug!("Received BroadcastTransaction request");
                                let result = {
                                    let nm_guard = network_manager.read().await;
                                    if let Some(ref nm) = *nm_guard {
                                        // Broadcast the transaction to all connected peers
                                        let message = dash_sdk::dpp::dashcore::network::message::NetworkMessage::Tx((*tx).clone());
                                        let results = nm.broadcast(message).await;
                                        // Check if at least one broadcast succeeded
                                        let mut success = false;
                                        let mut errors = Vec::new();
                                        for res in results {
                                            match res {
                                                Ok(_) => success = true,
                                                Err(e) => errors.push(e.to_string()),
                                            }
                                        }
                                        if success {
                                            tracing::info!("Transaction {} broadcast successfully", tx.txid());
                                            Ok(())
                                        } else if errors.is_empty() {
                                            Err("No peers connected to broadcast transaction".to_string())
                                        } else {
                                            Err(format!("Broadcast failed: {}", errors.join(", ")))
                                        }
                                    } else {
                                        Err("SPV network manager not available".to_string())
                                    }
                                };
                                let _ = response_tx.send(result);
                            }
                            None => {
                                tracing::warn!("SPV request channel closed");
                                break;
                            }
                        }
                    }
                }
            }
            tracing::info!("SPV request handler exiting");
        });
    }

    fn spawn_progress_watcher(
        &self,
        mut progress_rx: tokio::sync::watch::Receiver<WatchSyncProgress>,
    ) {
        let status = Arc::clone(&self.status);
        let sync_progress_state = Arc::clone(&self.sync_progress_state);
        let detailed_progress_state = Arc::clone(&self.detailed_progress_state);
        let progress_updated_at = Arc::clone(&self.progress_updated_at);
        let cancel = self.subtasks.cancellation_token.clone();

        self.subtasks.spawn_sync(async move {
            let sync_start_time = SystemTime::now();
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    result = progress_rx.changed() => {
                        if result.is_err() {
                            break; // Channel closed
                        }
                        let watch_progress = progress_rx.borrow();

                        // Extract rich progress data from all sub-managers
                        let header_height = watch_progress
                            .headers()
                            .map(|h| h.current_height())
                            .unwrap_or(0);
                        let masternode_height = watch_progress
                            .masternodes()
                            .map(|m| m.current_height())
                            .unwrap_or(0);
                        let filter_header_height = watch_progress
                            .filter_headers()
                            .map(|fh| fh.current_height())
                            .unwrap_or(0);
                        let filters_current = watch_progress
                            .filters()
                            .map(|f| f.current_height())
                            .unwrap_or(0);
                        let filters_downloaded = watch_progress
                            .filters()
                            .map(|f| f.downloaded() as u64)
                            .unwrap_or(0);

                        let peer_best_height = watch_progress
                            .headers()
                            .map(|h| h.target_height())
                            .unwrap_or(0);

                        // Build the flat types::SyncProgress
                        let sync_progress = SyncProgress {
                            header_height,
                            filter_header_height,
                            masternode_height,
                            peer_count: 0,
                            filter_sync_available: true,
                            filters_downloaded,
                            last_synced_filter_height: if filters_current > 0 {
                                Some(filters_current)
                            } else {
                                None
                            },
                            sync_start: sync_start_time,
                            last_update: SystemTime::now(),
                        };

                        // Determine the current sync stage from sub-manager states
                        let sync_stage = Self::determine_sync_stage(&watch_progress);

                        let percentage = if peer_best_height > 0 {
                            watch_progress.percentage()
                        } else {
                            0.0
                        };

                        let detailed = DetailedSyncProgress {
                            sync_progress: sync_progress.clone(),
                            peer_best_height,
                            percentage,
                            headers_per_second: 0.0,
                            bytes_per_second: 0,
                            estimated_time_remaining: None,
                            sync_stage,
                            total_headers_processed: header_height as u64,
                            total_bytes_downloaded: 0,
                            sync_start_time,
                            last_update_time: SystemTime::now(),
                        };

                        // Update both sync progress and detailed progress
                        if let Ok(mut stored_sync) = sync_progress_state.write() {
                            *stored_sync = Some(sync_progress);
                        }
                        if let Ok(mut stored_detailed) = detailed_progress_state.write() {
                            *stored_detailed = Some(detailed);
                        }
                        if let Ok(mut updated_at) = progress_updated_at.write() {
                            *updated_at = Some(SystemTime::now());
                        }

                        // Update status based on progress
                        if let Ok(mut status_guard) = status.write() {
                            if watch_progress.is_synced() {
                                *status_guard = SpvStatus::Running;
                            } else if !matches!(*status_guard, SpvStatus::Stopping | SpvStatus::Stopped | SpvStatus::Error) {
                                *status_guard = SpvStatus::Syncing;
                            }
                        }
                    }
                }
            }
            tracing::info!("SPV progress watcher exiting");
        });
    }

    /// Map the rich sync::SyncProgress sub-manager states into a types::SyncStage.
    ///
    /// Uses pipeline order (earliest incomplete stage wins) so the stage advances
    /// monotonically and never bounces back, even though dash-spv runs managers
    /// in parallel.
    fn determine_sync_stage(watch_progress: &WatchSyncProgress) -> SyncStage {
        if watch_progress.is_synced() {
            return SyncStage::Complete;
        }

        // Helper: a manager is "done" if it reported Synced or WaitForEvents.
        // If the manager hasn't started yet (Err), treat it as done/skipped.
        let is_done = |state: WatchSyncState| {
            matches!(
                state,
                WatchSyncState::Synced | WatchSyncState::WaitForEvents
            )
        };

        // Pipeline order: headers → masternodes → filter headers → filters → blocks.
        // The first stage that is NOT done determines the displayed stage.

        // 1. Headers
        let headers_done = watch_progress
            .headers()
            .map(|h| is_done(h.state()))
            .unwrap_or(true);
        if !headers_done {
            if let Ok(h) = watch_progress.headers() {
                if h.state() == WatchSyncState::WaitingForConnections {
                    return SyncStage::Connecting;
                }
                return SyncStage::DownloadingHeaders {
                    start: 0,
                    end: h.target_height(),
                };
            }
            return SyncStage::Connecting;
        }

        // 2. Masternodes
        let mn_done = watch_progress
            .masternodes()
            .map(|m| is_done(m.state()))
            .unwrap_or(true);
        if !mn_done {
            let diffs = watch_progress
                .masternodes()
                .map(|m| m.diffs_processed() as usize)
                .unwrap_or(0);
            return SyncStage::ValidatingHeaders { batch_size: diffs };
        }

        // 3. Filter headers
        let fh_done = watch_progress
            .filter_headers()
            .map(|fh| is_done(fh.state()))
            .unwrap_or(true);
        if !fh_done
            && let Ok(fh) = watch_progress.filter_headers()
        {
            return SyncStage::DownloadingFilterHeaders {
                current: fh.current_height(),
                target: fh.target_height(),
            };
        }

        // 4. Filters
        let filters_done = watch_progress
            .filters()
            .map(|f| is_done(f.state()))
            .unwrap_or(true);
        if !filters_done
            && let Ok(f) = watch_progress.filters()
        {
            return SyncStage::DownloadingFilters {
                completed: f.current_height(),
                total: f.target_height(),
            };
        }

        // 5. Blocks
        let blocks_done = watch_progress
            .blocks()
            .map(|b| is_done(b.state()))
            .unwrap_or(true);
        if !blocks_done
            && let Ok(b) = watch_progress.blocks()
        {
            return SyncStage::DownloadingBlocks {
                pending: b.requested().saturating_sub(b.processed()) as usize,
            };
        }

        SyncStage::Complete
    }

    fn spawn_sync_event_handler(&self, mut sync_rx: tokio::sync::broadcast::Receiver<SyncEvent>) {
        let reconcile_tx = self.reconcile_tx.lock().ok().and_then(|g| g.clone());
        let finality_tx = self.finality_tx.lock().ok().and_then(|g| g.clone());
        let status = Arc::clone(&self.status);
        let cancel = self.subtasks.cancellation_token.clone();

        self.subtasks.spawn_sync(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    result = sync_rx.recv() => {
                        match result {
                            Ok(event) => {
                                let should_signal = matches!(
                                    event,
                                    SyncEvent::BlockProcessed { .. }
                                    | SyncEvent::ChainLockReceived { .. }
                                    | SyncEvent::InstantLockReceived { .. }
                                    | SyncEvent::SyncComplete { .. }
                                );

                                // Forward finality-relevant events for asset lock proof construction
                                if let Some(ref ftx) = finality_tx {
                                    match &event {
                                        SyncEvent::InstantLockReceived { instant_lock, .. } => {
                                            if let Err(e) = ftx.try_send(AssetLockFinalityEvent::InstantLock {
                                                txid: instant_lock.txid,
                                                instant_lock: Box::new(instant_lock.clone()),
                                            }) {
                                                tracing::warn!("Failed to forward InstantLock finality event for txid {}: {}", instant_lock.txid, e);
                                            }
                                        }
                                        SyncEvent::ChainLockReceived { chain_lock, .. } => {
                                            if let Err(e) = ftx.try_send(AssetLockFinalityEvent::ChainLock {
                                                height: chain_lock.block_height,
                                            }) {
                                                tracing::warn!("Failed to forward ChainLock finality event for height {}: {}", chain_lock.block_height, e);
                                            }
                                        }
                                        _ => {}
                                    }
                                }

                                if matches!(event, SyncEvent::SyncComplete { .. })
                                    && let Ok(mut guard) = status.write()
                                {
                                    *guard = SpvStatus::Running;
                                }
                                if should_signal
                                    && let Some(ref tx) = reconcile_tx
                                {
                                    let _ = tx.try_send(());
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!("Sync event handler lagged by {} events", n);
                                // Trigger reconcile to catch up on any missed state changes
                                if let Some(ref tx) = reconcile_tx {
                                    let _ = tx.try_send(());
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
            tracing::info!("SPV sync event handler exiting");
        });
    }

    fn spawn_wallet_event_handler(
        &self,
        mut wallet_rx: tokio::sync::broadcast::Receiver<WalletEvent>,
    ) {
        let reconcile_tx = self.reconcile_tx.lock().ok().and_then(|g| g.clone());
        let cancel = self.subtasks.cancellation_token.clone();

        self.subtasks.spawn_sync(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    result = wallet_rx.recv() => {
                        match result {
                            Ok(_event) => {
                                // All wallet events trigger reconcile
                                if let Some(ref tx) = reconcile_tx {
                                    let _ = tx.try_send(());
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!("Wallet event handler lagged by {} events", n);
                                // Still trigger reconcile to catch up
                                if let Some(ref tx) = reconcile_tx {
                                    let _ = tx.try_send(());
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
            tracing::info!("SPV wallet event handler exiting");
        });
    }

    fn spawn_network_event_handler(
        &self,
        mut net_rx: tokio::sync::broadcast::Receiver<NetworkEvent>,
    ) {
        let connected_peers = Arc::clone(&self.connected_peers);
        let cancel = self.subtasks.cancellation_token.clone();

        self.subtasks.spawn_sync(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    result = net_rx.recv() => {
                        match result {
                            Ok(NetworkEvent::PeersUpdated { connected_count, .. }) => {
                                if let Ok(mut guard) = connected_peers.write() {
                                    *guard = connected_count;
                                }
                            }
                            Ok(_) => {
                                // PeerConnected / PeerDisconnected — PeersUpdated follows
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!("Network event handler lagged by {} events", n);
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            }
            tracing::info!("SPV network event handler exiting");
        });
    }

    async fn build_client(
        &self,
        has_wallets: bool,
    ) -> Result<
        DashSpvClient<WalletManager<ManagedWalletInfo>, PeerNetworkManager, DiskStorageManager>,
        String,
    > {
        // When wallets exist, scan from genesis so historical transactions are found via
        // compact block filters.  When no wallets are loaded, skip to chain tip (u32::MAX)
        // to avoid unnecessary work.  We check both the caller hint and the actual wallet
        // count (the wait loop in run_spv_loop should have ensured loading completed).
        let wallet_count = self.wallet.read().await.wallet_count();
        let start_height = if has_wallets || wallet_count > 0 {
            0
        } else {
            u32::MAX
        };
        let mut config = ClientConfig::new(self.network)
            .with_storage_path(self.data_dir.clone())
            .with_validation_mode(ValidationMode::Full)
            .with_start_height(start_height);

        // Configure peer discovery based on network type and user preference.
        // Devnet/Regtest always need explicit peers since they're local networks.
        // Mainnet/Testnet can use DNS seed discovery (default) or local node.
        if self.network == Network::Devnet || self.network == Network::Regtest {
            // Local networks always need explicit peer configuration
            if let Some(peer) = self.primary_peer_socket() {
                config.add_peer(peer);
            }
        } else if self.use_local_node() {
            // User has chosen to use their local Dash Core node
            if let Some(peer) = self.primary_peer_socket() {
                config.add_peer(peer);
            }
        }
        // Otherwise, no peers are added and SPV will use DNS seed discovery

        let network_manager = PeerNetworkManager::new(&config)
            .await
            .map_err(|e| format!("Failed to initialize SPV network manager: {e}"))?;

        // Store a clone of the network manager for broadcasting transactions
        {
            let mut nm_guard = self.network_manager.write().await;
            *nm_guard = Some(network_manager.clone());
        }

        let storage_manager = DiskStorageManager::new(&config)
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

impl fmt::Debug for SpvManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpvManager")
            .field("network", &self.network)
            .field("data_dir", &self.data_dir)
            .finish()
    }
}
