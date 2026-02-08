mod wallet_lifecycle;

use crate::app_dir::core_cookie_path;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::components::core_zmq_listener::ZMQConnectionEvent;
use crate::config::{Config, NetworkConfig};
use crate::context_provider::Provider as RpcProvider;
use crate::context_provider_spv::SpvProvider;
use crate::database::Database;
use crate::lock_helper::{MutexExt, RwLockExt};
use crate::model::contested_name::ContestedName;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::password_info::PasswordInfo;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::{DPNSNameInfo, QualifiedIdentity};
use crate::model::settings::Settings;
use crate::model::wallet::single_key::{SingleKeyHash, SingleKeyWallet};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::sdk_wrapper::initialize_sdk;
use crate::spv::{CoreBackendMode, SpvManager};
use crate::ui::RootScreenType;
use crate::ui::tokens::tokens_screen::{IdentityTokenBalance, IdentityTokenIdentifier};
use crate::utils::tasks::TaskManager;
use bincode::config;
use crossbeam_channel::{Receiver, Sender};
use dash_sdk::Sdk;
use dash_sdk::dashcore_rpc::dashcore::{InstantLock, Transaction};
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload::AssetLockPayloadType;
use dash_sdk::dpp::dashcore::{Address, Network, OutPoint, TxOut, Txid};
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::{AssetLockProof, CoreBlockHeight};
use dash_sdk::dpp::state_transition::StateTransitionSigningOptions;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::version::v11::PLATFORM_V11;
use dash_sdk::platform::{DataContract, Identifier};
use dash_sdk::query_types::IndexMap;
use egui::Context;
use rusqlite::Result;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard};

const ANIMATION_REFRESH_TIME: std::time::Duration = std::time::Duration::from_millis(100);

/// A guard that ensures settings cache invalidation happens atomically
///
/// This guard holds a write lock on the cached settings, preventing reads
/// until the database update is complete and the cache is properly invalidated.
type SettingsCacheGuard<'a> = RwLockWriteGuard<'a, Option<Settings>>;

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
    pub(crate) zmq_connection_status: Mutex<ZMQConnectionEvent>,
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
        let cookie_path = match core_cookie_path(network, &network_config.devnet_name) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(?network, "Failed to get core cookie path: {e}");
                return None;
            }
        };

        // Try cookie authentication first, then user/password
        let core_client = match Client::new(&addr, Auth::CookieFile(cookie_path.clone())) {
            Ok(client) => client,
            Err(_) => {
                tracing::info!(
                    "Failed to authenticate using .cookie file at {:?}, falling back to user/pass",
                    cookie_path,
                );
                match Client::new(
                    &addr,
                    Auth::UserPass(
                        network_config.core_rpc_user.to_string(),
                        network_config.core_rpc_password.to_string(),
                    ),
                ) {
                    Ok(client) => client,
                    Err(e) => {
                        tracing::error!(?network, "Failed to create CoreClient: {e}");
                        return None;
                    }
                }
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
            zmq_connection_status: Mutex::new(ZMQConnectionEvent::Disconnected),
            animate,
            cached_settings: RwLock::new(None),
            subtasks,
            spv_manager,
            core_backend_mode: AtomicU8::new(saved_core_backend_mode),
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
                initialize_sdk(&cfg, self.network, provider)?
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
                initialize_sdk(&cfg, self.network, rpc_provider)?
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

    /// Inserts a local qualified identity into the database
    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        wallet_and_identity_id_info: &Option<(WalletSeedHash, u32)>,
    ) -> Result<()> {
        self.db.insert_local_qualified_identity(
            qualified_identity,
            wallet_and_identity_id_info,
            self,
        )
    }

    /// Updates a local qualified identity in the database
    pub fn update_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
    ) -> Result<()> {
        self.db
            .update_local_qualified_identity(qualified_identity, self)
    }

    /// Sets the alias for an identity
    pub fn set_identity_alias(
        &self,
        identifier: &Identifier,
        new_alias: Option<&str>,
    ) -> Result<()> {
        self.db.set_identity_alias(identifier, new_alias)
    }

    pub fn set_contract_alias(
        &self,
        contract_id: &Identifier,
        new_alias: Option<&str>,
    ) -> Result<()> {
        self.db.set_contract_alias(contract_id, new_alias)
    }

    /// Gets the alias for an identity
    pub fn get_identity_alias(&self, identifier: &Identifier) -> Result<Option<String>> {
        self.db.get_identity_alias(identifier)
    }

    /// Fetches all local qualified identities from the database
    pub fn load_local_qualified_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        let wallets = self.wallets.read_or_recover();
        self.db.get_local_qualified_identities(self, &wallets)
    }

    /// Fetches all local qualified identities from the database
    #[allow(dead_code)] // May be used for loading identities in wallets
    pub fn load_local_qualified_identities_in_wallets(&self) -> Result<Vec<QualifiedIdentity>> {
        let wallets = self.wallets.read_or_recover();
        self.db
            .get_local_qualified_identities_in_wallets(self, &wallets)
    }

    pub fn get_identity_by_id(
        &self,
        identity_id: &Identifier,
    ) -> Result<Option<QualifiedIdentity>> {
        let wallets = self.wallets.read_or_recover();
        // Get the identity from the database
        let result = self.db.get_identity_by_id(identity_id, self, &wallets)?;

        Ok(result)
    }

    /// Fetches all voting identities from the database
    pub fn load_local_voting_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        self.db.get_local_voting_identities(self)
    }

    /// Fetches all local user identities from the database
    pub fn load_local_user_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        let identities = self.db.get_local_user_identities(self)?;

        Ok(identities
            .into_iter()
            .map(|(mut identity, wallet_hash)| {
                if let Some(wallet_id) = wallet_hash {
                    // Load wallets for each identity
                    self.load_wallet_for_identity(
                        &mut identity,
                        &[wallet_id],
                    )
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            identity = %identity.identity.id(),
                            error = ?e,
                            "cannot load wallet for identity when loading local user identities",
                        )
                    })
                } else {
                    tracing::debug!(
                        identity = %identity.identity.id(),
                        "no wallet hash found for identity when loading local user identities",
                    );
                }
                identity
            })
            .collect())
    }

    fn load_wallet_for_identity(
        &self,
        identity: &mut QualifiedIdentity,
        wallet_hashes: &[WalletSeedHash],
    ) -> Result<()> {
        let wallets = self.wallets.read_or_recover();
        for wallet_hash in wallet_hashes {
            if let Some(wallet) = wallets.get(wallet_hash) {
                identity
                    .associated_wallets
                    .insert(*wallet_hash, wallet.clone());
            } else {
                tracing::warn!(
                    wallet = %hex::encode(wallet_hash),
                    identity = %identity.identity.id(),
                    "wallet not found for identity when loading local user identities",
                );
            }
        }

        Ok(())
    }

    /// Fetches all contested names from the database including past and active ones
    pub fn all_contested_names(&self) -> Result<Vec<ContestedName>> {
        self.db.get_all_contested_names(self)
    }

    /// Fetches all ongoing contested names from the database
    pub fn ongoing_contested_names(&self) -> Result<Vec<ContestedName>> {
        self.db.get_ongoing_contested_names(self)
    }

    /// Inserts scheduled votes into the database
    pub fn insert_scheduled_votes(&self, scheduled_votes: &Vec<ScheduledDPNSVote>) -> Result<()> {
        self.db.insert_scheduled_votes(self, scheduled_votes)
    }

    /// Fetches all scheduled votes from the database
    pub fn get_scheduled_votes(&self) -> Result<Vec<ScheduledDPNSVote>> {
        self.db.get_scheduled_votes(self)
    }

    /// Clears all scheduled votes from the database
    pub fn clear_all_scheduled_votes(&self) -> Result<()> {
        self.db.clear_all_scheduled_votes(self)
    }

    /// Clears all executed scheduled votes from the database
    pub fn clear_executed_scheduled_votes(&self) -> Result<()> {
        self.db.clear_executed_scheduled_votes(self)
    }

    /// Deletes a scheduled vote from the database
    #[allow(clippy::ptr_arg)]
    pub fn delete_scheduled_vote(&self, identity_id: &[u8], contested_name: &String) -> Result<()> {
        self.db
            .delete_scheduled_vote(self, identity_id, contested_name)
    }

    /// Marks a scheduled vote as executed in the database
    pub fn mark_vote_executed(&self, identity_id: &[u8], contested_name: String) -> Result<()> {
        self.db
            .mark_vote_executed(self, identity_id, contested_name)
    }

    /// Fetches the local identities from the database and then maps them to their DPNS names.
    pub fn local_dpns_names(&self) -> Result<Vec<(Identifier, DPNSNameInfo)>> {
        let wallets = self.wallets.read_or_recover();
        let qualified_identities = self.db.get_local_qualified_identities(self, &wallets)?;

        // Map each identity's DPNS names to (Identifier, DPNSNameInfo) tuples
        let dpns_names = qualified_identities
            .iter()
            .flat_map(|qualified_identity| {
                qualified_identity.dpns_names.iter().map(|dpns_name_info| {
                    (
                        qualified_identity.identity.id(),
                        DPNSNameInfo {
                            name: dpns_name_info.name.clone(),
                            acquired_at: dpns_name_info.acquired_at,
                        },
                    )
                })
            })
            .collect::<Vec<(Identifier, DPNSNameInfo)>>();

        Ok(dpns_names)
    }

    /// Updates the `start_root_screen` in the settings table
    pub fn update_settings(&self, root_screen_type: RootScreenType) -> Result<()> {
        let _guard = self.invalidate_settings_cache();

        self.db
            .insert_or_update_settings(self.network, root_screen_type)
    }

    /// Updates the main password settings
    pub fn update_main_password(
        &self,
        salt: &[u8],
        nonce: &[u8],
        password_check: &[u8],
    ) -> Result<()> {
        let _guard = self.invalidate_settings_cache();

        self.db.update_main_password(salt, nonce, password_check)
    }

    /// Updates the Dash Core execution settings
    pub fn update_dash_core_execution_settings(
        &self,
        custom_dash_qt_path: Option<std::path::PathBuf>,
        overwrite_dash_conf: bool,
    ) -> Result<()> {
        let _guard = self.invalidate_settings_cache();

        self.db
            .update_dash_core_execution_settings(custom_dash_qt_path, overwrite_dash_conf)
    }

    /// Updates the disable_zmq flag in settings
    pub fn update_disable_zmq(&self, disable: bool) -> Result<()> {
        let _guard = self.invalidate_settings_cache();
        self.db.update_disable_zmq(disable)
    }

    /// Invalidates the settings cache and returns a guard
    ///
    /// The cache is invalidated immediately and the guard prevents concurrent access
    /// until the database operation is complete. This ensures atomicity and prevents
    /// race conditions regardless of whether the database operation succeeds or fails.
    pub fn invalidate_settings_cache(&'_ self) -> SettingsCacheGuard<'_> {
        let mut guard = self.cached_settings.write_or_recover();
        *guard = None;
        guard
    }

    /// Retrieves the current settings
    ///
    /// ## Cached
    ///
    /// This function uses a cache to avoid expensive database operations.
    /// The cache is invalidated when settings are updated.
    ///
    /// Use [`AppContext::invalidate_settings_cache`] to invalidate the cache.
    pub fn get_settings(&self) -> Result<Option<Settings>> {
        // First, try to read from cache
        {
            let cache = self.cached_settings.read_or_recover();
            if let Some(ref settings) = *cache {
                return Ok(Some(settings.clone()));
            }
        }

        // Cache miss, read from database
        let settings = self.db.get_settings()?.map(Settings::from);

        // Update cache with the fresh data
        {
            let mut cache = self.cached_settings.write_or_recover();
            *cache = settings.clone();
        }

        Ok(settings)
    }

    /// Retrieves all contracts from the database plus the system contracts from app context.
    pub fn get_contracts(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<QualifiedContract>> {
        // Get contracts from the database
        let mut contracts = self.db.get_contracts(self, limit, offset)?;

        // Add the DPNS contract to the list
        let dpns_contract = QualifiedContract {
            contract: Arc::clone(&self.dpns_contract).as_ref().clone(),
            alias: Some("dpns".to_string()),
        };

        // Insert the DPNS contract at 0
        contracts.insert(0, dpns_contract);

        // Add the token history contract to the list
        let token_history_contract = QualifiedContract {
            contract: Arc::clone(&self.token_history_contract).as_ref().clone(),
            alias: Some("token_history".to_string()),
        };

        // Insert the token history contract at 1
        contracts.insert(1, token_history_contract);

        // Add the withdrawal contract to the list
        let withdraws_contract = QualifiedContract {
            contract: Arc::clone(&self.withdraws_contract).as_ref().clone(),
            alias: Some("withdrawals".to_string()),
        };

        // Insert the withdrawal contract at 2
        contracts.insert(2, withdraws_contract);

        // Add the keyword search contract to the list
        let keyword_search_contract = QualifiedContract {
            contract: Arc::clone(&self.keyword_search_contract).as_ref().clone(),
            alias: Some("keyword_search".to_string()),
        };

        // Insert the keyword search contract at 3
        contracts.insert(3, keyword_search_contract);

        // Add the DashPay contract to the list
        let dashpay_contract = QualifiedContract {
            contract: Arc::clone(&self.dashpay_contract).as_ref().clone(),
            alias: Some("dashpay".to_string()),
        };

        // Insert the DashPay contract at 4
        contracts.insert(4, dashpay_contract);

        Ok(contracts)
    }

    pub fn get_contract_by_id(
        &self,
        contract_id: &Identifier,
    ) -> Result<Option<QualifiedContract>> {
        // Get the contract from the database
        self.db.get_contract_by_id(*contract_id, self)
    }

    pub fn get_unqualified_contract_by_id(
        &self,
        contract_id: &Identifier,
    ) -> Result<Option<DataContract>> {
        // Get the contract from the database
        self.db.get_unqualified_contract_by_id(*contract_id, self)
    }

    // Remove contract from the database by ID
    pub fn remove_contract(&self, contract_id: &Identifier) -> Result<()> {
        self.db.remove_contract(contract_id.as_bytes(), self)
    }

    pub fn replace_contract(
        &self,
        contract_id: Identifier,
        new_contract: &DataContract,
    ) -> Result<()> {
        self.db.replace_contract(contract_id, new_contract, self)
    }

    pub(crate) fn received_transaction_finality(
        &self,
        tx: &Transaction,
        islock: Option<InstantLock>,
        chain_locked_height: Option<CoreBlockHeight>,
    ) -> Result<Vec<(OutPoint, TxOut, Address)>> {
        // Initialize a vector to collect wallet outpoints
        let mut wallet_outpoints = Vec::new();

        // Identify the wallets associated with the transaction
        let wallets = self.wallets.read_or_recover();
        for wallet_arc in wallets.values() {
            let mut wallet = wallet_arc.write_or_recover();
            for (vout, tx_out) in tx.output.iter().enumerate() {
                let address = if let Ok(output_addr) =
                    Address::from_script(&tx_out.script_pubkey, self.network)
                {
                    if wallet.known_addresses.contains_key(&output_addr) {
                        output_addr
                    } else {
                        continue;
                    }
                } else {
                    continue;
                };
                self.db.insert_utxo(
                    tx.txid().as_byte_array(),
                    vout as u32,
                    &address,
                    tx_out.value,
                    &tx_out.script_pubkey.to_bytes(),
                    self.network,
                )?;
                self.db
                    .add_to_address_balance(&wallet.seed_hash(), &address, tx_out.value)?;

                // Create the OutPoint and insert it into the wallet.utxos entry
                let out_point = OutPoint::new(tx.txid(), vout as u32);
                wallet
                    .utxos
                    .entry(address.clone())
                    .or_insert_with(HashMap::new) // Initialize inner HashMap if needed
                    .insert(out_point, tx_out.clone()); // Insert the TxOut at the OutPoint

                // Collect the outpoint
                wallet_outpoints.push((out_point, tx_out.clone(), address.clone()));

                wallet
                    .address_balances
                    .entry(address.clone())
                    .and_modify(|balance| *balance += tx_out.value)
                    .or_insert(tx_out.value);

                // Check if this is a DashPay contact payment
                if let Ok(Some((owner_id, contact_id, address_index))) =
                    self.db.get_dashpay_address_mapping(&address)
                {
                    // Update the highest receive index if needed
                    if let Ok(indices) = self.db.get_contact_address_indices(&owner_id, &contact_id)
                        && address_index >= indices.highest_receive_index
                    {
                        let _ = self.db.update_highest_receive_index(
                            &owner_id,
                            &contact_id,
                            address_index + 1,
                        );
                    }

                    // Save the payment record
                    let _ = self.db.save_payment(
                        &tx.txid().to_string(),
                        &contact_id, // from contact
                        &owner_id,   // to us
                        tx_out.value as i64,
                        None, // memo not available for incoming
                        "received",
                    );

                    tracing::info!(
                        "DashPay payment received: {} duffs from contact {} to address {} (index {})",
                        tx_out.value,
                        contact_id.to_string(
                            dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58
                        ),
                        address,
                        address_index
                    );
                }
            }
        }
        if matches!(
            tx.special_transaction_payload,
            Some(AssetLockPayloadType(_))
        ) {
            self.received_asset_lock_finality(tx, islock, chain_locked_height)?;
        }
        Ok(wallet_outpoints)
    }

    /// Store the asset lock transaction in the database and update the wallet.
    pub(crate) fn received_asset_lock_finality(
        &self,
        tx: &Transaction,
        islock: Option<InstantLock>,
        chain_locked_height: Option<CoreBlockHeight>,
    ) -> Result<()> {
        // Extract the asset lock payload from the transaction
        let Some(AssetLockPayloadType(payload)) = tx.special_transaction_payload.as_ref() else {
            return Ok(());
        };

        let proof = if let Some(islock) = islock.as_ref() {
            // Deserialize the InstantLock
            Some(AssetLockProof::Instant(InstantAssetLockProof::new(
                islock.clone(),
                tx.clone(),
                0,
            )))
        } else {
            chain_locked_height.map(|chain_locked_height| {
                AssetLockProof::Chain(ChainAssetLockProof {
                    core_chain_locked_height: chain_locked_height,
                    out_point: OutPoint::new(tx.txid(), 0),
                })
            })
        };

        {
            let mut transactions = self.transactions_waiting_for_finality.lock_or_recover();

            if let Some(asset_lock_proof) = transactions.get_mut(&tx.txid()) {
                *asset_lock_proof = proof.clone();
            }
        }

        // Identify the wallet associated with the transaction
        let wallets = self.wallets.read_or_recover();
        for wallet_arc in wallets.values() {
            let mut wallet = wallet_arc.write_or_recover();

            // Check if any of the addresses in the transaction outputs match the wallet's known addresses
            let matches_wallet = payload.credit_outputs.iter().any(|tx_out| {
                if let Ok(output_addr) = Address::from_script(&tx_out.script_pubkey, self.network) {
                    wallet.known_addresses.contains_key(&output_addr)
                } else {
                    false
                }
            });

            if matches_wallet {
                // Calculate the total amount from the credit outputs
                let amount: u64 = payload
                    .credit_outputs
                    .iter()
                    .map(|tx_out| tx_out.value)
                    .sum();

                // Store the asset lock transaction in the database
                self.db.store_asset_lock_transaction(
                    tx,
                    amount,
                    islock.as_ref(),
                    &wallet.seed_hash(),
                    self.network,
                )?;

                let first = payload.credit_outputs.first().ok_or_else(|| {
                    rusqlite::Error::InvalidParameterName(
                        "Asset lock transaction has no credit outputs".to_string(),
                    )
                })?;

                let address =
                    Address::from_script(&first.script_pubkey, self.network).map_err(|e| {
                        rusqlite::Error::InvalidParameterName(format!(
                            "Failed to derive address from asset lock credit output script: {e}"
                        ))
                    })?;

                // Add the asset lock to the wallet's unused_asset_locks
                wallet
                    .unused_asset_locks
                    .push((tx.clone(), address, amount, islock, proof));

                break; // Exit the loop after updating the relevant wallet
            }
        }

        Ok(())
    }

    pub fn identity_token_balances(
        &self,
    ) -> Result<IndexMap<IdentityTokenIdentifier, IdentityTokenBalance>> {
        self.db.get_identity_token_balances(self)
    }

    pub fn remove_token_balance(
        &self,
        token_id: Identifier,
        identity_id: Identifier,
    ) -> Result<()> {
        self.db.remove_token_balance(&token_id, &identity_id, self)
    }

    pub fn insert_token(
        &self,
        token_id: &Identifier,
        token_name: &str,
        token_configuration: TokenConfiguration,
        contract_id: &Identifier,
        token_position: u16,
    ) -> Result<()> {
        let config = config::standard();
        let Some(serialized_token_configuration) =
            bincode::encode_to_vec(&token_configuration, config).ok()
        else {
            // We should always be able to serialize
            return Ok(());
        };

        self.db.insert_token(
            token_id,
            token_name,
            serialized_token_configuration.as_slice(),
            contract_id,
            token_position,
            self,
        )?;

        Ok(())
    }

    pub fn remove_token(&self, token_id: &Identifier) -> Result<()> {
        self.db.remove_token(token_id, self)
    }

    pub fn remove_wallet(&self, seed_hash: &WalletSeedHash) -> Result<(), String> {
        {
            let wallets = self
                .wallets
                .read()
                .map_err(|_| "Failed to access wallets".to_string())?;
            if !wallets.contains_key(seed_hash) {
                return Err("Wallet not found".to_string());
            }
        }

        self.db
            .remove_wallet(seed_hash, &self.network)
            .map_err(|e| e.to_string())?;

        let mut wallets = self
            .wallets
            .write()
            .map_err(|_| "Failed to update wallets".to_string())?;

        wallets.remove(seed_hash);
        let has_wallet = !wallets.is_empty();
        drop(wallets);

        self.has_wallet.store(has_wallet, Ordering::Relaxed);

        Ok(())
    }

    #[allow(dead_code)] // May be used for storing token balances
    pub fn insert_token_identity_balance(
        &self,
        token_id: &Identifier,
        identity_id: &Identifier,
        balance: u64,
    ) -> Result<()> {
        self.db
            .insert_identity_token_balance(token_id, identity_id, balance, self)?;

        Ok(())
    }

    pub fn get_contract_by_token_id(
        &self,
        token_id: &Identifier,
    ) -> Result<Option<QualifiedContract>> {
        let contract_id = self
            .db
            .get_contract_id_by_token_id(token_id, self)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
        self.db.get_contract_by_id(contract_id, self)
    }
}

pub(crate) struct DapiTransactionInfo {
    pub is_chain_locked: bool,
    pub height: u32,
    pub confirmations: u32,
}

/// Query transaction info from DAPI. Works in both SPV and RPC modes
/// since DAPI (platform gRPC) is always available via the SDK.
pub(crate) async fn get_transaction_info_via_dapi(
    sdk: &Sdk,
    tx_id: &Txid,
) -> Result<DapiTransactionInfo, String> {
    use dash_sdk::dapi_client::{DapiRequestExecutor, IntoInner, RequestSettings};
    use dash_sdk::dapi_grpc::core::v0::GetTransactionRequest;

    let response = sdk
        .execute(
            GetTransactionRequest {
                id: tx_id.to_string(),
            },
            RequestSettings::default(),
        )
        .await
        .into_inner()
        .map_err(|e| format!("DAPI GetTransaction failed: {}", e))?;

    Ok(DapiTransactionInfo {
        is_chain_locked: response.is_chain_locked,
        height: response.height,
        confirmations: response.confirmations,
    })
}

/// Returns the default platform version for the given network.
pub(crate) const fn default_platform_version(network: &Network) -> &'static PlatformVersion {
    // TODO: Use self.sdk.read().unwrap().version() instead of hardcoding
    match network {
        Network::Dash => &PLATFORM_V11,
        Network::Testnet => &PLATFORM_V11,
        Network::Devnet => &PLATFORM_V11,
        Network::Regtest => &PLATFORM_V11,
        _ => &PLATFORM_V11,
    }
}
