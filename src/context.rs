use crate::app_dir::{app_user_data_dir_path, core_cookie_path};
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::components::core_zmq_listener::ZMQConnectionEvent;
use crate::components::spv_manager::SpvManager;
use crate::config::{Config, NetworkConfig};
use crate::context_provider::Provider;
use crate::database::Database;
use crate::model::contested_name::ContestedName;
use crate::model::password_info::PasswordInfo;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::{DPNSNameInfo, QualifiedIdentity};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::sdk_wrapper::initialize_sdk;
use crate::ui::tokens::tokens_screen::{IdentityTokenBalance, IdentityTokenIdentifier};
use crate::ui::RootScreenType;
use bincode::config;
use crossbeam_channel::{Receiver, Sender};
use dash_sdk::dashcore_rpc::dashcore::{InstantLock, Transaction};
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload::AssetLockPayloadType;
use dash_sdk::dpp::dashcore::{Address, Network, OutPoint, TxOut, Txid};
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::prelude::{AssetLockProof, CoreBlockHeight};
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::state_transition::StateTransitionSigningOptions;
use dash_sdk::dpp::system_data_contracts::{load_system_data_contract, SystemDataContract};
use dash_sdk::dpp::version::v8::PLATFORM_V8;
use dash_sdk::dpp::version::v9::PLATFORM_V9;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{DataContract, Identifier};
use dash_sdk::query_types::IndexMap;
use dash_sdk::Sdk;
use rusqlite::Result;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

#[derive(Debug, Clone)]
pub struct SpvStatus {
    pub is_running: bool,
    pub header_height: Option<u32>,
    pub filter_height: Option<u32>,
    pub last_updated: std::time::Instant,
}

#[derive(Debug)]
pub struct AppContext {
    pub(crate) network: Network,
    pub(crate) developer_mode: AtomicBool,
    #[allow(dead_code)] // May be used for devnet identification
    pub(crate) devnet_name: Option<String>,
    pub(crate) db: Arc<Database>,
    pub(crate) sdk: RwLock<Sdk>,
    pub(crate) config: RwLock<NetworkConfig>,
    pub(crate) rx_zmq_status: Receiver<ZMQConnectionEvent>,
    pub(crate) sx_zmq_status: Sender<ZMQConnectionEvent>,
    pub(crate) zmq_connection_status: Mutex<ZMQConnectionEvent>,
    pub(crate) dpns_contract: Arc<DataContract>,
    pub(crate) withdraws_contract: Arc<DataContract>,
    pub(crate) token_history_contract: Arc<DataContract>,
    pub(crate) keyword_search_contract: Arc<DataContract>,
    pub(crate) core_client: RwLock<Client>,
    pub(crate) has_wallet: AtomicBool,
    pub(crate) wallets: RwLock<BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>>,
    #[allow(dead_code)] // May be used for password validation
    pub(crate) password_info: Option<PasswordInfo>,
    pub(crate) transactions_waiting_for_finality: Mutex<BTreeMap<Txid, Option<AssetLockProof>>>,
    pub(crate) spv_manager: Arc<SpvManager>,
    pub(crate) spv_status: Mutex<SpvStatus>,
    pub(crate) provider: RwLock<Option<Arc<Provider>>>,
}

impl AppContext {
    pub fn new(
        network: Network,
        db: Arc<Database>,
        password_info: Option<PasswordInfo>,
    ) -> Option<Arc<Self>> {
        let config = match Config::load() {
            Ok(config) => config,
            Err(e) => {
                println!("Failed to load config: {e}");
                return None;
            }
        };

        let mut network_config = config.config_for_network(network).clone()?;

        // Override connection type with the per-network setting from database
        if let Ok(saved_connection_type) = db.get_network_connection_type(network) {
            network_config.connection_type = saved_connection_type;
        }
        let (sx_zmq_status, rx_zmq_status) = crossbeam_channel::unbounded();

        // we create provider, but we need to set app context to it later, as we have a circular dependency
        let provider =
            Provider::new(db.clone(), network, &network_config).expect("Failed to initialize SDK");
        let provider = Arc::new(provider);

        let sdk = initialize_sdk(&network_config, network, (*provider).clone());
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

        // Initialize SPV manager
        let app_data_dir = app_user_data_dir_path().expect("Failed to get app user data directory");
        let spv_manager = Arc::new(SpvManager::new(app_data_dir, network));

        // Save connection type before moving network_config
        let _connection_type = network_config.connection_type.clone();

        let app_context = AppContext {
            network,
            developer_mode: AtomicBool::new(config.developer_mode.unwrap_or(false)),
            devnet_name: None,
            db,
            sdk: sdk.into(),
            config: network_config.into(),
            sx_zmq_status,
            rx_zmq_status,
            dpns_contract: Arc::new(dpns_contract),
            withdraws_contract: Arc::new(withdrawal_contract),
            token_history_contract: Arc::new(token_history_contract),
            keyword_search_contract: Arc::new(keyword_search_contract),
            core_client: core_client.into(),
            has_wallet: (!wallets.is_empty()).into(),
            wallets: RwLock::new(wallets),
            password_info,
            transactions_waiting_for_finality: Mutex::new(BTreeMap::new()),
            zmq_connection_status: Mutex::new(ZMQConnectionEvent::Disconnected),
            spv_manager,
            spv_status: Mutex::new(SpvStatus {
                is_running: false,
                header_height: None,
                filter_height: None,
                last_updated: std::time::Instant::now(),
            }),
            provider: RwLock::new(None),
        };

        let app_context = Arc::new(app_context);
        (*provider).bind_app_context(app_context.clone());

        // Store the provider reference in the app context
        {
            let mut provider_lock = app_context.provider.write().unwrap();
            *provider_lock = Some(provider);
        }

        // Bind SPV manager to app context asynchronously and auto-start if needed
        let spv_manager = app_context.spv_manager.clone();
        // Bind SPV manager to app context but don't auto-start it
        // Auto-starting SPV on initialization can cause the app to freeze
        let app_context_weak = Arc::downgrade(&app_context);
        tokio::spawn(async move {
            if let Some(app_ctx) = app_context_weak.upgrade() {
                spv_manager.bind_app_context(app_ctx.clone()).await;
                tracing::info!("SPV manager bound to app context");
            }
        });

        Some(app_context)
    }

    pub fn platform_version(&self) -> &'static PlatformVersion {
        default_platform_version(&self.network)
    }

    pub fn state_transition_options(&self) -> Option<StateTransitionCreationOptions> {
        if self.developer_mode.load(Ordering::Relaxed) {
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

        // 2. No longer preserving quorum key cache - removed functionality

        // 3. Rebuild the RPC client with the new password (only if using DashCore)
        if cfg.connection_type == crate::model::connection_type::ConnectionType::DashCore {
            let addr = format!("http://{}:{}", cfg.core_host, cfg.core_rpc_port);
            let new_client = Client::new(
                &addr,
                Auth::UserPass(cfg.core_rpc_user.clone(), cfg.core_rpc_password.clone()),
            )
            .map_err(|e| format!("Failed to create new Core RPC client: {e}"))?;

            {
                let mut client_lock = self
                    .core_client
                    .write()
                    .expect("Core client lock was poisoned");
                *client_lock = new_client;
            }
        }

        // 4. Rebuild the Sdk with the updated config
        let new_provider = Provider::new(self.db.clone(), self.network, &cfg)
            .map_err(|e| format!("Failed to init provider: {e}"))?;

        let new_sdk = initialize_sdk(&cfg, self.network, new_provider.clone());

        // 5. Swap the SDK
        {
            let mut sdk_lock = self.sdk.write().unwrap();
            *sdk_lock = new_sdk;
        }

        // Rebind the provider to the new app context
        new_provider.bind_app_context(self.clone());

        // Update the stored provider reference
        {
            let mut provider_lock = self.provider.write().unwrap();
            *provider_lock = Some(Arc::new(new_provider));
        }

        Ok(())
    }

    #[allow(dead_code)] // May be used for storing identities
    pub fn insert_local_identity(&self, identity: &Identity) -> Result<()> {
        self.db
            .insert_local_qualified_identity(&identity.clone().into(), None, self)
    }

    /// Inserts a local qualified identity into the database
    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
        wallet_and_identity_id_info: Option<(&[u8], u32)>,
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

        let wallets = self.wallets.read().unwrap();
        identities
            .into_iter()
            .map(|(mut identity, wallet_id)| {
                if let Some(wallet_id) = wallet_id {
                    // For each identity, we need to set the wallet information
                    if let Some(wallet) = wallets.get(&wallet_id) {
                        identity
                            .associated_wallets
                            .insert(wallet_id, wallet.clone());
                    } else {
                        tracing::warn!(
                            wallet = %hex::encode(wallet_id),
                            identity = %identity.identity.id(),
                            "wallet not found for identity when loading local user identities",
                        );
                    }
                }
                Ok(identity)
            })
            .collect()
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
        self.db
            .insert_or_update_settings(self.network, root_screen_type)
    }

    /// Retrieves the current `RootScreenType` from the settings
    #[allow(clippy::type_complexity)]
    pub fn get_settings(
        &self,
    ) -> Result<
        Option<(
            Network,
            RootScreenType,
            Option<PasswordInfo>,
            Option<String>,
            bool,
            crate::ui::theme::ThemeMode,
            crate::model::connection_type::ConnectionType,
        )>,
    > {
        self.db.get_settings()
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

    // Removed prefetch_quorum_keys_for_spv - no longer using key caching

    pub async fn switch_connection_type(
        self: &Arc<Self>,
        connection_type: crate::model::connection_type::ConnectionType,
    ) -> Result<crate::backend_task::BackendTaskSuccessResult, String> {
        tracing::info!("switch_connection_type called with: {:?}", connection_type);

        // Get current connection type from config
        let current_connection_type = { self.config.read().unwrap().connection_type.clone() };
        tracing::info!("Current connection type: {:?}", current_connection_type);

        // If we're already using the requested connection type, no need to switch
        if current_connection_type == connection_type {
            tracing::info!("Already using the requested connection type");
            return Ok(crate::backend_task::BackendTaskSuccessResult::Message(
                format!("Already using {} connection", connection_type.as_str()),
            ));
        }

        match connection_type {
            crate::model::connection_type::ConnectionType::DashCore => {
                // Stop SPV if it's running
                self.spv_manager
                    .stop()
                    .await
                    .map_err(|e| format!("Failed to stop SPV client: {}", e))?;

                // Update the connection type in config
                {
                    let mut config = self.config.write().unwrap();
                    config.connection_type = connection_type.clone();
                }

                // Reinitialize the SDK with the new connection type
                self.clone().reinit_core_client_and_sdk()?;

                // Update the database to match the config
                if let Err(e) = self
                    .db
                    .update_network_connection_type(self.network, connection_type)
                {
                    tracing::warn!("Failed to update connection type in database: {}", e);
                }

                Ok(crate::backend_task::BackendTaskSuccessResult::Message(
                    "Switched to Dash Core connection".to_string(),
                ))
            }
            crate::model::connection_type::ConnectionType::DashSpv => {
                tracing::info!("Switching to SPV connection");
                // Check if SPV is supported for this network
                match self.network {
                    Network::Dash | Network::Testnet => {
                        tracing::info!("SPV is supported for network: {:?}", self.network);

                        // Stop SPV if already running (clean restart)
                        if let Err(e) = self.spv_manager.stop().await {
                            tracing::warn!("Failed to stop existing SPV client: {}", e);
                        }

                        // Start SPV manager
                        tracing::info!("Starting SPV manager...");
                        match self.spv_manager.start().await {
                            Ok(_) => {
                                tracing::info!("SPV manager started successfully");
                                // Update the connection type in config
                                {
                                    let mut config = self.config.write().unwrap();
                                    config.connection_type = connection_type.clone();
                                }

                                // Reinitialize the SDK with the new connection type
                                tracing::info!("Reinitializing SDK for SPV...");
                                if let Err(e) = self.clone().reinit_core_client_and_sdk() {
                                    tracing::error!("Failed to reinitialize SDK: {}", e);
                                    // Try to stop SPV and revert
                                    let _ = self.spv_manager.stop().await;
                                    return Err(format!(
                                        "Failed to reinitialize SDK for SPV: {}",
                                        e
                                    ));
                                }
                                tracing::info!("SDK reinitialized successfully");

                                // Update the database to match the config
                                if let Err(e) = self
                                    .db
                                    .update_network_connection_type(self.network, connection_type)
                                {
                                    tracing::warn!(
                                        "Failed to update connection type in database: {}",
                                        e
                                    );
                                }

                                // No longer pre-fetching quorum keys - removed functionality

                                Ok(crate::backend_task::BackendTaskSuccessResult::Message(
                                    "Successfully switched to SPV connection".to_string(),
                                ))
                            }
                            Err(e) => Err(format!("Failed to start SPV client: {}", e)),
                        }
                    }
                    Network::Devnet | Network::Regtest => {
                        Err("SPV client only supports mainnet and testnet networks".to_string())
                    }
                    _ => Err("Unsupported network for SPV client".to_string()),
                }
            }
        }
    }

    pub async fn start_spv_sync(
        self: &Arc<Self>,
    ) -> Result<crate::backend_task::BackendTaskSuccessResult, String> {
        tracing::info!("start_spv_sync called");

        // Check if we're in SPV mode
        let connection_type = { self.config.read().unwrap().connection_type.clone() };
        if connection_type != crate::model::connection_type::ConnectionType::DashSpv {
            return Err("SPV sync can only be started when using SPV connection".to_string());
        }

        // Call the start_sync method on the SPV manager
        self.spv_manager.start_sync().await?;

        Ok(crate::backend_task::BackendTaskSuccessResult::Message(
            "SPV sync started - monitor_network is now running".to_string(),
        ))
    }
}

/// Returns the default platform version for the given network.
pub(crate) const fn default_platform_version(network: &Network) -> &'static PlatformVersion {
    // TODO: Use self.sdk.read().unwrap().version() instead of hardcoding
    match network {
        Network::Dash => &PLATFORM_V8,
        Network::Testnet => &PLATFORM_V9,
        Network::Devnet => &PLATFORM_V9,
        Network::Regtest => &PLATFORM_V9,
        _ => panic!("unsupported network"),
    }
}
