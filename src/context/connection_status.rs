use crate::app::AppAction;
use crate::app::TaskResult;
use crate::backend_task::BackendTask;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::components::core_zmq_listener::ZMQConnectionEvent;
use crate::spv::{CoreBackendMode, SpvStatus};
use dash_sdk::dash_spv::sync::{ProgressPercentage, SyncProgress as SpvSyncProgress, SyncState};
use dash_sdk::dpp::dashcore::{ChainLock, Network};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering};
use std::time::{Duration, Instant};

const REFRESH_CONNECTED: Duration = Duration::from_secs(4);
const REFRESH_DISCONNECTED: Duration = Duration::from_secs(1);

const SPV_PEER_TIMEOUT: Duration = Duration::from_secs(10);

/// Three-state connection indicator matching the UI's red/orange/green circle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OverallConnectionState {
    /// No connection at all — red indicator.
    Disconnected = 0,
    /// All subsystems connected but still syncing data — orange indicator.
    Syncing = 1,
    /// Fully connected and operational — green indicator.
    Synced = 2,
}

impl From<u8> for OverallConnectionState {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::Syncing,
            2 => Self::Synced,
            _ => Self::Disconnected,
        }
    }
}

/// Tracks the connection status to currently active network, and provides helper methods
/// to determine overall connectivity status.
///
/// Supports Dash Core and SPV.
#[derive(Debug)]
pub struct ConnectionStatus {
    rpc_online: AtomicBool,
    zmq_status: Mutex<ZMQConnectionEvent>,
    spv_status: AtomicU8,
    backend_mode: AtomicU8,
    disable_zmq: AtomicBool,
    overall_state: AtomicU8,
    last_update: Mutex<Instant>,
    spv_connected_peers: AtomicU16,
    /// When SPV first entered an active state (`Starting`/`Syncing`) with zero
    /// peers.  Reset to `None` once peers connect or SPV stops.
    spv_no_peers_since: Mutex<Option<Instant>>,
    dapi_total_endpoints: AtomicU16,
    dapi_available_endpoints: AtomicU16,
}

impl ConnectionStatus {
    pub fn new() -> Self {
        Self {
            rpc_online: AtomicBool::new(false),
            zmq_status: Mutex::new(ZMQConnectionEvent::Disconnected),
            spv_status: AtomicU8::new(SpvStatus::Idle as u8),
            backend_mode: AtomicU8::new(CoreBackendMode::Rpc.as_u8()),
            disable_zmq: AtomicBool::new(false),
            overall_state: AtomicU8::new(OverallConnectionState::Disconnected as u8),
            last_update: Mutex::new(Instant::now()),
            spv_connected_peers: AtomicU16::new(0),
            spv_no_peers_since: Mutex::new(None),
            dapi_total_endpoints: AtomicU16::new(0),
            dapi_available_endpoints: AtomicU16::new(0),
        }
    }

    /// Reset all connection state. Called when switching the active network
    /// so the status reflects the new network from a clean slate.
    ///
    /// `backend_mode` should be the new network's current backend mode so that
    /// `overall_state()` and `tooltip_text()` read the correct mode immediately.
    pub fn reset(&self, backend_mode: CoreBackendMode) {
        self.rpc_online.store(false, Ordering::Relaxed);
        if let Ok(mut status) = self.zmq_status.lock() {
            *status = ZMQConnectionEvent::Disconnected;
        }
        self.spv_status
            .store(SpvStatus::Idle as u8, Ordering::Relaxed);
        self.backend_mode
            .store(backend_mode.as_u8(), Ordering::Relaxed);
        self.disable_zmq.store(false, Ordering::Relaxed);
        self.spv_connected_peers.store(0, Ordering::Relaxed);
        if let Ok(mut since) = self.spv_no_peers_since.lock() {
            *since = None;
        }
        self.overall_state.store(
            OverallConnectionState::Disconnected as u8,
            Ordering::Relaxed,
        );
        // Set last_update to epoch so the next trigger_refresh fires immediately
        if let Ok(mut last) = self.last_update.lock() {
            *last = Instant::now() - REFRESH_CONNECTED;
        }
    }

    pub fn rpc_online(&self) -> bool {
        self.rpc_online.load(Ordering::Relaxed)
    }

    pub fn set_rpc_online(&self, online: bool) {
        self.rpc_online.store(online, Ordering::Relaxed);
    }

    pub fn zmq_connected(&self) -> bool {
        self.zmq_status
            .lock()
            .map(|status| matches!(*status, ZMQConnectionEvent::Connected))
            .unwrap_or(false)
    }

    pub fn set_zmq_status(&self, event: ZMQConnectionEvent) {
        if let Ok(mut status) = self.zmq_status.lock() {
            *status = event;
        }
    }

    pub fn spv_status(&self) -> SpvStatus {
        SpvStatus::from(self.spv_status.load(Ordering::Relaxed))
    }

    pub fn set_spv_status(&self, status: SpvStatus) {
        self.spv_status.store(status as u8, Ordering::Relaxed);
    }

    pub fn backend_mode(&self) -> CoreBackendMode {
        self.backend_mode.load(Ordering::Relaxed).into()
    }

    pub fn set_backend_mode(&self, mode: CoreBackendMode) {
        self.backend_mode.store(mode.as_u8(), Ordering::Relaxed);
    }

    pub fn disable_zmq(&self) -> bool {
        self.disable_zmq.load(Ordering::Relaxed)
    }

    pub fn set_disable_zmq(&self, disable: bool) {
        self.disable_zmq.store(disable, Ordering::Relaxed);
    }

    /// Reset the throttle timer so the next `trigger_refresh()` fires immediately.
    pub fn reset_timer(&self) {
        if let Ok(mut last) = self.last_update.lock() {
            *last = Instant::now() - REFRESH_CONNECTED;
        }
    }

    pub fn dapi_total_endpoints(&self) -> u16 {
        self.dapi_total_endpoints.load(Ordering::Relaxed)
    }

    pub fn dapi_available_endpoints(&self) -> u16 {
        self.dapi_available_endpoints.load(Ordering::Relaxed)
    }

    pub fn dapi_available(&self) -> bool {
        self.dapi_available_endpoints.load(Ordering::Relaxed) > 0
    }

    pub fn set_dapi_status(&self, total: u16, available: u16) {
        self.dapi_total_endpoints.store(total, Ordering::Relaxed);
        self.dapi_available_endpoints
            .store(available, Ordering::Relaxed);
    }

    /// Returns the DAPI status label suitable for display.
    pub fn dapi_status_label(&self) -> String {
        let total = self.dapi_total_endpoints();
        let available = self.dapi_available_endpoints();
        if total == 0 {
            "No endpoints configured".to_string()
        } else if available > 0 {
            format!("Available ({available} unbanned / {total} total endpoints)")
        } else {
            format!("All {total} endpoints banned")
        }
    }

    /// Returns `true` if SPV has been active with zero connected peers
    /// for longer than [`SPV_PEER_TIMEOUT`].
    pub fn spv_peer_timed_out(&self) -> bool {
        self.spv_no_peers_since
            .lock()
            .ok()
            .and_then(|g| *g)
            .is_some_and(|since| since.elapsed() >= SPV_PEER_TIMEOUT)
    }

    pub fn spv_connected(status: SpvStatus) -> bool {
        status.is_active()
    }

    pub fn overall_state(&self) -> OverallConnectionState {
        self.overall_state.load(Ordering::Relaxed).into()
    }

    pub fn refresh_state(&self) {
        let backend_mode = self.backend_mode();
        let disable_zmq = self.disable_zmq();
        let spv_status = self.spv_status();
        let dapi_available = self.dapi_available();

        let state = match backend_mode {
            CoreBackendMode::Rpc => {
                // RPC mode: no intermediate syncing state exposed, so red or green only.
                if self.rpc_online() && (disable_zmq || self.zmq_connected()) && dapi_available {
                    OverallConnectionState::Synced
                } else {
                    OverallConnectionState::Disconnected
                }
            }
            CoreBackendMode::Spv => {
                if !dapi_available {
                    OverallConnectionState::Disconnected
                } else {
                    match spv_status {
                        SpvStatus::Running => OverallConnectionState::Synced,
                        SpvStatus::Starting | SpvStatus::Syncing | SpvStatus::Stopping => {
                            OverallConnectionState::Syncing
                        }
                        _ => OverallConnectionState::Disconnected,
                    }
                }
            }
        };
        self.overall_state.store(state as u8, Ordering::Relaxed);
    }

    /// Build the tooltip string for the connection indicator.
    ///
    /// In SPV mode, fetches sync progress from the [`SpvManager`] to display
    /// a detailed phase summary (e.g. `"SPV: Headers: 12345 / 27000 (45%)"`)
    /// instead of the bare `"SPV: Syncing"`.
    pub fn tooltip_text(&self, app_context: &crate::context::AppContext) -> String {
        let backend_mode = self.backend_mode();
        let disable_zmq = self.disable_zmq();
        let spv_status = self.spv_status();
        let overall = self.overall_state();
        let dapi_status = format!("DAPI: {}", self.dapi_status_label());
        match backend_mode {
            CoreBackendMode::Rpc => {
                let rpc_status = if self.rpc_online() {
                    "RPC: Connected"
                } else {
                    "RPC: Disconnected"
                };
                let zmq_status = if disable_zmq {
                    "ZMQ: Disabled"
                } else if self.zmq_connected() {
                    "ZMQ: Connected"
                } else {
                    "ZMQ: Disconnected"
                };

                let header = match overall {
                    OverallConnectionState::Synced => "Connected to Dash Core Wallet",
                    // RPC mode doesn't currently produce Syncing, but kept for forward-compat.
                    OverallConnectionState::Syncing => "Syncing to Dash Core Wallet",
                    OverallConnectionState::Disconnected if self.rpc_online() => {
                        "Dash Core connection incomplete"
                    }
                    OverallConnectionState::Disconnected => {
                        "Disconnected from Dash Core Wallet. Click to start it."
                    }
                };
                format!("{header}\n{rpc_status}\n{zmq_status}\n{dapi_status}")
            }
            CoreBackendMode::Spv => {
                let header = match overall {
                    OverallConnectionState::Synced => "Ready",
                    OverallConnectionState::Syncing => "Syncing",
                    OverallConnectionState::Disconnected => "Disconnected",
                };
                let spv_label = if spv_status == SpvStatus::Running {
                    "SPV: Synced".to_string()
                } else {
                    app_context
                        .spv_manager()
                        .status()
                        .sync_progress
                        .as_ref()
                        .map(|p| format!("SPV: {}", spv_phase_summary(p)))
                        .unwrap_or_else(|| format!("SPV: {:?}", spv_status))
                };
                format!("{header}\n{spv_label}\n{dapi_status}")
            }
        }
    }

    pub fn update_from_chainlocks(
        &self,
        network: Network,
        mainnet_chainlock: &Option<ChainLock>,
        testnet_chainlock: &Option<ChainLock>,
        devnet_chainlock: &Option<ChainLock>,
        local_chainlock: &Option<ChainLock>,
    ) {
        let online = match network {
            Network::Dash => mainnet_chainlock.is_some(),
            Network::Testnet => testnet_chainlock.is_some(),
            Network::Devnet => devnet_chainlock.is_some(),
            Network::Regtest => local_chainlock.is_some(),
            _ => false,
        };
        self.set_rpc_online(online);
    }

    pub fn handle_task_result(&self, task_result: &TaskResult, active_network: Network) {
        match task_result {
            TaskResult::Success(message) => match message.as_ref() {
                BackendTaskSuccessResult::CoreItem(CoreItem::ChainLocks(
                    mainnet_chainlock,
                    testnet_chainlock,
                    devnet_chainlock,
                    local_chainlock,
                )) => {
                    self.update_from_chainlocks(
                        active_network,
                        mainnet_chainlock,
                        testnet_chainlock,
                        devnet_chainlock,
                        local_chainlock,
                    );
                    self.refresh_state();
                }
                BackendTaskSuccessResult::CoreItem(CoreItem::ChainLock(_, network)) => {
                    if *network == active_network {
                        self.set_rpc_online(true);
                        self.refresh_state();
                    }
                }
                _ => {}
            },
            TaskResult::Error(message) => {
                if message.contains(
                    "Failed to get best chain lock for mainnet, testnet, devnet, and local",
                ) {
                    self.set_rpc_online(false);
                    self.refresh_state();
                }
            }
            _ => {}
        }
    }

    pub fn trigger_refresh(&self, app_context: &crate::context::AppContext) -> AppAction {
        // throttle updates based on connection state (1s disconnected, 4s connected)
        let mut last_update = match self.last_update.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let now = Instant::now();
        let timeout = if self.spv_status() == SpvStatus::Stopping {
            // Poll frequently during SPV shutdown so the UI updates
            // within ~200ms of the Stopping → Stopped transition.
            Duration::from_millis(200)
        } else if self.overall_state() == OverallConnectionState::Synced {
            REFRESH_CONNECTED
        } else {
            REFRESH_DISCONNECTED
        };
        if now.duration_since(*last_update) < timeout {
            return AppAction::None;
        }
        *last_update = now;

        self.refresh_zmq_and_spv(app_context);
        // SPV mode does not use RPC chain lock polling.
        if self.backend_mode() == CoreBackendMode::Spv {
            return AppAction::None;
        }
        AppAction::BackendTask(BackendTask::CoreTask(CoreTask::GetBestChainLocks))
    }

    fn refresh_zmq_and_spv(&self, app_context: &crate::context::AppContext) {
        // Get current backend mode
        let backend_mode = app_context.core_backend_mode();
        self.set_backend_mode(backend_mode);

        match backend_mode {
            CoreBackendMode::Spv => {
                let snapshot = app_context.spv_manager().status();
                tracing::trace!(
                    "ConnectionStatus: polled SPV status = {:?}",
                    snapshot.status
                );
                self.set_spv_status(snapshot.status);
                let peers = snapshot.connected_peers as u16;
                self.spv_connected_peers.store(peers, Ordering::Relaxed);

                // Track how long we've been active with zero peers.
                if let Ok(mut since) = self.spv_no_peers_since.lock() {
                    if peers > 0 || !snapshot.status.is_active() {
                        *since = None;
                    } else if since.is_none() {
                        *since = Some(Instant::now());
                    }
                }
            }
            CoreBackendMode::Rpc => {
                // Update ZMQ status if there's a new event
                let disable_zmq = app_context
                    .get_settings()
                    .ok()
                    .flatten()
                    .map(|s| s.disable_zmq)
                    .unwrap_or(false);
                self.set_disable_zmq(disable_zmq);

                if let Ok(event) = app_context.rx_zmq_status.try_recv() {
                    self.set_zmq_status(event);
                }
            }
        }

        // Update DAPI endpoint status
        {
            let sdk = app_context.sdk.load();
            let address_list = sdk.address_list();
            let total = address_list.len() as u16;
            let available = address_list.get_live_addresses().len() as u16;
            self.set_dapi_status(total, available);
        }

        self.refresh_state();
    }
}

/// Compact text summary of the active SPV sync phase.
///
/// Returns e.g. `"Headers: 12345 / 27000 (45%)"`, `"Masternodes: 800 / 2000 (40%)"`,
/// or `"syncing..."` if no phase is actively syncing.
///
/// Phases are checked in pipeline execution order (early → late) so the user
/// sees progression from headers through to blocks.
pub fn spv_phase_summary(progress: &SpvSyncProgress) -> String {
    // Check phases in order of execution
    if let Ok(headers) = progress.headers()
        && headers.state() == SyncState::Syncing
    {
        let (cur, tgt) = (headers.current_height(), headers.target_height());
        return format!("Headers: {} / {} ({}%)", cur, tgt, pct(cur, tgt));
    }

    if let Ok(mn) = progress.masternodes()
        && mn.state() == SyncState::Syncing
    {
        let (cur, tgt) = (mn.current_height(), mn.target_height());
        return format!("Masternodes: {} / {} ({}%)", cur, tgt, pct(cur, tgt));
    }

    if let Ok(fh) = progress.filter_headers()
        && fh.state() == SyncState::Syncing
    {
        let (cur, tgt) = (fh.current_height(), fh.target_height());
        return format!("Filter Headers: {} / {} ({}%)", cur, tgt, pct(cur, tgt));
    }

    if let Ok(filters) = progress.filters()
        && filters.state() == SyncState::Syncing
    {
        let (cur, tgt) = (filters.current_height(), filters.target_height());
        return format!("Filters: {} / {} ({}%)", cur, tgt, pct(cur, tgt));
    }

    if let Ok(blocks) = progress.blocks()
        && blocks.state() == SyncState::Syncing
    {
        // Blocks doesn't expose its own target_height; use the best available
        // approximation: max of headers target and blocks last_processed.
        let target = progress
            .headers()
            .ok()
            .map(|h| h.target_height())
            .unwrap_or(0)
            .max(blocks.last_processed());
        let cur = blocks.last_processed();
        return format!("Blocks: {} / {} ({}%)", cur, target, pct(cur, target));
    }

    "syncing...".to_string()
}

fn pct(current: u32, target: u32) -> u32 {
    if target == 0 {
        0
    } else {
        ((current as f64 / target as f64) * 100.0).clamp(0.0, 100.0) as u32
    }
}

impl Default for ConnectionStatus {
    fn default() -> Self {
        Self::new()
    }
}
