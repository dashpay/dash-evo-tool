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
pub mod shielded;
mod single_key_wallet;
#[cfg(any(test, feature = "testing"))]
pub mod test_helpers;
mod tokens;
mod top_ups;
mod utxo;
mod wallet;
pub use wallet::WalletError;

use dash_sdk::dpp::dashcore::Network;
use rusqlite::{Connection, Params};
use std::sync::{Arc, Mutex};

/// Returns `true` when the error is a UNIQUE or PRIMARY KEY constraint violation.
pub(crate) fn is_unique_constraint_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::ConstraintViolation,
                extended_code: 1555 | 2067, // SQLITE_CONSTRAINT_PRIMARYKEY | SQLITE_CONSTRAINT_UNIQUE
                ..
            },
            _,
        )
    )
}

/// Error indicating a corrupted data blob in the database.
///
/// Converts into `rusqlite::Error::FromSqlConversionFailure` so it can
/// be propagated with `?` from any function returning `rusqlite::Result`.
///
/// When a corrupted blob is encountered, processing stops immediately
/// (fail-fast) rather than skipping the row. This is intentional: identity
/// blobs contain private keys and balance data, so silently ignoring
/// corruption could result in loss of funds.
#[derive(Debug, thiserror::Error)]
#[error("corrupted data detected: {0}")]
pub(crate) struct CorruptedBlobError(pub String);

impl From<CorruptedBlobError> for rusqlite::Error {
    fn from(e: CorruptedBlobError) -> Self {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(e))
    }
}

#[derive(Debug)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn execute<P: Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        conn.execute(sql, params)
    }

    /// Removes all application data tied to a specific Dash network.
    ///
    /// `data_dir` is needed to delete per-wallet commitment tree DB files.
    pub fn clear_network_data(
        &self,
        network: Network,
        data_dir: &std::path::Path,
    ) -> rusqlite::Result<()> {
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

        tx.execute(
            "DELETE FROM single_key_wallet WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        tx.execute(
            "DELETE FROM shielded_notes WHERE network = ?1",
            rusqlite::params![&network_str],
        )?;

        // Collect wallet seed hashes for this network so we can delete their
        // per-wallet commitment tree DB files after the transaction commits.
        let mut wallet_hashes: Vec<Vec<u8>> = Vec::new();
        {
            let mut stmt = tx.prepare("SELECT seed_hash FROM wallet WHERE network = ?1")?;
            let rows = stmt.query_map(rusqlite::params![&network_str], |row| {
                row.get::<_, Vec<u8>>(0)
            })?;
            for row in rows {
                wallet_hashes.push(row?);
            }
        }

        tx.commit()?;

        // Delete per-wallet commitment tree DB files outside the transaction.
        for hash_bytes in wallet_hashes {
            if let Ok(seed_hash) = <[u8; 32]>::try_from(hash_bytes.as_slice()) {
                let _ = shielded::delete_commitment_tree_db(data_dir, &seed_hash);
            }
        }

        Ok(())
    }
}
