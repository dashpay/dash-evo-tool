use crate::app_dir::app_user_data_dir_path;
use crate::config::NetworkConfig;
use crate::utils::tasks::TaskManager;
use dash_sdk::dash_spv::network::MultiPeerNetworkManager;
use dash_sdk::dash_spv::storage::DiskStorageManager;
use dash_sdk::dash_spv::types::{
    DetailedSyncProgress, SpvEvent, SyncProgress, SyncStage, ValidationMode,
};
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
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

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

/// Manages SPV client lifecycle and exposes status updates.
/// Uses dash-spv's built-in state management while maintaining a dedicated runtime for performance.
pub struct SpvManager {
    network: Network,
    data_dir: PathBuf,
    config: Arc<RwLock<NetworkConfig>>,
    subtasks: Arc<TaskManager>,
    wallet: Arc<AsyncRwLock<WalletManager<ManagedWalletInfo>>>,
    #[allow(clippy::type_complexity)]
    client: Arc<
        AsyncRwLock<
            Option<
                DashSpvClient<
                    WalletManager<ManagedWalletInfo>,
                    MultiPeerNetworkManager,
                    DiskStorageManager,
                >,
            >,
        >,
    >,
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
            client: Arc::new(AsyncRwLock::new(None)),
            status: Arc::new(RwLock::new(SpvStatus::Idle)),
            last_error: Arc::new(RwLock::new(None)),
            started_at: Arc::new(RwLock::new(None)),
            sync_progress_state: Arc::new(RwLock::new(None)),
            detailed_progress_state: Arc::new(RwLock::new(None)),
            progress_updated_at: Arc::new(RwLock::new(None)),
            det_wallets: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
            reconcile_tx: Mutex::new(None),
            stop_token: Mutex::new(None),
        });

        Ok(manager)
    }

    /// Async status method for getting full details including progress
    pub async fn status_async(&self) -> SpvStatusSnapshot {
        let _client_guard = self.client.read().await;
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
        *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Stopping;

        // Signal the runtime to stop
        if let Some(token) = self
            .stop_token
            .lock()
            .expect("SPV stop_token lock poisoned")
            .as_ref()
        {
            token.cancel();
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

    /// Attempt to resolve a quorum public key via the SPV client's masternode/quorum state.
    ///
    /// Note: This is a blocking, best-effort lookup. If SPV state is unavailable,
    /// or the key is not known yet, an error is returned for the caller to handle.
    pub fn get_quorum_public_key(
        &self,
        quorum_type: u32,
        quorum_hash: [u8; 32],
        core_chain_locked_height: u32,
    ) -> Result<[u8; 48], String> {
        // Try repeatedly to grab a non-blocking read guard.
        // We avoid blocking_read to prevent panicking inside Tokio runtime threads.
        let mut attempts = 0u32;
        loop {
            if let Ok(guard) = self.client.try_read() {
                if let Some(client) = guard.as_ref() {
                    if let Some(q) = client.get_quorum_at_height(
                        core_chain_locked_height,
                        quorum_type as u8,
                        &quorum_hash,
                    ) {
                        let pk48: [u8; 48] = *q.quorum_entry.quorum_public_key.as_ref();
                        return Ok(pk48);
                    } else {
                        return Err(format!(
                            "Quorum not found at height {} for llmq_type={} hash=0x{}",
                            core_chain_locked_height,
                            quorum_type,
                            hex::encode(quorum_hash)
                        ));
                    }
                } else {
                    return Err("SPV client not initialized".to_string());
                }
            }

            attempts = attempts.saturating_add(1);
            if attempts > 500 {
                return Err("SPV client busy; try again".to_string());
            }
            // Short backoff to yield to the writer; keep small to avoid stalling proof verification.
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
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

        // for registration in attachment.identity_registrations {
        //     let Some(wallet_id) = map.get(&registration.seed_hash).cloned() else {
        //         tracing::warn!(seed = %hex::encode(registration.seed_hash), "Missing wallet_id for identity registration batch" );
        //         continue;
        //     };

        //     for entry in registration.entries {
        //         if entry.addresses.is_empty() {
        //             continue;
        //         }

        //         if let Err(err) =
        //             Self::ensure_account_exists(&mut wm, &wallet_id, net_wallet, entry.account_type)
        //         {
        //             tracing::error!(seed = %hex::encode(registration.seed_hash), account = ?entry.account_type, "Failed to ensure account exists: {err}");
        //             continue;
        //         }

        //         match wm.register_known_addresses(
        //             &wallet_id,
        //             net_wallet,
        //             entry.account_type,
        //             entry.addresses.clone(),
        //         ) {
        //             Ok(added) => {
        //                 if added > 0 {
        //                     tracing::info!(seed = %hex::encode(registration.seed_hash), account = ?entry.account_type, added, "Registered identity addresses for watch-only SPV");
        //                 }
        //             }
        //             Err(e) => {
        //                 tracing::error!(seed = %hex::encode(registration.seed_hash), account = ?entry.account_type, error = ?e, "register_known_addresses failed");
        //             }
        //         }
        //     }
        // }

        // Optional: log monitored addresses count for debugging
        let addr_count = wm.monitored_addresses(attachment.network).len();
        tracing::info!(addresses = addr_count, network = ?attachment.network, "SPV now watching addresses");

        Ok(())
    }

    fn _ensure_account_exists(
        wm: &mut WalletManager<ManagedWalletInfo>,
        wallet_id: &WalletId,
        network: key_wallet::Network,
        account_type: AccountType,
    ) -> Result<(), String> {
        if let Err(e) = wm.create_account(wallet_id, account_type, network, None) {
            match e {
                WalletError::AccountCreation(msg) if msg.contains("already exists") => Ok(()),
                WalletError::AccountCreation(msg) if msg.contains("Account already exists") => {
                    Ok(())
                }
                other => Err(format!("create_account failed: {other}")),
            }
        } else {
            Ok(())
        }
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

        // Set up progress handler
        if let Some(progress_rx) = client.take_progress_receiver() {
            self.spawn_progress_handler(progress_rx);
        }

        // Set up event handler
        if let Some(event_rx) = client.take_event_receiver() {
            self.spawn_event_handler(event_rx);
        }

        *self.client.write().await = Some(client);
        *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Syncing;

        // Run sync and monitor
        self.run_sync_and_monitor(stop_token, global_cancel).await
    }

    async fn run_sync_and_monitor(
        self: Arc<Self>,
        stop_token: CancellationToken,
        global_cancel: CancellationToken,
    ) -> Result<(), String> {
        let client_guard = self.client.read().await;
        if let Some(client) = client_guard.as_ref() {
            // Wait for at least one peer to connect
            let mut waited_ms: u64 = 0;
            loop {
                // Check for cancellation
                if stop_token.is_cancelled() || global_cancel.is_cancelled() {
                    drop(client_guard);
                    let mut client_guard = self.client.write().await;
                    if let Some(mut client) = client_guard.take() {
                        let _ = client.stop().await;
                    }
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

            // Need to get a mutable reference for sync_to_tip
            drop(client_guard);
            let mut client_guard = self.client.write().await;
            if let Some(client) = client_guard.as_mut() {
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
                        *self.status.write().expect("SPV status lock poisoned") =
                            SpvStatus::Syncing;
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

                // Monitor network (this blocks until stopped or error)
                enum MonitorOutcome {
                    Completed(Result<(), dash_sdk::dash_spv::SpvError>),
                    StopRequested,
                    GlobalCancelled,
                }

                let outcome = {
                    let monitor_future = client.monitor_network();
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
                        *self.status.write().expect("SPV status lock poisoned") =
                            SpvStatus::Stopped;
                        Ok(())
                    }
                    MonitorOutcome::Completed(Err(err)) => {
                        let _ = client.stop().await;
                        let message = format!("monitor_network failed: {err}");
                        *self
                            .last_error
                            .write()
                            .expect("SPV last_error lock poisoned") = Some(message.clone());
                        *self.status.write().expect("SPV status lock poisoned") = SpvStatus::Error;
                        Err(message)
                    }
                    MonitorOutcome::StopRequested | MonitorOutcome::GlobalCancelled => {
                        let _ = client.stop().await;
                        *self.status.write().expect("SPV status lock poisoned") =
                            SpvStatus::Stopped;
                        Ok(())
                    }
                }
            } else {
                Err("Client not available".to_string())
            }
        } else {
            Err("Client not initialized".to_string())
        }
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
        DashSpvClient<
            WalletManager<ManagedWalletInfo>,
            MultiPeerNetworkManager,
            DiskStorageManager,
        >,
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
        } else if self.network == Network::Testnet {
            // For testnet testing, connect only to local Dash Core at 127.0.0.1:19999
            if let Ok(mut it) = "127.0.0.1:19999".to_socket_addrs()
                && let Some(peer) = it.next()
            {
                config.add_peer(peer);
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

impl fmt::Debug for SpvManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpvManager")
            .field("network", &self.network)
            .field("data_dir", &self.data_dir)
            .finish()
    }
}
