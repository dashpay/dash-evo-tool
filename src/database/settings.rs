//! Residual `settings`-table accessors.
//!
//! User preferences (theme, network, ZMQ, evonode tools, …) moved to
//! the upstream k/v store at [`AppSettings::KV_KEY`] in commit C3 of
//! the data.db unwire. What remains here is the small surface that the
//! later unwire steps still depend on:
//!
//! * `selected_wallet_hash` / `selected_single_key_hash` getters and
//!   setters (C4 moves these to a per-network blob).
//! * `database_version` writer used by the migration runner (C10).
//! * The column-addition helpers used by the migration ladder so old
//!   `data.db` files keep upgrading cleanly. These never create the
//!   columns on a fresh install — they only run when migrating an
//!   existing user from an old schema version, and the columns they
//!   touch are then ignored at read time.
//!
//! [`AppSettings::KV_KEY`]: crate::model::settings::AppSettings::KV_KEY

use crate::database::Database;
use rusqlite::{Connection, Result, params};

/// Selected wallet hash and single key hash tuple for database storage.
pub type SelectedWalletHashes = (Option<[u8; 32]>, Option<[u8; 32]>);

impl Database {
    /// Backfill `custom_dash_qt_path` / `overwrite_dash_conf` on an
    /// existing `settings` table. Kept only for the v3 migration arm —
    /// fresh installs never create these columns.
    pub fn add_custom_dash_qt_columns(&self, conn: &rusqlite::Connection) -> Result<()> {
        let custom_dash_qt_path_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='custom_dash_qt_path'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !custom_dash_qt_path_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN custom_dash_qt_path TEXT DEFAULT NULL;",
                (),
            )?;
        }

        let overwrite_dash_conf_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='overwrite_dash_conf'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !overwrite_dash_conf_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN overwrite_dash_conf INTEGER DEFAULT NULL;",
                (),
            )?;
        }

        Ok(())
    }

    /// Backfill `theme_preference` on an existing `settings` table.
    /// Kept only for the v10 migration arm.
    pub fn add_theme_preference_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        let theme_preference_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='theme_preference'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !theme_preference_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN theme_preference TEXT DEFAULT 'System';",
                (),
            )?;
        }

        Ok(())
    }

    /// Backfill `disable_zmq` on an existing `settings` table.
    /// Kept only for the v12 migration arm.
    pub fn add_disable_zmq_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        let disable_zmq_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='disable_zmq'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !disable_zmq_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN disable_zmq INTEGER DEFAULT 0;",
                (),
            )?;
        }

        Ok(())
    }

    /// Backfill `core_backend_mode` on an existing `settings` table.
    /// Kept only for the v15 migration arm.
    pub fn add_core_backend_mode_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='core_backend_mode'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !column_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN core_backend_mode INTEGER DEFAULT 1;",
                (),
            )?;
        }

        Ok(())
    }

    /// Backfill `onboarding_completed`, `show_evonode_tools`, and
    /// `user_mode` on an existing `settings` table. Kept only for the
    /// migration ladder.
    pub fn add_onboarding_columns(&self, conn: &rusqlite::Connection) -> Result<()> {
        let onboarding_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='onboarding_completed'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !onboarding_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN onboarding_completed INTEGER DEFAULT 0;",
                (),
            )?;
        }

        let evonode_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='show_evonode_tools'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !evonode_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN show_evonode_tools INTEGER DEFAULT 0;",
                (),
            )?;
        }

        let user_mode_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='user_mode'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !user_mode_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN user_mode TEXT DEFAULT 'Advanced';",
                (),
            )?;
        }

        Ok(())
    }

    /// Backfill `auto_start_spv` on an existing `settings` table.
    pub fn add_auto_start_spv_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='auto_start_spv'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !column_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN auto_start_spv INTEGER DEFAULT 0;",
                (),
            )?;
        }

        Ok(())
    }

    /// Backfill `close_dash_qt_on_exit` on an existing `settings` table.
    pub fn add_close_dash_qt_on_exit_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='close_dash_qt_on_exit'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !column_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN close_dash_qt_on_exit INTEGER DEFAULT 1;",
                (),
            )?;
        }

        Ok(())
    }

    /// Updates the database version in the settings table. Used by the
    /// migration runner — every other settings row stays untouched.
    pub fn update_database_version(&self, new_version: u16, conn: &Connection) -> Result<()> {
        conn.execute(
            "UPDATE settings
             SET database_version = ?
             WHERE id = 1",
            params![new_version],
        )?;
        Ok(())
    }

    /// Drop dead `settings` columns left behind by withdrawn features.
    ///
    /// Currently removes `dashpay_dip14_quarantine_active`. Existence-guarded
    /// and idempotent — safe to re-run.
    pub fn drop_dead_settings_columns(&self, conn: &rusqlite::Connection) -> Result<()> {
        const DEAD_COLUMNS: &[&str] = &["dashpay_dip14_quarantine_active"];
        for col in DEAD_COLUMNS {
            let exists: bool = conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name = ?1",
                rusqlite::params![col],
                |row| row.get::<_, i32>(0).map(|c| c > 0),
            )?;
            if exists {
                conn.execute(&format!("ALTER TABLE settings DROP COLUMN {col};"), ())?;
            }
        }
        Ok(())
    }

    /// Ensure the columns the residual settings layer still depends on
    /// exist on an upgraded `data.db`. Only the selected-wallet hashes
    /// and `database_version` are required here — the user-preference
    /// columns were unwired in C3 and are no longer touched at read
    /// time, so their backfill helpers are reserved for the migration
    /// ladder only.
    pub fn ensure_settings_columns_exist(&self, conn: &Connection) -> Result<()> {
        self.add_selected_wallet_columns_if_missing(conn)?;

        let version_column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='database_version'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !version_column_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN database_version INTEGER DEFAULT 0;",
                (),
            )?;
        }

        Ok(())
    }

    /// Adds selected wallet hash columns if they don't exist.
    pub fn add_selected_wallet_columns_if_missing(&self, conn: &Connection) -> Result<()> {
        let wallet_hash_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='selected_wallet_hash'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !wallet_hash_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN selected_wallet_hash BLOB DEFAULT NULL;",
                (),
            )?;
        }

        let single_key_hash_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='selected_single_key_hash'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !single_key_hash_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN selected_single_key_hash BLOB DEFAULT NULL;",
                (),
            )?;
        }

        Ok(())
    }

    /// Gets the selected wallet hashes from the settings table.
    /// Returns (selected_wallet_hash, selected_single_key_hash).
    pub fn get_selected_wallet_hashes(&self) -> Result<SelectedWalletHashes> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT selected_wallet_hash, selected_single_key_hash FROM settings WHERE id = 1",
            [],
            |row| {
                let wallet_hash: Option<Vec<u8>> = row.get(0)?;
                let single_key_hash: Option<Vec<u8>> = row.get(1)?;

                let wallet_hash_arr = wallet_hash.and_then(|v| {
                    if v.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&v);
                        Some(arr)
                    } else {
                        None
                    }
                });

                let single_key_hash_arr = single_key_hash.and_then(|v| {
                    if v.len() == 32 {
                        let mut arr = [0u8; 32];
                        arr.copy_from_slice(&v);
                        Some(arr)
                    } else {
                        None
                    }
                });

                Ok((wallet_hash_arr, single_key_hash_arr))
            },
        );

        match result {
            Ok(hashes) => Ok(hashes),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok((None, None)),
            Err(e) => Err(e),
        }
    }

    /// Updates the selected wallet hash in the settings table.
    pub fn update_selected_wallet_hash(&self, hash: Option<&[u8; 32]>) -> Result<()> {
        self.execute(
            "UPDATE settings SET selected_wallet_hash = ? WHERE id = 1",
            params![hash.map(|h| h.as_slice())],
        )?;
        Ok(())
    }

    /// Updates the selected single key hash in the settings table.
    pub fn update_selected_single_key_hash(&self, hash: Option<&[u8; 32]>) -> Result<()> {
        self.execute(
            "UPDATE settings SET selected_single_key_hash = ? WHERE id = 1",
            params![hash.map(|h| h.as_slice())],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::database::test_helpers::create_test_database;

    #[test]
    fn test_selected_wallet_hash_operations() {
        let db = create_test_database().expect("Failed to create test database");

        // Initially no wallet selected
        let (wallet_hash, single_key_hash) = db
            .get_selected_wallet_hashes()
            .expect("Failed to get wallet hashes");
        assert!(wallet_hash.is_none());
        assert!(single_key_hash.is_none());

        // Set a wallet hash
        let test_hash: [u8; 32] = [0x42; 32];
        db.update_selected_wallet_hash(Some(&test_hash))
            .expect("Failed to update wallet hash");

        let (wallet_hash, _) = db
            .get_selected_wallet_hashes()
            .expect("Failed to get wallet hashes");
        assert_eq!(wallet_hash, Some(test_hash));

        // Set a single key hash
        let single_key_test_hash: [u8; 32] = [0x24; 32];
        db.update_selected_single_key_hash(Some(&single_key_test_hash))
            .expect("Failed to update single key hash");

        let (_, single_key_hash) = db
            .get_selected_wallet_hashes()
            .expect("Failed to get wallet hashes");
        assert_eq!(single_key_hash, Some(single_key_test_hash));

        // Clear wallet hash
        db.update_selected_wallet_hash(None)
            .expect("Failed to clear wallet hash");

        let (wallet_hash, _) = db
            .get_selected_wallet_hashes()
            .expect("Failed to get wallet hashes");
        assert!(wallet_hash.is_none());
    }
}
