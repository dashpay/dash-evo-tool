use crate::app::AppAction;
use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::core::CoreItem;
use crate::components::core_zmq_listener::ZMQConnectionEvent;
use crate::model::spv_status::SpvStatus;
use dash_sdk::dash_spv::sync::{ProgressPercentage, SyncProgress as SpvSyncProgress, SyncState};
use dash_sdk::dpp::dashcore::{ChainLock, Network};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering};
use std::time::{Duration, Instant};

const REFRESH_CONNECTED: Duration = Duration::from_secs(4);
const REFRESH_DISCONNECTED: Duration = Duration::from_secs(1);

const SPV_PEER_DEGRADED_TIMEOUT: Duration = Duration::from_secs(30);

/// Five-state connection indicator matching the UI's colored circle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OverallConnectionState {
    /// No connection at all — red indicator.
    Disconnected = 0,
    /// SPV active but no peers connected yet — orange indicator (faster pulse).
    Connecting = 1,
    /// All subsystems connected but still syncing data — orange indicator.
    Syncing = 2,
    /// Fully connected and operational — green indicator.
    Synced = 3,
    /// Connected but sync failed — magenta indicator with "!" glyph.
    Error = 4,
}

impl From<u8> for OverallConnectionState {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::Connecting,
            2 => Self::Syncing,
            3 => Self::Synced,
            4 => Self::Error,
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
    disable_zmq: AtomicBool,
    overall_state: AtomicU8,
    // NOTE: Mutex (not RwLock) is intentional — single reader (tooltip hover),
    // single writer (poll cycle), minimal contention. RwLock overhead not justified.
    spv_last_error: Mutex<Option<String>>,
    rpc_last_error: Mutex<Option<String>>,
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
            disable_zmq: AtomicBool::new(false),
            overall_state: AtomicU8::new(OverallConnectionState::Disconnected as u8),
            spv_last_error: Mutex::new(None),
            rpc_last_error: Mutex::new(None),
            last_update: Mutex::new(Instant::now()),
            spv_connected_peers: AtomicU16::new(0),
            spv_no_peers_since: Mutex::new(None),
            dapi_total_endpoints: AtomicU16::new(0),
            dapi_available_endpoints: AtomicU16::new(0),
        }
    }

    /// Reset all connection state. Called when switching the active network
    /// so the status reflects the new network from a clean slate.
    pub fn reset(&self) {
        self.rpc_online.store(false, Ordering::Relaxed);
        if let Ok(mut status) = self.zmq_status.lock() {
            *status = ZMQConnectionEvent::Disconnected;
        }
        self.spv_status
            .store(SpvStatus::Idle as u8, Ordering::Relaxed);
        self.disable_zmq.store(false, Ordering::Relaxed);
        self.spv_connected_peers.store(0, Ordering::Relaxed);
        *self
            .spv_no_peers_since
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        self.overall_state.store(
            OverallConnectionState::Disconnected as u8,
            Ordering::Relaxed,
        );
        if let Ok(mut err) = self.spv_last_error.lock() {
            *err = None;
        }
        if let Ok(mut err) = self.rpc_last_error.lock() {
            *err = None;
        }
        // Set last_update to epoch so the next trigger_refresh fires immediately
        *self.last_update.lock().unwrap_or_else(|e| e.into_inner()) =
            Instant::now() - REFRESH_CONNECTED;
    }

    pub fn rpc_online(&self) -> bool {
        self.rpc_online.load(Ordering::Relaxed)
    }

    pub fn set_rpc_online(&self, online: bool) {
        self.rpc_online.store(online, Ordering::Relaxed);
        if online {
            self.set_rpc_last_error(None);
        }
    }

    /// Set the last RPC error message (from chain lock polling).
    pub fn set_rpc_last_error(&self, error: Option<String>) {
        let mut err = self
            .rpc_last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *err = error;
    }

    /// Get the last RPC error message, if any.
    pub fn rpc_last_error(&self) -> Option<String> {
        self.rpc_last_error.lock().ok().and_then(|g| g.clone())
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
        // Clear the "no peers" timer when SPV leaves an active state to avoid
        // stale peer-degraded warnings in tooltips.
        if !status.is_active() {
            let mut since = self
                .spv_no_peers_since
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *since = None;
        }
    }

    /// Set the last SPV error message (push-based from SpvManager event handlers).
    pub fn set_spv_last_error(&self, error: Option<String>) {
        let mut err = self
            .spv_last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *err = error;
    }

    /// Get the last SPV error message, if any.
    pub fn spv_last_error(&self) -> Option<String> {
        self.spv_last_error.lock().ok().and_then(|g| g.clone())
    }

    /// Update SPV connected peer count and maintain `spv_no_peers_since` tracking.
    ///
    /// Called from SpvManager's network event handler when peer count changes.
    pub fn set_spv_connected_peers(&self, count: u16) {
        self.spv_connected_peers.store(count, Ordering::Relaxed);
        let mut since = self
            .spv_no_peers_since
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if count > 0 || !self.spv_status().is_active() {
            *since = None;
        } else if since.is_none() {
            *since = Some(Instant::now());
        }
    }

    /// Current SPV connected peer count (push-based from the wallet-backend
    /// `EventBridge` `on_network_event` callback).
    pub fn spv_connected_peers(&self) -> u16 {
        self.spv_connected_peers.load(Ordering::Relaxed)
    }

    pub fn disable_zmq(&self) -> bool {
        self.disable_zmq.load(Ordering::Relaxed)
    }

    pub fn set_disable_zmq(&self, disable: bool) {
        self.disable_zmq.store(disable, Ordering::Relaxed);
    }

    /// Reset the throttle timer so the next `trigger_refresh()` fires immediately.
    pub fn reset_timer(&self) {
        *self.last_update.lock().unwrap_or_else(|e| e.into_inner()) =
            Instant::now() - REFRESH_CONNECTED;
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
    /// for longer than [`SPV_PEER_DEGRADED_TIMEOUT`].
    ///
    /// If the mutex is poisoned, recovers the inner value and evaluates it.
    pub fn spv_peer_degraded(&self) -> bool {
        self.spv_no_peers_since
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some_and(|since| since.elapsed() >= SPV_PEER_DEGRADED_TIMEOUT)
    }

    pub fn spv_connected(status: SpvStatus) -> bool {
        status.is_active()
    }

    pub fn overall_state(&self) -> OverallConnectionState {
        self.overall_state.load(Ordering::Relaxed).into()
    }

    /// Recompute the overall connection state from the individual subsystem
    /// flags.
    ///
    /// Each field is read with `Ordering::Relaxed` — there is no cross-field
    /// synchronisation, so a single call may observe a mix of "old" and "new"
    /// values.  This is acceptable because the function runs on every UI
    /// frame (1-4 s cadence) and any transient inconsistency self-corrects on
    /// the next poll.
    pub fn refresh_state(&self) {
        let spv_status = self.spv_status();
        let dapi_available = self.dapi_available();

        let state = if !dapi_available {
            OverallConnectionState::Disconnected
        } else {
            let has_peers = self.spv_connected_peers.load(Ordering::Relaxed) > 0;
            match spv_status {
                SpvStatus::Running if has_peers => OverallConnectionState::Synced,
                SpvStatus::Running
                | SpvStatus::Starting
                | SpvStatus::Syncing
                | SpvStatus::Stopping => {
                    if has_peers {
                        OverallConnectionState::Syncing
                    } else {
                        OverallConnectionState::Connecting
                    }
                }
                SpvStatus::Error => OverallConnectionState::Error,
                _ => OverallConnectionState::Disconnected,
            }
        };
        self.overall_state.store(state as u8, Ordering::Relaxed);
    }

    /// Build the tooltip string for the connection indicator.
    pub fn tooltip_text(&self, _app_context: &crate::context::AppContext) -> String {
        let spv_status = self.spv_status();
        let overall = self.overall_state();
        let dapi_status = format!("DAPI: {}", self.dapi_status_label());
        let header: std::borrow::Cow<'_, str> = match overall {
            OverallConnectionState::Synced => "Ready".into(),
            OverallConnectionState::Connecting => "Connecting...".into(),
            OverallConnectionState::Syncing => "Syncing".into(),
            OverallConnectionState::Error => {
                let detail = self
                    .spv_last_error()
                    .unwrap_or_else(|| "unknown error".to_string());
                format!("SPV sync error: {detail}").into()
            }
            OverallConnectionState::Disconnected => "Disconnected".into(),
        };
        let spv_label = match spv_status {
            SpvStatus::Running => "SPV: Synced".to_string(),
            SpvStatus::Error => "SPV: Error".to_string(),
            other => format!("SPV: {other:?}"),
        };
        let degraded_warning = if self.spv_peer_degraded() {
            "\nHaving trouble finding peers. Check your connection."
        } else {
            ""
        };
        format!("{header}\n{spv_label}{degraded_warning}\n{dapi_status}")
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
            Network::Mainnet => mainnet_chainlock.is_some(),
            Network::Testnet => testnet_chainlock.is_some(),
            Network::Devnet => devnet_chainlock.is_some(),
            Network::Regtest => local_chainlock.is_some(),
        };
        self.set_rpc_online(online);
    }

    /// Updates internal connection state from a task result.
    pub fn handle_task_result(&self, task_result: &TaskResult, active_network: Network) {
        if let TaskResult::Success(message) = task_result {
            match message.as_ref() {
                BackendTaskSuccessResult::CoreItem(CoreItem::ChainLocks(
                    mainnet_chainlock,
                    testnet_chainlock,
                    devnet_chainlock,
                    local_chainlock,
                    rpc_error,
                )) => {
                    self.update_from_chainlocks(
                        active_network,
                        mainnet_chainlock,
                        testnet_chainlock,
                        devnet_chainlock,
                        local_chainlock,
                    );
                    // INTENTIONAL(SEC-005): RPC error strings shown as-is in network
                    // status tooltip. Acceptable for desktop app — helps debugging.
                    self.set_rpc_last_error(rpc_error.clone());
                    self.refresh_state();
                }
                BackendTaskSuccessResult::CoreItem(CoreItem::ChainLock(_, network)) => {
                    if *network == active_network {
                        self.set_rpc_online(true);
                        self.refresh_state();
                    }
                }
                _ => {}
            }
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

        self.refresh_dapi(app_context);
        // SPV status is push-based; no RPC chain lock polling.
        AppAction::None
    }

    fn refresh_dapi(&self, app_context: &crate::context::AppContext) {
        // SPV status is push-based: chain sync (owned by upstream
        // platform-wallet) feeds set_spv_status / set_spv_connected_peers /
        // set_spv_last_error, so no polling is needed here.

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn spv_peer_degraded_returns_false_when_none() {
        let status = ConnectionStatus::new();
        // Default state: spv_no_peers_since is None.
        assert!(!status.spv_peer_degraded());
    }

    #[test]
    fn spv_peer_degraded_returns_false_before_threshold() {
        let status = ConnectionStatus::new();
        // Set to "just now" — well within the degraded window.
        *status.spv_no_peers_since.lock().unwrap() = Some(Instant::now());
        assert!(!status.spv_peer_degraded());
    }

    #[test]
    fn spv_peer_degraded_returns_true_after_threshold() {
        let status = ConnectionStatus::new();
        // Set to a point beyond the degraded threshold.
        *status.spv_no_peers_since.lock().unwrap() =
            Some(Instant::now() - SPV_PEER_DEGRADED_TIMEOUT - Duration::from_millis(1));
        assert!(status.spv_peer_degraded());
    }

    #[test]
    fn spv_peer_degraded_clears_on_reset() {
        let status = ConnectionStatus::new();
        // Set to a point beyond the degraded threshold so it would fire.
        *status.spv_no_peers_since.lock().unwrap() =
            Some(Instant::now() - SPV_PEER_DEGRADED_TIMEOUT - Duration::from_millis(1));
        assert!(status.spv_peer_degraded());

        // After reset the timestamp should be cleared.
        status.reset();
        assert!(!status.spv_peer_degraded());
    }
}
