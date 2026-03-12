pub mod connection_status;
mod contract_token_db;
mod identity_db;
mod settings_db;
mod transaction_processing;
mod wallet_lifecycle;

pub(crate) use transaction_processing::get_transaction_info;

use crate::app_dir::core_cookie_path;
use crate::backend_task::error::TaskError;
use crate::components::core_zmq_listener::ZMQConnectionEvent;
use crate::config::{Config, NetworkConfig};
use crate::context_provider::Provider as RpcProvider;
use crate::context_provider_spv::SpvProvider;
use crate::database::Database;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::password_info::PasswordInfo;
use crate::model::wallet::single_key::{SingleKeyHash, SingleKeyWallet};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::sdk_wrapper::initialize_sdk;
use crate::spv::{CoreBackendMode, SpvManager};
use crate::utils::tasks::TaskManager;
use arc_swap::ArcSwap;
use connection_status::ConnectionStatus;
use crossbeam_channel::{Receiver, Sender};
use dash_sdk::Sdk;
use dash_sdk::dashcore_rpc::{Auth, Client, RpcApi};
use dash_sdk::dpp::dashcore::{Address, Network, Txid};
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::dpp::state_transition::StateTransitionSigningOptions;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::version::v11::PLATFORM_V11;
use dash_sdk::platform::DataContract;
use egui::Context;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard};

use crate::model::settings::Settings;

const ANIMATION_REFRESH_TIME: std::time::Duration = std::time::Duration::from_millis(100);

/// A guard that ensures settings cache invalidation happens atomically
///
/// This guard holds a write lock on the cached settings, preventing reads
/// until the database update is complete and the cache is properly invalidated.
pub(crate) type SettingsCacheGuard<'a> = RwLockWriteGuard<'a, Option<Settings>>;

#[derive(Debug)]
pub struct AppContext {
    pub(crate) network: Network,
    developer_mode: AtomicBool,
    pub(crate) db: Arc<Database>,
    pub(crate) sdk: ArcSwap<Sdk>,
    // Context providers for SDK, so we can switch when backend mode changes
    spv_context_provider: RwLock<SpvProvider>,
    rpc_context_provider: RwLock<RpcProvider>,
    pub(crate) config: Arc<RwLock<NetworkConfig>>,
    pub(crate) rx_zmq_status: Receiver<ZMQConnectionEvent>,
    pub(crate) sx_zmq_status: Sender<ZMQConnectionEvent>,
    pub(crate) dpns_contract: Arc<DataContract>,
    pub(crate) withdraws_contract: Arc<DataContract>,
    pub(crate) dashpay_contract: Arc<DataContract>,
    pub(crate) token_history_contract: Arc<DataContract>,
    pub(crate) keyword_search_contract: Arc<DataContract>,
    pub(crate) core_client: RwLock<Client>,
    pub(crate) has_wallet: AtomicBool,
    pub(crate) wallets: RwLock<BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>>,
    pub(crate) single_key_wallets: RwLock<BTreeMap<SingleKeyHash, Arc<RwLock<SingleKeyWallet>>>>,
    #[allow(dead_code)] // May be used for password validation
    pub(crate) password_info: Option<PasswordInfo>,
    pub(crate) transactions_waiting_for_finality: Mutex<BTreeMap<Txid, Option<AssetLockProof>>>,
    /// Whether to animate the UI elements.
    ///
    /// This is used to control animations in the UI, such as loading spinners or transitions.
    /// Disable for automated tests.
    animate: AtomicBool,
    /// Cached settings to avoid expensive database reads
    /// Use RwLock to allow multiple readers but exclusive writers for cache invalidation
    cached_settings: RwLock<Option<Settings>>,
    // subtasks started by the app context, used for graceful shutdown
    pub(crate) subtasks: Arc<TaskManager>,
    pub(crate) spv_manager: Arc<SpvManager>,
    core_backend_mode: AtomicU8,
    /// Tracks the connection status to currently active network
    pub(crate) connection_status: Arc<ConnectionStatus>,
    /// Pending wallet selection - set after creating/importing a wallet
    /// so the wallet screen can auto-select the new wallet
    pub(crate) pending_wallet_selection: Mutex<Option<WalletSeedHash>>,
    /// Currently selected HD wallet (persisted across screen navigation)
    pub(crate) selected_wallet_hash: Mutex<Option<WalletSeedHash>>,
    /// Currently selected single key wallet (persisted across screen navigation)
    pub(crate) selected_single_key_hash: Mutex<Option<SingleKeyHash>>,
    /// Cached fee multiplier permille from current epoch (1000 = 1x, 2000 = 2x)
    /// Updated when epoch info is fetched from Platform
    fee_multiplier_permille: AtomicU64,
    /// The egui context, stored for use in non-UI code paths (e.g. display_task_result).
    /// Clone is O(1) — egui::Context is Arc-backed and the same instance for the app lifetime.
    egui_ctx: egui::Context,
}

impl AppContext {
    pub fn new(
        network: Network,
        db: Arc<Database>,
        password_info: Option<PasswordInfo>,
        subtasks: Arc<TaskManager>,
        connection_status: Arc<ConnectionStatus>,
        egui_ctx: egui::Context,
    ) -> Option<Arc<Self>> {
        let config = match Config::load() {
            Ok(config) => config,
            Err(e) => {
                tracing::error!("Failed to load config: {e}");
                return None;
            }
        };

        let network_config = config.config_for_network(network).clone()?;
        let config_lock = Arc::new(RwLock::new(network_config.clone()));
        let (sx_zmq_status, rx_zmq_status) = crossbeam_channel::unbounded();

        // Create both providers; bind to app context later (post construction) due to circularity
        let spv_provider = match SpvProvider::new(db.clone(), network) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(?network, "Failed to initialize SPV provider: {e}");
                return None;
            }
        };
        let rpc_provider = match RpcProvider::new(db.clone(), network, &network_config) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(?network, "Failed to initialize RPC provider: {e}");
                return None;
            }
        };

        // Default to SPV provider initially; UI can switch backend after
        let sdk = match initialize_sdk(&network_config, network, spv_provider.clone()) {
            Ok(sdk) => sdk,
            Err(e) => {
                tracing::error!("Failed to initialize SDK: {e}");
                return None;
            }
        };
        let platform_version = sdk.version();

        let dpns_contract =
            match load_system_data_contract(SystemDataContract::DPNS, platform_version) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(?network, "Failed to load DPNS contract: {e}");
                    return None;
                }
            };

        let withdrawal_contract =
            match load_system_data_contract(SystemDataContract::Withdrawals, platform_version) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(?network, "Failed to load Withdrawals contract: {e}");
                    return None;
                }
            };

        let token_history_contract =
            match load_system_data_contract(SystemDataContract::TokenHistory, platform_version) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(?network, "Failed to load TokenHistory contract: {e}");
                    return None;
                }
            };

        let keyword_search_contract =
            match load_system_data_contract(SystemDataContract::KeywordSearch, platform_version) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(?network, "Failed to load KeywordSearch contract: {e}");
                    return None;
                }
            };

        let dashpay_contract =
            match load_system_data_contract(SystemDataContract::Dashpay, platform_version) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(?network, "Failed to load Dashpay contract: {e}");
                    return None;
                }
            };

        let addr = format!(
            "http://{}:{}",
            network_config.core_host, network_config.core_rpc_port
        );
        let core_client = match Self::create_core_rpc_client(
            &addr,
            network,
            &network_config.devnet_name,
            &network_config,
        ) {
            Ok(client) => client,
            Err(e) => {
                tracing::error!(?network, "Failed to create CoreClient: {e}");
                return None;
            }
        };

        let wallets: BTreeMap<_, _> = match db.get_wallets(&network) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!(?network, "Failed to load wallets from database: {e}");
                return None;
            }
        }
        .into_iter()
        .map(|w| (w.seed_hash(), Arc::new(RwLock::new(w))))
        .collect();

        let single_key_wallets: BTreeMap<_, _> = match db.get_single_key_wallets(network) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!(
                    ?network,
                    "Failed to load single key wallets from database: {e}"
                );
                return None;
            }
        }
        .into_iter()
        .map(|w| (w.key_hash(), Arc::new(RwLock::new(w))))
        .collect();

        let developer_mode_enabled = config.developer_mode.unwrap_or(false);

        let animate = match developer_mode_enabled {
            true => {
                tracing::debug!("developer_mode is enabled, disabling animations");
                AtomicBool::new(false)
            }
            false => AtomicBool::new(true), // Animations are enabled by default
        };

        let spv_manager = match SpvManager::new(network, Arc::clone(&config_lock), subtasks.clone())
        {
            Ok(manager) => manager,
            Err(err) => {
                tracing::error!(?err, ?network, "Failed to initialize SPV manager");
                return None;
            }
        };

        // Load the use_local_spv_node setting and apply to SPV manager
        let use_local_spv_node = db.get_use_local_spv_node().unwrap_or(false);
        spv_manager.set_use_local_node(use_local_spv_node);

        // Load the core backend mode from settings, defaulting to RPC if not set
        let saved_core_backend_mode = db
            .get_settings()
            .ok()
            .flatten()
            .map(|s| s.7) // core_backend_mode is the 8th element (index 7)
            .unwrap_or(CoreBackendMode::Rpc.as_u8());

        // If not in developer mode, force RPC mode (SPV is gated behind dev mode)
        let saved_core_backend_mode = if developer_mode_enabled {
            saved_core_backend_mode
        } else {
            CoreBackendMode::Rpc.as_u8()
        };

        // Load saved wallet selection, validating that the wallets still exist
        let (saved_wallet_hash, saved_single_key_hash) =
            db.get_selected_wallet_hashes().unwrap_or((None, None));

        // Only use the saved hash if the wallet still exists
        let selected_wallet_hash = saved_wallet_hash.filter(|h| wallets.contains_key(h));
        let selected_single_key_hash =
            saved_single_key_hash.filter(|h| single_key_wallets.contains_key(h));

        let app_context = AppContext {
            network,
            developer_mode: AtomicBool::new(developer_mode_enabled),
            db,
            sdk: ArcSwap::from_pointee(sdk),
            spv_context_provider: spv_provider.into(),
            rpc_context_provider: rpc_provider.into(),
            config: config_lock,
            sx_zmq_status,
            rx_zmq_status,
            dpns_contract: Arc::new(dpns_contract),
            withdraws_contract: Arc::new(withdrawal_contract),
            dashpay_contract: Arc::new(dashpay_contract),
            token_history_contract: Arc::new(token_history_contract),
            keyword_search_contract: Arc::new(keyword_search_contract),
            core_client: core_client.into(),
            has_wallet: (!wallets.is_empty() || !single_key_wallets.is_empty()).into(),
            wallets: RwLock::new(wallets),
            single_key_wallets: RwLock::new(single_key_wallets),
            password_info,
            transactions_waiting_for_finality: Mutex::new(BTreeMap::new()),
            animate,
            cached_settings: RwLock::new(None),
            subtasks,
            spv_manager,
            core_backend_mode: AtomicU8::new(saved_core_backend_mode),
            connection_status,
            pending_wallet_selection: Mutex::new(None),
            selected_wallet_hash: Mutex::new(selected_wallet_hash),
            selected_single_key_hash: Mutex::new(selected_single_key_hash),
            fee_multiplier_permille: AtomicU64::new(
                PlatformFeeEstimator::DEFAULT_FEE_MULTIPLIER_PERMILLE,
            ),
            egui_ctx,
        };

        let app_context = Arc::new(app_context);
        // Bind providers to the newly created app_context.
        // Only the active provider is registered with the SDK here (SPV by default).
        if let Err(e) = app_context
            .spv_context_provider
            .read()
            .map_err(|_| {
                TaskError::LockPoisoned {
                    resource: "spv_context_provider",
                }
                .to_string()
            })
            .and_then(|provider| provider.bind_app_context(app_context.clone()))
        {
            tracing::error!("Failed to bind SPV provider: {}", e);
            return None;
        }

        // If defaulting to RPC, rebind the RPC provider (overrides SPV registration above).
        if app_context.core_backend_mode() == CoreBackendMode::Rpc
            && let Err(e) = app_context
                .rpc_context_provider
                .read()
                .map_err(|_| {
                    TaskError::LockPoisoned {
                        resource: "rpc_context_provider",
                    }
                    .to_string()
                })
                .and_then(|provider| provider.bind_app_context(app_context.clone()))
        {
            tracing::error!("Failed to bind RPC provider: {}", e);
            return None;
        }

        app_context.bootstrap_loaded_wallets();

        Some(app_context)
    }

    /// Enables animations in the UI.
    ///
    /// This is used to control whether UI elements should animate, such as loading spinners or transitions.
    pub fn enable_animations(&self, animate: bool) {
        self.animate.store(animate, Ordering::Relaxed);
    }

    pub fn enable_developer_mode(&self, enable: bool) {
        self.developer_mode.store(enable, Ordering::Relaxed);
        // Animations are reverse of developer mode
        self.enable_animations(!enable);
    }

    pub fn core_backend_mode(&self) -> CoreBackendMode {
        self.core_backend_mode.load(Ordering::Relaxed).into()
    }

    pub fn connection_status(&self) -> &ConnectionStatus {
        &self.connection_status
    }

    pub fn egui_ctx(&self) -> &egui::Context {
        &self.egui_ctx
    }

    pub fn set_core_backend_mode(self: &Arc<Self>, mode: CoreBackendMode) {
        self.core_backend_mode
            .store(mode.as_u8(), Ordering::Relaxed);

        // Persist the mode to the database (hold the guard to ensure cache invalidation)
        let _guard = self.invalidate_settings_cache();
        if let Err(e) = self.db.update_core_backend_mode(mode.as_u8()) {
            tracing::error!("Failed to persist core backend mode: {}", e);
        }

        // Switch SDK context provider to match the selected backend.
        // Early returns are defensive: if code is added after this match, a failed
        // bind should not proceed with a stale provider.
        #[allow(clippy::needless_return)]
        match mode {
            CoreBackendMode::Spv => {
                if let Err(e) = self
                    .spv_context_provider
                    .read()
                    .map_err(|_| {
                        TaskError::LockPoisoned {
                            resource: "spv_context_provider",
                        }
                        .to_string()
                    })
                    .and_then(|provider| provider.bind_app_context(Arc::clone(self)))
                {
                    tracing::error!("Failed to bind SPV provider: {}", e);
                    return;
                }
            }
            CoreBackendMode::Rpc => {
                if let Err(e) = self
                    .rpc_context_provider
                    .read()
                    .map_err(|_| {
                        TaskError::LockPoisoned {
                            resource: "rpc_context_provider",
                        }
                        .to_string()
                    })
                    .and_then(|provider| provider.bind_app_context(Arc::clone(self)))
                {
                    tracing::error!("Failed to bind RPC provider: {}", e);
                    return;
                }
            }
        }
    }

    /// Get the cached fee multiplier permille (1000 = 1x, 2000 = 2x)
    pub fn fee_multiplier_permille(&self) -> u64 {
        self.fee_multiplier_permille.load(Ordering::Relaxed)
    }

    /// Update the cached fee multiplier from epoch info
    pub fn set_fee_multiplier_permille(&self, multiplier: u64) {
        self.fee_multiplier_permille
            .store(multiplier, Ordering::Relaxed);
    }

    /// Get a fee estimator configured with the cached fee multiplier.
    /// Use this instead of `PlatformFeeEstimator::new()` to get accurate fee estimates
    /// that reflect the current network fee multiplier.
    pub fn fee_estimator(&self) -> PlatformFeeEstimator {
        PlatformFeeEstimator::with_fee_multiplier(self.fee_multiplier_permille())
    }

    pub fn is_developer_mode(&self) -> bool {
        self.developer_mode.load(Ordering::Relaxed)
    }

    /// Repaints the UI if animations are enabled.
    ///
    /// Called by UI elements that need to trigger a repaint, such as loading spinners or animated icons.
    pub(super) fn repaint_animation(&self, ctx: &Context) {
        if self.animate.load(Ordering::Relaxed) {
            // Request a repaint after a short delay to allow for animations
            ctx.request_repaint_after(ANIMATION_REFRESH_TIME);
        }
    }

    pub fn platform_version(&self) -> &'static PlatformVersion {
        default_platform_version(&self.network)
    }

    pub fn state_transition_options(&self) -> Option<StateTransitionCreationOptions> {
        if self.is_developer_mode() {
            Some(StateTransitionCreationOptions {
                signing_options: StateTransitionSigningOptions {
                    allow_signing_with_any_security_level: true,
                    allow_signing_with_any_purpose: true,
                },
                batch_feature_version: None,
                method_feature_version: None,
                base_feature_version: None,
            })
        } else {
            None
        }
    }

    /// Rebuild both the Dash RPC `core_client` and the `Sdk` using the
    /// updated `NetworkConfig` from `self.config`.
    pub fn reinit_core_client_and_sdk(self: Arc<Self>) -> Result<(), String> {
        // 1. Grab a fresh snapshot of your NetworkConfig
        let cfg = {
            let cfg_lock = self
                .config
                .read()
                .map_err(|_| TaskError::LockPoisoned { resource: "config" }.to_string())?;
            cfg_lock.clone()
        };

        // Note: developer_mode is now global and managed separately

        // 2. Rebuild the RPC client with the new password
        let addr = format!("http://{}:{}", cfg.core_host, cfg.core_rpc_port);
        let new_client = Client::new(
            &addr,
            Auth::UserPass(cfg.core_rpc_user.clone(), cfg.core_rpc_password.clone()),
        )
        .map_err(|e| format!("Failed to create new Core RPC client: {e}"))?;

        // 3. Rebuild the Sdk with the updated config and current backend mode
        let new_sdk = match self.core_backend_mode() {
            CoreBackendMode::Spv => {
                // Reuse existing SPV provider (rebinding below to ensure context is set)
                let provider = self
                    .spv_context_provider
                    .read()
                    .map_err(|_| {
                        TaskError::LockPoisoned {
                            resource: "spv_context_provider",
                        }
                        .to_string()
                    })?
                    .clone();
                initialize_sdk(&cfg, self.network, provider)?
            }
            CoreBackendMode::Rpc => {
                // Create a fresh RPC provider with the new config
                let rpc_provider = RpcProvider::new(self.db.clone(), self.network, &cfg)
                    .map_err(|e| format!("Failed to init RPC provider: {e}"))?;
                // Swap in the updated RPC provider for future switches
                {
                    let mut guard = self.rpc_context_provider.write().map_err(|_| {
                        TaskError::LockPoisoned {
                            resource: "rpc_context_provider",
                        }
                        .to_string()
                    })?;
                    *guard = rpc_provider.clone();
                }
                initialize_sdk(&cfg, self.network, rpc_provider)?
            }
        };

        // 4. Swap them in
        {
            let mut client_lock = self.core_client.write().map_err(|_| {
                TaskError::LockPoisoned {
                    resource: "core_client",
                }
                .to_string()
            })?;
            *client_lock = new_client;
        }
        self.sdk.store(Arc::new(new_sdk));

        // Rebind providers to ensure they hold the new AppContext reference.
        // bind_app_context also registers the provider with the SDK, so the
        // active provider (last bound) wins.
        self.spv_context_provider
            .read()
            .map_err(|_| {
                TaskError::LockPoisoned {
                    resource: "spv_context_provider",
                }
                .to_string()
            })?
            .bind_app_context(self.clone())?;
        if self.core_backend_mode() == CoreBackendMode::Rpc {
            self.rpc_context_provider
                .read()
                .map_err(|_| {
                    TaskError::LockPoisoned {
                        resource: "rpc_context_provider",
                    }
                    .to_string()
                })?
                .bind_app_context(self.clone())?;
        }

        Ok(())
    }

    /// Create a Core RPC client for the given URL, trying cookie authentication
    /// first and falling back to user/password credentials.
    fn create_core_rpc_client(
        url: &str,
        network: Network,
        devnet_name: &Option<String>,
        cfg: &NetworkConfig,
    ) -> Result<Client, TaskError> {
        if let Ok(cookie_path) = core_cookie_path(network, devnet_name) {
            if let Ok(client) = Client::new(url, Auth::CookieFile(cookie_path.clone())) {
                return Ok(client);
            }
            tracing::debug!(
                "Failed to authenticate using .cookie file at {:?}, falling back to user/pass",
                cookie_path,
            );
        }
        Client::new(
            url,
            Auth::UserPass(cfg.core_rpc_user.clone(), cfg.core_rpc_password.clone()),
        )
        .map_err(|e| format!("Failed to create Core RPC client: {e}").into())
    }

    /// Build an RPC client targeting a specific Core wallet by name.
    /// Returns the base (no-wallet) client if `wallet_name` is `None`.
    pub fn core_client_for_wallet(&self, wallet_name: Option<&str>) -> Result<Client, TaskError> {
        let cfg = self
            .config
            .read()
            .map_err(|_| "Config lock poisoned".to_string())?;
        let base = format!("http://{}:{}", cfg.core_host, cfg.core_rpc_port);
        let url = match wallet_name {
            Some(name) if !name.is_empty() => {
                if name.contains("..") {
                    return Err(format!("Invalid Core wallet name: '{}'", name).into());
                }
                let encoded = urlencoding::encode(name);
                format!("{}/wallet/{}", base, encoded)
            }
            _ => base,
        };
        Self::create_core_rpc_client(&url, self.network, &cfg.devnet_name, &cfg)
    }

    /// Import an address into the correct Core wallet if it's not already known.
    /// Uses `core_wallet_name` to target the right wallet on multi-wallet nodes.
    /// No-op if the address is already watched/mine.
    pub fn ensure_address_imported(
        &self,
        address: &Address,
        core_wallet_name: Option<&str>,
        label: Option<&str>,
    ) -> Result<(), TaskError> {
        let client = self.core_client_for_wallet(core_wallet_name)?;
        let info = client.get_address_info(address)?;
        if !(info.is_watchonly || info.is_mine) {
            client.import_address(address, label, Some(false))?;
        }
        Ok(())
    }

    /// Import address into Core, ignoring errors. For best-effort registration.
    pub fn try_import_address(
        &self,
        address: &Address,
        core_wallet_name: Option<&str>,
        label: Option<&str>,
    ) {
        if let Ok(client) = self.core_client_for_wallet(core_wallet_name) {
            let _ = client.import_address(address, label, Some(false));
        }
    }

    /// List wallets currently loaded in Dash Core.
    pub fn list_core_wallets(&self) -> Result<Vec<String>, TaskError> {
        let client = self.core_client_for_wallet(None)?;
        client
            .list_wallets()
            .map_err(|e| format!("Failed to list Core wallets: {e}").into())
    }

    /// Try to detect which loaded Core wallet owns the given address.
    ///
    /// Returns `Ok(Some(name))` if exactly one wallet recognizes it,
    /// `Ok(None)` if ambiguous (0 or >1 matches).
    pub fn try_detect_core_wallet_for_address(
        &self,
        address: &Address,
    ) -> Result<Option<String>, TaskError> {
        let core_wallets = self.list_core_wallets()?;
        if core_wallets.is_empty() {
            return Err("No wallets loaded in Dash Core".to_string().into());
        }
        if core_wallets.len() == 1 {
            return Ok(Some(core_wallets.into_iter().next().unwrap()));
        }
        // Multiple wallets — check which one recognizes the address
        let mut matches = Vec::new();
        for wallet_name in &core_wallets {
            let client = self.core_client_for_wallet(Some(wallet_name))?;
            match client.get_address_info(address) {
                Ok(info) if info.is_mine || info.is_watchonly => {
                    matches.push(wallet_name.clone());
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!(?e, wallet_name, "get_address_info failed");
                }
            }
        }
        match matches.len() {
            1 => Ok(Some(matches.into_iter().next().unwrap())),
            _ => Ok(None), // 0 or >1 matches — ambiguous
        }
    }
}

/// Returns the default platform version for the given network.
pub(crate) const fn default_platform_version(network: &Network) -> &'static PlatformVersion {
    // TODO: Ideally use sdk.load().version() but this is a free function with no sdk access
    match network {
        Network::Dash => &PLATFORM_V11,
        Network::Testnet => &PLATFORM_V11,
        Network::Devnet => &PLATFORM_V11,
        Network::Regtest => &PLATFORM_V11,
        _ => panic!("Unsupported network for default_platform_version"),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wallet_name_with_spaces_is_url_encoded() {
        let base = "http://127.0.0.1:9998";
        let name = "my test wallet";
        let encoded = urlencoding::encode(name);
        let url = format!("{}/wallet/{}", base, encoded);
        assert_eq!(url, "http://127.0.0.1:9998/wallet/my%20test%20wallet");
        assert!(!url.contains(' '));
    }
}
