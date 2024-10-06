use rusqlite::{params, Connection, Result};
use std::path::Path;
use dpp::dashcore::Network;
use dpp::identity::accessors::IdentityGettersV0;
use dpp::identity::Identity;
use dpp::platform_value::string_encoding::Encoding;
use dpp::serialization::{PlatformDeserializable, PlatformSerializable};
use crate::context::AppContext;

#[derive(Debug)]
pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Self { conn })
    }

    pub fn initialize(&self) -> Result<()> {
        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS identities (
                id BLOB PRIMARY KEY,
                data BLOB NOT NULL,
                network TEXT NOT NULL
            )",
            [],
        )?;
        Ok(())
    }
    pub fn insert_identity(&self, identity: &Identity, network_info: &AppContext) -> Result<()> {
        let id = identity.id().to_string(Encoding::Hex);
        let data = identity.serialize_to_bytes().expect("expected to serialize");

        // Combine network and devnet_name (if applicable)
        let network = match network_info.network {
            Network::Dash => "dash".to_string(),
            Network::Testnet => "testnet".to_string(),
            Network::Devnet => format!("devnet:{}", network_info.devnet_name.clone().unwrap_or_default()),
            Network::Regtest => "regtest".to_string(),
            _ => "unknown".to_string(),
        };

        self.conn.execute(
            "INSERT OR REPLACE INTO identities (id, data, network) VALUES (?, ?, ?)",
            params![id, data, network],
        )?;
        Ok(())
    }

    pub fn get_identities(&self, network_info: &AppContext) -> Result<Vec<Identity>> {
        let network = match network_info.network {
            Network::Dash => "dash".to_string(),
            Network::Testnet => "testnet".to_string(),
            Network::Devnet => format!("devnet:{}", network_info.devnet_name.clone().unwrap_or_default()),
            Network::Regtest => "regtest".to_string(),
            _ => "unknown".to_string(),
        };

        let mut stmt = self.conn.prepare("SELECT data FROM identities WHERE network = ?")?;
        let identity_iter = stmt.query_map(params![network], |row| {
            let data: Vec<u8> = row.get(0)?;
            let identity: Identity = Identity::deserialize_from_bytes(&data).expect("expected to deserialize identity");
            Ok(identity)
        })?;

        let identities: Result<Vec<Identity>> = identity_iter.collect();
        identities
    }
}