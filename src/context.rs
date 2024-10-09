use crate::config::{Config, NetworkConfig};
use crate::context_provider::Provider;
use crate::database::Database;
use crate::model::contested_name::ContestedName;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::sdk_wrapper::initialize_sdk;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::Identity;
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::DataContract;
use dash_sdk::Sdk;
use rusqlite::Result;
use std::sync::Arc;

#[derive(Debug)]
pub struct AppContext {
    pub(crate) network: Network,
    pub(crate) devnet_name: Option<String>,
    pub(crate) db: Arc<Database>,
    pub(crate) sdk: Sdk,
    pub(crate) config: NetworkConfig,
    pub(crate) dpns_contract: Arc<Option<DataContract>>,
    pub(crate) platform_version: &'static PlatformVersion,
}

impl AppContext {
    pub fn new(network: Network) -> Option<Arc<Self>> {
        let db = Arc::new(Database::new("identities.db").unwrap());

        let config = Config::load();

        db.initialize().unwrap();

        let network_config = config.config_for_network(network).clone()?;

        // we create provider, but we need to set app context to it later, as we have a circular dependency
        let provider =
            Provider::new(db.clone(), &network_config).expect("Failed to initialize SDK");

        let sdk = initialize_sdk(&network_config, network, provider.clone());

        let mut app_context = AppContext {
            network,
            devnet_name: None,
            db,
            sdk,
            config: network_config,
            dpns_contract: Arc::new(None),
            platform_version: PlatformVersion::latest(),
        };

        let contract = app_context
            .db
            .get_contract_by_name("dpns", &app_context)
            .expect("expected to be able to get contract");

        if let Some(contract) = contract {
            app_context.dpns_contract = Arc::new(Some(contract));
        }
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

    pub fn load_contested_names(&self) -> Result<Vec<ContestedName>> {
        self.db.get_contested_names(self)
    }
}
