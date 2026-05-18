pub mod connection_status;
mod contract_token_db;
mod identity_db;
mod settings_db;
pub mod shielded;
mod transaction_processing;
mod wallet_lifecycle;

pub(crate) use transaction_processing::get_transaction_info;

use crate::app_dir::core_cookie_path;
use crate::backend_task::error::{TaskError, is_rpc_connection_error};
use crate::components::core_zmq_listener::ZMQConnectionEvent;
use crate::config::{Config, NetworkConfig};
use crate::context_provider_spv::SpvProvider;
use crate::database::Database;
use crate::model::feature_gate::FeatureGate;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::password_info::PasswordInfo;
use crate::model::proof_log_item::RequestType;
use crate::model::wallet::single_key::{SingleKeyHash, SingleKeyWallet};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::sdk_wrapper::initialize_sdk;
use crate::utils::tasks::TaskManager;
use arc_swap::ArcSwap;
use connection_status::ConnectionStatus;
use crossbeam_channel::{Receiver, Sender};
use dash_sdk::Sdk;
use dash_sdk::dapi_client::AddressList;
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::{Address, Network, Txid};
#[cfg(any(test, feature = "testing"))]
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::prelude::AssetLockProof;
use dash_sdk::dpp::state_transition::StateTransitionSigningOptions;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::version::v11::PLATFORM_V11;
use dash_sdk::platform::DataContract;
#[cfg(any(test, feature = "testing"))]
use dash_sdk::platform::Identifier;
use egui::Context;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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
    pub(crate) data_dir: PathBuf,
    pub(crate) network: Network,
    developer_mode: AtomicBool,
    pub(crate) db: Arc<Database>,
    pub(crate) sdk: ArcSwap<Sdk>,
    // SDK context provider (quorum keys via DAPI). Chain sync is SPV-only,
    // owned by upstream platform-wallet.
    spv_context_provider: RwLock<SpvProvider>,
    pub(crate) config: Arc<RwLock<NetworkConfig>>,
    // TODO(P0.5): ZMQ listener usage is audited in P4 (Decision #3); the
    // receiver is retained until that audit decides its fate.
    #[allow(dead_code)]
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
    /// Cached protocol version from the current epoch on the connected network.
    /// Updated alongside fee_multiplier when epoch info is fetched.
    /// 0 means not yet fetched from the network.
    platform_protocol_version: AtomicU32,
    /// Per-wallet shielded state (initialized lazily, keyed by wallet seed hash)
    pub(crate) shielded_states: Mutex<
        std::collections::HashMap<
            WalletSeedHash,
            crate::model::wallet::shielded::ShieldedWalletState,
        >,
    >,
    /// The egui context, stored for use in non-UI code paths (e.g. display_task_result).
    /// Clone is O(1) — egui::Context is Arc-backed and the same instance for the app lifetime.
    egui_ctx: egui::Context,
}

impl AppContext {
    pub fn new(
        data_dir: PathBuf,
        network: Network,
        db: Arc<Database>,
        password_info: Option<PasswordInfo>,
        subtasks: Arc<TaskManager>,
        connection_status: Arc<ConnectionStatus>,
        egui_ctx: egui::Context,
    ) -> Option<Arc<Self>> {
        let config = match Config::load_from(&data_dir) {
            Ok(config) => config,
            Err(e) => {
                tracing::error!("Failed to load config: {e}");
                return None;
            }
        };

        let network_config = config.config_for_network(network).clone()?;
        let config_lock = Arc::new(RwLock::new(network_config.clone()));
        let (sx_zmq_status, rx_zmq_status) = crossbeam_channel::unbounded();

        // Create the SDK context provider; bind to app context later
        // (post construction) due to circularity.
        let spv_provider = match SpvProvider::new(db.clone(), network) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(?network, "Failed to initialize SPV provider: {e}");
                return None;
            }
        };

        // Parse configured DAPI addresses directly (no auto-discovery at startup)
        let address_list = match &network_config.dapi_addresses {
            Some(addrs) if !addrs.trim().is_empty() => match AddressList::from_str(addrs.trim()) {
                Ok(list) => list,
                Err(e) => {
                    tracing::error!(
                        ?network,
                        error = %e,
                        "Failed to parse configured DAPI addresses"
                    );
                    return None;
                }
            },
            _ => {
                tracing::error!(
                    ?network,
                    "No DAPI addresses configured. Use Refresh DAPI endpoints in Network Settings or add addresses to .env."
                );
                return None;
            }
        };

        // Default to SPV provider initially; UI can switch backend after
        let sdk = match initialize_sdk(address_list, network, spv_provider.clone()) {
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
            network_config.rpc_host(),
            network_config.rpc_port(network)
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

        // Load saved wallet selection, validating that the wallets still exist
        let (saved_wallet_hash, saved_single_key_hash) =
            db.get_selected_wallet_hashes().unwrap_or((None, None));

        // Only use the saved hash if the wallet still exists
        let selected_wallet_hash = saved_wallet_hash.filter(|h| wallets.contains_key(h));
        let selected_single_key_hash =
            saved_single_key_hash.filter(|h| single_key_wallets.contains_key(h));

        let app_context = AppContext {
            data_dir,
            network,
            developer_mode: AtomicBool::new(developer_mode_enabled),
            db,
            sdk: ArcSwap::from_pointee(sdk),
            spv_context_provider: spv_provider.into(),
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
            connection_status,
            pending_wallet_selection: Mutex::new(None),
            selected_wallet_hash: Mutex::new(selected_wallet_hash),
            selected_single_key_hash: Mutex::new(selected_single_key_hash),
            fee_multiplier_permille: AtomicU64::new(
                PlatformFeeEstimator::DEFAULT_FEE_MULTIPLIER_PERMILLE,
            ),
            platform_protocol_version: AtomicU32::new(0),
            shielded_states: Mutex::new(std::collections::HashMap::new()),
            egui_ctx,
        };

        let app_context = Arc::new(app_context);
        // Bind the SDK context provider. Chain sync is SPV-only (owned by
        // upstream platform-wallet); the SPV provider is the sole SDK
        // quorum/context provider.
        if let Err(e) = app_context
            .spv_context_provider
            .read()
            .map_err(|e| e.to_string())
            .and_then(|provider| provider.bind_app_context(app_context.clone()))
        {
            tracing::error!("Failed to bind SPV provider: {}", e);
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

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn connection_status(&self) -> &ConnectionStatus {
        &self.connection_status
    }

    pub fn egui_ctx(&self) -> &egui::Context {
        &self.egui_ctx
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

    /// Get the cached platform protocol version from the connected network.
    /// Returns 0 if not yet fetched from the network.
    pub fn platform_protocol_version(&self) -> u32 {
        self.platform_protocol_version.load(Ordering::Relaxed)
    }

    /// Update the cached platform protocol version from epoch info.
    ///
    /// When the version crosses the shielded threshold for the first time,
    /// retroactively initializes shielded wallets that were unlocked before
    /// the protocol version was known.
    pub fn set_platform_protocol_version(self: &Arc<Self>, version: u32) {
        let was_shielded = FeatureGate::Shielded.is_available(self);

        self.platform_protocol_version
            .swap(version, Ordering::Relaxed);

        if !was_shielded && FeatureGate::Shielded.is_available(self) {
            self.init_missing_shielded_wallets();
        }
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
    pub fn reinit_core_client_and_sdk(self: Arc<Self>) -> Result<(), TaskError> {
        // 1. Grab a fresh snapshot of your NetworkConfig
        let cfg = {
            let cfg_lock = self.config.read()?;
            cfg_lock.clone()
        };

        // Note: developer_mode is now global and managed separately

        // 2. Rebuild the RPC client with the new credentials (cookie auth first, then user/pass).
        let addr = format!("http://{}:{}", cfg.rpc_host(), cfg.rpc_port(self.network));
        let new_client = Self::create_core_rpc_client(&addr, self.network, &cfg.devnet_name, &cfg)?;

        // 3. Parse DAPI addresses from config and rebuild the SDK
        let address_list = match &cfg.dapi_addresses {
            Some(addrs) if !addrs.trim().is_empty() => AddressList::from_str(addrs.trim())
                .map_err(|source| {
                    crate::backend_task::dapi_discovery::DapiDiscoveryError::InvalidAddresses {
                        source,
                    }
                })?,
            _ => {
                return Err(
                    crate::backend_task::dapi_discovery::DapiDiscoveryError::AddressesRequired {
                        network: self.network,
                    }
                    .into(),
                );
            }
        };

        let provider = self.spv_context_provider.read()?.clone();
        let new_sdk = initialize_sdk(address_list, self.network, provider)
            .map_err(|e| TaskError::SdkInitializationFailed { detail: e })?;

        // 4. Swap in the new SDK and client
        {
            let mut client_lock = self.core_client.write()?;
            *client_lock = new_client;
        }
        self.sdk.store(Arc::new(new_sdk));

        // Rebind the provider to hold the new AppContext reference.
        // bind_app_context also registers the provider with the SDK.
        self.spv_context_provider
            .read()?
            .bind_app_context(self.clone())
            .map_err(|e| TaskError::SdkInitializationFailed { detail: e })?;

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
            tracing::trace!(
                "Cookie auth unavailable at {:?}, using user/pass",
                cookie_path,
            );
        }
        Client::new(
            url,
            Auth::UserPass(
                cfg.core_rpc_user.clone().unwrap_or_default(),
                cfg.core_rpc_password.clone().unwrap_or_default(),
            ),
        )
        .map_err(|e| TaskError::CoreRpc { source: e })
    }

    /// Ensure an address is tracked for incoming funds.
    ///
    /// No-op: chain sync is SPV-only and owned by upstream `platform-wallet`,
    /// which watches the BIP44 account derived from the wallet seed. There is
    /// no Dash Core node to import addresses into.
    pub fn ensure_address_imported(
        &self,
        _address: &Address,
        _core_wallet_name: Option<&str>,
        _label: Option<&str>,
    ) -> Result<(), TaskError> {
        Ok(())
    }

    /// Best-effort address registration. No-op for the same reason as
    /// [`Self::ensure_address_imported`].
    pub fn try_import_address(
        &self,
        _address: &Address,
        _core_wallet_name: Option<&str>,
        _label: Option<&str>,
    ) {
    }

    /// Convert an RPC error to `TaskError`, enriching connection failures with
    /// the configured host:port so the user knows which address was unreachable.
    pub(crate) fn rpc_error_with_url(&self, e: dash_sdk::dashcore_rpc::Error) -> TaskError {
        if is_rpc_connection_error(&e) {
            let url = self
                .config
                .read()
                .ok()
                .map(|c| format!("{}:{}", c.rpc_host(), c.rpc_port(self.network)))
                .unwrap_or_else(|| "unknown".to_string());
            TaskError::CoreRpcConnectionFailed {
                url,
                source: Some(Box::new(e)),
            }
        } else {
            TaskError::from(e)
        }
    }

    /// Convert an SDK error to a [`TaskError`], with special handling for
    /// [`dash_sdk::Error::DriveProofError`]: logs the proof data to the database
    /// and returns [`TaskError::ProofError`] with the SDK error preserved as the source.
    ///
    /// All other SDK errors are converted via [`TaskError::from`].
    pub(crate) fn log_drive_proof_error(
        &self,
        e: dash_sdk::Error,
        request_type: RequestType,
    ) -> TaskError {
        use crate::model::proof_log_item::ProofLogItem;
        match e {
            dash_sdk::Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                if let Err(db_err) = self.db.insert_proof_log_item(ProofLogItem {
                    request_type,
                    request_bytes: vec![],
                    verification_path_query_bytes: vec![],
                    height: block_info.height,
                    time_ms: block_info.time_ms,
                    proof_bytes: proof_bytes.clone(),
                    error: Some(proof_error.to_string()),
                }) {
                    tracing::warn!(
                        height = block_info.height,
                        proof_error = %proof_error,
                        "Failed to persist proof log entry for {request_type:?}: {}",
                        db_err
                    );
                }
                TaskError::ProofError {
                    source_error: Box::new(dash_sdk::Error::DriveProofError(
                        proof_error,
                        proof_bytes,
                        block_info,
                    )),
                }
            }
            e => TaskError::from(e),
        }
    }
}

/// Test-only accessors for fields that are normally `pub(crate)`.
#[cfg(any(test, feature = "testing"))]
impl AppContext {
    /// Returns a clone of the current SDK instance.
    pub fn sdk(&self) -> Sdk {
        self.sdk.load().as_ref().clone()
    }

    /// Returns a reference to the database.
    pub fn db(&self) -> &Arc<Database> {
        &self.db
    }

    /// Returns a reference to the wallets map.
    pub fn wallets(&self) -> &RwLock<BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>> {
        &self.wallets
    }

    /// Returns the DashPay contract identifier.
    pub fn dashpay_contract_id(&self) -> Identifier {
        self.dashpay_contract.id()
    }
}

/// Returns the default platform version for the given network.
pub(crate) const fn default_platform_version(network: &Network) -> &'static PlatformVersion {
    // TODO: Ideally use sdk.load().version() but this is a free function with no sdk access
    match network {
        Network::Mainnet => &PLATFORM_V11,
        Network::Testnet => &PLATFORM_V11,
        Network::Devnet => &PLATFORM_V11,
        Network::Regtest => &PLATFORM_V11,
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

    /// A fresh data directory (no pre-existing settings) must persist the
    /// SPV backend marker (`settings.core_backend_mode` column defaults to 1).
    /// The column is retained until the P3 migration; chain sync is SPV-only.
    #[test]
    fn fresh_db_resolves_to_spv_backend_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let db_file_path = tmp.path().join("test_data.db");
        let db = crate::database::Database::new(&db_file_path).unwrap();
        db.initialize(&db_file_path).unwrap();

        let settings = db
            .get_settings()
            .expect("settings read")
            .expect("settings row present");
        // Settings tuple: index 7 is the legacy core_backend_mode column.
        assert_eq!(settings.7, 1, "fresh DB should default to SPV (=1)");
    }
}
