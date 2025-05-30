use crate::app_dir::core_cookie_path;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::components::core_zmq_listener::ZMQConnectionEvent;
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
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, RwLock};

#[derive(Debug)]
pub struct AppContext {
    pub(crate) network: Network,
    pub(crate) developer_mode: bool,
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
    pub(crate) password_info: Option<PasswordInfo>,
    pub(crate) transactions_waiting_for_finality: Mutex<BTreeMap<Txid, Option<AssetLockProof>>>,
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

        let network_config = config.config_for_network(network).clone()?;
        let (sx_zmq_status, rx_zmq_status) = crossbeam_channel::unbounded();

        // we create provider, but we need to set app context to it later, as we have a circular dependency
        let provider =
            Provider::new(db.clone(), network, &network_config).expect("Failed to initialize SDK");

        let sdk = initialize_sdk(&network_config, network, provider.clone());
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

        let app_context = AppContext {
            network,
            developer_mode: network_config.developer_mode.unwrap_or(false),
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
        };

        let app_context = Arc::new(app_context);
        provider.bind_app_context(app_context.clone());

        Some(app_context)
    }

    pub fn platform_version(&self) -> &'static PlatformVersion {
        default_platform_version(&self.network)
    }

    pub fn state_transition_options(&self) -> Option<StateTransitionCreationOptions> {
        if self.developer_mode {
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

        // 2. Rebuild the RPC client with the new password
        let addr = format!("http://{}:{}", cfg.core_host, cfg.core_rpc_port);
        let new_client = Client::new(
            &addr,
            Auth::UserPass(cfg.core_rpc_user.clone(), cfg.core_rpc_password.clone()),
        )
        .map_err(|e| format!("Failed to create new Core RPC client: {e}"))?;

        // 3. Rebuild the Sdk with the updated config
        let provider = Provider::new(self.db.clone(), self.network, &cfg)
            .map_err(|e| format!("Failed to init provider: {e}"))?;
        let new_sdk = initialize_sdk(&cfg, self.network, provider.clone());

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

        // Rebind the provider to the new app context
        provider.bind_app_context(self.clone());

        Ok(())
    }

    /// Returns the network name as a string
    #[deprecated(
        since = "0.9.0",
        note = "Replace with `Network::to_string()` to avoid confusion"
    )]
    pub(crate) fn network_string(&self) -> String {
        self.network.to_string()
    }

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

    /// This is for before we know if Platform will accept the identity
    pub fn insert_local_qualified_identity_in_creation(
        &self,
        qualified_identity: &QualifiedIdentity,
        wallet_id: &[u8],
        identity_index: u32,
    ) -> Result<()> {
        self.db.insert_local_qualified_identity_in_creation(
            qualified_identity,
            wallet_id,
            identity_index,
            self,
        )
    }

    /// Fetches all local qualified identities from the database
    pub fn load_local_qualified_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        let wallets = self.wallets.read().unwrap();
        self.db.get_local_qualified_identities(self, &wallets)
    }

    /// Fetches all local qualified identities from the database
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
        self.db.insert_scheduled_votes(self, &scheduled_votes)
    }

    /// Fetches all scheduled votes from the database
    pub fn get_scheduled_votes(&self) -> Result<Vec<ScheduledDPNSVote>> {
        self.db.get_scheduled_votes(&self)
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
    pub fn delete_scheduled_vote(&self, identity_id: &[u8], contested_name: &String) -> Result<()> {
        self.db
            .delete_scheduled_vote(self, identity_id, &contested_name)
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
    pub fn get_settings(
        &self,
    ) -> Result<
        Option<(
            Network,
            RootScreenType,
            Option<PasswordInfo>,
            Option<String>,
            bool,
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
        let contract = self.db.get_contract_by_id(*contract_id, self)?;

        // If the contract is not found in the database, return None
        if contract.is_none() {
            return Ok(None);
        }

        // If the contract is found, return it
        Ok(Some(contract.unwrap()))
    }

    // Remove contract from the database by ID
    pub fn remove_contract(&self, contract_id: &Identifier) -> Result<()> {
        self.db.remove_contract(contract_id.as_bytes(), &self)
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
                    .insert(out_point.clone(), tx_out.clone()); // Insert the TxOut at the OutPoint

                // Collect the outpoint
                wallet_outpoints.push((out_point.clone(), tx_out.clone(), address.clone()));

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
        } else if let Some(chain_locked_height) = chain_locked_height {
            Some(AssetLockProof::Chain(ChainAssetLockProof {
                core_chain_locked_height: chain_locked_height,
                out_point: OutPoint::new(tx.txid(), 0),
            }))
        } else {
            None
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
        Network::Dash => &PLATFORM_V8,
        Network::Testnet => &PLATFORM_V9,
        Network::Devnet => &PLATFORM_V9,
        Network::Regtest => &PLATFORM_V9,
        _ => panic!("unsupported network"),
    }
}
