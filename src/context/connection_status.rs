use crate::app::AppAction;
use crate::app::TaskResult;
use crate::backend_task::BackendTask;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::components::core_zmq_listener::ZMQConnectionEvent;
use crate::spv::{CoreBackendMode, SpvStatus};
use dash_sdk::dpp::dashcore::{ChainLock, Network};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering};
use std::time::{Duration, Instant};

const REFRESH_CONNECTED: Duration = Duration::from_secs(10);
const REFRESH_DISCONNECTED: Duration = Duration::from_secs(2);
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
    overall_connected: AtomicBool,
    last_update: Mutex<Instant>,
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
            overall_connected: AtomicBool::new(false),
            last_update: Mutex::new(Instant::now()),
            dapi_total_endpoints: AtomicU16::new(0),
            dapi_available_endpoints: AtomicU16::new(0),
        }
    }

    /// Reset all connection state. Called when switching the active network
    /// so the status reflects the new network from a clean slate.
    ///
    /// `backend_mode` should be the new network's current backend mode so that
    /// `overall_connected()` and `tooltip_text()` read the correct mode immediately.
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
        self.overall_connected.store(false, Ordering::Relaxed);
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
            format!("Available ({available}/{total} endpoints)")
        } else {
            format!("All {total} endpoints banned")
        }
    }

    pub fn spv_connected(status: SpvStatus) -> bool {
        status.is_active()
    }

    pub fn overall_connected(&self) -> bool {
        self.overall_connected.load(Ordering::Relaxed)
    }

    pub fn refresh_overall(&self) {
        let backend_mode = self.backend_mode();
        let disable_zmq = self.disable_zmq();
        let spv_status = self.spv_status();
        let dapi_available = self.dapi_available();
        let connected = match backend_mode {
            CoreBackendMode::Rpc => {
                self.rpc_online() && (disable_zmq || self.zmq_connected()) && dapi_available
            }
            CoreBackendMode::Spv => Self::spv_connected(spv_status) && dapi_available,
        };
        self.overall_connected.store(connected, Ordering::Relaxed);
    }

    pub fn tooltip_text(&self) -> String {
        let backend_mode = self.backend_mode();
        let disable_zmq = self.disable_zmq();
        let spv_status = self.spv_status();
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

                if self.overall_connected() {
                    format!(
                        "Connected to Dash Core Wallet\n{rpc_status}\n{zmq_status}\n{dapi_status}"
                    )
                } else if self.rpc_online() {
                    format!(
                        "Dash Core connection incomplete\n{rpc_status}\n{zmq_status}\n{dapi_status}"
                    )
                } else {
                    format!(
                        "Disconnected from Dash Core Wallet. Click to start it.\n{rpc_status}\n{zmq_status}\n{dapi_status}"
                    )
                }
            }
            CoreBackendMode::Spv => {
                let spv_label = format!("SPV: {:?}", spv_status);
                if self.overall_connected() {
                    format!("SPV connected\n{spv_label}\n{dapi_status}")
                } else {
                    format!("SPV disconnected\n{spv_label}\n{dapi_status}")
                }
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
                    self.refresh_overall();
                }
                BackendTaskSuccessResult::CoreItem(CoreItem::ChainLock(_, network)) => {
                    if *network == active_network {
                        self.set_rpc_online(true);
                        self.refresh_overall();
                    }
                }
                _ => {}
            },
            TaskResult::Error(message) => {
                if message.contains(
                    "Failed to get best chain lock for mainnet, testnet, devnet, and local",
                ) {
                    self.set_rpc_online(false);
                    self.refresh_overall();
                }
            }
            _ => {}
        }
    }

    pub fn trigger_refresh(&self, app_context: &crate::context::AppContext) -> AppAction {
        // throttle updates to once every 2 seconds
        let mut last_update = match self.last_update.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let now = Instant::now();
        let timeout = if self.overall_connected() {
            REFRESH_CONNECTED
        } else {
            REFRESH_DISCONNECTED
        };
        if now.duration_since(*last_update) < timeout {
            return AppAction::None;
        }
        *last_update = now;

        self.refresh_zmq_and_spv(app_context);
        AppAction::BackendTask(BackendTask::CoreTask(CoreTask::GetBestChainLocks))
    }

    fn refresh_zmq_and_spv(&self, app_context: &crate::context::AppContext) {
        // Get current backend mode
        let backend_mode = app_context.core_backend_mode();
        self.set_backend_mode(backend_mode);

        match backend_mode {
            CoreBackendMode::Spv => {
                // SPV status is updated elsewhere
                let spv_status = app_context.spv_manager().status().status;
                self.set_spv_status(spv_status);
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
        if let Ok(sdk) = app_context.sdk.read() {
            let address_list = sdk.address_list();
            let total = address_list.len() as u16;
            // get_live_address() returns Option<&Uri>, so count it as 1 if available, 0 if not
            // TODO: once Dash Platform SDK supports rust-dashcore v0.42,
            // update platform and use get_live_addresses() which returns all available addresses
            //for more accurate status
            let available = if address_list.get_live_address().is_some() {
                1
            } else {
                0
            };
            self.set_dapi_status(total, available);
        }

        self.refresh_overall();
    }
}

impl Default for ConnectionStatus {
    fn default() -> Self {
        Self::new()
    }
}
