mod asset_lock_transaction;
mod contested_names;
pub(crate) mod contracts;
mod identities;
mod initialization;
mod proof_log;
mod scheduled_votes;
mod settings;
mod tokens;
mod top_ups;
mod utxo;
mod wallet;

use rusqlite::{Connection, Params};
use std::sync::Mutex;

/// Network names used by the v0.9.3 database schema.
pub(crate) fn network_name(network: &dash_sdk::dpp::dashcore::Network) -> &'static str {
    use dash_sdk::dpp::dashcore::Network;
    match network {
        Network::Mainnet => "dash",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
    }
}

#[derive(Debug)]
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn execute<P: Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        conn.execute(sql, params)
    }
}
