use crate::database::Database;
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

    /// Retrieves the settings from the database.
    pub fn get_settings(&self) -> Result<Option<(Network, RootScreenType)>> {
        // Query the settings row
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT network, start_root_screen FROM settings WHERE id = 1")?;

        let result = stmt.query_row([], |row| {
            let network: String = row.get(0)?;
            let start_root_screen: u32 = row.get(1)?;

            // Convert network from string to enum
            let parsed_network =
                Network::from_str(&network).map_err(|_| rusqlite::Error::InvalidQuery)?;

            // Convert start_root_screen from int to enum
            let root_screen_type = RootScreenType::from_int(start_root_screen)
                .ok_or_else(|| rusqlite::Error::InvalidQuery)?;

            Ok((parsed_network, root_screen_type))
        });

        match result {
            Ok(settings) => Ok(Some(settings)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
