use crate::app_dir::core_cookie_path;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::components::core_zmq_listener::ZMQConnectionEvent;
use crate::config::{Config, NetworkConfig};
use crate::context_provider::Provider as RpcProvider;
use crate::context_provider_spv::SpvProvider;
use crate::database::Database;
use crate::model::contested_name::ContestedName;
use crate::model::password_info::PasswordInfo;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::{DPNSNameInfo, QualifiedIdentity};
use crate::model::settings::Settings;
use crate::model::wallet::{
    AddressInfo as WalletAddressInfo, DerivationPathReference, DerivationPathType, Wallet,
    WalletSeedHash, WalletTransaction,
};
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
use dash_sdk::dpp::key_wallet::Network as WalletNetwork;
use dash_sdk::dpp::key_wallet::account::AccountType;
use dash_sdk::dpp::key_wallet::bip32::{ChildNumber, DerivationPath};
use dash_sdk::dpp::key_wallet::wallet::managed_wallet_info::{
    ManagedWalletInfo, wallet_info_interface::WalletInfoInterface,
};
use dash_sdk::dpp::prelude::{AssetLockProof, CoreBlockHeight};
use dash_sdk::dpp::state_transition::StateTransitionSigningOptions;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::dpp::version::v10::PLATFORM_V10;
use dash_sdk::platform::{DataContract, Identifier};
use dash_sdk::query_types::IndexMap;
use egui::Context;
use rusqlite::Result;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
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
            has_wallet: (!wallets.is_empty()).into(),
            wallets: RwLock::new(wallets),
            password_info,
            transactions_waiting_for_finality: Mutex::new(BTreeMap::new()),
            zmq_connection_status: Mutex::new(ZMQConnectionEvent::Disconnected),
            animate,
            cached_settings: RwLock::new(None),
            subtasks,
            spv_manager,
            core_backend_mode: AtomicU8::new(CoreBackendMode::Spv.as_u8()),
        };

        let app_context = Arc::new(app_context);
        // Bind providers to the newly created app_context.
        // Only the active provider is registered with the SDK here (SPV by default).
        app_context
            .spv_context_provider
            .read()
            .unwrap()
            .bind_app_context(app_context.clone());

        // If defaulting to RPC is desired, swap provider after binding.
        if app_context.core_backend_mode() == CoreBackendMode::Rpc {
            app_context
                .rpc_context_provider
                .read()
                .unwrap()
                .bind_app_context(app_context.clone());
        } else {
            // Ensure SDK uses the SPV provider
            let sdk_lock = app_context.sdk.write().expect("SDK lock poisoned");
            let provider = app_context.spv_context_provider.read().unwrap().clone();
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

        // Switch SDK context provider to match the selected backend
        match mode {
            CoreBackendMode::Spv => {
                // Make sure SPV provider knows about the app context
                self.spv_context_provider
                    .read()
                    .unwrap()
                    .bind_app_context(Arc::clone(self));
                let sdk = self.sdk.write().expect("SDK lock poisoned");
                let provider = self.spv_context_provider.read().unwrap().clone();
                sdk.set_context_provider(provider);
            }
            CoreBackendMode::Rpc => {
                // RPC provider binding also sets itself on the SDK
                self.rpc_context_provider
                    .read()
                    .unwrap()
                    .bind_app_context(Arc::clone(self));
            }
        }
    }

    pub fn spv_manager(&self) -> &Arc<SpvManager> {
        &self.spv_manager
    }

    pub fn clear_spv_data(&self) -> Result<(), String> {
        self.spv_manager.clear_data_dir()
    }

    pub fn clear_network_database(&self) -> Result<(), String> {
        self.db
            .clear_network_data(self.network)
            .map_err(|e| e.to_string())?;

        if let Ok(mut wallets) = self.wallets.write() {
            wallets.clear();
            self.has_wallet.store(false, Ordering::Relaxed);
        }

        Ok(())
    }

    pub fn start_spv(self: &Arc<Self>) -> Result<(), String> {
        self.spv_manager.start()?;
        self.spv_setup_reconcile_listener();
        Ok(())
    }

    pub fn bootstrap_wallet_addresses(&self, wallet: &Arc<RwLock<Wallet>>) {
        if let Ok(mut guard) = wallet.write()
            && guard.known_addresses.is_empty()
        {
            tracing::info!(wallet = %hex::encode(guard.seed_hash()), "Bootstrapping wallet addresses");
            guard.bootstrap_known_addresses(self);
        }
    }

    pub fn handle_wallet_unlocked(self: &Arc<Self>, wallet: &Arc<RwLock<Wallet>>) {
        if let Some((seed_hash, seed_bytes)) = Self::wallet_seed_snapshot(wallet) {
            self.queue_spv_wallet_load(seed_hash, seed_bytes);
        }
    }

    pub fn handle_wallet_locked(self: &Arc<Self>, wallet: &Arc<RwLock<Wallet>>) {
        let seed_hash = match wallet.read() {
            Ok(guard) => guard.seed_hash(),
            Err(err) => {
                tracing::warn!(error = %err, "Unable to read wallet during lock handling");
                return;
            }
        };
        self.queue_spv_wallet_unload(seed_hash);
    }

    fn wallet_seed_snapshot(wallet: &Arc<RwLock<Wallet>>) -> Option<(WalletSeedHash, [u8; 64])> {
        let guard = wallet.read().ok()?;
        if !guard.is_open() {
            return None;
        }
        let seed_bytes = match guard.seed_bytes() {
            Ok(bytes) => *bytes,
            Err(err) => {
                tracing::warn!(error = %err, wallet = %hex::encode(guard.seed_hash()), "Unable to snapshot wallet seed for SPV load");
                return None;
            }
        };
        Some((guard.seed_hash(), seed_bytes))
    }

    fn queue_spv_wallet_load(self: &Arc<Self>, seed_hash: WalletSeedHash, seed_bytes: [u8; 64]) {
        let spv = Arc::clone(&self.spv_manager);
        self.subtasks.spawn_sync(async move {
            if let Err(error) = spv.load_wallet_from_seed(seed_hash, seed_bytes).await {
                tracing::error!(seed = %hex::encode(seed_hash), %error, "Failed to load SPV wallet from seed");
            }
        });
    }

    fn queue_spv_wallet_unload(self: &Arc<Self>, seed_hash: WalletSeedHash) {
        let spv = Arc::clone(&self.spv_manager);
        self.subtasks.spawn_sync(async move {
            if let Err(error) = spv.unload_wallet(seed_hash).await {
                tracing::error!(seed = %hex::encode(seed_hash), %error, "Failed to unload SPV wallet");
            }
        });
    }

    pub fn bootstrap_loaded_wallets(self: &Arc<Self>) {
        let wallets: Vec<_> = {
            let guard = self.wallets.read().unwrap();
            guard.values().cloned().collect()
        };

        for wallet in wallets {
            self.bootstrap_wallet_addresses(&wallet);
            self.handle_wallet_unlocked(&wallet);
        }
    }

    pub(crate) async fn generate_receive_address(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, String> {
        let wallet_arc = {
            let wallets = self.wallets.read().unwrap();
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or_else(|| "Wallet not found".to_string())?
        };

        let address_string = if self.core_backend_mode() == CoreBackendMode::Spv {
            let derived = self
                .spv_manager
                .next_bip44_receive_address(seed_hash, 0)
                .await?;

            let _ = self.register_spv_address(
                &wallet_arc,
                derived.address.clone(),
                derived.derivation_path.clone(),
                DerivationPathType::CLEAR_FUNDS,
                DerivationPathReference::BIP44,
            )?;

            derived.address.to_string()
        } else {
            let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;
            wallet
                .receive_address(self.network, false, Some(self))?
                .to_string()
        };

        Ok(BackendTaskSuccessResult::GeneratedReceiveAddress {
            seed_hash,
            address: address_string,
        })
    }

    /// Fetch Platform address balances and nonces from Platform
    pub(crate) async fn fetch_platform_address_balances(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, String> {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        use dash_sdk::platform::FetchMany;
        use dash_sdk::query_types::AddressInfo as SdkAddressInfo;
        use std::collections::BTreeSet;

        let wallet_arc = {
            let wallets = self.wallets.read().unwrap();
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or_else(|| "Wallet not found".to_string())?
        };

        // Get all Platform addresses from the wallet
        let platform_addresses: Vec<(Address, PlatformAddress)> = {
            let wallet = wallet_arc.read().map_err(|e| e.to_string())?;
            wallet.platform_addresses(self.network)
        };

        if platform_addresses.is_empty() {
            return Ok(BackendTaskSuccessResult::PlatformAddressBalances {
                seed_hash,
                balances: std::collections::BTreeMap::new(),
            });
        }

        // Create a set of PlatformAddresses for the query
        let address_set: BTreeSet<PlatformAddress> = platform_addresses
            .iter()
            .map(|(_, pa)| *pa)
            .collect();

        // Fetch from Platform using the SDK
        let sdk = {
            let guard = self.sdk.read().map_err(|e| e.to_string())?;
            guard.clone()
        };
        let address_infos = SdkAddressInfo::fetch_many(&sdk, address_set)
            .await
            .map_err(|e| format!("Failed to fetch Platform address info: {}", e))?;

        // Update wallet and database with the fetched balances
        let mut balances = std::collections::BTreeMap::new();
        {
            let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;
            let wallet_seed_hash = wallet.seed_hash();

            for (core_addr, platform_addr) in &platform_addresses {
                // address_infos.get() returns Option<&Option<AddressInfo>>
                // Flatten to get the actual AddressInfo if it exists
                if let Some(Some(info)) = address_infos.get(platform_addr) {
                    // Update in-memory wallet state
                    wallet.set_platform_address_info(
                        core_addr.clone(),
                        info.balance,
                        info.nonce,
                    );

                    // Update database
                    if let Err(e) = self.db.set_platform_address_info(
                        &wallet_seed_hash,
                        core_addr,
                        info.balance,
                        info.nonce,
                        &self.network,
                    ) {
                        tracing::warn!(
                            "Failed to store Platform address info in database: {}",
                            e
                        );
                    }

                    balances.insert(core_addr.to_string(), (info.balance, info.nonce));
                } else {
                    // Address not found on Platform (never funded) - set to 0
                    balances.insert(core_addr.to_string(), (0, 0));
                }
            }
        }

        Ok(BackendTaskSuccessResult::PlatformAddressBalances {
            seed_hash,
            balances,
        })
    }

    /// Transfer credits between Platform addresses
    pub(crate) async fn transfer_platform_credits(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        inputs: std::collections::BTreeMap<
            dash_sdk::dpp::address_funds::PlatformAddress,
            dash_sdk::dpp::balances::credits::Credits,
        >,
        outputs: std::collections::BTreeMap<
            dash_sdk::dpp::address_funds::PlatformAddress,
            dash_sdk::dpp::balances::credits::Credits,
        >,
    ) -> Result<BackendTaskSuccessResult, String> {
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::platform::transition::transfer_address_funds::TransferAddressFunds;

        // Clone wallet and SDK before the async operation to avoid holding guards across await
        let (wallet, sdk) = {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or_else(|| "Wallet not found".to_string())?
            };
            let wallet = wallet_arc.read().map_err(|e| e.to_string())?.clone();
            let sdk = self.sdk.read().map_err(|e| e.to_string())?.clone();
            (wallet, sdk)
        };

        // Simple fee strategy: deduct from first input
        let fee_strategy = vec![AddressFundsFeeStrategyStep::DeductFromInput(0)];

        // Use the SDK to transfer
        let _result = sdk
            .transfer_address_funds(inputs, outputs, fee_strategy, &wallet, None)
            .await
            .map_err(|e| format!("Failed to transfer Platform credits: {}", e))?;

        // Trigger a balance refresh
        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformCreditsTransferred { seed_hash })
    }

    fn register_spv_address(
        &self,
        wallet: &Arc<RwLock<Wallet>>,
        address: Address,
        derivation_path: DerivationPath,
        path_type: DerivationPathType,
        path_reference: DerivationPathReference,
    ) -> Result<bool, String> {
        let mut guard = wallet.write().map_err(|e| e.to_string())?;
        if guard.known_addresses.contains_key(&address) {
            return Ok(false);
        }

        let (path_reference, path_type) =
            self.classify_derivation_metadata(&derivation_path, path_reference, path_type);

        let seed_hash = guard.seed_hash();

        self.db
            .add_address_if_not_exists(
                &seed_hash,
                &address,
                &self.network,
                &derivation_path,
                path_reference,
                path_type,
                None,
            )
            .map_err(|e| e.to_string())?;

        guard
            .known_addresses
            .insert(address.clone(), derivation_path.clone());
        guard.watched_addresses.insert(
            derivation_path,
            WalletAddressInfo {
                address,
                path_type,
                path_reference,
            },
        );

        Ok(true)
    }

    pub(crate) fn wallet_network_key(&self) -> WalletNetwork {
        match self.network {
            Network::Dash => WalletNetwork::Dash,
            Network::Testnet => WalletNetwork::Testnet,
            Network::Devnet => WalletNetwork::Devnet,
            Network::Regtest => WalletNetwork::Regtest,
            _ => WalletNetwork::Dash,
        }
    }

    fn sync_spv_account_addresses(
        &self,
        wallet_info: &ManagedWalletInfo,
        wallet_arc: &Arc<RwLock<Wallet>>,
    ) {
        let net = self.wallet_network_key();
        let Some(collection) = wallet_info.accounts(net) else {
            return;
        };

        let mut inserted = 0u32;
        for account in collection.all_accounts() {
            let account_type = account.account_type.to_account_type();
            if matches!(account_type, AccountType::Standard { .. }) {
                continue;
            }
            let Some((path_reference, path_type)) = Self::spv_account_metadata(&account_type)
            else {
                continue;
            };

            for address in account.account_type.all_addresses() {
                if let Some(info) = account.get_address_info(&address)
                    && let Ok(true) = self.register_spv_address(
                        wallet_arc,
                        address.clone(),
                        info.path.clone(),
                        path_type,
                        path_reference,
                    )
                {
                    inserted += 1;
                }
            }
        }

        if inserted > 0 {
            tracing::debug!(added = inserted, "Registered SPV-managed addresses");
        }
    }

    fn spv_account_metadata(
        account_type: &AccountType,
    ) -> Option<(DerivationPathReference, DerivationPathType)> {
        match account_type {
            AccountType::IdentityRegistration => Some((
                DerivationPathReference::BlockchainIdentityCreditRegistrationFunding,
                DerivationPathType::CREDIT_FUNDING,
            )),
            AccountType::IdentityInvitation => Some((
                DerivationPathReference::BlockchainIdentityCreditInvitationFunding,
                DerivationPathType::CREDIT_FUNDING,
            )),
            AccountType::IdentityTopUp { .. } | AccountType::IdentityTopUpNotBoundToIdentity => {
                Some((
                    DerivationPathReference::BlockchainIdentityCreditTopupFunding,
                    DerivationPathType::CREDIT_FUNDING,
                ))
            }
            AccountType::Standard { .. } => Some((
                DerivationPathReference::BIP44,
                DerivationPathType::CLEAR_FUNDS,
            )),
            _ => None,
        }
    }

    fn classify_derivation_metadata(
        &self,
        derivation_path: &DerivationPath,
        default_ref: DerivationPathReference,
        default_type: DerivationPathType,
    ) -> (DerivationPathReference, DerivationPathType) {
        let components = derivation_path.as_ref();
        if components.len() >= 5
            && matches!(components[0], ChildNumber::Hardened { index: 9 })
            && matches!(components[2], ChildNumber::Hardened { index: 5 })
            && matches!(components[3], ChildNumber::Hardened { .. })
        {
            let hardened_leaf = matches!(components.last(), Some(ChildNumber::Hardened { .. }));
            if !hardened_leaf {
                return (
                    DerivationPathReference::BlockchainIdentities,
                    DerivationPathType::SINGLE_USER_AUTHENTICATION,
                );
            }
        }

        (default_ref, default_type)
    }

    /// Subscribe to SPV reconcile signals and debounce updates.
    pub fn spv_setup_reconcile_listener(self: &Arc<Self>) {
        use tokio::time::{Duration, Instant, sleep};
        let rx = self.spv_manager.register_reconcile_channel();
        let ctx = Arc::clone(self);
        self.subtasks.spawn_sync(async move {
            tokio::pin!(rx);
            let mut last = Instant::now();
            loop {
                tokio::select! {
                    maybe = rx.recv() => {
                        if maybe.is_none() { break; }
                        // simple debounce window
                        if last.elapsed() > Duration::from_millis(300) {
                            if let Err(e) = ctx.reconcile_spv_wallets().await { tracing::debug!("SPV reconcile error: {}", e); }
                            last = Instant::now();
                        } else {
                            sleep(Duration::from_millis(300)).await;
                            if let Err(e) = ctx.reconcile_spv_wallets().await { tracing::debug!("SPV reconcile error: {}", e); }
                            last = Instant::now();
                        }
                    }
                }
            }
        });
    }

    /// Reconcile SPV wallet state into DET.
    pub async fn reconcile_spv_wallets(&self) -> Result<(), String> {
        let wm_arc = self.spv_manager.wallet();
        let wm = wm_arc.read().await;
        let mapping = self.spv_manager.det_wallets_snapshot();

        // Take a snapshot of known addresses per wallet so we can scope DB updates
        let wallets_guard = self.wallets.read().unwrap();

        for (seed_hash, wallet_id) in mapping.iter() {
            // Log total balance for visibility
            let balance = wm
                .get_wallet_balance(wallet_id)
                .map_err(|e| format!("get_wallet_balance failed: {e}"))?;
            tracing::debug!(wallet = %hex::encode(seed_hash), confirmed = balance.confirmed, unconfirmed = balance.unconfirmed, total = balance.total, "SPV balance snapshot");

            let Some(wallet_info) = wm.get_wallet_info(wallet_id) else {
                continue;
            };

            let Some(wallet_arc) = wallets_guard.get(seed_hash).cloned() else {
                continue;
            };

            self.sync_spv_account_addresses(wallet_info, &wallet_arc);

            if let Ok(mut wallet) = wallet_arc.write() {
                wallet.update_spv_balances(balance.confirmed, balance.unconfirmed, balance.total);
            }

            // Get the wallet's known addresses (only update those to avoid cross-wallet churn)
            let mut known_addresses: std::collections::BTreeSet<dash_sdk::dpp::dashcore::Address> = {
                let w = wallet_arc.read().unwrap();
                w.known_addresses.keys().cloned().collect()
            };

            // Clear existing UTXOs for these addresses in this network
            for addr in &known_addresses {
                let _ = self.db.execute(
                    "DELETE FROM utxos WHERE address = ? AND network = ?",
                    rusqlite::params![addr.to_string(), self.network.to_string()],
                );
            }

            // Read current UTXOs from SPV and re-insert, registering unknown addresses if derivation metadata is available
            let utxos = wm
                .wallet_utxos(wallet_id)
                .map_err(|e| format!("wallet_utxos failed: {e}"))?;

            use dash_sdk::dpp::dashcore::Address as CoreAddress;
            // no-op

            let mut per_address_sum: std::collections::BTreeMap<CoreAddress, u64> =
                Default::default();

            for u in utxos {
                // Best-effort accessors for outpoint/txout; adjust if API differs
                // Try field access (common struct layout): `outpoint` + `txout`
                let outpoint = u.outpoint;
                let tx_out = u.txout.clone();

                // Derive address from script
                let address = match CoreAddress::from_script(&tx_out.script_pubkey, self.network) {
                    Ok(a) => a,
                    Err(_) => continue,
                };

                // If address unknown to DET, try to register using SPV metadata
                if !known_addresses.contains(&address) {
                    let net = self.wallet_network_key();
                    if let Some(collection) = wallet_info.accounts(net) {
                        let mut registered = false;
                        for acc in collection.all_accounts() {
                            if let Some(ai) = acc.get_address_info(&address) {
                                let account_type = acc.account_type.to_account_type();
                                let (path_reference, path_type) =
                                    Self::spv_account_metadata(&account_type).unwrap_or((
                                        DerivationPathReference::BIP44,
                                        DerivationPathType::CLEAR_FUNDS,
                                    ));

                                if let Ok(inserted) = self.register_spv_address(
                                    &wallet_arc,
                                    address.clone(),
                                    ai.path.clone(),
                                    path_type,
                                    path_reference,
                                ) {
                                    if inserted {
                                        known_addresses.insert(address.clone());
                                    }
                                    registered = true;
                                }
                                break;
                            }
                        }
                        if !registered {
                            continue;
                        }
                    } else {
                        continue;
                    }
                }

                // Insert UTXO row
                self.db
                    .insert_utxo(
                        outpoint.txid.as_ref(),
                        outpoint.vout,
                        &address,
                        tx_out.value,
                        &tx_out.script_pubkey.to_bytes(),
                        self.network,
                    )
                    .map_err(|e| e.to_string())?;

                // Sum per address for balance update
                *per_address_sum.entry(address).or_default() += tx_out.value;
            }

            // Write per-address balances into DB and wallet model
            if let Some(wref) = wallets_guard.get(seed_hash)
                && let Ok(mut w) = wref.write()
            {
                for (addr, sum) in per_address_sum.into_iter() {
                    // Update wallet and DB through model helper
                    let _ = w.update_address_balance(&addr, sum, self);
                }
            }

            let history = wm
                .wallet_transaction_history(wallet_id)
                .map_err(|e| format!("wallet_transaction_history failed: {e}"))?;
            let wallet_transactions: Vec<WalletTransaction> = history
                .into_iter()
                .map(|record| WalletTransaction {
                    txid: record.txid,
                    transaction: record.transaction.clone(),
                    timestamp: record.timestamp,
                    height: record.height,
                    block_hash: record.block_hash,
                    net_amount: record.net_amount,
                    fee: record.fee,
                    label: record.label.clone(),
                    is_ours: record.is_ours,
                })
                .collect();

            self.db
                .replace_wallet_transactions(seed_hash, &self.network, &wallet_transactions)
                .map_err(|e| e.to_string())?;

            if let Some(wref) = wallets_guard.get(seed_hash)
                && let Ok(mut wallet) = wref.write()
            {
                wallet.set_transactions(wallet_transactions.clone());
            }
        }

        Ok(())
    }

    pub fn stop_spv(&self) {
        self.spv_manager.stop();
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
            let cfg_lock = self.config.read().unwrap();
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
                let provider = self.spv_context_provider.read().unwrap().clone();
                initialize_sdk(&cfg, self.network, provider)
            }
            CoreBackendMode::Rpc => {
                // Create a fresh RPC provider with the new config
                let rpc_provider = RpcProvider::new(self.db.clone(), self.network, &cfg)
                    .map_err(|e| format!("Failed to init RPC provider: {e}"))?;
                // Swap in the updated RPC provider for future switches
                {
                    let mut guard = self.rpc_context_provider.write().unwrap();
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
                .expect("Core client lock was poisoned");
            *client_lock = new_client;
        }
        {
            let mut sdk_lock = self.sdk.write().unwrap();
            *sdk_lock = new_sdk;
        }

        // Rebind providers to ensure they hold the new AppContext reference
        self.spv_context_provider
            .read()
            .unwrap()
            .bind_app_context(self.clone());
        if self.core_backend_mode() == CoreBackendMode::Rpc {
            self.rpc_context_provider
                .read()
                .unwrap()
                .bind_app_context(self.clone());
        } else {
            let sdk_lock = self.sdk.write().expect("SDK lock poisoned");
            let provider = self.spv_context_provider.read().unwrap().clone();
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
        let wallets = self.wallets.read().unwrap();
        self.db.get_local_qualified_identities(self, &wallets)
    }

    /// Fetches all local qualified identities from the database
    #[allow(dead_code)] // May be used for loading identities in wallets
    pub fn load_local_qualified_identities_in_wallets(&self) -> Result<Vec<QualifiedIdentity>> {
        let wallets = self.wallets.read().unwrap();
        self.db
            .get_local_qualified_identities_in_wallets(self, &wallets)
    }

    pub fn get_identity_by_id(
        &self,
        identity_id: &Identifier,
    ) -> Result<Option<QualifiedIdentity>> {
        let wallets = self.wallets.read().unwrap();
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
        let wallets = self.wallets.read().unwrap();
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
        let wallets = self.wallets.read().unwrap();
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
        let mut guard = self.cached_settings.write().unwrap();
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
            let cache = self.cached_settings.read().unwrap();
            if let Some(ref settings) = *cache {
                return Ok(Some(settings.clone()));
            }
        }

        // Cache miss, read from database
        let settings = self.db.get_settings()?.map(Settings::from);

        // Update cache with the fresh data
        {
            let mut cache = self.cached_settings.write().unwrap();
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
        let wallets = self.wallets.read().unwrap();
        for wallet_arc in wallets.values() {
            let mut wallet = wallet_arc.write().unwrap();
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
                    .entry(address)
                    .and_modify(|balance| *balance += tx_out.value)
                    .or_insert(tx_out.value);
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
            let mut transactions = self.transactions_waiting_for_finality.lock().unwrap();

            if let Some(asset_lock_proof) = transactions.get_mut(&tx.txid()) {
                *asset_lock_proof = proof.clone();
            }
        }

        // Identify the wallet associated with the transaction
        let wallets = self.wallets.read().unwrap();
        for wallet_arc in wallets.values() {
            let mut wallet = wallet_arc.write().unwrap();

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

                let first = payload
                    .credit_outputs
                    .first()
                    .expect("Expected at least one credit output");

                let address = Address::from_script(&first.script_pubkey, self.network)
                    .expect("expected an address");

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

/// Returns the default platform version for the given network.
pub(crate) const fn default_platform_version(network: &Network) -> &'static PlatformVersion {
    // TODO: Use self.sdk.read().unwrap().version() instead of hardcoding
    match network {
        Network::Dash => &PLATFORM_V10,
        Network::Testnet => &PLATFORM_V10,
        Network::Devnet => &PLATFORM_V10,
        Network::Regtest => &PLATFORM_V10,
        _ => panic!("unsupported network"),
    }
}
