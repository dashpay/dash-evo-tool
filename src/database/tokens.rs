use dash_sdk::platform::Identifier;
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
                token_name TEXT NOT NULL,
                identity_id BLOB NOT NULL,
                balance INTEGER NOT NULL,
                data_contract_id BLOB NOT NULL,
                token_position INTEGER NOT NULL,
                network TEXT NOT NULL,
                PRIMARY KEY(token_id, identity_id, network),
                FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
             )",
        )?;
        Ok(())
    }

    pub fn insert_identity_token_balance(
        &self,
        token_identifier: &Identifier,
        token_name: &str,
        identity_id: &Identifier,
        balance: u64,
        data_contract_id: &Identifier,
        token_position: u16,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        // make sure the table / PK exists
        self.ensure_identity_token_balances_table_exists()?;

        // prepare values
        let network = app_context.network_string();
        let token_id_bytes = token_identifier.to_vec();
        let identity_id_bytes = identity_id.to_vec();
        let data_contract_bytes = data_contract_id.to_vec();

        // `token_name` is only in the INSERT part – not in the UPDATE part
        self.execute(
            "INSERT INTO identity_token_balances
                  (token_id, token_name, identity_id, balance,
                   data_contract_id, token_position, network)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(token_id, identity_id, network) DO UPDATE SET
                  balance          = excluded.balance,
                  data_contract_id = excluded.data_contract_id,
                  token_position   = excluded.token_position
                  -- token_name intentionally left unchanged",
            params![
                token_id_bytes,
                token_name,
                identity_id_bytes,
                balance,
                data_contract_bytes,
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
                    row.get(1)?,
                    Identifier::from_vec(row.get(2)?),
                    row.get(3)?,
                    Identifier::from_vec(row.get(4)?),
                    row.get(5)?,
                ))
            })?;

            let mut temp = Vec::new();
            for row in rows {
                temp.push(row?);
            }
            temp
        }; // <- The lock is dropped here because `conn` goes out of scope.

        let mut result = Vec::new();
        for (
            token_identifier,
            token_name,
            identity_id,
            balance,
            data_contract_id,
            token_position,
        ) in rows_data
        {
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
