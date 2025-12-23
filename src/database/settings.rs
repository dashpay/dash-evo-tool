use crate::database::Database;
use crate::database::initialization::DEFAULT_DB_VERSION;
use crate::model::password_info::PasswordInfo;
use crate::model::settings::UserMode;
use crate::ui::RootScreenType;
use crate::ui::theme::ThemeMode;
use dash_sdk::dpp::dashcore::Network;
use rusqlite::{Connection, Result, params};
use std::{path::PathBuf, str::FromStr};

impl Database {
    /// Inserts or updates the settings in the database. This method ensures that only one row exists.
    ///
    /// Don't call this method directly, use `AppContext` methods instead to ensure proper caching behavior.
    pub fn insert_or_update_settings(
        &self,
        network: Network,
        start_root_screen: RootScreenType,
    ) -> Result<()> {
        let network_str = network.to_string();
        let screen_type_int = start_root_screen.to_int();
        self.execute(
            "INSERT INTO settings (id, network, start_root_screen, database_version)
             VALUES (1, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                network = excluded.network,
                start_root_screen = excluded.start_root_screen",
            params![network_str, screen_type_int, DEFAULT_DB_VERSION],
        )?;
        Ok(())
    }

    /// Updates the main password information in the settings table.
    ///
    /// Don't call this method directly, use `AppContext` methods instead to ensure proper caching behavior.
    pub fn update_main_password(
        &self,
        salt: &[u8],
        nonce: &[u8],
        password_check: &[u8],
    ) -> Result<()> {
        // Update the settings table with the provided salt, nonce, and password_check
        self.execute(
            "UPDATE settings
            SET main_password_salt = ?,
                main_password_nonce = ?,
                password_check = ?
            WHERE id = 1",
            rusqlite::params![salt, nonce, password_check],
        )?;

        Ok(())
    }
    /// Updates the Dash Core execution settings in the settings table.
    ///
    /// Don't call this method directly, use `AppContext` methods instead to ensure proper caching behavior.
    pub fn update_dash_core_execution_settings(
        &self,
        custom_dash_qt_path: Option<PathBuf>,
        overwrite_dash_conf: bool,
    ) -> Result<()> {
        let dash_qt_path = custom_dash_qt_path.map(|p| p.to_string_lossy().to_string());
        self.execute(
            "UPDATE settings
            SET custom_dash_qt_path = ?,
                overwrite_dash_conf = ?
            WHERE id = 1",
            rusqlite::params![dash_qt_path, overwrite_dash_conf],
        )?;

        Ok(())
    }

    pub fn add_custom_dash_qt_columns(&self, conn: &rusqlite::Connection) -> Result<()> {
        // Check if custom_dash_qt_path column exists
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

        // Check if overwrite_dash_conf column exists
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

    pub fn add_theme_preference_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        // Check if theme_preference column exists
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

    pub fn add_disable_zmq_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        // Check if disable_zmq column exists
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
    /// Updates the theme preference in the settings table.
    ///
    /// Don't call this method directly, use `AppContext` methods instead to ensure proper caching behavior.
    pub fn update_theme_preference(&self, theme_preference: ThemeMode) -> Result<()> {
        let theme_str = match theme_preference {
            ThemeMode::Light => "Light",
            ThemeMode::Dark => "Dark",
            ThemeMode::System => "System",
        };

        self.execute(
            "UPDATE settings
            SET theme_preference = ?
            WHERE id = 1",
            rusqlite::params![theme_str],
        )?;

        Ok(())
    }

    /// Updates the disable_zmq flag in the settings table.
    pub fn update_disable_zmq(&self, disable: bool) -> Result<()> {
        self.execute(
            "UPDATE settings SET disable_zmq = ? WHERE id = 1",
            rusqlite::params![disable],
        )?;
        Ok(())
    }

    /// Adds the core_backend_mode column to the settings table (migration for version 15).
    pub fn add_core_backend_mode_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        // Check if core_backend_mode column exists
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='core_backend_mode'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !column_exists {
            // Default to 1 (SPV mode) to match current app behavior
            conn.execute(
                "ALTER TABLE settings ADD COLUMN core_backend_mode INTEGER DEFAULT 1;",
                (),
            )?;
        }

        Ok(())
    }

    /// Updates the core backend mode (SPV=1, RPC=0) in the settings table.
    ///
    /// Don't call this method directly, use `AppContext` methods instead to ensure proper caching behavior.
    pub fn update_core_backend_mode(&self, mode: u8) -> Result<()> {
        self.execute(
            "UPDATE settings SET core_backend_mode = ? WHERE id = 1",
            rusqlite::params![mode],
        )?;
        Ok(())
    }

    /// Adds onboarding-related columns to the settings table.
    pub fn add_onboarding_columns(&self, conn: &rusqlite::Connection) -> Result<()> {
        // Check and add onboarding_completed column
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

        // Check and add show_evonode_tools column
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

        // Check and add user_mode column (Beginner or Advanced)
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

    /// Updates the onboarding completed flag in the settings table.
    pub fn update_onboarding_completed(&self, completed: bool) -> Result<()> {
        self.execute(
            "UPDATE settings SET onboarding_completed = ? WHERE id = 1",
            rusqlite::params![completed],
        )?;
        Ok(())
    }

    /// Updates the show_evonode_tools flag in the settings table.
    pub fn update_show_evonode_tools(&self, show: bool) -> Result<()> {
        self.execute(
            "UPDATE settings SET show_evonode_tools = ? WHERE id = 1",
            rusqlite::params![show],
        )?;
        Ok(())
    }

    /// Updates the user mode (Beginner/Advanced) in the settings table.
    pub fn update_user_mode(&self, mode: &str) -> Result<()> {
        self.execute(
            "UPDATE settings SET user_mode = ? WHERE id = 1",
            rusqlite::params![mode],
        )?;
        Ok(())
    }

    /// Updates the database version in the settings table.
    pub fn update_database_version(&self, new_version: u16, conn: &Connection) -> Result<()> {
        // Ensure the database version is updated
        conn.execute(
            "UPDATE settings
             SET database_version = ?
             WHERE id = 1",
            params![new_version],
        )?;

        Ok(())
    }

    /// Adds the use_local_spv_node column to the settings table.
    pub fn add_use_local_spv_node_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='use_local_spv_node'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !column_exists {
            // Default to false - use DNS seed discovery by default
            conn.execute(
                "ALTER TABLE settings ADD COLUMN use_local_spv_node INTEGER DEFAULT 0;",
                (),
            )?;
        }

        Ok(())
    }

    /// Adds the auto_start_spv column to the settings table.
    pub fn add_auto_start_spv_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='auto_start_spv'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !column_exists {
            // Default to true - auto-start SPV on startup
            conn.execute(
                "ALTER TABLE settings ADD COLUMN auto_start_spv INTEGER DEFAULT 1;",
                (),
            )?;
        }

        Ok(())
    }

    /// Updates the use_local_spv_node flag in the settings table.
    pub fn update_use_local_spv_node(&self, use_local: bool) -> Result<()> {
        self.execute(
            "UPDATE settings SET use_local_spv_node = ? WHERE id = 1",
            rusqlite::params![use_local],
        )?;
        Ok(())
    }

    /// Gets the use_local_spv_node flag from the settings table.
    pub fn get_use_local_spv_node(&self) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let result: Option<bool> = conn.query_row(
            "SELECT use_local_spv_node FROM settings WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(result.unwrap_or(false))
    }

    /// Updates the auto_start_spv flag in the settings table.
    pub fn update_auto_start_spv(&self, auto_start: bool) -> Result<()> {
        self.execute(
            "UPDATE settings SET auto_start_spv = ? WHERE id = 1",
            rusqlite::params![auto_start],
        )?;
        Ok(())
    }

    /// Gets the auto_start_spv flag from the settings table.
    pub fn get_auto_start_spv(&self) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let result: Option<bool> = conn.query_row(
            "SELECT auto_start_spv FROM settings WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(result.unwrap_or(true)) // Default to true
    }

    /// Adds the close_dash_qt_on_exit column to the settings table.
    pub fn add_close_dash_qt_on_exit_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        let column_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='close_dash_qt_on_exit'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !column_exists {
            // Default to true - close Dash-Qt on exit by default
            conn.execute(
                "ALTER TABLE settings ADD COLUMN close_dash_qt_on_exit INTEGER DEFAULT 1;",
                (),
            )?;
        }

        Ok(())
    }

    /// Updates the close_dash_qt_on_exit flag in the settings table.
    pub fn update_close_dash_qt_on_exit(&self, close_on_exit: bool) -> Result<()> {
        self.execute(
            "UPDATE settings SET close_dash_qt_on_exit = ? WHERE id = 1",
            rusqlite::params![close_on_exit],
        )?;
        Ok(())
    }

    /// Gets the close_dash_qt_on_exit flag from the settings table.
    pub fn get_close_dash_qt_on_exit(&self) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let result: Option<bool> = conn.query_row(
            "SELECT close_dash_qt_on_exit FROM settings WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        Ok(result.unwrap_or(true)) // Default to true
    }

    /// Ensures all required columns exist in the settings table.
    /// This handles the case where an old database has a settings table with missing columns.
    pub fn ensure_settings_columns_exist(&self, conn: &Connection) -> Result<()> {
        self.add_custom_dash_qt_columns(conn)?;
        self.add_theme_preference_column(conn)?;
        self.add_disable_zmq_column(conn)?;
        self.add_core_backend_mode_column(conn)?;
        self.add_onboarding_columns(conn)?;
        self.add_use_local_spv_node_column(conn)?;
        self.add_auto_start_spv_column(conn)?;
        self.add_close_dash_qt_on_exit_column(conn)?;

        // Ensure database_version column exists
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

    /// Retrieves the settings from the database.
    ///
    /// Don't call this method directly, use `AppContext` methods instead to ensure proper caching behavior.
    #[allow(clippy::type_complexity)]
    pub fn get_settings(
        &self,
    ) -> Result<
        Option<(
            Network,
            RootScreenType,
            Option<PasswordInfo>,
            Option<PathBuf>,
            bool,
            bool,
            ThemeMode,
            u8,
            bool,     // onboarding_completed
            bool,     // show_evonode_tools
            UserMode, // user_mode
            bool,     // close_dash_qt_on_exit
        )>,
    > {
        // Query the settings row
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT network, start_root_screen, password_check, main_password_salt, main_password_nonce, custom_dash_qt_path, overwrite_dash_conf, disable_zmq, theme_preference, core_backend_mode, onboarding_completed, show_evonode_tools, user_mode, close_dash_qt_on_exit FROM settings WHERE id = 1",
        )?;

        let result = stmt.query_row([], |row| {
            let network: String = row.get(0)?;
            let start_root_screen: u32 = row.get(1)?;
            let password_check: Option<Vec<u8>> = row.get(2)?;
            let main_password_salt: Option<Vec<u8>> = row.get(3)?;
            let main_password_nonce: Option<Vec<u8>> = row.get(4)?;
            let custom_dash_qt_path: Option<String> = row.get(5)?;
            let overwrite_dash_conf: Option<bool> = row.get(6)?;
            let disable_zmq: Option<bool> = row.get(7)?;
            let theme_preference: Option<String> = row.get(8)?;
            let core_backend_mode: Option<u8> = row.get(9)?;
            let onboarding_completed: Option<bool> = row.get(10)?;
            let show_evonode_tools: Option<bool> = row.get(11)?;
            let user_mode: Option<String> = row.get(12)?;
            let close_dash_qt_on_exit: Option<bool> = row.get(13)?;

            // Combine the password-related fields if all are present, otherwise set to None
            let password_data = match (password_check, main_password_salt, main_password_nonce) {
                (Some(password_checker), Some(salt), Some(nonce)) => Some(PasswordInfo {
                    password_checker,
                    salt,
                    nonce,
                }),
                _ => None,
            };

            // Convert network from string to enum
            let parsed_network =
                Network::from_str(&network).map_err(|_| rusqlite::Error::InvalidQuery)?;

            // Convert start_root_screen from int to enum
            let root_screen_type = RootScreenType::from_int(start_root_screen)
                .ok_or_else(|| rusqlite::Error::InvalidQuery)?;

            // Parse theme preference
            let theme_mode = match theme_preference.as_deref() {
                Some("Light") => ThemeMode::Light,
                Some("Dark") => ThemeMode::Dark,
                Some("System") | None => ThemeMode::System, // Default to System if missing
                _ => ThemeMode::System,                     // Default to System for unknown values
            };

            // Parse user mode
            let user_mode = match user_mode.as_deref() {
                Some("Beginner") => UserMode::Beginner,
                Some("Advanced") | None => UserMode::Advanced, // Default to Advanced
                _ => UserMode::Advanced,
            };

            Ok((
                parsed_network,
                root_screen_type,
                password_data,
                custom_dash_qt_path.map(PathBuf::from),
                overwrite_dash_conf.unwrap_or(true),
                disable_zmq.unwrap_or(false),
                theme_mode,
                core_backend_mode.unwrap_or(1), // Default to SPV (1)
                onboarding_completed.unwrap_or(false),
                show_evonode_tools.unwrap_or(false),
                user_mode,
                close_dash_qt_on_exit.unwrap_or(true), // Default to true
            ))
        });

        match result {
            Ok(settings) => Ok(Some(settings)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
