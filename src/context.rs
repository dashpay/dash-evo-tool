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
use crate::model::wallet::single_key::{SingleKeyHash, SingleKeyWallet};
use crate::model::wallet::{
    AddressInfo as WalletAddressInfo, DerivationPathHelpers, DerivationPathReference,
    DerivationPathType, Wallet, WalletSeedHash, WalletTransaction,
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
use dash_sdk::dpp::version::v11::PLATFORM_V11;
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

        // Load the core backend mode from settings, defaulting to SPV if not set
        let saved_core_backend_mode = db
            .get_settings()
            .ok()
            .flatten()
            .map(|s| s.7) // core_backend_mode is the 8th element (index 7)
            .unwrap_or(CoreBackendMode::Spv.as_u8());

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
            selected_wallet_hash: Mutex::new(None),
            selected_single_key_hash: Mutex::new(None),
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

        // Persist the mode to the database (hold the guard to ensure cache invalidation)
        let _guard = self.invalidate_settings_cache();
        if let Err(e) = self.db.update_core_backend_mode(mode.as_u8()) {
            tracing::error!("Failed to persist core backend mode: {}", e);
        }

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
        }

        if let Ok(mut single_key_wallets) = self.single_key_wallets.write() {
            single_key_wallets.clear();
        }

        self.has_wallet.store(false, Ordering::Relaxed);

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
            self.queue_platform_address_sync(seed_hash);
            // In RPC mode, also refresh Core UTXOs
            self.queue_core_wallet_refresh(wallet.clone());
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

    fn queue_platform_address_sync(self: &Arc<Self>, seed_hash: WalletSeedHash) {
        let ctx = Arc::clone(self);
        self.subtasks.spawn_sync(async move {
            if let Err(error) = ctx.fetch_platform_address_balances(seed_hash).await {
                tracing::warn!(
                    seed = %hex::encode(seed_hash),
                    %error,
                    "Failed to sync Platform address balances"
                );
            }
        });
    }

    /// Queue a Core wallet UTXO refresh (only effective in RPC mode)
    fn queue_core_wallet_refresh(self: &Arc<Self>, wallet: Arc<RwLock<Wallet>>) {
        // Only refresh in RPC mode - SPV mode handles this differently
        if self.core_backend_mode() != crate::spv::CoreBackendMode::Rpc {
            return;
        }

        let ctx = Arc::clone(self);
        self.subtasks.spawn_sync(async move {
            // Run the synchronous refresh_wallet_info in a blocking context
            let result = tokio::task::spawn_blocking(move || ctx.refresh_wallet_info(wallet)).await;

            match result {
                Ok(Ok(_)) => {
                    tracing::debug!("Successfully refreshed Core wallet UTXOs on unlock");
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, "Failed to refresh Core wallet UTXOs on unlock");
                }
                Err(join_error) => {
                    tracing::warn!(%join_error, "Task panicked while refreshing Core wallet UTXOs");
                }
            }
        });
    }

    /// Queue automatic discovery of identities derived from a wallet.
    /// Checks identity indices 0 through max_identity_index for existing identities on the network.
    pub fn queue_wallet_identity_discovery(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        max_identity_index: u32,
    ) {
        let ctx = Arc::clone(self);
        let wallet_clone = Arc::clone(wallet);
        self.subtasks.spawn_sync(async move {
            if let Err(error) = ctx
                .discover_identities_from_wallet(&wallet_clone, max_identity_index)
                .await
            {
                tracing::warn!(
                    %error,
                    "Failed to discover identities from wallet"
                );
            }
        });
    }

    /// Discover and load identities derived from a wallet by checking the network.
    async fn discover_identities_from_wallet(
        self: &Arc<Self>,
        wallet: &Arc<RwLock<Wallet>>,
        max_identity_index: u32,
    ) -> Result<(), String> {
        use dash_sdk::platform::Fetch;
        use dash_sdk::platform::types::identity::NonUniquePublicKeyHashQuery;

        const AUTH_KEY_LOOKUP_WINDOW: u32 = 12;

        let sdk = self.sdk.read().map_err(|e| e.to_string())?.clone();
        let seed_hash = wallet.read().map_err(|e| e.to_string())?.seed_hash();

        tracing::info!(
            seed = %hex::encode(seed_hash),
            "Starting identity discovery for wallet (checking indices 0..{})",
            max_identity_index
        );

        let mut found_count = 0;

        for identity_index in 0..=max_identity_index {
            // Try to find an identity at this index by checking authentication keys
            let mut fetched_identity = None;
            let mut matched_key_index = None;

            for key_index in 0..AUTH_KEY_LOOKUP_WINDOW {
                let public_key = {
                    let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
                    match wallet_guard.identity_authentication_ecdsa_public_key(
                        self.network,
                        identity_index,
                        key_index,
                    ) {
                        Ok(key) => key,
                        Err(e) => {
                            tracing::debug!(
                                "Could not derive key at index {}/{}: {}",
                                identity_index,
                                key_index,
                                e
                            );
                            continue;
                        }
                    }
                };

                let key_hash = public_key.pubkey_hash().into();
                let query = NonUniquePublicKeyHashQuery {
                    key_hash,
                    after: None,
                };

                match dash_sdk::platform::Identity::fetch(&sdk, query).await {
                    Ok(Some(identity)) => {
                        fetched_identity = Some(identity);
                        matched_key_index = Some(key_index);
                        break;
                    }
                    Ok(None) => continue,
                    Err(e) => {
                        tracing::debug!(
                            "Error querying identity at index {}/{}: {}",
                            identity_index,
                            key_index,
                            e
                        );
                        continue;
                    }
                }
            }

            // If we found an identity, process and store it
            if let Some(identity) = fetched_identity {
                let identity_id = identity.id();
                tracing::info!(
                    identity_id = %identity_id,
                    identity_index,
                    key_index = ?matched_key_index,
                    "Discovered identity from wallet"
                );

                // Check if we already have this identity stored
                let already_exists = {
                    let wallets = self.wallets.read().map_err(|e| e.to_string())?;
                    let existing = self.db.get_identity_by_id(&identity_id, self, &wallets);
                    existing.is_ok() && existing.unwrap().is_some()
                };

                if already_exists {
                    tracing::info!(
                        identity_id = %identity_id,
                        "Identity already loaded, skipping"
                    );
                    continue;
                }

                // Build qualified identity with wallet key derivation paths
                match self
                    .build_qualified_identity_from_wallet(&sdk, identity, wallet, identity_index)
                    .await
                {
                    Ok(qualified_identity) => {
                        // Store the identity
                        if let Err(e) = self.insert_local_qualified_identity(
                            &qualified_identity,
                            &Some((seed_hash, identity_index)),
                        ) {
                            tracing::warn!(
                                identity_id = %identity_id,
                                error = %e,
                                "Failed to store discovered identity"
                            );
                        } else {
                            // Add to wallet's identities map
                            if let Ok(mut wallet_guard) = wallet.write() {
                                wallet_guard
                                    .identities
                                    .insert(identity_index, qualified_identity.identity.clone());
                            }
                            found_count += 1;
                            tracing::info!(
                                identity_id = %identity_id,
                                "Successfully loaded discovered identity"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            identity_id = %identity_id,
                            error = %e,
                            "Failed to build qualified identity"
                        );
                    }
                }
            }
        }

        tracing::info!(
            seed = %hex::encode(seed_hash),
            found_count,
            "Identity discovery complete"
        );

        Ok(())
    }

    /// Build a QualifiedIdentity from a fetched Identity with wallet key derivation paths
    async fn build_qualified_identity_from_wallet(
        &self,
        sdk: &dash_sdk::Sdk,
        identity: dash_sdk::platform::Identity,
        wallet: &Arc<RwLock<Wallet>>,
        identity_index: u32,
    ) -> Result<crate::model::qualified_identity::QualifiedIdentity, String> {
        use crate::model::qualified_identity::encrypted_key_storage::{
            PrivateKeyData, WalletDerivationPath,
        };
        use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
        use crate::model::qualified_identity::{
            IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
        };
        use dash_sdk::dpp::identity::KeyType;
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
        use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, KeyDerivationType};

        let seed_hash = wallet.read().map_err(|e| e.to_string())?.seed_hash();

        // Get the highest key ID in the identity to know how many keys to derive
        let highest_key_id = identity.public_keys().keys().max().copied().unwrap_or(0);
        let derive_up_to = highest_key_id.saturating_add(6); // Add buffer for future keys

        // Derive authentication keys from wallet and build lookup maps
        let mut public_key_to_index: std::collections::BTreeMap<Vec<u8>, u32> =
            std::collections::BTreeMap::new();
        let mut public_key_hash_to_index: std::collections::BTreeMap<[u8; 20], u32> =
            std::collections::BTreeMap::new();

        {
            let wallet_guard = wallet.read().map_err(|e| e.to_string())?;
            for key_index in 0..=derive_up_to {
                if let Ok(public_key) = wallet_guard.identity_authentication_ecdsa_public_key(
                    self.network,
                    identity_index,
                    key_index,
                ) {
                    public_key_to_index.insert(public_key.to_bytes().to_vec(), key_index);
                    public_key_hash_to_index.insert(public_key.pubkey_hash().into(), key_index);
                }
            }
        }

        // Match identity keys with wallet derivation paths
        let private_keys_map: std::collections::BTreeMap<_, _> = identity
            .public_keys()
            .iter()
            .filter_map(|(key_id, identity_key)| {
                // Try to match by full public key or by hash
                let matched_index = match identity_key.key_type() {
                    KeyType::ECDSA_SECP256K1 => public_key_to_index
                        .get(identity_key.data().as_slice())
                        .copied(),
                    KeyType::ECDSA_HASH160 => {
                        let hash: [u8; 20] = identity_key.data().as_slice().try_into().ok()?;
                        public_key_hash_to_index.get(&hash).copied()
                    }
                    _ => None,
                }?;

                let derivation_path = DerivationPath::identity_authentication_path(
                    self.network,
                    KeyDerivationType::ECDSA,
                    identity_index,
                    matched_index,
                );

                let wallet_derivation_path = WalletDerivationPath {
                    wallet_seed_hash: seed_hash,
                    derivation_path,
                };

                Some((
                    (PrivateKeyTarget::PrivateKeyOnMainIdentity, *key_id),
                    (
                        QualifiedIdentityPublicKey::from_identity_public_key_in_wallet(
                            identity_key.clone(),
                            Some(wallet_derivation_path.clone()),
                        ),
                        PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path),
                    ),
                ))
            })
            .collect();

        // Fetch DPNS names for this identity
        let dpns_names = {
            use dash_sdk::dpp::document::DocumentV0Getters;
            use dash_sdk::dpp::platform_value::Value;
            use dash_sdk::drive::query::{WhereClause, WhereOperator};
            use dash_sdk::platform::{Document, DocumentQuery, FetchMany};

            let query = DocumentQuery {
                data_contract: self.dpns_contract.clone(),
                document_type_name: "domain".to_string(),
                where_clauses: vec![WhereClause {
                    field: "records.identity".to_string(),
                    operator: WhereOperator::Equal,
                    value: Value::Identifier(identity.id().into()),
                }],
                order_by_clauses: vec![],
                limit: 100,
                start: None,
            };

            match Document::fetch_many(sdk, query).await {
                Ok(document_map) => document_map
                    .values()
                    .filter_map(|maybe_doc| {
                        maybe_doc.as_ref().and_then(|doc| {
                            let name = doc
                                .get("label")
                                .map(|label| label.to_str().unwrap_or_default());
                            let acquired_at = doc
                                .created_at()
                                .into_iter()
                                .chain(doc.transferred_at())
                                .max();

                            match (name, acquired_at) {
                                (Some(name), Some(acquired_at)) => Some(DPNSNameInfo {
                                    name: name.to_string(),
                                    acquired_at,
                                }),
                                _ => None,
                            }
                        })
                    })
                    .collect::<Vec<DPNSNameInfo>>(),
                Err(e) => {
                    tracing::warn!("Failed to fetch DPNS names for identity: {}", e);
                    Vec::new()
                }
            }
        };

        // Build the qualified identity
        let mut associated_wallets = std::collections::BTreeMap::new();
        associated_wallets.insert(seed_hash, Arc::clone(wallet));

        Ok(QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: private_keys_map.into(),
            dpns_names,
            associated_wallets,
            wallet_index: Some(identity_index),
            top_ups: Default::default(),
            status: IdentityStatus::Unknown,
            network: self.network,
        })
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

    /// Fetch Platform address balances using privacy-preserving sync with WalletAddressProvider
    pub(crate) async fn fetch_platform_address_balances(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
    ) -> Result<BackendTaskSuccessResult, String> {
        use crate::model::wallet::WalletAddressProvider;

        let wallet_arc = {
            let wallets = self.wallets.read().unwrap();
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or_else(|| "Wallet not found".to_string())?
        };

        // Create provider (requires wallet to be open)
        let mut provider = {
            let wallet = wallet_arc.read().map_err(|e| e.to_string())?;
            WalletAddressProvider::new(&wallet, self.network)?
        };

        // Sync using SDK's privacy-preserving method
        let sdk = {
            let guard = self.sdk.read().map_err(|e| e.to_string())?;
            guard.clone()
        };

        let result = sdk
            .sync_address_balances(&mut provider, None)
            .await
            .map_err(|e| format!("Failed to sync Platform addresses: {}", e))?;

        tracing::info!(
            "Platform address sync complete: found={}, absent={}, highest_index={:?}, checkpoint_height={}",
            result.found.len(),
            result.absent.len(),
            result.highest_found_index,
            result.checkpoint_height
        );

        // Log the found balances from provider
        for (addr, balance) in provider.found_balances() {
            use dash_sdk::dpp::address_funds::PlatformAddress;
            let platform_addr_str = PlatformAddress::try_from(addr.clone())
                .map(|p| p.to_bech32m_string(self.network))
                .unwrap_or_else(|_| addr.to_string());
            tracing::info!(
                "Sync found address: {} with balance: {}",
                platform_addr_str,
                balance
            );
        }

        // Step 2: Fetch recent balance changes (terminal updates after checkpoint)
        // This catches any balance changes that happened after the checkpoint the trunk/branch sync used
        self.apply_recent_balance_changes(&sdk, &wallet_arc, &mut provider, result.checkpoint_height)
            .await;

        // Apply results to wallet and persist
        let balances = {
            let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;

            provider.apply_results_to_wallet(&mut wallet);

            // Persist to database
            for (address, balance) in provider.found_balances() {
                let nonce = wallet
                    .platform_address_info
                    .get(address)
                    .map(|info| info.nonce)
                    .unwrap_or(0);
                if let Err(e) = self.db.set_platform_address_info(
                    &seed_hash,
                    address,
                    *balance,
                    nonce,
                    &self.network,
                ) {
                    tracing::warn!("Failed to persist Platform address info: {}", e);
                }
            }

            // Return balances for result (nonce preserved from existing info or 0)
            provider
                .found_balances()
                .iter()
                .map(|(addr, bal)| {
                    let nonce = wallet
                        .platform_address_info
                        .get(addr)
                        .map(|info| info.nonce)
                        .unwrap_or(0);
                    (addr.to_string(), (*bal, nonce))
                })
                .collect()
        };

        Ok(BackendTaskSuccessResult::PlatformAddressBalances {
            seed_hash,
            balances,
        })
    }

    /// Apply recent balance changes (terminal updates) to catch changes after the checkpoint.
    ///
    /// The trunk/branch sync provides balances as of a checkpoint (every ~10 minutes).
    /// This function fetches balance changes since the checkpoint to provide
    /// more up-to-date balances.
    ///
    /// Two queries are performed in sequence:
    /// 1. RecentCompactedAddressBalanceChanges - merged changes for ranges of blocks
    /// 2. RecentAddressBalanceChanges - individual per-block changes for most recent blocks
    async fn apply_recent_balance_changes(
        &self,
        sdk: &Sdk,
        wallet_arc: &Arc<RwLock<Wallet>>,
        provider: &mut crate::model::wallet::WalletAddressProvider,
        checkpoint_height: u64,
    ) {
        use dash_sdk::dpp::address_funds::PlatformAddress;
        use dash_sdk::dpp::balances::credits::CreditOperation;
        use dash_sdk::platform::{
            Fetch, RecentAddressBalanceChangesQuery, RecentCompactedAddressBalanceChangesQuery,
        };
        use dash_sdk::query_types::{RecentAddressBalanceChanges, RecentCompactedAddressBalanceChanges};

        // The trunk/branch sync provides balances as of the checkpoint height.
        // We query for compacted changes starting from that checkpoint height,
        // then query recent non-compacted changes starting from where compacted ends.

        tracing::debug!(
            "Fetching terminal balance updates from checkpoint height {}",
            checkpoint_height
        );

        // Get the wallet's platform addresses to filter relevant changes
        let wallet_platform_addresses: std::collections::HashSet<PlatformAddress> = {
            let wallet = match wallet_arc.read() {
                Ok(w) => w,
                Err(_) => return,
            };
            wallet
                .platform_addresses(self.network)
                .into_iter()
                .map(|(_, platform_addr)| platform_addr)
                .collect()
        };

        let mut updates_applied = 0;
        let mut latest_block_height = checkpoint_height;

        // Step 1: Fetch compacted balance changes (merged changes for ranges of blocks)
        // Start from checkpoint_height to get changes since the trunk/branch sync
        let compacted_query = RecentCompactedAddressBalanceChangesQuery::new(checkpoint_height);
        if let Ok(Some(compacted_changes)) =
            RecentCompactedAddressBalanceChanges::fetch(sdk, compacted_query).await
        {
            for block_changes in compacted_changes.into_inner() {
                // Track the latest block height we've processed
                if block_changes.end_block_height > latest_block_height {
                    latest_block_height = block_changes.end_block_height;
                }

                for (platform_addr, credit_op) in block_changes.changes {
                    if wallet_platform_addresses.contains(&platform_addr) {
                        let core_addr = platform_addr.to_address_with_network(self.network);
                        let current_balance = provider
                            .found_balances()
                            .get(&core_addr)
                            .copied()
                            .unwrap_or(0);

                        let new_balance = match credit_op {
                            CreditOperation::SetCredits(credits) => credits,
                            CreditOperation::AddToCredits(credits) => {
                                current_balance.saturating_add(credits)
                            }
                        };

                        if new_balance != current_balance {
                            provider.update_balance(&core_addr, new_balance);
                            let addr_str = platform_addr.to_bech32m_string(self.network);
                            tracing::info!(
                                "Compacted update: {} balance {} -> {}",
                                addr_str,
                                current_balance,
                                new_balance
                            );
                            updates_applied += 1;
                        }
                    }
                }
            }
        }

        // Step 2: Fetch non-compacted balance changes (individual per-block changes)
        // Use the latest block height from compacted changes + 1 as the start
        let recent_query = RecentAddressBalanceChangesQuery::new(latest_block_height + 1);
        if let Ok(Some(recent_changes)) =
            RecentAddressBalanceChanges::fetch(sdk, recent_query).await
        {
            for block_changes in recent_changes.into_inner() {
                for (platform_addr, credit_op) in block_changes.changes {
                    if wallet_platform_addresses.contains(&platform_addr) {
                        let core_addr = platform_addr.to_address_with_network(self.network);
                        let current_balance = provider
                            .found_balances()
                            .get(&core_addr)
                            .copied()
                            .unwrap_or(0);

                        let new_balance = match credit_op {
                            CreditOperation::SetCredits(credits) => credits,
                            CreditOperation::AddToCredits(credits) => {
                                current_balance.saturating_add(credits)
                            }
                        };

                        if new_balance != current_balance {
                            provider.update_balance(&core_addr, new_balance);
                            let addr_str = platform_addr.to_bech32m_string(self.network);
                            tracing::info!(
                                "Recent update: {} balance {} -> {}",
                                addr_str,
                                current_balance,
                                new_balance
                            );
                            updates_applied += 1;
                        }
                    }
                }
            }
        }

        if updates_applied > 0 {
            tracing::info!(
                "Applied {} terminal balance updates from recent blocks",
                updates_applied
            );
        }
    }

    /// Update wallet platform address info from SDK-returned AddressInfos.
    /// This uses the proof-verified data from SDK operations rather than fetching.
    pub(crate) fn update_wallet_platform_address_info_from_sdk(
        &self,
        seed_hash: WalletSeedHash,
        address_infos: &dash_sdk::query_types::AddressInfos,
    ) -> Result<(), String> {
        let wallet_arc = {
            let wallets = self.wallets.read().unwrap();
            wallets
                .get(&seed_hash)
                .cloned()
                .ok_or_else(|| "Wallet not found".to_string())?
        };

        let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;

        for (platform_addr, maybe_info) in address_infos.iter() {
            if let Some(info) = maybe_info {
                // Convert PlatformAddress to core Address using the network
                let core_addr = platform_addr.to_address_with_network(self.network);

                // Update in-memory wallet state
                wallet.set_platform_address_info(core_addr.clone(), info.balance, info.nonce);

                // Update database
                if let Err(e) = self.db.set_platform_address_info(
                    &seed_hash,
                    &core_addr,
                    info.balance,
                    info.nonce,
                    &self.network,
                ) {
                    tracing::warn!("Failed to store Platform address info in database: {}", e);
                }

                tracing::debug!(
                    "Updated platform address {} balance={} nonce={} from SDK response",
                    core_addr,
                    info.balance,
                    info.nonce
                );
            }
        }

        Ok(())
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
        let fee_strategy = vec![AddressFundsFeeStrategyStep::ReduceOutput(0)];

        // Use the SDK to transfer - returns proof-verified updated address infos
        let address_infos = sdk
            .transfer_address_funds(inputs, outputs, fee_strategy, &wallet, None)
            .await
            .map_err(|e| format!("Failed to transfer Platform credits: {}", e))?;

        // Update wallet balances from the proof-verified response (no extra fetch needed)
        self.update_wallet_platform_address_info_from_sdk(seed_hash, &address_infos)?;

        Ok(BackendTaskSuccessResult::PlatformCreditsTransferred { seed_hash })
    }

    /// Fund Platform addresses from an asset lock
    pub(crate) async fn fund_platform_address_from_asset_lock(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        asset_lock_proof: AssetLockProof,
        asset_lock_address: Address,
        outputs: std::collections::BTreeMap<
            dash_sdk::dpp::address_funds::PlatformAddress,
            Option<dash_sdk::dpp::balances::credits::Credits>,
        >,
    ) -> Result<BackendTaskSuccessResult, String> {
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::dpp::dashcore::OutPoint;
        use dash_sdk::platform::transition::top_up_address::TopUpAddress;

        // Clone wallet and SDK before the async operation to avoid holding guards across await
        let (wallet, sdk, asset_lock_private_key) = {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or_else(|| "Wallet not found".to_string())?
            };
            let wallet = wallet_arc.read().map_err(|e| e.to_string())?.clone();
            let sdk = self.sdk.read().map_err(|e| e.to_string())?.clone();

            // Get the private key for the asset lock address
            let private_key = wallet
                .private_key_for_address(&asset_lock_address, self.network)
                .map_err(|e| format!("Failed to get private key: {}", e))?
                .ok_or_else(|| "Asset lock address not found in wallet".to_string())?;

            (wallet, sdk, private_key)
        };

        // Check if we need to convert an old instant lock proof to a chain lock proof
        use dash_sdk::dashcore_rpc::RpcApi;
        use dash_sdk::dpp::block::extended_epoch_info::ExtendedEpochInfo;
        use dash_sdk::platform::Fetch;

        let asset_lock_proof = if let AssetLockProof::Instant(instant_asset_lock_proof) =
            &asset_lock_proof
        {
            // Get the transaction ID from the instant lock proof
            let tx_id = instant_asset_lock_proof.transaction().txid();

            // Query the core client to check if the transaction has been chain-locked
            let raw_transaction_info = self
                .core_client
                .read()
                .expect("Core client lock was poisoned")
                .get_raw_transaction_info(&tx_id, None)
                .map_err(|e| format!("Failed to get transaction info: {}", e))?;

            if raw_transaction_info.chainlock
                && raw_transaction_info.height.is_some()
                && raw_transaction_info.confirmations.is_some()
                && raw_transaction_info.confirmations.unwrap() > 8
            {
                // Transaction has been chain-locked with sufficient confirmations
                let tx_block_height = raw_transaction_info.height.unwrap() as u32;

                // Check if the platform has caught up to this block height
                let (_, metadata) = ExtendedEpochInfo::fetch_with_metadata(&sdk, 0, None)
                    .await
                    .map_err(|e| format!("Failed to get platform metadata: {}", e))?;

                if tx_block_height <= metadata.core_chain_locked_height {
                    // Platform has synced past this block, use chain lock proof
                    AssetLockProof::Chain(ChainAssetLockProof {
                        core_chain_locked_height: tx_block_height,
                        out_point: OutPoint::new(tx_id, 0),
                    })
                } else {
                    // Platform hasn't verified this Core block yet - can't use chain lock proof
                    // and instant lock is stale. User needs to wait.
                    return Err(format!(
                        "Cannot use this asset lock yet. The instant lock proof has expired (quorum rotated), \
                            and Platform hasn't verified Core block {} yet (Platform has verified up to Core block {}). \
                            Please wait for Platform to sync with Core chain.",
                        tx_block_height, metadata.core_chain_locked_height
                    ));
                }
            } else {
                // Use the instant lock proof as-is (transaction is recent)
                asset_lock_proof
            }
        } else {
            // Already a chain lock proof, use as-is
            asset_lock_proof
        };

        // Simple fee strategy: reduce from first output
        let fee_strategy = vec![AddressFundsFeeStrategyStep::ReduceOutput(0)];

        // Get the transaction ID before consuming the asset lock proof
        let tx_id = match &asset_lock_proof {
            AssetLockProof::Instant(instant) => instant.transaction().txid(),
            AssetLockProof::Chain(chain) => chain.out_point.txid,
        };

        // Use the SDK to top up Platform addresses from asset lock
        let _result = outputs
            .top_up(
                &sdk,
                asset_lock_proof,
                asset_lock_private_key,
                fee_strategy,
                &wallet,
                None,
            )
            .await
            .map_err(|e| format!("Failed to fund Platform address from asset lock: {}", e))?;

        // Remove the used asset lock from the wallet and database
        {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets.get(&seed_hash).cloned()
            };
            if let Some(wallet_arc) = wallet_arc {
                let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;
                wallet
                    .unused_asset_locks
                    .retain(|(tx, _, _, _, _)| tx.txid() != tx_id);
            }
            // Also remove from database
            if let Err(e) = self.db.delete_asset_lock_transaction(&tx_id.to_string()) {
                tracing::warn!("Failed to delete asset lock from database: {}", e);
            }
        }

        // Trigger a balance refresh
        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
    }

    /// Withdraw from Platform addresses to Core
    pub(crate) async fn withdraw_from_platform_address(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        inputs: std::collections::BTreeMap<
            dash_sdk::dpp::address_funds::PlatformAddress,
            dash_sdk::dpp::balances::credits::Credits,
        >,
        output_script: dash_sdk::dpp::identity::core_script::CoreScript,
        core_fee_per_byte: u32,
    ) -> Result<BackendTaskSuccessResult, String> {
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::dpp::withdrawal::Pooling;
        use dash_sdk::platform::transition::address_credit_withdrawal::WithdrawAddressFunds;

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

        // Use the SDK to withdraw
        let _result = sdk
            .withdraw_address_funds(
                inputs,
                None, // No change output
                fee_strategy,
                core_fee_per_byte,
                Pooling::Never,
                output_script,
                &wallet,
                None,
            )
            .await
            .map_err(|e| format!("Failed to withdraw from Platform address: {}", e))?;

        // Trigger a balance refresh
        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressWithdrawal { seed_hash })
    }

    /// Fund a platform address directly from wallet UTXOs.
    /// Creates an asset lock, broadcasts it, waits for confirmation, then funds the destination.
    pub(crate) async fn fund_platform_address_from_wallet_utxos(
        self: &Arc<Self>,
        seed_hash: WalletSeedHash,
        amount: u64,
        destination: dash_sdk::dpp::address_funds::PlatformAddress,
    ) -> Result<BackendTaskSuccessResult, String> {
        use dash_sdk::dashcore_rpc::RpcApi;
        use dash_sdk::dpp::address_funds::AddressFundsFeeStrategyStep;
        use dash_sdk::dpp::prelude::AssetLockProof;
        use dash_sdk::platform::transition::top_up_address::TopUpAddress;
        use std::time::Duration;

        // Step 1: Create the asset lock transaction
        let (asset_lock_transaction, asset_lock_private_key, _asset_lock_address, used_utxos) = {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or_else(|| "Wallet not found".to_string())?
            };

            let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;

            // Try to create the asset lock transaction, reload UTXOs if needed
            match wallet.generic_asset_lock_transaction(
                self.network,
                amount,
                true, // allow_take_fee_from_amount
                Some(self),
            ) {
                Ok((tx, private_key, address, _change, utxos)) => (tx, private_key, address, utxos),
                Err(_) => {
                    // Reload UTXOs and try again
                    wallet
                        .reload_utxos(
                            &self
                                .core_client
                                .read()
                                .expect("Core client lock was poisoned"),
                            self.network,
                            Some(self),
                        )
                        .map_err(|e| e.to_string())?;

                    let (tx, private_key, address, _change, utxos) = wallet
                        .generic_asset_lock_transaction(self.network, amount, true, Some(self))?;
                    (tx, private_key, address, utxos)
                }
            }
        };

        let tx_id = asset_lock_transaction.txid();

        // Step 2: Register this transaction as waiting for finality
        {
            let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
            proofs.insert(tx_id, None);
        }

        // Step 3: Broadcast the transaction
        self.core_client
            .read()
            .expect("Core client lock was poisoned")
            .send_raw_transaction(&asset_lock_transaction)
            .map_err(|e| format!("Failed to broadcast asset lock transaction: {}", e))?;

        // Step 4: Remove used UTXOs from wallet
        {
            let wallet_arc = {
                let wallets = self.wallets.read().unwrap();
                wallets
                    .get(&seed_hash)
                    .cloned()
                    .ok_or_else(|| "Wallet not found".to_string())?
            };

            let mut wallet = wallet_arc.write().map_err(|e| e.to_string())?;
            wallet.utxos.retain(|_, utxo_map| {
                utxo_map.retain(|outpoint, _| !used_utxos.contains_key(outpoint));
                !utxo_map.is_empty()
            });

            for utxo in used_utxos.keys() {
                self.db
                    .drop_utxo(utxo, &self.network.to_string())
                    .map_err(|e| e.to_string())?;
            }

            // Update address_balances for affected addresses
            let affected_addresses: std::collections::BTreeSet<_> =
                used_utxos.values().map(|(_, addr)| addr.clone()).collect();
            for address in affected_addresses {
                // Recalculate balance from remaining UTXOs for this address
                let new_balance = wallet
                    .utxos
                    .get(&address)
                    .map(|utxo_map| utxo_map.values().map(|tx_out| tx_out.value).sum())
                    .unwrap_or(0);
                let _ = wallet.update_address_balance(&address, new_balance, self);
            }
        }

        // Step 5: Wait for asset lock proof (InstantLock or ChainLock)
        let asset_lock_proof: AssetLockProof;
        loop {
            {
                let proofs = self.transactions_waiting_for_finality.lock().unwrap();
                if let Some(Some(proof)) = proofs.get(&tx_id) {
                    asset_lock_proof = proof.clone();
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        // Step 6: Clean up the finality tracking
        {
            let mut proofs = self.transactions_waiting_for_finality.lock().unwrap();
            proofs.remove(&tx_id);
        }

        // Step 7: Get wallet and SDK for the platform funding operation
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

        // Step 8: Fund the destination platform address
        let mut outputs = std::collections::BTreeMap::new();
        outputs.insert(destination, None); // None means use all available funds

        let fee_strategy = vec![AddressFundsFeeStrategyStep::ReduceOutput(0)];

        outputs
            .top_up(
                &sdk,
                asset_lock_proof,
                asset_lock_private_key,
                fee_strategy,
                &wallet,
                None,
            )
            .await
            .map_err(|e| format!("Failed to fund platform address: {}", e))?;

        // Step 9: Refresh platform address balances
        self.fetch_platform_address_balances(seed_hash).await?;

        Ok(BackendTaskSuccessResult::PlatformAddressFunded { seed_hash })
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
        let collection = wallet_info.accounts();

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
                // Persist balances to database
                if let Err(e) = self.db.update_wallet_balances(
                    seed_hash,
                    balance.confirmed,
                    balance.unconfirmed,
                    balance.total,
                ) {
                    tracing::warn!(wallet = %hex::encode(seed_hash), error = %e, "Failed to persist wallet balances");
                }
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
                    let collection = wallet_info.accounts();
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
        Network::Dash => &PLATFORM_V11,
        Network::Testnet => &PLATFORM_V11,
        Network::Devnet => &PLATFORM_V11,
        Network::Regtest => &PLATFORM_V11,
        _ => panic!("unsupported network"),
    }
}
