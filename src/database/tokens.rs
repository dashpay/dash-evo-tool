use dash_sdk::{
    dpp::{
        data_contract::{
            accessors::v1::DataContractV1Getters,
            associated_token::token_configuration_convention::TokenConfigurationConvention,
        },
        platform_value::string_encoding::Encoding,
    },
    platform::Identifier,
};
use dash_sdk::dpp::data_contract::associated_token::token_configuration_localization::accessors::v0::TokenConfigurationLocalizationV0Getters;
use egui::TextBuffer;
use rusqlite::params;

use crate::{context::AppContext, ui::tokens::tokens_screen::IdentityTokenBalance};

use super::Database;

impl Database {
    /// Creates the identity_token_balances table if it doesn't already exist
    fn ensure_identity_token_balances_table_exists(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS identity_token_balances (
                token_id BLOB NOT NULL,
                identity_id BLOB NOT NULL,
                balance INTEGER NOT NULL,
                data_contract_id BLOB NOT NULL,
                token_position INTEGER NOT NULL,
                network TEXT NOT NULL,
                PRIMARY KEY(token_id, identity_id, network)
             )",
        )?;
        Ok(())
    }

    pub fn insert_identity_token_balance(
        &self,
        token_identifier: &Identifier,
        identity_id: &Identifier,
        balance: u64,
        data_contract_id: &Identifier,
        token_position: u16,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        self.ensure_identity_token_balances_table_exists()?;

        let network = app_context.network_string();
        let token_identifier_vec = token_identifier.to_vec();
        let identity_id_vec = identity_id.to_vec();
        let data_contract_id_vec = data_contract_id.to_vec();

        self.execute(
            "INSERT OR REPLACE INTO identity_token_balances
             (token_id, identity_id, balance, data_contract_id, token_position, network)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                token_identifier_vec,
                identity_id_vec,
                balance,
                data_contract_id_vec,
                token_position,
                network
            ],
        )?;

        Ok(())
    }

    pub fn get_identity_token_balances(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<Vec<IdentityTokenBalance>> {
        let network = app_context.network_string();

        // 1) Lock and read everything into memory first.
        let rows_data = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT *
             FROM identity_token_balances
             WHERE network = ?",
            )?;

            // Map all the rows into a Vec in memory.
            let rows = stmt.query_map(params![network], |row| {
                Ok((
                    Identifier::from_vec(row.get(0)?),
                    Identifier::from_vec(row.get(1)?),
                    row.get(2)?,
                    Identifier::from_vec(row.get(3)?),
                    row.get(4)?,
                ))
            })?;

            let mut temp = Vec::new();
            for row in rows {
                temp.push(row?);
            }
            temp
        }; // <- The lock is dropped here because `conn` goes out of scope.

        let mut result = Vec::new();
        for (token_identifier, identity_id, balance, data_contract_id, token_position) in rows_data
        {
            let token_name = match self.get_contract_by_id(
                data_contract_id
                    .clone()
                    .expect("Expected to convert data_contract_id from vec to Identifier"),
                app_context,
            ) {
                Ok(Some(qualified_contract)) => {
                    let token_configuration = qualified_contract
                        .contract
                        .expected_token_configuration(token_position)
                        .expect("Expected to get token configuration")
                        .as_cow_v0();
                    let conventions = match &token_configuration.conventions {
                        TokenConfigurationConvention::V0(conventions) => conventions,
                    };
                    match conventions
                        .localizations
                        .get("en")
                        .map(|l| l.singular_form().to_string())
                    {
                        Some(token_name) => token_name,
                        None => token_identifier
                            .clone()
                            .expect("Expected to convert token_identifier from vec to Identifier")
                            .to_string(Encoding::Base58),
                    }
                }
                _ => token_identifier
                    .clone()
                    .expect("Expected to convert identity_id from vec to Identifier")
                    .to_string(Encoding::Base58),
            };
            let identity_token_balance = IdentityTokenBalance {
                token_identifier: token_identifier
                    .clone()
                    .expect("Expected to convert token_identifier from vec to Identifier"),
                token_name,
                identity_id: identity_id
                    .expect("Expected to convert identity_id from vec to Identifier"),
                balance,
                data_contract_id: data_contract_id
                    .expect("Expected to convert data_contract_id from vec to Identifier"),
                token_position,
            };
            result.push(identity_token_balance);
        }

        Ok(result)
    }

    pub fn remove_token_balance(
        &self,
        token_identifier: &Identifier,
        identity_id: &Identifier,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let network = app_context.network_string();
        let token_identifier_vec = token_identifier.to_vec();
        let identity_id_vec = identity_id.to_vec();

        self.execute(
            "DELETE FROM identity_token_balances
             WHERE token_id = ? AND identity_id = ? AND network = ?",
            params![token_identifier_vec, identity_id_vec, network],
        )?;

        Ok(())
    }

    /// Creates the identity_order table if it doesn't already exist
    /// with two columns: `pos` (int) and `identity_id` (blob).
    /// pos is the "position" in the custom ordering.
    fn ensure_token_order_table_exists(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS token_order (
                pos INTEGER NOT NULL,
                token_id BLOB NOT NULL,
                identity_id BLOB NOT NULL,
                PRIMARY KEY(pos)
             )",
        )?;
        Ok(())
    }

    /// Saves the user’s custom identity order (the entire list).
    /// This method overwrites whatever was there before.
    pub fn save_token_order(&self, all_ids: Vec<(Identifier, Identifier)>) -> rusqlite::Result<()> {
        // Make sure table exists
        self.ensure_token_order_table_exists()?;

        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        // Clear existing rows
        tx.execute("DELETE FROM token_order", [])?;

        // Insert each ID with a numeric pos = 0..N
        for (pos, (token_id, identity_id)) in all_ids.iter().enumerate() {
            let token_id_bytes = token_id.to_vec();
            let identity_id_bytes = identity_id.to_vec();
            tx.execute(
                "INSERT INTO token_order (pos, token_id, identity_id)
                 VALUES (?1, ?2, ?3)",
                params![pos as i64, token_id_bytes, identity_id_bytes],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Loads the custom identity order from the DB, returning a list of Identifiers in the stored order.
    /// If there's no data, returns an empty Vec.
    pub fn load_token_order(&self) -> rusqlite::Result<Vec<(Identifier, Identifier)>> {
        // Make sure table exists (in case it doesn't)
        self.ensure_token_order_table_exists()?;

        let conn = self.conn.lock().unwrap();

        // Read all rows sorted by pos
        let mut stmt = conn.prepare(
            "SELECT token_id, identity_id FROM token_order
             ORDER BY pos ASC",
        )?;

        let mut rows = stmt.query([])?;
        let mut result = Vec::new();

        while let Some(row) = rows.next()? {
            let token_id_bytes: Vec<u8> = row.get(0)?;
            let identity_id_bytes: Vec<u8> = row.get(1)?;
            // Convert from raw bytes to an Identifier
            if let Ok(token_id) = Identifier::from_vec(token_id_bytes) {
                if let Ok(identity_id) = Identifier::from_vec(identity_id_bytes) {
                    result.push((token_id, identity_id));
                } else {
                    // If for some reason it fails to parse, skip it or handle error
                }
            } else {
                // If for some reason it fails to parse, skip it or handle error
            }
        }

        Ok(result)
    }
}
