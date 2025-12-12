use crate::app_dir::app_user_data_dir_path;
use crate::config::NetworkConfig;
use crate::model::wallet::WalletSeedHash;
use crate::utils::tasks::TaskManager;
use dash_sdk::dash_spv::client::interface::{DashSpvClientCommand, DashSpvClientInterface};
use dash_sdk::dash_spv::network::PeerNetworkManager;
use dash_sdk::dash_spv::storage::DiskStorageManager;
use dash_sdk::dash_spv::types::{
    DetailedSyncProgress, SpvEvent, SyncProgress, SyncStage, ValidationMode,
};
use dash_sdk::dash_spv::{ClientConfig, DashSpvClient, Hash, LLMQType, QuorumHash};
use dash_sdk::dpp::dashcore::{Address, Network, Transaction};
use dash_sdk::dpp::key_wallet;
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, ExtendedPrivKey};
use dash_sdk::dpp::key_wallet::wallet::initialization::WalletAccountCreationOptions;
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::{
    ManagedWalletInfo, transaction_building::AccountTypePreference,
    wallet_info_interface::WalletInfoInterface,
};
use dash_sdk::dpp::key_wallet_manager::wallet_manager::{WalletError, WalletId, WalletManager};
// use dash_sdk::dpp::key_wallet::bip32::ExtendedPubKey; // not needed directly here
use std::fmt;
use std::fs;
use std::net::ToSocketAddrs;
use std::path::PathBuf;
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
}

/// Type alias for the SPV client with our specific configuration
type SpvClient =
    DashSpvClient<WalletManager<ManagedWalletInfo>, PeerNetworkManager, DiskStorageManager>;

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
    // Cancellation token for clean shutdown
    stop_token: Mutex<Option<CancellationToken>>,
    // Channel to send requests to the SPV runtime thread
    request_tx: Mutex<Option<mpsc::Sender<SpvRequest>>>,
}

/// Requests that can be sent to the SPV runtime thread
///
/// Note: These requests are handled in the same async context where the client lives,
/// allowing direct access to client methods without additional locking overhead.
enum SpvRequest {
    BroadcastTransaction {
        #[allow(dead_code)]
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
            stop_token: Mutex::new(None),
            request_tx: Mutex::new(None),
        });

        Ok(manager)
    }

    /// Async status method for getting full details including progress
    pub async fn status_async(&self) -> SpvStatusSnapshot {
        let status = *self.status.read().expect("SPV status lock poisoned");
        let last_error = self
            .last_error
            .read()
            .expect("SPV last_error lock poisoned")
            .clone();
        let started_at = *self
            .started_at
            .read()
            .expect("SPV started_at lock poisoned");
        let sync_progress = self
            .sync_progress_state
            .read()
            .expect("SPV sync_progress lock poisoned")
            .clone();
        let detailed_progress = self
            .detailed_progress_state
            .read()
            .expect("SPV detailed_progress lock poisoned")
            .clone();
        let last_updated = (*self
            .progress_updated_at
            .read()
            .expect("SPV progress_updated lock poisoned"))
        .or(Some(SystemTime::now()));

        SpvStatusSnapshot {
            status,
            sync_progress,
            detailed_progress,
            last_error,
            started_at,
            last_updated,
        }
    }

    /// Sync status method for UI updates (doesn't fetch detailed progress)
    pub fn status(&self) -> SpvStatusSnapshot {
        let status = *self.status.read().expect("SPV status lock poisoned");
        let last_error = self
            .last_error
            .read()
            .expect("SPV last_error lock poisoned")
            .clone();
        let started_at = *self
            .started_at
            .read()
            .expect("SPV started_at lock poisoned");
        let sync_progress = self
            .sync_progress_state
            .read()
            .expect("SPV sync_progress lock poisoned")
            .clone();
        let detailed_progress = self
            .detailed_progress_state
            .read()
            .expect("SPV detailed_progress lock poisoned")
            .clone();
        let last_updated = (*self
            .progress_updated_at
            .read()
            .expect("SPV progress_updated lock poisoned"))
        .or(Some(SystemTime::now()));

        SpvStatusSnapshot {
            status,
            sync_progress,
            detailed_progress,
            last_error,
            started_at,
            last_updated,
        }
    }

    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        // Check if already running
        {
            let stop_token_guard = self
                .stop_token
                .lock()
                .expect("SPV stop_token lock poisoned");
            if stop_token_guard.is_some() {
                return Ok(());
            }
        }

        *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Starting;
        *self
            .last_error
            .write()
            .expect("SPV last_error lock poisoned") = None;
        *self
            .started_at
            .write()
            .expect("SPV started_at lock poisoned") = Some(SystemTime::now());
        *self
            .sync_progress_state
            .write()
            .expect("SPV sync_progress lock poisoned") = None;
        *self
            .detailed_progress_state
            .write()
            .expect("SPV detailed_progress lock poisoned") = None;
        *self
            .progress_updated_at
            .write()
            .expect("SPV progress_updated lock poisoned") = None;

        let stop_token = CancellationToken::new();
        *self
            .stop_token
            .lock()
            .expect("SPV stop_token lock poisoned") = Some(stop_token.clone());

        let manager = Arc::clone(self);
        let global_cancel = self.subtasks.cancellation_token.clone();

        // Spawn a dedicated OS thread with a multi-thread Tokio runtime for SPV operations
        // This ensures SPV sync doesn't compete with UI thread resources
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
                    let manager_for_loop = Arc::clone(&manager);
                    if let Err(err) = manager_for_loop.run_spv_loop(stop_token, global_cancel).await {
                        tracing::error!(error = %err, network = ?manager.network, "SPV runtime failed");
                        *manager.last_error.write().expect("SPV last_error lock poisoned") = Some(err.clone());
                        *manager.status.write().expect("SPV status lock poisoned") = SpvStatus::Error;
                    }

                    // Clean up on exit
                    *manager.stop_token.lock().expect("SPV stop_token lock poisoned") = None;
                });
            })
            .map_err(|e| format!("Failed to spawn SPV thread: {e}"))?;

        Ok(())
    }

    pub fn stop(&self) {
        let maybe_token = self
            .stop_token
            .lock()
            .expect("SPV stop_token lock poisoned")
            .clone();

        if let Some(token) = maybe_token {
            *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Stopping;
            token.cancel();
        } else {
            *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Stopped;
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
            .expect("request_tx poisoned")
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
        let mut guard = self.reconcile_tx.lock().expect("reconcile_tx poisoned");
        *guard = Some(tx);
        rx
    }

    /// Remove all cached SPV data on disk for the current network.
    ///
    /// This requires the SPV runtime to be stopped first; otherwise the
    /// on-disk files could be re-created immediately by the running client.
    pub fn clear_data_dir(&self) -> Result<(), String> {
        let status = *self.status.read().expect("SPV status lock poisoned");
        if status.is_active() {
            return Err("Stop the SPV client before clearing its data".to_string());
        }

        {
            let mut storage_guard = self.storage.lock().expect("storage lock poisoned");
            *storage_guard = None;
        }

        {
            let mut interface_guard = self
                .client_interface
                .write()
                .expect("client_interface lock poisoned");
            *interface_guard = None;
        }

        {
            let mut request_guard = self.request_tx.lock().expect("request_tx poisoned");
            *request_guard = None;
        }

        {
            let mut wallet_map = self.det_wallets.write().map_err(|e| e.to_string())?;
            wallet_map.clear();
        }

        *self
            .sync_progress_state
            .write()
            .expect("SPV sync_progress lock poisoned") = None;
        *self
            .detailed_progress_state
            .write()
            .expect("SPV detailed_progress lock poisoned") = None;
        *self
            .progress_updated_at
            .write()
            .expect("SPV progress_updated lock poisoned") = None;
        *self
            .started_at
            .write()
            .expect("SPV started_at lock poisoned") = None;
        *self
            .last_error
            .write()
            .expect("SPV last_error lock poisoned") = None;
        *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Idle;

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

        let llmq_type = LLMQType::try_from(quorum_type as u8)
            .map_err(|e| format!("Invalid LLMQ type {}: {}", quorum_type, e))?;
        let qh = QuorumHash::from_byte_array(quorum_hash);

        tracing::debug!(
            "SPV quorum public key lookup in progress: type={}, hash={}, height={}",
            quorum_type,
            hex::encode(quorum_hash),
            core_chain_locked_height
        );

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                interface
                    .get_quorum_by_height(core_chain_locked_height, llmq_type, qh)
                    .await
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
                        format!(
                            "Quorum lookup failed at height {} for llmq_type={} hash=0x{}: {}",
                            core_chain_locked_height,
                            quorum_type,
                            hex::encode(quorum_hash),
                            e
                        )
                    })
            })
        })
    }

    pub async fn load_wallet_from_seed(
        &self,
        seed_hash: WalletSeedHash,
        mut seed_bytes: [u8; 64],
    ) -> Result<WalletId, String> {
        let wallet_network = Self::wallet_network(self.network);

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

        let wallet_id = match wm.import_wallet_from_extended_priv_key(
            &xprv_str,
            wallet_network,
            account_options,
        ) {
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
        let network = Self::wallet_network(self.network);

        let result = wm
            .get_receive_address(
                &wallet_id,
                network,
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
            let collection = info
                .accounts(network)
                .ok_or_else(|| "Account collection not found".to_string())?;
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

    fn wallet_network(network: Network) -> key_wallet::Network {
        match network {
            Network::Dash => key_wallet::Network::Dash,
            Network::Testnet => key_wallet::Network::Testnet,
            Network::Devnet => key_wallet::Network::Devnet,
            Network::Regtest => key_wallet::Network::Regtest,
            other => {
                tracing::warn!(
                    ?other,
                    "Unknown dashcore::Network; defaulting to Dash for wallet mapping"
                );
                key_wallet::Network::Dash
            }
        }
    }

    fn default_account_creation_options() -> WalletAccountCreationOptions {
        WalletAccountCreationOptions::Default
    }

    async fn run_spv_loop(
        self: Arc<Self>,
        stop_token: CancellationToken,
        global_cancel: CancellationToken,
    ) -> Result<(), String> {
        // Build and start the client
        let mut client = self.build_client().await?;
        client
            .start()
            .await
            .map_err(|e| format!("SPV start failed: {e}"))?;

        // Store the shared storage reference for later access
        {
            let storage = client.storage();
            let mut storage_guard = self.storage.lock().expect("storage lock poisoned");
            *storage_guard = Some(storage);
        }

        // Set up progress handler
        if let Some(progress_rx) = client.take_progress_receiver() {
            self.spawn_progress_handler(progress_rx);
        }

        // Set up event handler
        if let Some(event_rx) = client.take_event_receiver() {
            self.spawn_event_handler(event_rx);
        }

        // Set up request handler with access to shared components
        let (request_tx, request_rx) = mpsc::channel(32);
        {
            let mut guard = self.request_tx.lock().expect("request_tx poisoned");
            *guard = Some(request_tx);
        }

        // Spawn request handler in a separate task
        self.spawn_request_handler(request_rx, stop_token.clone());

        // Create command channel for the DashSpvClientInterface
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

        *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Syncing;

        // Run sync and monitor with the client owned in this scope
        let result = self
            .clone()
            .run_sync_and_monitor(client, command_receiver, stop_token, global_cancel)
            .await;

        // Clear the interface since the client is done
        {
            if let Ok(mut guard) = self.client_interface.write() {
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
        // Wait for at least one peer to connect
        let mut waited_ms: u64 = 0;
        loop {
            // Check for cancellation
            if stop_token.is_cancelled() || global_cancel.is_cancelled() {
                let _ = client.stop().await;
                *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Stopped;
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

        // Sync to tip
        match client.sync_to_tip().await {
            Ok(progress) => {
                tracing::info!("Initial sync progress snapshot: {:?}", progress);
                {
                    let mut stored_sync = self
                        .sync_progress_state
                        .write()
                        .expect("SPV sync_progress lock poisoned");
                    *stored_sync = Some(progress.clone());
                }
                {
                    let mut updated_at = self
                        .progress_updated_at
                        .write()
                        .expect("SPV progress_updated lock poisoned");
                    *updated_at = Some(SystemTime::now());
                }
                // Stay in Syncing mode until detailed progress reports completion.
                *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Syncing;
            }
            Err(err) => {
                tracing::error!("Initial sync failed: {}", err);
                let _ = client.stop().await;
                *self
                    .last_error
                    .write()
                    .expect("SPV last_error lock poisoned") =
                    Some(format!("Initial sync failed: {err}"));
                *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Error;
                return Err(format!("Initial sync failed: {err}"));
            }
        }

        // Monitor network continuously - this is designed to run once and keep running
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
                *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Stopped;
                Ok(())
            }
            Outcome::MonitorCompleted(Err(err)) => {
                let message = format!("monitor_network failed: {err}");
                *self
                    .last_error
                    .write()
                    .expect("SPV last_error lock poisoned") = Some(message.clone());
                *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Error;
                Err(message)
            }
            Outcome::StopRequested | Outcome::GlobalCancelled => {
                *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Stopped;
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
        self.subtasks.spawn_sync(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        tracing::info!("SPV request handler cancelled");
                        break;
                    }
                    request = request_rx.recv() => {
                        match request {
                            Some(SpvRequest::BroadcastTransaction { response_tx, .. }) => {
                                tracing::debug!("Received BroadcastTransaction request");
                                // Note: broadcast_transaction would need access to the client
                                // For now, just return not implemented
                                let _ = response_tx.send(Err("Broadcast not yet implemented".to_string()));
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

    fn spawn_progress_handler(
        &self,
        mut progress_rx: tokio::sync::mpsc::UnboundedReceiver<DetailedSyncProgress>,
    ) {
        let status = Arc::clone(&self.status);
        let last_error = Arc::clone(&self.last_error);
        let sync_progress_state = Arc::clone(&self.sync_progress_state);
        let detailed_progress_state = Arc::clone(&self.detailed_progress_state);
        let progress_updated_at = Arc::clone(&self.progress_updated_at);
        let cancel = self.subtasks.cancellation_token.clone();

        self.subtasks.spawn_sync(async move {
            let mut last_update = std::time::Instant::now();
            let min_interval = std::time::Duration::from_millis(500);

            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    msg = progress_rx.recv() => {
                        match msg {
                            Some(detailed) => {
                                {
                                    let mut stored_detailed = detailed_progress_state
                                        .write()
                                        .expect("SPV detailed_progress lock poisoned");
                                    *stored_detailed = Some(detailed.clone());
                                }
                                {
                                    let mut stored_sync = sync_progress_state
                                        .write()
                                        .expect("SPV sync_progress lock poisoned");
                                    *stored_sync = Some(detailed.sync_progress.clone());
                                }
                                {
                                    let mut updated_at = progress_updated_at
                                        .write()
                                        .expect("SPV progress_updated lock poisoned");
                                    *updated_at = Some(detailed.last_update_time);
                                }

                                if last_update.elapsed() >= min_interval {
                                    // Update status based on progress stage and completeness
                                    let mut status_guard = status
                                        .write()
                                        .expect("SPV status lock poisoned");
                                    let current = *status_guard;
                                    match &detailed.sync_stage {
                                        SyncStage::Complete => {
                                            *status_guard = SpvStatus::Running;
                                        }
                                        SyncStage::Failed(message) => {
                                            *status_guard = SpvStatus::Error;
                                            let mut err_guard = last_error
                                                .write()
                                                .expect("SPV last_error lock poisoned");
                                            *err_guard = Some(format!("SPV sync failed: {message}"));
                                        }
                                        _ => {
                                            if !matches!(
                                                current,
                                                SpvStatus::Stopping | SpvStatus::Stopped | SpvStatus::Error
                                            ) {
                                                *status_guard = SpvStatus::Syncing;
                                            }
                                        }
                                    }
                                    last_update = std::time::Instant::now();
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        });
    }

    fn spawn_event_handler(&self, mut event_rx: tokio::sync::mpsc::UnboundedReceiver<SpvEvent>) {
        let reconcile_tx = self
            .reconcile_tx
            .lock()
            .expect("reconcile_tx poisoned")
            .clone();
        let cancel = self.subtasks.cancellation_token.clone();

        self.subtasks.spawn_sync(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    evt = event_rx.recv() => {
                        match evt {
                            Some(event) => {
                                // Push reconcile signal for wallet-related updates
                                let should_signal = matches!(event,
                                    SpvEvent::TransactionDetected { .. } |
                                    SpvEvent::BalanceUpdate { .. } |
                                    SpvEvent::BlockProcessed { .. }
                                );
                                if should_signal
                                    && let Some(ref tx) = reconcile_tx {
                                    let _ = tx.try_send(());
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        });
    }

    async fn build_client(
        &self,
    ) -> Result<
        DashSpvClient<WalletManager<ManagedWalletInfo>, PeerNetworkManager, DiskStorageManager>,
        String,
    > {
        let start_height = {
            let guard = self.wallet.read().await;
            if guard.wallet_count() == 0 {
                u32::MAX
            } else {
                0
            }
        };
        let mut config = ClientConfig::new(self.network)
            .with_storage_path(self.data_dir.clone())
            .with_validation_mode(ValidationMode::Full)
            .with_start_height(start_height);

        // Pin peers when running against local nodes to avoid random peers.
        if self.network == Network::Devnet || self.network == Network::Regtest {
            if let Some(peer) = self.primary_peer_socket() {
                config.add_peer(peer);
            }
        } else if self.network == Network::Testnet || self.network == Network::Dash {
            // For testnet testing, connect only to local Dash Core at 127.0.0.1:19999
            if let Ok(mut it) = "127.0.0.1:19999".to_socket_addrs()
                && let Some(peer) = it.next()
            {
                config.add_peer(peer);
            }
        }

        let network_manager = PeerNetworkManager::new(&config)
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

impl fmt::Debug for SpvManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpvManager")
            .field("network", &self.network)
            .field("data_dir", &self.data_dir)
            .finish()
    }
}
