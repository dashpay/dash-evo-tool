mod asset_lock_transaction;
mod contacts;
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
