use crate::database::Database;
use crate::model::wallet::WalletSeedHash;
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

/// Return the path to a wallet's dedicated commitment tree SQLite database.
///
/// Each wallet gets its own file under `<data_dir>/shielded/` so that
/// commitment trees are fully isolated between wallets.
pub fn commitment_tree_db_path(data_dir: &Path, seed_hash: &WalletSeedHash) -> PathBuf {
    let hex = hex::encode(seed_hash.as_slice());
    data_dir.join("shielded").join(format!("{hex}.db"))
}

/// Open (or create) the per-wallet commitment tree SQLite database.
///
/// Creates the `<data_dir>/shielded/` directory if it does not exist.
pub fn open_commitment_tree_connection(
    data_dir: &Path,
    seed_hash: &WalletSeedHash,
) -> Result<rusqlite::Connection, rusqlite::Error> {
    let db_path = commitment_tree_db_path(data_dir, seed_hash);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!(
                "Failed to create shielded DB directory: {e}"
            ))
        })?;
    }
    rusqlite::Connection::open(&db_path)
}

/// Delete a wallet's dedicated commitment tree database file.
///
/// Returns `Ok(true)` if a file was removed, `Ok(false)` if it did not exist.
pub fn delete_commitment_tree_db(
    data_dir: &Path,
    seed_hash: &WalletSeedHash,
) -> Result<bool, std::io::Error> {
    let db_path = commitment_tree_db_path(data_dir, seed_hash);
    match std::fs::remove_file(&db_path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

impl Database {
    /// Create shielded pool tables (v28 migration).
    pub(crate) fn create_shielded_tables(&self, conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS shielded_notes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                wallet_seed_hash BLOB NOT NULL,
                note_data BLOB NOT NULL,
                position INTEGER NOT NULL,
                cmx BLOB NOT NULL,
                nullifier BLOB NOT NULL,
                block_height INTEGER NOT NULL,
                is_spent INTEGER NOT NULL DEFAULT 0,
                value INTEGER NOT NULL,
                network TEXT NOT NULL,
                UNIQUE(wallet_seed_hash, nullifier, network),
                FOREIGN KEY (wallet_seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_shielded_notes_wallet_network
             ON shielded_notes (wallet_seed_hash, network)",
            [],
        )?;

        Ok(())
    }

    /// Insert a shielded note into the database.
    pub fn insert_shielded_note(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        note: &InsertShieldedNote<'_>,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO shielded_notes
             (wallet_seed_hash, note_data, position, cmx, nullifier, block_height, value, network)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                wallet_seed_hash.as_slice(),
                note.note_data,
                note.position as i64,
                note.cmx.as_slice(),
                note.nullifier.as_slice(),
                note.block_height as i64,
                note.value as i64,
                note.network,
            ],
        )?;
        Ok(())
    }

    /// Get all unspent shielded notes for a wallet on a given network.
    pub fn get_unspent_shielded_notes(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> rusqlite::Result<Vec<ShieldedNoteRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, note_data, position, cmx, nullifier, block_height, value
             FROM shielded_notes
             WHERE wallet_seed_hash = ?1 AND network = ?2 AND is_spent = 0
             ORDER BY position ASC",
        )?;

        let rows = stmt.query_map(params![wallet_seed_hash.as_slice(), network], |row| {
            Ok(ShieldedNoteRow {
                id: row.get(0)?,
                note_data: row.get(1)?,
                position: row.get::<_, i64>(2)? as u64,
                cmx: {
                    let bytes: Vec<u8> = row.get(3)?;
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    arr
                },
                nullifier: {
                    let bytes: Vec<u8> = row.get(4)?;
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    arr
                },
                block_height: row.get::<_, i64>(5)? as u64,
                value: row.get::<_, i64>(6)? as u64,
            })
        })?;

        rows.collect()
    }

    /// Get all shielded notes (spent and unspent) for a wallet on a given network.
    pub fn get_all_shielded_notes(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> rusqlite::Result<Vec<ShieldedNoteRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, note_data, position, cmx, nullifier, block_height, value
             FROM shielded_notes
             WHERE wallet_seed_hash = ?1 AND network = ?2
             ORDER BY position ASC",
        )?;

        let rows = stmt.query_map(params![wallet_seed_hash.as_slice(), network], |row| {
            Ok(ShieldedNoteRow {
                id: row.get(0)?,
                note_data: row.get(1)?,
                position: row.get::<_, i64>(2)? as u64,
                cmx: {
                    let bytes: Vec<u8> = row.get(3)?;
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    arr
                },
                nullifier: {
                    let bytes: Vec<u8> = row.get(4)?;
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&bytes);
                    arr
                },
                block_height: row.get::<_, i64>(5)? as u64,
                value: row.get::<_, i64>(6)? as u64,
            })
        })?;

        rows.collect()
    }

    /// Mark a shielded note as spent by its nullifier.
    pub fn mark_shielded_note_spent(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        nullifier: &[u8; 32],
        network: &str,
    ) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE shielded_notes SET is_spent = 1
             WHERE wallet_seed_hash = ?1 AND nullifier = ?2 AND network = ?3",
            params![wallet_seed_hash.as_slice(), nullifier.as_slice(), network],
        )
    }

    /// Delete all shielded notes for a wallet (used by resync).
    pub fn delete_shielded_notes(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM shielded_notes WHERE wallet_seed_hash = ?1 AND network = ?2",
            params![wallet_seed_hash.as_slice(), network],
        )
    }

    /// Clear a wallet's commitment tree data by deleting its dedicated DB file.
    ///
    /// Each wallet stores its `ClientPersistentCommitmentTree` in a separate
    /// SQLite file under `<data_dir>/shielded/<hex>.db`. This removes that file
    /// so a fresh tree can be opened on next initialization.
    pub fn clear_commitment_tree_for_wallet(
        data_dir: &Path,
        seed_hash: &WalletSeedHash,
    ) -> rusqlite::Result<()> {
        tracing::warn!(
            "Clearing commitment tree for wallet {}",
            hex::encode(seed_hash.as_slice()),
        );
        delete_commitment_tree_db(data_dir, seed_hash).map_err(|e| {
            rusqlite::Error::InvalidParameterName(format!(
                "Failed to delete commitment tree DB: {e}"
            ))
        })?;
        Ok(())
    }

    /// Create the shielded_wallet_meta table (v29 migration).
    pub(crate) fn create_shielded_wallet_meta_table(
        &self,
        conn: &Connection,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS shielded_wallet_meta (
                wallet_seed_hash BLOB NOT NULL,
                network TEXT NOT NULL,
                last_nullifier_sync_height INTEGER NOT NULL DEFAULT 0,
                last_nullifier_sync_timestamp INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (wallet_seed_hash, network),
                FOREIGN KEY (wallet_seed_hash) REFERENCES wallet(seed_hash) ON DELETE CASCADE
            )",
            [],
        )?;
        Ok(())
    }

    /// Migration: Add last_nullifier_sync_timestamp column (v30).
    pub(crate) fn add_nullifier_sync_timestamp_column(
        &self,
        conn: &Connection,
    ) -> rusqlite::Result<()> {
        let table_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='shielded_wallet_meta'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if table_exists {
            let has_column: bool = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('shielded_wallet_meta') WHERE name='last_nullifier_sync_timestamp'",
                [],
                |row| row.get::<_, i32>(0).map(|count| count > 0),
            )?;

            if !has_column {
                conn.execute(
                    "ALTER TABLE shielded_wallet_meta ADD COLUMN last_nullifier_sync_timestamp INTEGER NOT NULL DEFAULT 0",
                    [],
                )?;
            }
        }

        Ok(())
    }

    /// Get the last nullifier sync height and timestamp for a wallet on a given network.
    pub fn get_nullifier_sync_info(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> Result<(u64, u64), String> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT last_nullifier_sync_height, last_nullifier_sync_timestamp FROM shielded_wallet_meta
             WHERE wallet_seed_hash = ?1 AND network = ?2",
            params![wallet_seed_hash.as_slice(), network],
            |row| {
                let height: i64 = row.get(0)?;
                let timestamp: i64 = row.get(1)?;
                Ok((height as u64, timestamp as u64))
            },
        );
        match result {
            Ok(info) => Ok(info),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok((0, 0)),
            Err(e) => Err(format!("Failed to get nullifier sync info: {e}")),
        }
    }

    /// Set the last nullifier sync height and timestamp for a wallet on a given network.
    pub fn set_nullifier_sync_info(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
        height: u64,
        timestamp: u64,
    ) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO shielded_wallet_meta
             (wallet_seed_hash, network, last_nullifier_sync_height, last_nullifier_sync_timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                wallet_seed_hash.as_slice(),
                network,
                height as i64,
                timestamp as i64
            ],
        )
        .map_err(|e| format!("Failed to set nullifier sync info: {e}"))?;
        Ok(())
    }

    /// Get total shielded balance (sum of unspent note values) for a wallet.
    pub fn get_shielded_balance(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> rusqlite::Result<u64> {
        let conn = self.conn.lock().unwrap();
        let result: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(value), 0) FROM shielded_notes
             WHERE wallet_seed_hash = ?1 AND network = ?2 AND is_spent = 0",
                params![wallet_seed_hash.as_slice(), network],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(result as u64)
    }
}

/// Parameters for inserting a shielded note.
pub struct InsertShieldedNote<'a> {
    pub note_data: &'a [u8],
    pub position: u64,
    pub cmx: &'a [u8; 32],
    pub nullifier: &'a [u8; 32],
    pub block_height: u64,
    pub value: u64,
    pub network: &'a str,
}

/// Row data for a shielded note from the database.
pub struct ShieldedNoteRow {
    pub id: i64,
    pub note_data: Vec<u8>,
    pub position: u64,
    pub cmx: [u8; 32],
    pub nullifier: [u8; 32],
    pub block_height: u64,
    pub value: u64,
}
