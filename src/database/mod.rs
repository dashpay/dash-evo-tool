mod initialization;
#[cfg(test)]
pub(crate) use initialization::DEFAULT_DB_VERSION;
pub(crate) mod legacy_import;
mod settings;
mod single_key_wallet;
#[cfg(any(test, feature = "testing"))]
pub mod test_helpers;
mod utxo;
mod wallet;
pub use wallet::WalletError;

use dash_sdk::dpp::dashcore::Network;
use rusqlite::{Connection, OpenFlags, Params};
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

/// Whether `table` exists in the SQLite schema at `conn`.
///
/// The one schema-existence probe: legacy `data.db` readers, the migration
/// ladder, and the migration tasks all run against tables that a fresh install
/// never creates, so "missing" is a normal answer, not an error. Callers that
/// need a domain-typed error map the `rusqlite::Error` themselves.
pub(crate) fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
}

/// Whether `table` has a column named `column`.
///
/// A missing table has no columns, so it yields `false` rather than an error —
/// the migration ladder relies on that to stay idempotent.
pub(crate) fn column_exists(
    conn: &Connection,
    table: &str,
    column: &str,
) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
        rusqlite::params![table, column],
        |row| row.get::<_, i64>(0).map(|count| count > 0),
    )
}

/// Open a pre-update `data.db` as a bare read-only connection.
///
/// The one place any reader opens the legacy file, so "SQLite itself refuses
/// the write" is a property of the file handle rather than of each caller's
/// discipline. [`Database::open_legacy_read_only`] wraps the same connection
/// for callers that want the pooled handle instead.
pub(crate) fn open_legacy_connection_read_only(
    path: &std::path::Path,
) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
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

    /// Open an existing pre-update database with SQLite write operations
    /// disabled. The storage update treats this file as a recovery artifact;
    /// all current state is written to the dedicated store and vault files.
    pub(crate) fn open_legacy_read_only<P: AsRef<std::path::Path>>(
        path: P,
    ) -> rusqlite::Result<Self> {
        let path_ref = path.as_ref();
        let conn = open_legacy_connection_read_only(path_ref)?;
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

    /// Legacy-database writer retained only for isolated compatibility tests.
    ///
    /// Production opens a pre-update `data.db` read-only and must never call
    /// this method; current network data is cleared through its owning stores.
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
