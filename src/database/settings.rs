use crate::database::Database;
use crate::database::initialization::DEFAULT_DB_VERSION;
use crate::model::password_info::PasswordInfo;
use crate::model::settings::ConnectionMode;
use crate::ui::RootScreenType;
use crate::ui::theme::ThemeMode;
use dash_sdk::dpp::dashcore::Network;
use rusqlite::{Connection, Result, params};
use std::{path::PathBuf, str::FromStr};

impl Database {
    /// Inserts or updates the settings in the database. This method ensures that only one row exists.
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

    pub fn add_connection_mode_column(&self, conn: &rusqlite::Connection) -> Result<()> {
        // Check if connection_mode column exists
        let connection_mode_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='connection_mode'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        if !connection_mode_exists {
            conn.execute(
                "ALTER TABLE settings ADD COLUMN connection_mode TEXT DEFAULT 'Core';",
                (),
            )?;
        }

        Ok(())
    }

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

    pub fn update_connection_mode(&self, connection_mode: ConnectionMode) -> Result<()> {
        let mode_str = match connection_mode {
            ConnectionMode::Core => "Core",
            ConnectionMode::Spv => "Spv",
        };

        self.execute(
            "UPDATE settings
            SET connection_mode = ?
            WHERE id = 1",
            rusqlite::params![mode_str],
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

    /// Retrieves the settings from the database.
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
            ThemeMode,
            ConnectionMode,
        )>,
    > {
        // Query the settings row
        let conn = self.conn.lock().unwrap();

        // First check if settings table exists
        let table_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='settings')",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !table_exists {
            return Ok(None);
        }

        // Check if there are any rows
        let has_rows: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM settings WHERE id = 1)",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !has_rows {
            return Ok(None);
        }

        // Check if connection_mode column exists
        let connection_mode_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='connection_mode'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        // Build query dynamically based on available columns
        let base_columns = "network, start_root_screen, password_check, main_password_salt, main_password_nonce, custom_dash_qt_path, overwrite_dash_conf";

        // Check if theme_preference column exists
        let theme_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM pragma_table_info('settings') WHERE name='theme_preference'",
            [],
            |row| row.get::<_, i32>(0).map(|count| count > 0),
        )?;

        let query = if connection_mode_exists && theme_exists {
            format!("{}, theme_preference, connection_mode", base_columns)
        } else if theme_exists {
            format!("{}, theme_preference", base_columns)
        } else {
            base_columns.to_string()
        };

        let query = format!("SELECT {} FROM settings WHERE id = 1", query);
        let mut stmt = conn.prepare(&query)?;

        let has_connection_mode = connection_mode_exists;
        let has_theme = theme_exists;
        let result = stmt.query_row([], move |row| {
            let mut col_idx = 0;
            let network: String = row.get(col_idx)?;
            col_idx += 1;
            let start_root_screen: u32 = row.get(col_idx)?;
            col_idx += 1;
            let password_check: Option<Vec<u8>> = row.get(col_idx)?;
            col_idx += 1;
            let main_password_salt: Option<Vec<u8>> = row.get(col_idx)?;
            col_idx += 1;
            let main_password_nonce: Option<Vec<u8>> = row.get(col_idx)?;
            col_idx += 1;
            let custom_dash_qt_path: Option<String> = row.get(col_idx)?;
            col_idx += 1;
            let overwrite_dash_conf: Option<bool> = row.get(col_idx)?;
            col_idx += 1;

            let theme_preference: Option<String> = if has_theme {
                let val = row.get(col_idx)?;
                col_idx += 1;
                val
            } else {
                None
            };

            let connection_mode: Option<String> = if has_connection_mode {
                row.get(col_idx)?
            } else {
                None
            };

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

            // Parse connection mode
            let connection_mode = match connection_mode.as_deref() {
                Some("Spv") => ConnectionMode::Spv,
                Some("Core") | None => ConnectionMode::Core, // Default to Core if missing
                _ => ConnectionMode::Core,                   // Default to Core for unknown values
            };

            Ok((
                parsed_network,
                root_screen_type,
                password_data,
                custom_dash_qt_path.map(PathBuf::from),
                overwrite_dash_conf.unwrap_or(true),
                theme_mode,
                connection_mode,
            ))
        });

        match result {
            Ok(settings) => Ok(Some(settings)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
