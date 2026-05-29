mod asset_lock_transaction;
mod initialization;
mod settings;
pub mod shielded;
mod single_key_wallet;
#[cfg(any(test, feature = "testing"))]
pub mod test_helpers;
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
    /// The on-disk DB file path (`None` for in-memory test DBs). Currently
    /// only used by test fixtures that re-open the same file after a drop.
    #[allow(dead_code)]
    path: Option<std::path::PathBuf>,
}

impl Database {
    pub fn new<P: AsRef<std::path::Path>>(path: P) -> rusqlite::Result<Self> {
        let path_ref = path.as_ref();
        let conn = Connection::open(path_ref)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path: Some(path_ref.to_path_buf()),
        })
    }

    /// On-disk DB file path, if this is a file-backed database.
    #[allow(dead_code)]
    pub(crate) fn db_file_path(&self) -> Option<std::path::PathBuf> {
        self.path.clone()
    }

    #[cfg(test)]
    pub(crate) fn shared_connection(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }

    pub fn execute<P: Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        conn.execute(sql, params)
    }

    /// Removes all application data tied to a specific Dash network.
    pub fn clear_network_data(&self, network: Network) -> rusqlite::Result<()> {
        let network_str = network.to_string();

        {
            let mut conn = self.conn.lock().unwrap();
            let tx = conn.transaction()?;

            // DashPay tables (dashpay_profiles, dashpay_contacts,
            // dashpay_contact_requests, dashpay_payments,
            // dashpay_contact_address_indices, dashpay_address_mappings)
            // and contact_private_info were retired in D4d — the upstream
            // ManagedIdentity owns contact/profile/payment state and a
            // per-network k/v sidecar owns DET-only overlays (private
            // memo, blocked/rejected markers, timestamps, address index,
            // address mapping). The sidecar sweep lives in
            // `AppContext::clear_network_database` because the k/v
            // adapter is not reachable from `Database`.
            //
            // token / identity_token_balances / identity tables are no
            // longer managed (C7) — token registry, per-identity balances
            // and identity records all live in the per-network k/v store.
            // Fresh installs do not create the tables; legacy installs
            // keep the rows dormant.
            //
            // contract / contestant / contested_name tables are no longer
            // managed (C6). Fresh installs do not have them; legacy
            // installs keep the rows dormant.

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

            tx.commit()?;
        } // conn lock released here

        // Shielded commitment-tree data now lives in a per-network sidecar
        // SQLite file under `<spv_dir>/<network>/`, not in `data.db`. The
        // `AppContext::clear_network_database` caller unlinks that file
        // after this method returns successfully.

        Ok(())
    }
}
