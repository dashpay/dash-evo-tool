use crate::config::{Config, NetworkConfig};
use crate::context_provider::Provider;
use crate::database::Database;
use crate::model::contested_name::ContestedName;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::{DPNSNameInfo, QualifiedIdentity};
use crate::model::wallet::Wallet;
use crate::sdk_wrapper::initialize_sdk;
use crate::ui::RootScreenType;
use dash_sdk::dashcore_rpc::{Auth, Client};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::system_data_contracts::{load_system_data_contract, SystemDataContract};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{DataContract, Identifier};
use dash_sdk::Sdk;
use rusqlite::Result;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};

#[derive(Debug)]
pub struct AppContext {
    pub(crate) network: Network,
    pub(crate) developer_mode: bool,
    pub(crate) devnet_name: Option<String>,
    pub(crate) db: Arc<Database>,
    pub(crate) sdk: Sdk,
    pub(crate) config: NetworkConfig,
    pub(crate) dpns_contract: Arc<DataContract>,
    pub(crate) withdraws_contract: Arc<DataContract>,
    pub(crate) core_client: Client,
    pub(crate) has_wallet: AtomicBool,
    pub(crate) wallets: RwLock<Vec<Arc<RwLock<Wallet>>>>,
    pub(crate) platform_version: &'static PlatformVersion,
}

impl AppContext {
    pub fn new(network: Network, db: Arc<Database>) -> Option<Arc<Self>> {
        let config = match Config::load() {
            Ok(config) => config,
            Err(e) => {
                println!("Failed to load config: {e}");
                return None;
            }
        };

        let network_config = config.config_for_network(network).clone()?;

        // we create provider, but we need to set app context to it later, as we have a circular dependency
        let provider =
            Provider::new(db.clone(), &network_config).expect("Failed to initialize SDK");

        let sdk = initialize_sdk(&network_config, network, provider.clone());

        let dpns_contract =
            load_system_data_contract(SystemDataContract::DPNS, PlatformVersion::latest())
                .expect("expected to load dpns contract");

        let withdrawal_contract =
            load_system_data_contract(SystemDataContract::Withdrawals, PlatformVersion::latest())
                .expect("expected to get withdrawal contract");

        let addr = format!(
            "http://{}:{}",
            network_config.core_host, network_config.core_rpc_port
        );
        let core_client = Client::new(
            &addr,
            Auth::UserPass(
                network_config.core_rpc_user.to_string(),
                network_config.core_rpc_password.to_string(),
            ),
        )
        .ok()?;

        let wallets: Vec<_> = db
            .get_wallets(&network)
            .expect("expected to get wallets")
            .into_iter()
            .map(|w| Arc::new(RwLock::new(w)))
            .collect();

        let app_context = AppContext {
            network,
            developer_mode: false,
            devnet_name: None,
            db,
            sdk,
            config: network_config,
            dpns_contract: Arc::new(dpns_contract),
            withdraws_contract: Arc::new(withdrawal_contract),
            core_client,
            has_wallet: (!wallets.is_empty()).into(),
            wallets: RwLock::new(wallets),
            platform_version: PlatformVersion::latest(),
        };

        let app_context = Arc::new(app_context);
        provider.bind_app_context(app_context.clone());

        Some(app_context)
    }

    pub(crate) fn network_string(&self) -> String {
        match self.network {
            Network::Dash => "dash".to_string(),
            Network::Testnet => "testnet".to_string(),
            Network::Devnet => format!("devnet:{}", self.devnet_name.clone().unwrap_or_default()),
            Network::Regtest => "regtest".to_string(),
            _ => "unknown".to_string(),
        }
    }

    pub fn insert_local_identity(&self, identity: &Identity) -> Result<()> {
        self.db
            .insert_local_qualified_identity(&identity.clone().into(), self)
    }

    pub fn insert_local_qualified_identity(
        &self,
        qualified_identity: &QualifiedIdentity,
    ) -> Result<()> {
        self.db
            .insert_local_qualified_identity(qualified_identity, self)
    }

    pub fn load_local_qualified_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        self.db.get_local_qualified_identities(self)
    }

    pub fn all_contested_names(&self) -> Result<Vec<ContestedName>> {
        self.db.get_all_contested_names(self)
    }

    pub fn ongoing_contested_names(&self) -> Result<Vec<ContestedName>> {
        self.db.get_ongoing_contested_names(self)
    }

    /// Fetches the local identities from the database and then maps them to their DPNS names.
    /// A vector of tuples is used here for easy column sorting in the UI.
    pub fn local_dpns_names(&self) -> Result<Vec<(Identifier, DPNSNameInfo)>> {
        let identities = self.db.get_local_qualified_identities(self)?;

        // Map each identity's DPNS names to (Identifier, DPNSNameInfo) tuples
        let dpns_names = identities
            .into_iter()
            .flat_map(|identity| {
                identity
                    .dpns_names
                    .unwrap_or_default()
                    .into_iter()
                    .map(move |name| {
                        let clean_name = name
                            .name
                            .strip_prefix("string ")
                            .unwrap_or(&name.name)
                            .to_string();
                        (
                            identity.identity.id(),
                            DPNSNameInfo {
                                name: clean_name,
                                acquired_at: name.acquired_at,
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
    pub fn get_settings(&self) -> Result<Option<(Network, RootScreenType)>> {
        self.db.get_settings()
    }

    /// Retrieves the DPNS contract along with other contracts from the database.
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
            alias: Some("dpns".to_string()), // You can adjust the alias as needed
        };

        // Insert the DPNS contract at the beginning
        contracts.insert(0, dpns_contract);

        Ok(contracts)
    }
}
