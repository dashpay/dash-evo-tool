//! Per-network shielded sidecar (T-SH-01).
//!
//! [`ShieldedView`] wraps a private SQLite file at
//! `<data_dir>/spv/<network>/det-shielded.sqlite` that mirrors the shielded
//! tables in `src/database/shielded.rs` byte-for-byte. The schema is
//! intentionally identical so T-SH-02 can migrate legacy rows with a plain
//! `ATTACH` + `INSERT OR REPLACE`, and so callers swapped over in T-SH-03
//! see no behavioural change.
//!
//! Lazy materialization (FR-3.3): the file is opened-and-created only on
//! the first **write** (`insert_shielded_note`, `mark_shielded_note_spent`,
//! `delete_shielded_notes`, `set_nullifier_sync_info`). Reads against an
//! absent sidecar return the empty answer — no notes, zero balance, zero
//! sync height — so a user with no shielded activity never gets a stray
//! sidecar on disk.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use crate::model::wallet::WalletSeedHash;

/// Sidecar filename used under every `<data_dir>/spv/<network>/` directory.
pub const SHIELDED_SIDECAR_FILE: &str = "det-shielded.sqlite";

/// Per-network shielded-notes sidecar. One instance per [`WalletBackend`]
/// (so one per network) — the path is fixed at construction time and the
/// connection is materialised lazily on first write.
///
/// All methods mirror their `Database::*_shielded_*` counterparts so the
/// migration in T-SH-02 only has to copy rows, not translate them. The view
/// is `Send + Sync` via the inner [`Mutex`].
pub struct ShieldedView {
    path: PathBuf,
    /// `None` until the first write materialises the file + schema.
    conn: Mutex<Option<Connection>>,
}

impl ShieldedView {
    /// Resolve the sidecar path for a given SPV directory.
    pub fn sidecar_path(spv_dir: &Path) -> PathBuf {
        spv_dir.join(SHIELDED_SIDECAR_FILE)
    }

    /// Construct a view rooted at `spv_dir`. Does not touch the filesystem.
    pub fn new(spv_dir: &Path) -> Self {
        Self {
            path: Self::sidecar_path(spv_dir),
            conn: Mutex::new(None),
        }
    }

    /// Path of the sidecar file. Exists on disk only after the first write.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Force the sidecar file (and its schema) into existence without
    /// writing any business rows. Idempotent. Used by the T-SH-02
    /// migrator so the legacy `data.db` can `ATTACH` the sidecar in a
    /// known-good state before bulk-copying rows.
    pub fn ensure_materialized(&self) -> rusqlite::Result<()> {
        self.open_or_create()
    }

    /// Open (creating if needed) and stash the connection. Idempotent.
    fn open_or_create(&self) -> rusqlite::Result<()> {
        let mut guard = self.conn.lock().expect("shielded sidecar mutex poisoned");
        if guard.is_some() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                    Some(e.to_string()),
                )
            })?;
        }
        let conn = Connection::open(&self.path)?;
        Self::create_schema(&conn)?;
        *guard = Some(conn);
        Ok(())
    }

    /// Open the existing file in read mode. Returns `Ok(None)` when the
    /// sidecar has not been materialised yet — the lazy-provisioning
    /// contract: readers never create the file.
    fn open_existing(&self) -> rusqlite::Result<bool> {
        let mut guard = self.conn.lock().expect("shielded sidecar mutex poisoned");
        if guard.is_some() {
            return Ok(true);
        }
        if !self.path.exists() {
            return Ok(false);
        }
        let conn = Connection::open(&self.path)?;
        // Defensive: a half-created file from a crashed write would lack
        // the schema; create it idempotently so reads do not error.
        Self::create_schema(&conn)?;
        *guard = Some(conn);
        Ok(true)
    }

    /// Schema mirrors `Database::create_shielded_tables` /
    /// `create_shielded_wallet_meta_table` 1:1 so T-SH-02 can copy rows
    /// across with a flat `INSERT OR REPLACE`.
    fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
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
            "CREATE TABLE IF NOT EXISTS shielded_wallet_meta (
                wallet_seed_hash BLOB NOT NULL,
                network TEXT NOT NULL,
                last_nullifier_sync_height INTEGER NOT NULL DEFAULT 0,
                last_nullifier_sync_timestamp INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (wallet_seed_hash, network)
            )",
            [],
        )?;
        Ok(())
    }

    /// Insert a shielded note. Materialises the sidecar on first call.
    pub fn insert_shielded_note(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        note: &InsertShieldedNote<'_>,
    ) -> rusqlite::Result<()> {
        self.open_or_create()?;
        let guard = self.conn.lock().expect("shielded sidecar mutex poisoned");
        let conn = guard.as_ref().expect("opened above");
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

    /// All unspent shielded notes for a wallet on `network`.
    pub fn get_unspent_shielded_notes(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> rusqlite::Result<Vec<ShieldedNoteRow>> {
        if !self.open_existing()? {
            return Ok(Vec::new());
        }
        let guard = self.conn.lock().expect("shielded sidecar mutex poisoned");
        let conn = guard.as_ref().expect("opened above");
        let mut stmt = conn.prepare(
            "SELECT id, note_data, position, cmx, nullifier, block_height, value
             FROM shielded_notes
             WHERE wallet_seed_hash = ?1 AND network = ?2 AND is_spent = 0
             ORDER BY position ASC",
        )?;
        let rows = stmt.query_map(params![wallet_seed_hash.as_slice(), network], map_note_row)?;
        rows.collect()
    }

    /// All shielded notes (spent and unspent) for a wallet on `network`.
    pub fn get_all_shielded_notes(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> rusqlite::Result<Vec<ShieldedNoteRow>> {
        if !self.open_existing()? {
            return Ok(Vec::new());
        }
        let guard = self.conn.lock().expect("shielded sidecar mutex poisoned");
        let conn = guard.as_ref().expect("opened above");
        let mut stmt = conn.prepare(
            "SELECT id, note_data, position, cmx, nullifier, block_height, value
             FROM shielded_notes
             WHERE wallet_seed_hash = ?1 AND network = ?2
             ORDER BY position ASC",
        )?;
        let rows = stmt.query_map(params![wallet_seed_hash.as_slice(), network], map_note_row)?;
        rows.collect()
    }

    /// Mark a note as spent by its nullifier. Materialises the sidecar so
    /// the spend is durable even if the matching INSERT happened in a
    /// foreign process / migration before the first DET-side insert.
    pub fn mark_shielded_note_spent(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        nullifier: &[u8; 32],
        network: &str,
    ) -> rusqlite::Result<usize> {
        self.open_or_create()?;
        let guard = self.conn.lock().expect("shielded sidecar mutex poisoned");
        let conn = guard.as_ref().expect("opened above");
        conn.execute(
            "UPDATE shielded_notes SET is_spent = 1
             WHERE wallet_seed_hash = ?1 AND nullifier = ?2 AND network = ?3",
            params![wallet_seed_hash.as_slice(), nullifier.as_slice(), network],
        )
    }

    /// Delete all notes for a wallet on `network` (resync path).
    pub fn delete_shielded_notes(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> rusqlite::Result<usize> {
        if !self.open_existing()? {
            return Ok(0);
        }
        let guard = self.conn.lock().expect("shielded sidecar mutex poisoned");
        let conn = guard.as_ref().expect("opened above");
        conn.execute(
            "DELETE FROM shielded_notes WHERE wallet_seed_hash = ?1 AND network = ?2",
            params![wallet_seed_hash.as_slice(), network],
        )
    }

    /// Clear the nullifier-sync cursor for a wallet on `network`.
    ///
    /// Removes the wallet's `shielded_wallet_meta` row so a subsequent
    /// [`Self::get_nullifier_sync_info`] returns the zero cursor. Used when a
    /// wallet is forgotten: leaving the cursor behind would make a
    /// same-seed-hash re-import skip already-spent scan windows. A missing
    /// sidecar is a no-op.
    pub fn delete_shielded_wallet_meta(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> rusqlite::Result<usize> {
        if !self.open_existing()? {
            return Ok(0);
        }
        let guard = self.conn.lock().expect("shielded sidecar mutex poisoned");
        let conn = guard.as_ref().expect("opened above");
        conn.execute(
            "DELETE FROM shielded_wallet_meta WHERE wallet_seed_hash = ?1 AND network = ?2",
            params![wallet_seed_hash.as_slice(), network],
        )
    }

    /// Last nullifier sync `(height, unix_timestamp)`. Zeroes when the
    /// sidecar or the row is absent — same contract as the legacy table.
    pub fn get_nullifier_sync_info(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> rusqlite::Result<(u64, u64)> {
        if !self.open_existing()? {
            return Ok((0, 0));
        }
        let guard = self.conn.lock().expect("shielded sidecar mutex poisoned");
        let conn = guard.as_ref().expect("opened above");
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT last_nullifier_sync_height, last_nullifier_sync_timestamp
                 FROM shielded_wallet_meta
                 WHERE wallet_seed_hash = ?1 AND network = ?2",
                params![wallet_seed_hash.as_slice(), network],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(row.map(|(h, t)| (h as u64, t as u64)).unwrap_or((0, 0)))
    }

    /// Set the last nullifier sync `(height, unix_timestamp)`. Upserts.
    pub fn set_nullifier_sync_info(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
        height: u64,
        timestamp: u64,
    ) -> rusqlite::Result<()> {
        self.open_or_create()?;
        let guard = self.conn.lock().expect("shielded sidecar mutex poisoned");
        let conn = guard.as_ref().expect("opened above");
        conn.execute(
            "INSERT OR REPLACE INTO shielded_wallet_meta
             (wallet_seed_hash, network, last_nullifier_sync_height, last_nullifier_sync_timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                wallet_seed_hash.as_slice(),
                network,
                height as i64,
                timestamp as i64,
            ],
        )?;
        Ok(())
    }

    /// Total unspent value for a wallet on `network`.
    pub fn get_shielded_balance(
        &self,
        wallet_seed_hash: &WalletSeedHash,
        network: &str,
    ) -> rusqlite::Result<u64> {
        if !self.open_existing()? {
            return Ok(0);
        }
        let guard = self.conn.lock().expect("shielded sidecar mutex poisoned");
        let conn = guard.as_ref().expect("opened above");
        let sum: i64 = conn.query_row(
            "SELECT COALESCE(SUM(value), 0) FROM shielded_notes
             WHERE wallet_seed_hash = ?1 AND network = ?2 AND is_spent = 0",
            params![wallet_seed_hash.as_slice(), network],
            |row| row.get(0),
        )?;
        Ok(sum.max(0) as u64)
    }
}

fn map_note_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ShieldedNoteRow> {
    let cmx_bytes: Vec<u8> = row.get(3)?;
    let nullifier_bytes: Vec<u8> = row.get(4)?;
    let mut cmx = [0u8; 32];
    let mut nullifier = [0u8; 32];
    cmx.copy_from_slice(&cmx_bytes);
    nullifier.copy_from_slice(&nullifier_bytes);
    Ok(ShieldedNoteRow {
        id: row.get(0)?,
        note_data: row.get(1)?,
        position: row.get::<_, i64>(2)? as u64,
        cmx,
        nullifier,
        block_height: row.get::<_, i64>(5)? as u64,
        value: row.get::<_, i64>(6)? as u64,
    })
}

/// Parameters for inserting a shielded note. Mirrors the legacy
/// `crate::database::shielded::InsertShieldedNote`.
pub struct InsertShieldedNote<'a> {
    pub note_data: &'a [u8],
    pub position: u64,
    pub cmx: &'a [u8; 32],
    pub nullifier: &'a [u8; 32],
    pub block_height: u64,
    pub value: u64,
    pub network: &'a str,
}

/// Row data for a shielded note. Mirrors the legacy
/// `crate::database::shielded::ShieldedNoteRow`.
pub struct ShieldedNoteRow {
    pub id: i64,
    pub note_data: Vec<u8>,
    pub position: u64,
    pub cmx: [u8; 32],
    pub nullifier: [u8; 32],
    pub block_height: u64,
    pub value: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_view(dir: &Path) -> ShieldedView {
        // Mirrors the layout WalletBackend uses: an `spv/<network>/` dir.
        let spv_dir = dir.join("spv").join("testnet");
        std::fs::create_dir_all(&spv_dir).expect("create spv dir");
        ShieldedView::new(&spv_dir)
    }

    fn sample_note(value: u64, position: u64, nullifier_seed: u8) -> InsertOwnedNote {
        InsertOwnedNote {
            note_data: vec![0xAA; 16],
            position,
            cmx: [position as u8; 32],
            nullifier: [nullifier_seed; 32],
            block_height: 100 + position,
            value,
        }
    }

    /// Owned counterpart to [`InsertShieldedNote`] so tests can pass refs
    /// without lifetime gymnastics. Schema-equivalent.
    struct InsertOwnedNote {
        note_data: Vec<u8>,
        position: u64,
        cmx: [u8; 32],
        nullifier: [u8; 32],
        block_height: u64,
        value: u64,
    }

    impl InsertOwnedNote {
        fn as_param<'a>(&'a self, network: &'a str) -> InsertShieldedNote<'a> {
            InsertShieldedNote {
                note_data: &self.note_data,
                position: self.position,
                cmx: &self.cmx,
                nullifier: &self.nullifier,
                block_height: self.block_height,
                value: self.value,
                network,
            }
        }
    }

    /// TC-SH-009: lazy provisioning. The sidecar file MUST NOT exist
    /// until the first write, and reads on a virgin view return the
    /// empty answer (no notes, zero balance, zero sync height).
    #[test]
    fn tc_sh_009_lazy_provisioning_no_file_until_first_write() {
        let tmp = tempdir().expect("tempdir");
        let view = make_view(tmp.path());

        // Path is computed, but nothing is on disk yet.
        assert!(
            !view.path().exists(),
            "sidecar file must not exist before first write: {}",
            view.path().display()
        );

        let seed: WalletSeedHash = [0x11; 32];

        // Reads return the empty answer and DO NOT create the file.
        let notes = view
            .get_unspent_shielded_notes(&seed, "testnet")
            .expect("read unspent");
        assert!(notes.is_empty(), "no notes on a virgin sidecar");
        let all = view
            .get_all_shielded_notes(&seed, "testnet")
            .expect("read all");
        assert!(all.is_empty(), "no notes on a virgin sidecar");
        let bal = view
            .get_shielded_balance(&seed, "testnet")
            .expect("read balance");
        assert_eq!(bal, 0, "zero balance on a virgin sidecar");
        let info = view
            .get_nullifier_sync_info(&seed, "testnet")
            .expect("read sync info");
        assert_eq!(info, (0, 0), "zero sync info on a virgin sidecar");

        assert!(
            !view.path().exists(),
            "reads must not materialise the sidecar (FR-3.3): {}",
            view.path().display()
        );

        // First write materialises the file.
        let owned = sample_note(7, 0, 0x01);
        view.insert_shielded_note(&seed, &owned.as_param("testnet"))
            .expect("first insert");
        assert!(
            view.path().exists(),
            "first write must materialise the sidecar: {}",
            view.path().display()
        );
    }

    /// TC-SH-003: schema parity with `database/shielded.rs`. The sidecar
    /// must hold the same two tables with the same column names, types,
    /// PK/UNIQUE constraints, and index — so T-SH-02 can migrate legacy
    /// rows with a plain `ATTACH` + `INSERT OR REPLACE`. Also covers a
    /// full round-trip on every public method.
    #[test]
    fn tc_sh_003_schema_parity_and_roundtrip() {
        let tmp = tempdir().expect("tempdir");
        let view = make_view(tmp.path());
        let seed: WalletSeedHash = [0x22; 32];

        // Insert two notes + a sync info row to force materialisation.
        let n1 = sample_note(10, 0, 0xA1);
        let n2 = sample_note(15, 1, 0xA2);
        view.insert_shielded_note(&seed, &n1.as_param("testnet"))
            .expect("insert n1");
        view.insert_shielded_note(&seed, &n2.as_param("testnet"))
            .expect("insert n2");
        view.set_nullifier_sync_info(&seed, "testnet", 12345, 67890)
            .expect("set sync info");

        // Inspect the on-disk schema directly through a second connection
        // so the assertions hold regardless of the view's caching.
        let conn = Connection::open(view.path()).expect("open sidecar");

        // Expected columns for shielded_notes (name, type, notnull, pk).
        let expected_notes_cols: &[(&str, &str, i32, i32)] = &[
            ("id", "INTEGER", 0, 1),
            ("wallet_seed_hash", "BLOB", 1, 0),
            ("note_data", "BLOB", 1, 0),
            ("position", "INTEGER", 1, 0),
            ("cmx", "BLOB", 1, 0),
            ("nullifier", "BLOB", 1, 0),
            ("block_height", "INTEGER", 1, 0),
            ("is_spent", "INTEGER", 1, 0),
            ("value", "INTEGER", 1, 0),
            ("network", "TEXT", 1, 0),
        ];
        let mut stmt = conn
            .prepare("PRAGMA table_info(shielded_notes)")
            .expect("pragma notes");
        let actual: Vec<(String, String, i32, i32)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i32>(3)?,
                    row.get::<_, i32>(5)?,
                ))
            })
            .expect("query notes pragma")
            .collect::<rusqlite::Result<_>>()
            .expect("collect notes pragma");
        assert_eq!(
            actual.len(),
            expected_notes_cols.len(),
            "shielded_notes column count must match legacy schema"
        );
        for (got, want) in actual.iter().zip(expected_notes_cols) {
            assert_eq!(got.0, want.0, "column name");
            assert_eq!(got.1, want.1, "column type for {}", got.0);
            assert_eq!(got.2, want.2, "notnull flag for {}", got.0);
            assert_eq!(got.3, want.3, "pk flag for {}", got.0);
        }

        // UNIQUE(wallet_seed_hash, nullifier, network) must exist. SQLite
        // auto-names the implicit index `sqlite_autoindex_<table>_N`, so we
        // probe via PRAGMAs rather than parsing the `sqlite_master.sql`.
        let unique_indexes: Vec<String> = {
            let mut stmt = conn
                .prepare("PRAGMA index_list('shielded_notes')")
                .expect("index_list");
            stmt.query_map([], |row| {
                let name: String = row.get(1)?;
                let unique: i64 = row.get(2)?;
                Ok((name, unique == 1))
            })
            .expect("query index_list")
            .filter_map(|r| r.ok().filter(|(_, u)| *u).map(|(n, _)| n))
            .collect()
        };
        let mut found_unique_triple = false;
        for idx_name in &unique_indexes {
            let mut stmt = conn
                .prepare(&format!("PRAGMA index_info('{idx_name}')"))
                .expect("index_info");
            let cols: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(2))
                .expect("query index_info")
                .collect::<rusqlite::Result<_>>()
                .expect("collect index_info");
            if cols == ["wallet_seed_hash", "nullifier", "network"] {
                found_unique_triple = true;
                break;
            }
        }
        assert!(
            found_unique_triple,
            "UNIQUE(wallet_seed_hash, nullifier, network) must mirror legacy schema"
        );

        // The wallet/network covering index must exist.
        let cover_idx_present: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_shielded_notes_wallet_network'",
                [],
                |row| row.get(0),
            )
            .expect("cover index probe");
        assert!(cover_idx_present, "wallet/network index missing");

        // Expected columns for shielded_wallet_meta.
        let expected_meta_cols: &[(&str, &str, i32, i32)] = &[
            ("wallet_seed_hash", "BLOB", 1, 1),
            ("network", "TEXT", 1, 2),
            ("last_nullifier_sync_height", "INTEGER", 1, 0),
            ("last_nullifier_sync_timestamp", "INTEGER", 1, 0),
        ];
        let mut stmt = conn
            .prepare("PRAGMA table_info(shielded_wallet_meta)")
            .expect("pragma meta");
        let actual_meta: Vec<(String, String, i32, i32)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i32>(3)?,
                    row.get::<_, i32>(5)?,
                ))
            })
            .expect("query meta pragma")
            .collect::<rusqlite::Result<_>>()
            .expect("collect meta pragma");
        assert_eq!(
            actual_meta.len(),
            expected_meta_cols.len(),
            "shielded_wallet_meta column count must match legacy schema"
        );
        for (got, want) in actual_meta.iter().zip(expected_meta_cols) {
            assert_eq!(got.0, want.0, "meta column name");
            assert_eq!(got.1, want.1, "meta column type for {}", got.0);
            assert_eq!(got.2, want.2, "meta notnull flag for {}", got.0);
            assert_eq!(got.3, want.3, "meta pk flag for {}", got.0);
        }

        // Round-trip every public method now that the sidecar exists.
        let unspent = view
            .get_unspent_shielded_notes(&seed, "testnet")
            .expect("unspent");
        assert_eq!(unspent.len(), 2, "two unspent notes after insert");
        assert_eq!(unspent[0].position, 0);
        assert_eq!(unspent[1].position, 1);
        assert_eq!(unspent[0].cmx, n1.cmx);
        assert_eq!(unspent[0].nullifier, n1.nullifier);
        assert_eq!(unspent[0].value, 10);

        let all = view.get_all_shielded_notes(&seed, "testnet").expect("all");
        assert_eq!(all.len(), 2);

        let balance = view
            .get_shielded_balance(&seed, "testnet")
            .expect("balance");
        assert_eq!(balance, 25, "balance is the sum of unspent values");

        let info = view
            .get_nullifier_sync_info(&seed, "testnet")
            .expect("sync info");
        assert_eq!(info, (12345, 67890));

        let updated = view
            .mark_shielded_note_spent(&seed, &n1.nullifier, "testnet")
            .expect("mark spent");
        assert_eq!(updated, 1);
        let unspent_after = view
            .get_unspent_shielded_notes(&seed, "testnet")
            .expect("unspent post-spend");
        assert_eq!(unspent_after.len(), 1);
        assert_eq!(unspent_after[0].position, 1);
        let balance_after = view
            .get_shielded_balance(&seed, "testnet")
            .expect("balance post-spend");
        assert_eq!(balance_after, 15);

        let deleted = view
            .delete_shielded_notes(&seed, "testnet")
            .expect("delete");
        assert_eq!(deleted, 2, "both notes deleted regardless of spent flag");
        assert!(
            view.get_all_shielded_notes(&seed, "testnet")
                .expect("post-delete read")
                .is_empty()
        );
    }

    /// TC-SH-004 — note insert via the adapter persists to the
    /// per-network sidecar (and *only* the sidecar). Mirrors the path
    /// T-SH-03 wires `backend_task::shielded::sync::sync_notes` onto.
    #[test]
    fn tc_sh_004_note_insert_via_adapter_persists_to_sidecar() {
        let tmp = tempdir().expect("tempdir");
        let view = make_view(tmp.path());
        let seed: WalletSeedHash = [0x44; 32];

        let n = sample_note(11, 0, 0xD1);
        view.insert_shielded_note(&seed, &n.as_param("testnet"))
            .expect("insert via adapter");

        // The note must be readable via the same view.
        let all = view
            .get_all_shielded_notes(&seed, "testnet")
            .expect("read after insert");
        assert_eq!(all.len(), 1, "single note in sidecar");
        assert_eq!(all[0].value, 11);
        assert_eq!(all[0].position, 0);
        assert_eq!(all[0].nullifier, n.nullifier);

        // The on-disk file is the sidecar — confirm the row landed
        // there rather than vanishing into a different writer.
        let conn = Connection::open(view.path()).expect("open sidecar");
        let cnt: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM shielded_notes WHERE network = ?1",
                params!["testnet"],
                |row| row.get(0),
            )
            .expect("count row");
        assert_eq!(cnt, 1, "row landed in the sidecar file");
    }

    /// F17/F20 — clearing the nullifier cursor resets the sync info to the
    /// zero cursor so a same-seed-hash re-import does not skip scan windows.
    #[test]
    fn delete_shielded_wallet_meta_resets_nullifier_cursor() {
        let tmp = tempdir().expect("tempdir");
        let view = make_view(tmp.path());
        let seed: WalletSeedHash = [0x77; 32];

        view.set_nullifier_sync_info(&seed, "testnet", 4242, 9999)
            .expect("set cursor");
        assert_eq!(
            view.get_nullifier_sync_info(&seed, "testnet").unwrap(),
            (4242, 9999),
            "precondition: cursor is set before deletion"
        );

        let deleted = view
            .delete_shielded_wallet_meta(&seed, "testnet")
            .expect("delete meta");
        assert_eq!(deleted, 1, "the cursor row is removed");
        assert_eq!(
            view.get_nullifier_sync_info(&seed, "testnet").unwrap(),
            (0, 0),
            "the cursor resets to zero after deletion"
        );

        // Idempotent — a second delete removes nothing and still reads zero.
        assert_eq!(
            view.delete_shielded_wallet_meta(&seed, "testnet")
                .expect("idempotent delete"),
            0
        );
    }

    /// TC-SH-005 — balance read via the adapter returns the sidecar
    /// sum. This is the read path `send_screen.rs` uses to surface a
    /// last-known shielded balance when the in-memory state was
    /// dropped.
    #[test]
    fn tc_sh_005_balance_read_returns_sidecar_sum() {
        let tmp = tempdir().expect("tempdir");
        let view = make_view(tmp.path());
        let seed: WalletSeedHash = [0x55; 32];

        // Virgin sidecar: zero balance + no file on disk.
        assert_eq!(view.get_shielded_balance(&seed, "testnet").unwrap(), 0);
        assert!(!view.path().exists(), "read must not materialise");

        view.insert_shielded_note(&seed, &sample_note(10, 0, 0xE1).as_param("testnet"))
            .expect("insert n1");
        view.insert_shielded_note(&seed, &sample_note(25, 1, 0xE2).as_param("testnet"))
            .expect("insert n2");
        view.insert_shielded_note(&seed, &sample_note(7, 2, 0xE3).as_param("testnet"))
            .expect("insert n3");

        let bal = view
            .get_shielded_balance(&seed, "testnet")
            .expect("balance");
        assert_eq!(
            bal, 42,
            "balance is the sum of all unspent values (10+25+7)"
        );

        // Marking one spent shrinks the balance accordingly — same path
        // `nullifiers::check_nullifiers` rewires onto.
        let marked = view
            .mark_shielded_note_spent(&seed, &[0xE2; 32], "testnet")
            .expect("mark spent");
        assert_eq!(marked, 1);
        assert_eq!(
            view.get_shielded_balance(&seed, "testnet").unwrap(),
            17,
            "balance drops by the spent note's value (25)"
        );

        // Foreign network reads zero — covers the per-network isolation
        // guarantee callers depend on.
        assert_eq!(view.get_shielded_balance(&seed, "mainnet").unwrap(), 0);
    }

    /// TC-SH-006 — `mark_shielded_note_spent` via the adapter durably
    /// flips the row's `is_spent` flag. Mirrors the path
    /// `context::shielded::mark_notes_spent` rewires onto.
    #[test]
    fn tc_sh_006_mark_spent_via_adapter() {
        let tmp = tempdir().expect("tempdir");
        let view = make_view(tmp.path());
        let seed: WalletSeedHash = [0x66; 32];

        let n = sample_note(9, 0, 0xF1);
        view.insert_shielded_note(&seed, &n.as_param("testnet"))
            .expect("insert");

        assert_eq!(
            view.get_unspent_shielded_notes(&seed, "testnet")
                .unwrap()
                .len(),
            1,
            "note starts unspent"
        );

        let updated = view
            .mark_shielded_note_spent(&seed, &n.nullifier, "testnet")
            .expect("mark spent");
        assert_eq!(updated, 1, "exactly one row updated");

        // Unspent set is now empty…
        assert!(
            view.get_unspent_shielded_notes(&seed, "testnet")
                .unwrap()
                .is_empty(),
            "no unspent notes after mark"
        );
        // …but the row itself is still present (it's flagged, not deleted).
        assert_eq!(
            view.get_all_shielded_notes(&seed, "testnet").unwrap().len(),
            1,
            "spent row remains, only flagged"
        );

        // Idempotent: marking again is a silent no-op success.
        let again = view
            .mark_shielded_note_spent(&seed, &n.nullifier, "testnet")
            .expect("re-mark");
        assert_eq!(again, 1, "UPDATE still matches the row by nullifier");

        // Cross-network: marking on a foreign network must NOT touch
        // this network's row.
        let foreign = view
            .mark_shielded_note_spent(&seed, &n.nullifier, "mainnet")
            .expect("foreign-network mark");
        assert_eq!(foreign, 0, "no testnet rows touched by mainnet update");
    }

    /// TC-SH-007 — sync cursor read/write round-trip via the adapter.
    /// Mirrors the path `nullifiers::check_nullifiers` and
    /// `context::shielded::initialize_shielded_wallet` use to persist
    /// and resume the nullifier sync checkpoint.
    #[test]
    fn tc_sh_007_sync_cursor_read_write_via_adapter() {
        let tmp = tempdir().expect("tempdir");
        let view = make_view(tmp.path());
        let seed: WalletSeedHash = [0x77; 32];

        // Virgin read returns (0, 0) and does NOT materialise the file —
        // the lazy-provisioning contract every reader depends on.
        let pre = view
            .get_nullifier_sync_info(&seed, "testnet")
            .expect("virgin read");
        assert_eq!(pre, (0, 0));
        assert!(!view.path().exists(), "read must not materialise");

        view.set_nullifier_sync_info(&seed, "testnet", 1_234_567, 1_700_000_000)
            .expect("set cursor");
        assert!(view.path().exists(), "write materialised the sidecar");

        let (h, ts) = view
            .get_nullifier_sync_info(&seed, "testnet")
            .expect("read cursor");
        assert_eq!((h, ts), (1_234_567, 1_700_000_000));

        // Upsert semantics: re-writing replaces the value rather than
        // accumulating rows. Callers rely on this so cold-start
        // progress is monotonic, not history.
        view.set_nullifier_sync_info(&seed, "testnet", 2_000_000, 1_800_000_000)
            .expect("upsert cursor");
        let (h2, ts2) = view
            .get_nullifier_sync_info(&seed, "testnet")
            .expect("read cursor v2");
        assert_eq!((h2, ts2), (2_000_000, 1_800_000_000));

        // Foreign network has its own cursor — must not leak.
        assert_eq!(
            view.get_nullifier_sync_info(&seed, "mainnet").unwrap(),
            (0, 0),
        );
    }

    /// UNIQUE(wallet_seed_hash, nullifier, network) makes a duplicate
    /// insert a silent no-op (INSERT OR IGNORE) — mirrors legacy
    /// behaviour so the migration is idempotent.
    #[test]
    fn duplicate_insert_is_ignored() {
        let tmp = tempdir().expect("tempdir");
        let view = make_view(tmp.path());
        let seed: WalletSeedHash = [0x33; 32];
        let n = sample_note(5, 0, 0xCC);
        view.insert_shielded_note(&seed, &n.as_param("testnet"))
            .expect("first insert");
        view.insert_shielded_note(&seed, &n.as_param("testnet"))
            .expect("duplicate insert is OK");
        let all = view
            .get_all_shielded_notes(&seed, "testnet")
            .expect("read all");
        assert_eq!(all.len(), 1, "duplicate must be ignored");
    }
}
