use bincode::{self, config::standard};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use dash_sdk::query_types::IndexMap;
use rusqlite::params;
use rusqlite::OptionalExtension;

use super::Database;
use crate::ui::tokens::tokens_screen::{IdentityTokenIdentifier, TokenInfo};
use crate::{context::AppContext, ui::tokens::tokens_screen::IdentityTokenBalance};

impl Database {
    pub fn initialize_token_table(&self) -> rusqlite::Result<()> {
        // Create the token table
        self.execute(
            "CREATE TABLE IF NOT EXISTS token (
                id BLOB PRIMARY KEY,
                token_alias TEXT NOT NULL,
                token_config BLOB NOT NULL,
                data_contract_id BLOB NOT NULL,
                token_position INTEGER NOT NULL,
                network TEXT NOT NULL,
                FOREIGN KEY (data_contract_id, network)
                    REFERENCES contract(contract_id, network)
                    ON DELETE CASCADE
            )",
            [],
        )?;
        Ok(())
    }

    pub fn get_token_config_for_id(
        &self,
        token_id: &Identifier,
        app_context: &AppContext,
    ) -> rusqlite::Result<Option<TokenConfiguration>> {
        let network = app_context.network_string();
        let token_id_bytes = token_id.to_vec();

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT token_config
             FROM token
             WHERE id = ? AND network = ?",
        )?;

        let token_config_bytes: Option<Vec<u8>> = stmt
            .query_row(params![token_id_bytes, network], |row| row.get(0))
            .optional()?;

        match token_config_bytes {
            Some(bytes) => {
                match bincode::decode_from_slice::<TokenConfiguration, _>(&bytes, standard()) {
                    Ok((config, _)) => Ok(Some(config)),
                    Err(_) => Ok(None),
                }
            }
            None => Ok(None),
        }
    }

    pub fn get_contract_id_by_token_id(
        &self,
        token_id: &Identifier,
        app_context: &AppContext,
    ) -> rusqlite::Result<Option<Identifier>> {
        let network = app_context.network_string();
        let token_id_bytes = token_id.to_vec();

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT data_contract_id
             FROM token
             WHERE id = ? AND network = ?",
        )?;

        let contract_id_bytes: Option<Vec<u8>> = stmt
            .query_row(params![token_id_bytes, network], |row| row.get(0))
            .optional()?;

        match contract_id_bytes {
            Some(bytes) => {
                Ok(Some(Identifier::from_vec(bytes).map_err(|e| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                })?))
            }
            None => Ok(None),
        }
    }

    pub fn insert_token(
        &self,
        token_id: &Identifier,
        token_alias: &str,
        token_config: &[u8],
        data_contract_id: &Identifier,
        token_position: u16,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let network = app_context.network_string();
        let token_id_bytes = token_id.to_vec();
        let data_contract_bytes = data_contract_id.to_vec();

        self.execute(
            "INSERT INTO token
              (id, token_alias, token_config, data_contract_id, token_position, network)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
              token_alias = excluded.token_alias,
              token_config = excluded.token_config,
              data_contract_id = excluded.data_contract_id,
              token_position = excluded.token_position,
              network = excluded.network",
            params![
                token_id_bytes,
                token_alias,
                token_config,
                data_contract_bytes,
                token_position,
                network
            ],
        )?;

        // Insert an identity token balance of 0 for each identity for this token
        let wallets = app_context.wallets.read().unwrap();
        for identity in self.get_local_qualified_identities(app_context, &wallets)? {
            self.insert_identity_token_balance(&token_id, &identity.identity.id(), 0, app_context)?;
        }

        Ok(())
    }

    /// Drops the identity_token_balances table (if necessary to enforce schema update)
    pub fn drop_identity_token_balances_table(&self) -> rusqlite::Result<()> {
        self.execute("DROP TABLE IF EXISTS identity_token_balances", [])?;

        Ok(())
    }

    /// Creates the identity_token_balances table if it doesn't already exist
    pub fn initialize_identity_token_balances_table(&self) -> rusqlite::Result<()> {
        self.execute(
            "CREATE TABLE IF NOT EXISTS identity_token_balances (
                token_id BLOB NOT NULL,
                identity_id BLOB NOT NULL,
                balance INTEGER NOT NULL,
                network TEXT NOT NULL,
                PRIMARY KEY(token_id, identity_id, network),
                FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE,
                FOREIGN KEY (token_id) REFERENCES token(id) ON DELETE CASCADE
             )",
            [],
        )?;
        Ok(())
    }

    pub fn insert_identity_token_balance(
        &self,
        token_identifier: &Identifier,
        identity_id: &Identifier,
        balance: u64,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let network = app_context.network_string();
        let token_id_bytes = token_identifier.to_vec();
        let identity_id_bytes = identity_id.to_vec();

        self.execute(
            "INSERT INTO identity_token_balances
              (token_id, identity_id, balance, network)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(token_id, identity_id, network) DO UPDATE SET
              balance = excluded.balance",
            params![token_id_bytes, identity_id_bytes, balance, network],
        )?;

        Ok(())
    }

    /// Retrieves all known tokens as a map from token ID to `TokenInfo`.
    ///
    /// Now also fetches and decodes the **`token_config`** blob.
    pub fn get_all_known_tokens(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<IndexMap<Identifier, TokenInfo>> {
        let network = app_context.network_string();
        let conn = self.conn.lock().unwrap();

        // -- 1.  query id / alias / config / contract / position ────────────────
        let mut stmt = conn.prepare(
            "SELECT id,
                token_alias,
                token_config,
                data_contract_id,
                token_position
         FROM   token
         WHERE  network = ?
         ORDER  BY token_alias ASC",
        )?;

        // -- 2.  map each row, decoding `token_config` with bincode -─────────────
        let rows = stmt.query_map(params![network], |row| {
            let bytes: Vec<u8> = row.get(2)?; // token_config blob
            let cfg = bincode::decode_from_slice::<TokenConfiguration, _>(&bytes, standard())
                .map(|(c, _)| c)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

            Ok((
                Identifier::from_vec(row.get(0)?),
                row.get::<_, String>(1)?,
                cfg, // decoded config
                Identifier::from_vec(row.get(3)?),
                row.get::<_, u16>(4)?,
            ))
        })?;

        // -- 3.  build the IndexMap result ───────────────────────────────────────
        let mut result = IndexMap::new();

        for row in rows {
            let (token_id_res, token_alias, token_cfg, contract_id_res, pos) = row?;

            let token_id = token_id_res.expect("Failed to parse token ID");
            let data_contract_id = contract_id_res.expect("Failed to parse contract ID");

            result.insert(
                token_id,
                TokenInfo {
                    token_id,
                    token_name: token_alias,
                    data_contract_id,
                    token_position: pos,
                    token_configuration: token_cfg,
                    description: None,
                },
            );
        }

        Ok(result)
    }

    pub fn get_identity_token_balances(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<IndexMap<IdentityTokenIdentifier, IdentityTokenBalance>> {
        let network = app_context.network_string();

        let rows_data = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT b.token_id, t.token_alias, t.token_config, b.identity_id, b.balance, t.data_contract_id, t.token_position
             FROM identity_token_balances AS b
             JOIN token AS t ON b.token_id = t.id
             WHERE b.network = ?",
            )?;

            let rows = stmt.query_map(params![network], |row| {
                let config = standard();
                let bytes: Vec<u8> = row.get(2)?;
                let token_config: Result<(TokenConfiguration, _), _> =
                    bincode::decode_from_slice(&bytes, config);
                Ok((
                    Identifier::from_vec(row.get(0)?),
                    row.get(1)?,
                    token_config,
                    Identifier::from_vec(row.get(3)?),
                    row.get(4)?,
                    Identifier::from_vec(row.get(5)?),
                    row.get(6)?,
                ))
            })?;

            let mut temp = Vec::new();
            for row in rows {
                temp.push(row?);
            }
            temp
        };

        let mut result = IndexMap::new();
        for (
            token_id_res,
            token_name,
            token_config,
            identity_id_res,
            balance,
            data_contract_id_res,
            token_position,
        ) in rows_data
        {
            let token_id = token_id_res.expect("Failed to parse token_identifier");
            let token_config = token_config.expect("Missing token_config").0;
            let identity_id = identity_id_res.expect("Failed to parse identity_id");
            let data_contract_id = data_contract_id_res.expect("Failed to parse data_contract_id");

            let identity_token_balance = IdentityTokenBalance {
                token_id,
                token_alias: token_name,
                token_config,
                identity_id,
                balance,
                estimated_unclaimed_rewards: None,
                data_contract_id,
                token_position,
            };

            result.insert(
                IdentityTokenIdentifier {
                    identity_id,
                    token_id,
                },
                identity_token_balance,
            );
        }

        Ok(result)
    }

    /// Removes a token and all associated entries (balances, order) by `token_id`.
    ///
    /// This will cascade delete from `identity_token_balances` and `token_order` due to foreign key constraints.
    pub fn remove_token(
        &self,
        token_id: &Identifier,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let network = app_context.network_string();
        let token_id_bytes = token_id.to_vec();

        self.execute(
            "DELETE FROM token WHERE id = ? AND network = ?",
            params![token_id_bytes, network],
        )?;

        Ok(())
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

        // Also remove from the order table
        self.execute(
            "DELETE FROM token_order
             WHERE token_id = ? AND identity_id = ?",
            params![token_identifier_vec, identity_id_vec],
        )?;

        Ok(())
    }

    /// (Re)creates the `token_order` table with proper foreign keys.
    pub fn initialize_token_order_table(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();

        // Drop the table if it already exists
        conn.execute("DROP TABLE IF EXISTS token_order", [])?;

        // Recreate with foreign keys
        conn.execute(
            "CREATE TABLE token_order (
            pos INTEGER NOT NULL,
            token_id BLOB NOT NULL,
            identity_id BLOB NOT NULL,
            PRIMARY KEY(pos, token_id),
            FOREIGN KEY (token_id) REFERENCES token(id) ON DELETE CASCADE,
            FOREIGN KEY (identity_id) REFERENCES identity(id) ON DELETE CASCADE
        )",
            [],
        )?;

        Ok(())
    }

    /// Saves the user’s custom identity order (the entire list).
    /// This method overwrites whatever was there before.
    pub fn save_token_order(&self, all_ids: Vec<(Identifier, Identifier)>) -> rusqlite::Result<()> {
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

    /// Deletes all local tokens and related entries (identity_token_balances, token_order) in Devnet.
    pub fn delete_all_local_tokens_in_devnet(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        if app_context.network != Network::Devnet {
            return Ok(());
        }
        let network = app_context.network_string();

        let conn = self.conn.lock().unwrap();

        // Delete tokens and cascade deletions in related tables due to foreign keys
        conn.execute("DELETE FROM token WHERE network = ?", params![network])?;

        Ok(())
    }
}
