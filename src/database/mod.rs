mod asset_lock_transaction;
pub(crate) mod contacts;
mod contested_names;
pub(crate) mod contracts;
mod dashpay;
mod identities;
mod initialization;
mod proof_log;
mod scheduled_votes;
mod settings;
mod tokens;
mod top_ups;
mod utxo;
mod wallet;

use dash_sdk::dpp::dashcore::Network;
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

    /// Removes all application data tied to a specific Dash network.
    pub fn clear_network_data(&self, network: Network) -> rusqlite::Result<()> {
        let network_str = network.to_string();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;

        // Remove DashPay/contact data referencing identities from this network.
        tx.execute(
            "DELETE FROM dashpay_payments
             WHERE from_identity_id IN (SELECT id FROM identity WHERE network = ?1)
                OR to_identity_id IN (SELECT id FROM identity WHERE network = ?1)",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM dashpay_contact_requests
             WHERE from_identity_id IN (SELECT id FROM identity WHERE network = ?1)
                OR to_identity_id IN (SELECT id FROM identity WHERE network = ?1)",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM dashpay_contacts
             WHERE owner_identity_id IN (SELECT id FROM identity WHERE network = ?1)
                OR contact_identity_id IN (SELECT id FROM identity WHERE network = ?1)",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM contact_private_info
             WHERE owner_identity_id IN (SELECT id FROM identity WHERE network = ?1)
                OR contact_identity_id IN (SELECT id FROM identity WHERE network = ?1)",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM dashpay_profiles
             WHERE identity_id IN (SELECT id FROM identity WHERE network = ?1)",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM identity_token_balances WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM token WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM contract WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM scheduled_votes WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM wallet_transactions WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM utxos WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM asset_lock_transaction WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM contestant WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM contested_name WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM identity WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM wallet WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.commit()
    }
}
