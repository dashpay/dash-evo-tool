pub mod connection_status;
mod contract_token_db;
mod identity_db;
mod settings_db;
mod transaction_processing;
mod wallet_lifecycle;

pub(crate) use transaction_processing::get_transaction_info_via_dapi;

use crate::app_dir::core_cookie_path;
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
use connection_status::ConnectionStatus;
use crossbeam_channel::{Receiver, Sender};
use dash_sdk::Sdk;
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::{Network, Txid};
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
    #[allow(dead_code)] // May be used for devnet identification
    pub(crate) devnet_name: Option<String>,
    pub(crate) db: Arc<Database>,
    pub(crate) sdk: RwLock<Sdk>,
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
}

impl AppContext {
    pub fn new(
        network: Network,
        db: Arc<Database>,
        password_info: Option<PasswordInfo>,
        subtasks: Arc<TaskManager>,
        connection_status: Arc<ConnectionStatus>,
    ) -> Option<Arc<Self>> {
        let config = match Config::load() {
            Ok(config) => config,
            Err(e) => {
                println!("Failed to load config: {e}");
                return None;
            }
        };

        let network_config = config.config_for_network(network).clone()?;
        let config_lock = Arc::new(RwLock::new(network_config.clone()));
        let (sx_zmq_status, rx_zmq_status) = crossbeam_channel::unbounded();

        // Create both providers; bind to app context later (post construction) due to circularity
        let spv_provider =
            SpvProvider::new(db.clone(), network).expect("Failed to initialize SPV provider");
        let rpc_provider = RpcProvider::new(db.clone(), network, &network_config)
            .expect("Failed to initialize RPC provider");

        // Default to SPV provider initially; UI can switch backend after
        let sdk = initialize_sdk(&network_config, network, spv_provider.clone());
        let platform_version = sdk.version();

        let dpns_contract = load_system_data_contract(SystemDataContract::DPNS, platform_version)
            .expect("expected to load dpns contract");

        let withdrawal_contract =
            load_system_data_contract(SystemDataContract::Withdrawals, platform_version)
                .expect("expected to get withdrawal contract");

        let token_history_contract =
            load_system_data_contract(SystemDataContract::TokenHistory, platform_version)
                .expect("expected to get token history contract");

        let keyword_search_contract =
            load_system_data_contract(SystemDataContract::KeywordSearch, platform_version)
                .expect("expected to get keyword search contract");

        let dashpay_contract =
            load_system_data_contract(SystemDataContract::Dashpay, platform_version)
                .expect("expected to get dashpay contract");

        let addr = format!(
            "http://{}:{}",
            network_config.core_host, network_config.core_rpc_port
        );
        let cookie_path = core_cookie_path(network, &network_config.devnet_name)
            .expect("expected to get cookie path");

        // Try cookie authentication first
        let core_client = match Client::new(&addr, Auth::CookieFile(cookie_path.clone())) {
            Ok(client) => Ok(client),
            Err(_) => {
                // If cookie auth fails, try user/password authentication
                tracing::info!(
                    "Failed to authenticate using .cookie file at {:?}, falling back to user/pass",
                    cookie_path,
                );
                Client::new(
                    &addr,
                    Auth::UserPass(
                        network_config.core_rpc_user.to_string(),
                        network_config.core_rpc_password.to_string(),
                    ),
                )
            }
        }
        .expect("Failed to create CoreClient");

        let wallets: BTreeMap<_, _> = db
            .get_wallets(&network)
            .expect("expected to get wallets")
            .into_iter()
            .map(|w| (w.seed_hash(), Arc::new(RwLock::new(w))))
            .collect();

        let single_key_wallets: BTreeMap<_, _> = db
            .get_single_key_wallets(network)
            .expect("expected to get single key wallets")
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
            devnet_name: None,
            db,
            sdk: sdk.into(),
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
        };

        let app_context = Arc::new(app_context);
        // Bind providers to the newly created app_context.
        // Only the active provider is registered with the SDK here (SPV by default).
        if let Err(e) = app_context
            .spv_context_provider
            .read()
            .map_err(|_| "SPV provider lock poisoned".to_string())
            .and_then(|provider| provider.bind_app_context(app_context.clone()))
        {
            tracing::error!("Failed to bind SPV provider: {}", e);
            return None;
        }

        // If defaulting to RPC is desired, swap provider after binding.
        if app_context.core_backend_mode() == CoreBackendMode::Rpc {
            if let Err(e) = app_context
                .rpc_context_provider
                .read()
                .map_err(|_| "RPC provider lock poisoned".to_string())
                .and_then(|provider| provider.bind_app_context(app_context.clone()))
            {
                tracing::error!("Failed to bind RPC provider: {}", e);
                return None;
            }
        } else {
            // Ensure SDK uses the SPV provider
            let sdk_lock = match app_context.sdk.write() {
                Ok(lock) => lock,
                Err(_) => {
                    tracing::error!("SDK lock poisoned");
                    return None;
                }
            };
            let provider = match app_context.spv_context_provider.read() {
                Ok(p) => p.clone(),
                Err(_) => {
                    tracing::error!("SPV provider lock poisoned");
                    return None;
                }
            };
            sdk_lock.set_context_provider(provider);
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

    pub fn set_core_backend_mode(self: &Arc<Self>, mode: CoreBackendMode) {
        self.core_backend_mode
            .store(mode.as_u8(), Ordering::Relaxed);

        // Persist the mode to the database (hold the guard to ensure cache invalidation)
        let _guard = self.invalidate_settings_cache();
        if let Err(e) = self.db.update_core_backend_mode(mode.as_u8()) {
            tracing::error!("Failed to persist core backend mode: {}", e);
        }

        // Switch SDK context provider to match the selected backend
        match mode {
            CoreBackendMode::Spv => {
                // Make sure SPV provider knows about the app context
                if let Err(e) = self
                    .spv_context_provider
                    .read()
                    .map_err(|_| "SPV provider lock poisoned".to_string())
                    .and_then(|provider| provider.bind_app_context(Arc::clone(self)))
                {
                    tracing::error!("Failed to bind SPV provider: {}", e);
                    return;
                }
                let sdk = match self.sdk.write() {
                    Ok(lock) => lock,
                    Err(_) => {
                        tracing::error!("SDK lock poisoned in set_core_backend_mode");
                        return;
                    }
                };
                let provider = match self.spv_context_provider.read() {
                    Ok(p) => p.clone(),
                    Err(_) => {
                        tracing::error!("SPV provider lock poisoned");
                        return;
                    }
                };
                sdk.set_context_provider(provider);
            }
            CoreBackendMode::Rpc => {
                // RPC provider binding also sets itself on the SDK
                if let Err(e) = self
                    .rpc_context_provider
                    .read()
                    .map_err(|_| "RPC provider lock poisoned".to_string())
                    .and_then(|provider| provider.bind_app_context(Arc::clone(self)))
                {
                    tracing::error!("Failed to bind RPC provider: {}", e);
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
                .map_err(|_| "Config lock poisoned".to_string())?;
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
                    .map_err(|_| "SPV provider lock poisoned".to_string())?
                    .clone();
                initialize_sdk(&cfg, self.network, provider)
            }
            CoreBackendMode::Rpc => {
                // Create a fresh RPC provider with the new config
                let rpc_provider = RpcProvider::new(self.db.clone(), self.network, &cfg)
                    .map_err(|e| format!("Failed to init RPC provider: {e}"))?;
                // Swap in the updated RPC provider for future switches
                {
                    let mut guard = self
                        .rpc_context_provider
                        .write()
                        .map_err(|_| "RPC provider lock poisoned".to_string())?;
                    *guard = rpc_provider.clone();
                }
                initialize_sdk(&cfg, self.network, rpc_provider)
            }
        };

        // 4. Swap them in
        {
            let mut client_lock = self
                .core_client
                .write()
                .map_err(|_| "Core client lock poisoned".to_string())?;
            *client_lock = new_client;
        }
        {
            let mut sdk_lock = self
                .sdk
                .write()
                .map_err(|_| "SDK lock poisoned".to_string())?;
            *sdk_lock = new_sdk;
        }

        // Rebind providers to ensure they hold the new AppContext reference
        self.spv_context_provider
            .read()
            .map_err(|_| "SPV provider lock poisoned".to_string())?
            .bind_app_context(self.clone())?;
        if self.core_backend_mode() == CoreBackendMode::Rpc {
            self.rpc_context_provider
                .read()
                .map_err(|_| "RPC provider lock poisoned".to_string())?
                .bind_app_context(self.clone())?;
        } else {
            let sdk_lock = self
                .sdk
                .write()
                .map_err(|_| "SDK lock poisoned".to_string())?;
            let provider = self
                .spv_context_provider
                .read()
                .map_err(|_| "SPV provider lock poisoned".to_string())?
                .clone();
            sdk_lock.set_context_provider(provider);
        }

        Ok(())
    }
}

/// Returns the default platform version for the given network.
pub(crate) const fn default_platform_version(network: &Network) -> &'static PlatformVersion {
    // TODO: Use self.sdk.read().unwrap().version() instead of hardcoding
    match network {
        Network::Dash => &PLATFORM_V11,
        Network::Testnet => &PLATFORM_V11,
        Network::Devnet => &PLATFORM_V11,
        Network::Regtest => &PLATFORM_V11,
        _ => panic!("unsupported network"),
    }
}
