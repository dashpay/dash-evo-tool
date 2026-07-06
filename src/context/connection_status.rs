use crate::app::AppAction;
use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::core::CoreItem;
use crate::model::spv_status::{SpvStatus, SpvStatusSnapshot};
use dash_sdk::dash_spv::sync::{ProgressPercentage, SyncProgress as SpvSyncProgress, SyncState};
use dash_sdk::dpp::dashcore::{ChainLock, Network};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::watch;

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
    spv_status: AtomicU8,
    /// Event-driven mirror of `spv_status` for async waiters. Every transition
    /// funnelled through [`Self::set_spv_status`] is broadcast here.
    /// [`Self::begin_spv_stop`] bypasses this (it writes the atomic directly) —
    /// `Stopping` is not a waited-on state, so that is intentional.
    /// [`Self::reset`] explicitly sends `Idle` to keep the watch coherent across
    /// network switches.
    spv_status_tx: watch::Sender<SpvStatus>,
    overall_state: AtomicU8,
    /// `true` once the SPV masternode list has finished syncing, so quorum
    /// public keys are resolvable. Platform/identity sync must wait on this:
    /// firing proof-verifying DAPI calls before quorums exist makes every
    /// queried node fail locally and get banned by the SDK. Push-based from
    /// the wallet-backend `EventBridge` `on_progress` callback. Reset by
    /// [`Self::reset`] on disconnect / network switch.
    ///
    /// ADVISORY MIRROR: read only by the `get_quorum_public_key` defense
    /// backstop. `Relaxed` is sufficient because it guards no other memory —
    /// the authoritative coordinator-start ordering lives in `CoordinatorGate`,
    /// whose own `masternodes_ready`/`fired` swap is `SeqCst`.
    masternodes_ready: AtomicBool,
    // NOTE: Mutex (not RwLock) is intentional — single reader (tooltip hover),
    // single writer (poll cycle), minimal contention. RwLock overhead not justified.
    spv_last_error: Mutex<Option<String>>,
    /// Latest per-phase chain-sync progress pushed by the wallet-backend
    /// `EventBridge` `on_progress` callback. Drives the determinate progress
    /// bars in the network and wallet screens.
    spv_sync_progress: Mutex<Option<SpvSyncProgress>>,
    /// Latest downloaded-notes progress for an in-flight shielded sync pass,
    /// pushed by the wallet-backend `EventBridge` `on_shielded_sync_progress`
    /// callback. `None` between passes. Cleared by [`Self::reset`] and on pass
    /// completion.
    shielded_sync_progress: Mutex<Option<ShieldedSyncProgress>>,
    /// Latest committed-to-tree progress for an in-flight shielded sync pass,
    /// pushed by `on_shielded_tree_progress`. `None` between passes.
    shielded_tree_progress: Mutex<Option<ShieldedTreeProgress>>,
    rpc_last_error: Mutex<Option<String>>,
    last_update: Mutex<Instant>,
    spv_connected_peers: AtomicU16,
    /// When SPV first entered an active state (`Starting`/`Syncing`) with zero
    /// peers.  Reset to `None` once peers connect or SPV stops.
    spv_no_peers_since: Mutex<Option<Instant>>,
    dapi_total_endpoints: AtomicU16,
    dapi_available_endpoints: AtomicU16,
}

/// Downloaded-notes progress for an in-flight shielded sync pass (DET-shaped —
/// no upstream type crosses the seam). Network-scoped: a single pass covers
/// every viewing key on the coordinator. `None` when no pass is in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShieldedSyncProgress {
    /// Cumulative encrypted notes scanned so far in the current pass.
    pub cumulative_scanned: u64,
    /// Latest block height observed during the pass.
    pub block_height: u64,
}

/// Committed-to-tree progress for an in-flight shielded sync pass (DET-shaped).
/// Distinct from [`ShieldedSyncProgress`]: this counts commitments appended to
/// the local Orchard tree (the "checked" signal). `total_target == 0` means the
/// on-chain leaf count was unavailable, so progress is indeterminate (render a
/// spinner, not a determinate bar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShieldedTreeProgress {
    /// Cumulative tree leaf count committed so far.
    pub leaves_committed: u64,
    /// On-chain MMR total leaf count fetched at pass start; `0` = indeterminate.
    pub total_target: u64,
}

impl ConnectionStatus {
    pub fn new() -> Self {
        let (spv_status_tx, _) = watch::channel(SpvStatus::Idle);
        Self {
            rpc_online: AtomicBool::new(false),
            spv_status: AtomicU8::new(SpvStatus::Idle as u8),
            spv_status_tx,
            overall_state: AtomicU8::new(OverallConnectionState::Disconnected as u8),
            masternodes_ready: AtomicBool::new(false),
            spv_last_error: Mutex::new(None),
            spv_sync_progress: Mutex::new(None),
            shielded_sync_progress: Mutex::new(None),
            shielded_tree_progress: Mutex::new(None),
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
        self.spv_status
            .store(SpvStatus::Idle as u8, Ordering::Relaxed);
        // Keep the watch coherent: any waiter sleeping across a network switch
        // sees `Idle` and continues to wait for the next `Running`/`Error`.
        let _ = self.spv_status_tx.send_if_modified(|cur| {
            if *cur != SpvStatus::Idle {
                *cur = SpvStatus::Idle;
                true
            } else {
                false
            }
        });
        self.masternodes_ready.store(false, Ordering::Relaxed);
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
        *self
            .spv_sync_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        *self
            .shielded_sync_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        *self
            .shielded_tree_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
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

    pub fn spv_status(&self) -> SpvStatus {
        SpvStatus::from(self.spv_status.load(Ordering::Relaxed))
    }

    pub fn set_spv_status(&self, status: SpvStatus) {
        self.spv_status.store(status as u8, Ordering::Relaxed);
        // Notify async waiters (e.g. ensure_spv_synced in mcp/resolve.rs).
        // send_if_modified avoids a spurious wakeup when the caller sets the
        // same status twice in a row (common during steady-state Running).
        let _ = self.spv_status_tx.send_if_modified(|cur| {
            if *cur != status {
                *cur = status;
                true
            } else {
                false
            }
        });
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

    /// Subscribe to SPV-status changes. Each call creates a new
    /// [`watch::Receiver`] that receives the current value immediately via
    /// [`watch::Receiver::borrow_and_update`] and then wakes on every
    /// [`set_spv_status`] transition (deduped — same-value writes are
    /// suppressed). The sender is app-lifetime, so `changed()` never returns
    /// `Err` during normal operation.
    pub fn subscribe_spv_status(&self) -> watch::Receiver<SpvStatus> {
        self.spv_status_tx.subscribe()
    }

    /// Atomically claim a disconnect: transition the SPV indicator to
    /// [`SpvStatus::Stopping`] iff it is currently in a stoppable state
    /// (`Starting`/`Syncing`/`Running`), returning `true` for the single caller
    /// that won the transition.
    ///
    /// Returns `false` when no stop is needed or one is already in flight (the
    /// status is already `Stopping`, or it is `Idle`/`Stopped`/`Error`). This is
    /// the synchronous dedupe guard the Disconnect button relies on: the winning
    /// dispatch flips the indicator to `Stopping` on the same frame as the
    /// click — so the button disables immediately — and a fast second click
    /// loses the race and spawns no second teardown.
    pub fn begin_spv_stop(&self) -> bool {
        for current in [SpvStatus::Starting, SpvStatus::Syncing, SpvStatus::Running] {
            if self
                .spv_status
                .compare_exchange(
                    current as u8,
                    SpvStatus::Stopping as u8,
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                return true;
            }
        }
        false
    }

    /// Whether the SPV masternode list has finished syncing and quorum public
    /// keys are resolvable. Platform/identity sync gates on this to avoid the
    /// DAPI self-ban storm (see [`Self::masternodes_ready`] field docs).
    pub fn masternodes_ready(&self) -> bool {
        self.masternodes_ready.load(Ordering::Relaxed)
    }

    /// Record whether the SPV masternode list has finished syncing. Push-based
    /// from the wallet-backend `EventBridge` `on_progress` callback.
    pub fn set_masternodes_ready(&self, ready: bool) {
        self.masternodes_ready.store(ready, Ordering::Relaxed);
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

    /// Store the latest chain-sync progress (push-based from the wallet-backend
    /// `EventBridge` `on_progress` callback). `None` clears it (e.g. on stop).
    pub fn set_spv_sync_progress(&self, progress: Option<SpvSyncProgress>) {
        *self
            .spv_sync_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = progress;
    }

    /// Latest chain-sync progress, if any has been reported.
    pub fn spv_sync_progress(&self) -> Option<SpvSyncProgress> {
        self.spv_sync_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Store the latest shielded downloaded-notes progress (push-based from the
    /// `EventBridge` `on_shielded_sync_progress` callback). `None` clears it.
    pub fn set_shielded_sync_progress(&self, progress: Option<ShieldedSyncProgress>) {
        *self
            .shielded_sync_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = progress;
    }

    /// Latest shielded downloaded-notes progress, if a pass is in flight.
    pub fn shielded_sync_progress(&self) -> Option<ShieldedSyncProgress> {
        *self
            .shielded_sync_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Store the latest shielded committed-to-tree progress (push-based from
    /// the `EventBridge` `on_shielded_tree_progress` callback). `None` clears it.
    pub fn set_shielded_tree_progress(&self, progress: Option<ShieldedTreeProgress>) {
        *self
            .shielded_tree_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = progress;
    }

    /// Latest shielded committed-to-tree progress, if a pass is in flight.
    pub fn shielded_tree_progress(&self) -> Option<ShieldedTreeProgress> {
        *self
            .shielded_tree_progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Build a [`SpvStatusSnapshot`] for UI rendering from the live
    /// push-based state. This is the single source of truth the network and
    /// wallet screens read instead of an inert default snapshot.
    pub fn spv_status_snapshot(&self) -> SpvStatusSnapshot {
        SpvStatusSnapshot {
            status: self.spv_status(),
            sync_progress: self.spv_sync_progress(),
            last_error: self.spv_last_error(),
            started_at: None,
            last_updated: None,
            connected_peers: self.spv_connected_peers() as usize,
        }
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

    /// Test seam: force the overall connection state directly. Bypasses
    /// [`Self::refresh_state`] so a test that drives a consumer in isolation (not
    /// the throttled frame loop) sees a stable state. Compiled only under the
    /// `testing` feature.
    #[cfg(feature = "testing")]
    pub fn set_overall_state(&self, state: OverallConnectionState) {
        self.overall_state.store(state as u8, Ordering::Relaxed);
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

        // An SPV start/sync failure surfaces as `Error` regardless of DAPI
        // availability — the DAPI gate below must not mask it, otherwise a
        // failed chain-sync start would silently read as `Disconnected`.
        let state = if spv_status == SpvStatus::Error {
            OverallConnectionState::Error
        } else if !dapi_available {
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

/// Number of phases in the SPV sync pipeline — the total in the blocking
/// overlay's "Step N of {total}" counter. Single source of truth: shared with
/// the overlay adopter so the displayed total can never drift from the phase
/// count [`spv_phase_step`] actually walks.
pub const SPV_SYNC_PHASE_COUNT: u32 = 5;

/// The currently-active SPV sync phase as `(1-based step, current height)`, or
/// `None` when no phase is actively syncing yet. Mirrors the pipeline order of
/// [`spv_phase_summary`]; the height is the phase's processed tip (Blocks reports
/// `last_processed`, every other phase its `current_height`). Shared by
/// [`spv_phase_step`] (the shown counter) and [`spv_progress_token`] (the hidden
/// watchdog liveness signal) so the two can never disagree on which phase is live.
fn active_spv_phase(progress: &SpvSyncProgress) -> Option<(u32, u32)> {
    let is_syncing = |state: SyncState| state == SyncState::Syncing;
    if let Ok(p) = progress.headers()
        && is_syncing(p.state())
    {
        Some((1, p.current_height()))
    } else if let Ok(p) = progress.masternodes()
        && is_syncing(p.state())
    {
        Some((2, p.current_height()))
    } else if let Ok(p) = progress.filter_headers()
        && is_syncing(p.state())
    {
        Some((3, p.current_height()))
    } else if let Ok(p) = progress.filters()
        && is_syncing(p.state())
    {
        Some((4, p.current_height()))
    } else if let Ok(p) = progress.blocks()
        && is_syncing(p.state())
    {
        Some((SPV_SYNC_PHASE_COUNT, p.last_processed()))
    } else {
        None
    }
}

/// Map the currently-active SPV sync phase to a 1-based step number for the
/// blocking overlay's "Step N of {total}" counter — Headers=1, Masternodes=2,
/// Filter Headers=3, Filters=4, Blocks=[`SPV_SYNC_PHASE_COUNT`] — or `None` when
/// no phase is actively syncing yet. Mirrors the pipeline order of
/// [`spv_phase_summary`].
///
/// The step is always in range by construction (every arm in
/// [`active_spv_phase`] returns a literal `<= SPV_SYNC_PHASE_COUNT`), but that
/// invariant lives across two hand-maintained sites — a phase added or removed
/// from one without updating the other would drift. This is checked at runtime
/// (not `debug_assert!`, which compiles out in release, silently hiding the
/// drift from every production build) so a regression is visible in logs
/// instead of just a vanished "Step N of M" counter downstream.
pub fn spv_phase_step(progress: &SpvSyncProgress) -> Option<u32> {
    let step = active_spv_phase(progress)?.0;
    if step > SPV_SYNC_PHASE_COUNT {
        tracing::error!(
            step,
            SPV_SYNC_PHASE_COUNT,
            "SPV phase step exceeds SPV_SYNC_PHASE_COUNT — active_spv_phase and the constant have drifted; bump SPV_SYNC_PHASE_COUNT"
        );
    }
    Some(step)
}

/// A **hidden, monotonic** liveness token for the blocking overlay's no-progress
/// watchdog (A-1). It strictly increases as SPV sync advances — within a phase the
/// active phase's height climbs (low 32 bits), and across phases the 1-based step
/// climbs (high 32 bits) — so a slow-but-advancing phase (e.g. Headers on a slow
/// link) keeps resetting the watchdog even though the shown "Step N of 5" copy is
/// unchanged. NEVER rendered; no height or number leaks into user-facing copy.
/// `None` when no phase is actively syncing yet.
pub fn spv_progress_token(progress: &SpvSyncProgress) -> Option<u64> {
    let (step, height) = active_spv_phase(progress)?;
    Some(((step as u64) << 32) | height as u64)
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
    use dash_sdk::dash_spv::sync::{BlockHeadersProgress, MasternodesProgress};
    use std::sync::Arc;
    use std::time::Duration;

    /// A `SyncProgress` mid-headers-download: 5000 / 10000 (50%).
    fn syncing_progress() -> SpvSyncProgress {
        let mut headers = BlockHeadersProgress::default();
        headers.set_state(SyncState::Syncing);
        headers.update_target_height(10_000);
        headers.update_tip_height(5_000);
        let mut progress = SpvSyncProgress::default();
        progress.update_headers(headers);
        progress
    }

    #[test]
    fn spv_sync_progress_round_trips() {
        let status = ConnectionStatus::new();
        assert!(status.spv_sync_progress().is_none());

        status.set_spv_sync_progress(Some(syncing_progress()));
        let got = status.spv_sync_progress().expect("progress stored");
        let headers = got.headers().expect("headers phase present");
        assert_eq!(headers.target_height(), 10_000);
        assert_eq!(headers.current_height(), 5_000);

        status.set_spv_sync_progress(None);
        assert!(status.spv_sync_progress().is_none());
    }

    /// F-SPV-2 — the blocking overlay's "Step N of 5" counter maps to the active
    /// phase, and the summary reflects the live headers phase.
    #[test]
    fn spv_phase_step_and_summary_track_active_phase() {
        let progress = syncing_progress();
        assert_eq!(spv_phase_step(&progress), Some(1), "headers phase → step 1");
        assert!(
            spv_phase_summary(&progress).starts_with("Headers: 5000 / 10000"),
            "summary reflects the live headers phase"
        );

        // No phase actively syncing → no step (the overlay shows the generic line).
        assert_eq!(spv_phase_step(&SpvSyncProgress::default()), None);
    }

    /// Item B — the hidden watchdog liveness token advances as the active phase's
    /// height climbs (so a slow-but-advancing phase resets the no-progress
    /// watchdog), is monotonic across the step transition, and is `None` when idle.
    #[test]
    fn spv_progress_token_advances_with_height_and_is_monotonic() {
        // Idle progress → no token.
        assert_eq!(spv_progress_token(&SpvSyncProgress::default()), None);

        // Headers at 5000: a token exists.
        let progress = syncing_progress();
        let t1 = spv_progress_token(&progress).expect("syncing → token");

        // Same phase, higher tip (7000) → strictly larger token.
        let mut headers = BlockHeadersProgress::default();
        headers.set_state(SyncState::Syncing);
        headers.update_target_height(10_000);
        headers.update_tip_height(7_000);
        let mut advanced = SpvSyncProgress::default();
        advanced.update_headers(headers);
        let t2 = spv_progress_token(&advanced).expect("syncing → token");
        assert!(
            t2 > t1,
            "an advancing height yields a strictly larger token"
        );

        // The step lives in the high 32 bits, so a later phase always out-ranks an
        // earlier one regardless of height — the token is monotonic across phases.
        assert_eq!(t1 >> 32, 1, "headers maps to step 1 in the high bits");

        // Cross-phase, high-bits-dominate: a LATER phase at the LOWEST height must
        // out-rank an EARLIER phase at a near-maximal height. Headers (step 1) near
        // the top of the u32 range still loses to masternodes (step 2) at height 0,
        // because the step in the high 32 bits dominates the height in the low bits —
        // the exact invariant this test's name claims.
        let mut headers_high = BlockHeadersProgress::default();
        headers_high.set_state(SyncState::Syncing);
        headers_high.update_target_height(4_000_000_000);
        headers_high.update_tip_height(4_000_000_000);
        let mut early_high = SpvSyncProgress::default();
        early_high.update_headers(headers_high);
        let early_high_token = spv_progress_token(&early_high).expect("syncing → token");
        assert_eq!(early_high_token >> 32, 1, "headers is step 1");

        let mut masternodes_low = MasternodesProgress::default();
        masternodes_low.set_state(SyncState::Syncing);
        // current_height stays at its default 0 — the lowest possible.
        let mut late_low = SpvSyncProgress::default();
        late_low.update_masternodes(masternodes_low);
        let late_low_token = spv_progress_token(&late_low).expect("syncing → token");
        assert_eq!(late_low_token >> 32, 2, "masternodes is step 2");

        assert!(
            late_low_token > early_high_token,
            "a later phase at height 0 out-ranks an earlier phase near the u32 ceiling \
             — the high-bit step dominates the low-bit height"
        );
    }

    #[test]
    fn spv_status_snapshot_reflects_live_state() {
        let status = ConnectionStatus::new();
        status.set_spv_status(SpvStatus::Syncing);
        status.set_spv_connected_peers(3);
        status.set_spv_sync_progress(Some(syncing_progress()));

        let snap = status.spv_status_snapshot();
        assert_eq!(snap.status, SpvStatus::Syncing);
        assert_eq!(snap.connected_peers, 3);
        let progress = snap.sync_progress.expect("snapshot carries live progress");
        assert_eq!(progress.headers().unwrap().target_height(), 10_000);
    }

    #[test]
    fn reset_clears_sync_progress() {
        let status = ConnectionStatus::new();
        status.set_spv_sync_progress(Some(syncing_progress()));
        assert!(status.spv_sync_progress().is_some());

        status.reset();
        assert!(status.spv_sync_progress().is_none());
    }

    #[test]
    fn masternodes_ready_defaults_false_and_toggles() {
        let status = ConnectionStatus::new();
        assert!(
            !status.masternodes_ready(),
            "a fresh status must report masternodes not ready"
        );

        status.set_masternodes_ready(true);
        assert!(status.masternodes_ready(), "setter must flip the flag");

        status.set_masternodes_ready(false);
        assert!(!status.masternodes_ready(), "setter must clear the flag");
    }

    #[test]
    fn reset_clears_masternodes_ready() {
        let status = ConnectionStatus::new();
        status.set_masternodes_ready(true);
        assert!(status.masternodes_ready());

        status.reset();
        assert!(
            !status.masternodes_ready(),
            "reset (disconnect / network switch) must re-arm the gate"
        );
    }

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

    /// QA-006: an SPV `Error` must surface as the `Error` overall state even
    /// when DAPI is unavailable. The DAPI gate previously short-circuited to
    /// `Disconnected` first, masking a failed chain-sync start.
    #[test]
    fn spv_error_not_masked_by_unavailable_dapi() {
        let status = ConnectionStatus::new();
        status.set_dapi_status(0, 0); // DAPI unavailable
        assert!(!status.dapi_available(), "precondition: DAPI unavailable");

        status.set_spv_status(SpvStatus::Error);
        status.refresh_state();

        assert_eq!(
            status.overall_state(),
            OverallConnectionState::Error,
            "SPV error must not be masked by the DAPI gate"
        );
    }

    /// `begin_spv_stop` is the single-winner dedupe guard behind the Disconnect
    /// button: from an active state it claims the stop (flips to `Stopping`,
    /// returns `true`) exactly once; an immediate second call loses (returns
    /// `false`) so a fast double-click spawns no second teardown.
    #[test]
    fn begin_spv_stop_claims_once_from_active() {
        for active in [SpvStatus::Starting, SpvStatus::Syncing, SpvStatus::Running] {
            let status = ConnectionStatus::new();
            status.set_spv_status(active);

            assert!(
                status.begin_spv_stop(),
                "first stop from {active:?} must win the claim"
            );
            assert_eq!(
                status.spv_status(),
                SpvStatus::Stopping,
                "winning claim must flip the indicator to Stopping synchronously"
            );
            assert!(
                !status.begin_spv_stop(),
                "second stop from {active:?} must lose while one is in flight"
            );
        }
    }

    /// `begin_spv_stop` is a no-op (returns `false`, leaves status untouched)
    /// when there is nothing to stop: already inactive or already stopping.
    #[test]
    fn begin_spv_stop_noop_when_not_stoppable() {
        for inactive in [SpvStatus::Idle, SpvStatus::Stopped, SpvStatus::Error] {
            let status = ConnectionStatus::new();
            status.set_spv_status(inactive);
            assert!(
                !status.begin_spv_stop(),
                "no stop to claim from {inactive:?}"
            );
            assert_eq!(status.spv_status(), inactive, "status must be untouched");
        }

        let status = ConnectionStatus::new();
        status.set_spv_status(SpvStatus::Stopping);
        assert!(
            !status.begin_spv_stop(),
            "a stop already in flight must not be re-claimed"
        );
        assert_eq!(status.spv_status(), SpvStatus::Stopping);
    }

    /// Without an SPV error, an unavailable DAPI still reads as `Disconnected`
    /// — the new error-precedence branch must not change the DAPI gate for the
    /// non-error path.
    #[test]
    fn unavailable_dapi_reads_disconnected_without_spv_error() {
        let status = ConnectionStatus::new();
        status.set_dapi_status(0, 0);
        status.set_spv_status(SpvStatus::Syncing);
        status.refresh_state();

        assert_eq!(
            status.overall_state(),
            OverallConnectionState::Disconnected,
            "unavailable DAPI without an SPV error should read Disconnected"
        );
    }

    #[test]
    fn shielded_sync_progress_round_trips_and_clears() {
        let status = ConnectionStatus::new();
        assert!(status.shielded_sync_progress().is_none());

        status.set_shielded_sync_progress(Some(ShieldedSyncProgress {
            cumulative_scanned: 4_096,
            block_height: 123_456,
        }));
        let got = status.shielded_sync_progress().expect("progress stored");
        assert_eq!(got.cumulative_scanned, 4_096);
        assert_eq!(got.block_height, 123_456);

        status.set_shielded_sync_progress(None);
        assert!(status.shielded_sync_progress().is_none());
    }

    #[test]
    fn shielded_tree_progress_round_trips_and_clears() {
        let status = ConnectionStatus::new();
        assert!(status.shielded_tree_progress().is_none());

        status.set_shielded_tree_progress(Some(ShieldedTreeProgress {
            leaves_committed: 2_048,
            total_target: 10_000,
        }));
        let got = status.shielded_tree_progress().expect("progress stored");
        assert_eq!(got.leaves_committed, 2_048);
        assert_eq!(got.total_target, 10_000);

        status.set_shielded_tree_progress(None);
        assert!(status.shielded_tree_progress().is_none());
    }

    // ── watch channel tests ───────────────────────────────────────────────────

    /// subscribe_spv_status() returns a receiver whose first borrow yields the
    /// current (Idle) value, and a subsequent set_spv_status(Running) wakes it.
    #[tokio::test]
    async fn subscribe_spv_status_wakes_on_running() {
        let status = ConnectionStatus::new();
        let mut rx = status.subscribe_spv_status();

        // First borrow: current value is Idle (channel was created at Idle).
        assert_eq!(*rx.borrow_and_update(), SpvStatus::Idle);

        // Drive a transition from a separate task.
        let status_arc = Arc::new(status);
        let status_clone = Arc::clone(&status_arc);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            status_clone.set_spv_status(SpvStatus::Running);
        });

        // changed() must wake when Running is set.
        rx.changed().await.expect("sender is alive");
        assert_eq!(*rx.borrow_and_update(), SpvStatus::Running);
    }

    /// Lost-wakeup guard: if Running is set BEFORE subscribing, the first
    /// borrow_and_update() reads Running immediately (no wait needed).
    #[test]
    fn subscribe_after_running_reads_immediately() {
        let status = ConnectionStatus::new();
        status.set_spv_status(SpvStatus::Running);

        let mut rx = status.subscribe_spv_status();
        // borrow_and_update on a freshly subscribed receiver returns the
        // current (Running) value even though we subscribed after the set.
        assert_eq!(*rx.borrow_and_update(), SpvStatus::Running);
    }

    /// set_spv_status(Error) wakes a waiter and the value is Error.
    #[tokio::test]
    async fn subscribe_spv_status_wakes_on_error() {
        let status = ConnectionStatus::new();
        let mut rx = status.subscribe_spv_status();
        assert_eq!(*rx.borrow_and_update(), SpvStatus::Idle);

        let status_arc = Arc::new(status);
        let status_clone = Arc::clone(&status_arc);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            status_clone.set_spv_status(SpvStatus::Error);
        });

        rx.changed().await.expect("sender is alive");
        assert_eq!(*rx.borrow_and_update(), SpvStatus::Error);
    }

    /// reset() mirrors Idle into the watch; a waiter sleeping across a network
    /// switch sees Idle and resumes waiting (it does not spuriously return Ok).
    #[tokio::test]
    async fn reset_mirrors_idle_to_watch() {
        let status = ConnectionStatus::new();
        status.set_spv_status(SpvStatus::Running);

        let mut rx = status.subscribe_spv_status();
        assert_eq!(*rx.borrow_and_update(), SpvStatus::Running);

        status.reset();
        rx.changed().await.expect("sender is alive");
        assert_eq!(*rx.borrow_and_update(), SpvStatus::Idle);
    }

    #[test]
    fn reset_clears_shielded_progress() {
        let status = ConnectionStatus::new();
        status.set_shielded_sync_progress(Some(ShieldedSyncProgress {
            cumulative_scanned: 1,
            block_height: 2,
        }));
        status.set_shielded_tree_progress(Some(ShieldedTreeProgress {
            leaves_committed: 3,
            total_target: 4,
        }));
        assert!(status.shielded_sync_progress().is_some());
        assert!(status.shielded_tree_progress().is_some());

        status.reset();
        assert!(
            status.shielded_sync_progress().is_none(),
            "reset must clear in-flight shielded sync progress"
        );
        assert!(
            status.shielded_tree_progress().is_none(),
            "reset must clear in-flight shielded tree progress"
        );
    }
}
