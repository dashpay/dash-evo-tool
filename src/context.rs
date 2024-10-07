use crate::config::Config;
use crate::database::Database;
use crate::model::contested_name::ContestedName;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::sdk_wrapper::initialize_sdk;
use dash_sdk::Sdk;
use dpp::dashcore::Network;
use dpp::identity::Identity;
use rusqlite::Result;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct AppContext {
    pub(crate) network: Network,
    pub(crate) devnet_name: Option<String>,
    pub(crate) db: Arc<Database>,
    pub(crate) sdk: Arc<Mutex<Sdk>>,
    pub(crate) config: Config,
}

impl AppContext {
    pub fn new() -> Self {
        let db = Arc::new(Database::new("identities.db").unwrap());

        let config = Config::load();

        db.initialize().unwrap();

        let sdk = Arc::new(Mutex::new(initialize_sdk(&config)));

        AppContext {
            network: config.core_network(),
            devnet_name: None,
            db,
            sdk,
            config,
        }
    }
    pub fn insert_identity(&self, identity: &Identity) -> Result<()> {
        self.db.insert_identity(&identity.clone().into(), self)
    }

    pub fn insert_qualified_identity(&self, qualified_identity: &QualifiedIdentity) -> Result<()> {
        self.db.insert_identity(qualified_identity, self)
    }

    pub fn load_identities(&self) -> Result<Vec<QualifiedIdentity>> {
        self.db.get_identities(self)
    }

    pub fn load_contested_names(&self) -> Result<Vec<ContestedName>> {
        Ok(vec![])
    }
}
