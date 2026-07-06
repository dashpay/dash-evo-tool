//! Residual `settings`-table accessors.
//!
//! User preferences (theme, network, ZMQ, evonode tools, …) moved to
//! the upstream k/v store at [`AppSettings::KV_KEY`] in commit C3 of
//! the data.db unwire. The selected-wallet pointer
//! (`selected_wallet_hash` / `selected_single_key_hash`) moved to the
//! per-network wallet k/v store at
//! [`SelectedWallet::KV_KEY`](crate::model::selected_wallet::SelectedWallet::KV_KEY)
//! in commit C4. What remains here is the small surface that the later
//! unwire steps still depend on:
//!
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
    /// Kept only for the v15 migration arm — the column is later dropped by
    /// [`Self::drop_core_backend_mode_column`] in the v38 arm.
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

    /// Drop the retired `core_backend_mode` column from the `settings` table.
    /// The RPC/SPV backend selector it held is gone (chain sync is SPV-only);
    /// only pre-C3 DBs still carry the column. Existence-guarded and
    /// idempotent — safe to re-run and a no-op on DBs that never had it.
    /// Used by the v38 migration arm.
    pub fn drop_core_backend_mode_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='core_backend_mode'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if column_exists {
            conn.execute("ALTER TABLE settings DROP COLUMN core_backend_mode;", ())?;
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
    /// exist on an upgraded `data.db`. Only `database_version` is
    /// required here — user-preference columns were unwired in C3 and
    /// the selected-wallet pair was unwired in C4. Their backfill
    /// helpers are reserved for the migration ladder only.
    pub fn ensure_settings_columns_exist(&self, conn: &Connection) -> Result<()> {
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
}
