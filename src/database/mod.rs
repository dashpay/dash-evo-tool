mod initialization;
pub(crate) mod legacy_import;
mod settings;
mod single_key_wallet;
#[cfg(any(test, feature = "testing"))]
pub mod test_helpers;
mod utxo;
mod wallet;
pub use wallet::WalletError;

use dash_sdk::dpp::dashcore::Network;
use rusqlite::{Connection, Params};
use std::sync::{Arc, Mutex};

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
    /// The on-disk DB file path (`None` for in-memory test DBs). Read back by
    /// the migration tasks that re-open the same file.
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
    pub(crate) fn db_file_path(&self) -> Option<std::path::PathBuf> {
        self.path.clone()
    }

    /// Lock the connection mutex, recovering a poisoned guard instead of
    /// panicking.
    ///
    /// The mutex is poisoned only when a thread panics while holding the DB
    /// lock. A `rusqlite::Connection` is a plain handle with no cross-call
    /// invariant that a panic can break — SQLite manages statement/transaction
    /// state internally — so the guard is safe to recover. Panicking here would
    /// turn one unrelated panic into a permanent, cascading failure of every
    /// subsequent database call, so recovery (matching the wallet-backend
    /// poison discipline for rebuildable state) is the correct behavior.
    pub(crate) fn locked_conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn execute<P: Params>(&self, sql: &str, params: P) -> rusqlite::Result<usize> {
        let conn = self.locked_conn();
        conn.execute(sql, params)
    }

    /// Removes all application data tied to a specific Dash network.
    pub fn clear_network_data(&self, network: Network) -> rusqlite::Result<()> {
        let network_str = network.to_string();

        {
            let mut conn = self.locked_conn();
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

            // The legacy wallet-family tables are gated out of the fresh
            // schema (they live in the upstream persistor now), so each
            // DELETE is existence-guarded — a fresh install has none of them
            // and an unguarded DELETE would error on the first missing table,
            // aborting the whole clear. `asset_lock_transaction` is omitted
            // entirely: its module was deleted and the migration tool drains
            // it via git history.
            for table in [
                "wallet_transactions",
                "utxos",
                "wallet",
                "single_key_wallet",
                "shielded_notes",
            ] {
                if self.table_exists(&tx, table)? {
                    tx.execute(
                        &format!("DELETE FROM {table} WHERE network = ?1"),
                        rusqlite::params![&network_str],
                    )?;
                }
            }

            tx.commit()?;
        } // conn lock released here

        // Shielded commitment-tree data now lives in a per-network sidecar
        // SQLite file under `<spv_dir>/<network>/`, not in `data.db`. The
        // `AppContext::clear_network_database` caller unlinks that file
        // after this method returns successfully.

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// "Clear database" must succeed on a truly-fresh install. The legacy
    /// wallet-family tables (`wallet_transactions`, `utxos`, `wallet`,
    /// `single_key_wallet`, `shielded_notes`) are gated out of the fresh
    /// schema, so `clear_network_data` previously errored on the first
    /// `DELETE FROM` of a missing table and committed nothing. Each DELETE is
    /// now guarded by an existence check, so a fresh install clears cleanly.
    #[test]
    fn clear_network_data_succeeds_on_fresh_install() {
        use dash_sdk::dpp::dashcore::Network;

        let temp_dir = tempfile::tempdir().unwrap();
        let db_file = temp_dir.path().join("test_data.db");
        let db = super::Database::new(&db_file).unwrap();
        db.initialize(&db_file).unwrap();

        // Precondition: the fresh schema must not carry the legacy wallet
        // table, which is what made the unguarded DELETE fail.
        {
            let conn = db.conn.lock().unwrap();
            let wallet_exists: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='wallet')",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(
                !wallet_exists,
                "precondition: fresh install must not create the legacy `wallet` table"
            );
        }

        db.clear_network_data(Network::Testnet)
            .expect("clear_network_data must succeed on a fresh install");
    }
}
