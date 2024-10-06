use dpp::dashcore::Network;
use dpp::identity::Identity;
use crate::database::Database;
use rusqlite::Result;
use std::sync::{Arc, Mutex};
use dash_sdk::Sdk;
use crate::config::Config;
use crate::sdk_wrapper::initialize_sdk;

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
        self.db.insert_identity(identity, self)
    }

    pub fn load_identities(&self) -> Result<Vec<Identity>> {
        self.db.get_identities(self)
    }
}