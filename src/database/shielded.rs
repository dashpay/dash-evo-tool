use crate::database::Database;
use crate::model::wallet::WalletSeedHash;
use rusqlite::{Connection, params};

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
                UNIQUE(wallet_seed_hash, nullifier, network)
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_shielded_notes_wallet_network
             ON shielded_notes (wallet_seed_hash, network)",
            [],
        )?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS shielded_tree_state (
                wallet_seed_hash BLOB NOT NULL,
                network TEXT NOT NULL,
                tree_data BLOB NOT NULL,
                last_synced_index INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (wallet_seed_hash, network)
            )",
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

    /// Save the serialized commitment tree state for a wallet.
    pub fn save_shielded_tree_state(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        tree_data: &[u8],
        last_synced_index: u64,
        network: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO shielded_tree_state (wallet_seed_hash, network, tree_data, last_synced_index)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(wallet_seed_hash, network)
             DO UPDATE SET tree_data = excluded.tree_data, last_synced_index = excluded.last_synced_index",
            params![
                wallet_seed_hash.as_slice(),
                network,
                tree_data,
                last_synced_index as i64,
            ],
        )?;
        Ok(())
    }

    /// Load the commitment tree state for a wallet.
    pub fn load_shielded_tree_state(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> rusqlite::Result<Option<(Vec<u8>, u64)>> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT tree_data, last_synced_index FROM shielded_tree_state
             WHERE wallet_seed_hash = ?1 AND network = ?2",
            params![wallet_seed_hash.as_slice(), network],
            |row| {
                let tree_data: Vec<u8> = row.get(0)?;
                let last_synced_index: i64 = row.get(1)?;
                Ok((tree_data, last_synced_index as u64))
            },
        );

        match result {
            Ok(data) => Ok(Some(data)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
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
