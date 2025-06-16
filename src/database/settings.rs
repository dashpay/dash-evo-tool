use crate::database::initialization::DEFAULT_DB_VERSION;
use crate::database::Database;
use crate::model::password_info::PasswordInfo;
use crate::ui::theme::ThemeMode;
use crate::ui::RootScreenType;
use dash_sdk::dpp::dashcore::Network;
use rusqlite::{params, Connection, Result};
use std::str::FromStr;

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
        custom_dash_path: Option<String>,
        overwrite_dash_conf: bool,
    ) -> Result<()> {
        self.execute(
            "UPDATE settings
            SET custom_dash_qt_path = ?,
                overwrite_dash_conf = ?
            WHERE id = 1",
            rusqlite::params![custom_dash_path, overwrite_dash_conf],
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
            Option<String>,
            bool,
            ThemeMode,
        )>,
    > {
        // Query the settings row
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT network, start_root_screen, password_check, main_password_salt, main_password_nonce, custom_dash_qt_path, overwrite_dash_conf, theme_preference FROM settings WHERE id = 1")?;

        let result = stmt.query_row([], |row| {
            let network: String = row.get(0)?;
            let start_root_screen: u32 = row.get(1)?;
            let password_check: Option<Vec<u8>> = row.get(2)?;
            let main_password_salt: Option<Vec<u8>> = row.get(3)?;
            let main_password_nonce: Option<Vec<u8>> = row.get(4)?;
            let custom_dash_qt_path: Option<String> = row.get(5)?;
            let overwrite_dash_conf: Option<bool> = row.get(6)?;
            let theme_preference: Option<String> = row.get(7)?;

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

            Ok((
                parsed_network,
                root_screen_type,
                password_data,
                custom_dash_qt_path,
                overwrite_dash_conf.unwrap_or(true),
                theme_mode,
            ))
        });

        match result {
            Ok(settings) => Ok(Some(settings)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
