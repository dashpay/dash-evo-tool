use crate::database::Database;
use crate::model::password_info::PasswordInfo;
use crate::ui::RootScreenType;
use dash_sdk::dpp::dashcore::Network;
use rusqlite::{params, Result};
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
             VALUES (1, ?, ?, 1)
             ON CONFLICT(id) DO UPDATE SET
                network = excluded.network,
                start_root_screen = excluded.start_root_screen",
            params![network_str, screen_type_int],
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

    pub fn add_custom_dash_qt_columns(&self) -> Result<()> {
        self.execute(
            "ALTER TABLE settings ADD COLUMN custom_dash_qt_path TEXT DEFAULT NULL;",
            (),
        )?;
        self.execute(
            "ALTER TABLE settings ADD COLUMN overwrite_dash_conf INTEGER DEFAULT NULL;",
            (),
        )?;

        Ok(())
    }

    /// Updates the database version in the settings table.
    pub fn update_database_version(&self, new_version: u16) -> Result<()> {
        // Ensure the database version is updated
        self.execute(
            "UPDATE settings
             SET database_version = ?
             WHERE id = 1",
            params![new_version],
        )?;

        Ok(())
    }

    /// Retrieves the settings from the database.
    pub fn get_settings(
        &self,
    ) -> Result<
        Option<(
            Network,
            RootScreenType,
            Option<PasswordInfo>,
            Option<String>,
            bool,
        )>,
    > {
        // Query the settings row
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT network, start_root_screen, password_check, main_password_salt, main_password_nonce, custom_dash_qt_path, overwrite_dash_conf FROM settings WHERE id = 1")?;

        let result = stmt.query_row([], |row| {
            let network: String = row.get(0)?;
            let start_root_screen: u32 = row.get(1)?;
            let password_check: Option<Vec<u8>> = row.get(2)?;
            let main_password_salt: Option<Vec<u8>> = row.get(3)?;
            let main_password_nonce: Option<Vec<u8>> = row.get(4)?;
            let custom_dash_qt_path: Option<String> = row.get(5)?;
            let overwrite_dash_conf: Option<bool> = row.get(6)?;

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

            Ok((
                parsed_network,
                root_screen_type,
                password_data,
                custom_dash_qt_path,
                overwrite_dash_conf.unwrap_or(true),
            ))
        });

        match result {
            Ok(settings) => Ok(Some(settings)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
