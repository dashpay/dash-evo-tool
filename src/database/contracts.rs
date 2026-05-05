use crate::context::AppContext;
use crate::database::Database;
use crate::model::qualified_contract::QualifiedContract;
use bincode::config;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::{DataContract, TokenContractPosition};
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::serialization::{
    PlatformDeserializableWithPotentialValidationFromVersionedStructure,
    PlatformSerializableWithPlatformVersion,
};
use rusqlite::{Connection, Result, params, params_from_iter};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum InsertTokensToo {
    AllTokensShouldBeAdded,
    NoTokensShouldBeAdded,
    SomeTokensShouldBeAdded(Vec<TokenContractPosition>),
}

impl Database {
    pub fn insert_contract_if_not_exists(
        &self,
        data_contract: &DataContract,
        contract_alias: Option<&str>,
        insert_tokens_too: InsertTokensToo,
        app_context: &AppContext,
    ) -> Result<()> {
        let data_contract_id = data_contract.id();
        if app_context.is_system_contract_id(&data_contract_id) {
            tracing::debug!(
                ?data_contract_id,
                ?contract_alias,
                ?insert_tokens_too,
                "Skipping insert for system contract"
            );
            return Ok(());
        }

        // Serialize the contract
        let contract_bytes = data_contract
            .serialize_to_bytes_with_platform_version(app_context.platform_version())
            .expect("expected to serialize contract");
        let contract_id = data_contract_id.to_vec();
        let network = app_context.network.to_string();

        // Insert the contract if it does not exist
        self.execute(
            "INSERT OR IGNORE INTO contract (contract_id, contract, alias, network) VALUES (?, ?, ?, ?)",
            params![contract_id, contract_bytes, contract_alias, network],
        )?;

        // Next, if the contract has tokens, add the tokens
        if !data_contract.tokens().is_empty() {
            let positions = match insert_tokens_too {
                InsertTokensToo::AllTokensShouldBeAdded => {
                    data_contract.tokens().keys().cloned().collect()
                }
                InsertTokensToo::NoTokensShouldBeAdded => {
                    return Ok(());
                }
                InsertTokensToo::SomeTokensShouldBeAdded(positions) => positions,
            };
            for token_contract_position in positions {
                if let Some(token_id) = data_contract.token_id(token_contract_position)
                    && let Ok(token_configuration) =
                        data_contract.expected_token_configuration(token_contract_position)
                {
                    let config = config::standard();
                    let Some(serialized_token_configuration) =
                        bincode::encode_to_vec(token_configuration, config).ok()
                    else {
                        // We should always be able to serialize
                        return Ok(());
                    };
                    let token_name = token_configuration
                        .conventions()
                        .singular_form_by_language_code_or_default("en");
                    self.insert_token(
                        &token_id,
                        token_name,
                        serialized_token_configuration.as_slice(),
                        &data_contract.id(),
                        token_contract_position,
                        app_context,
                    )?;
                }
            }
        }

        Ok(())
    }

    pub fn get_contract_by_id(
        &self,
        contract_id: Identifier,
        app_context: &AppContext,
    ) -> Result<Option<QualifiedContract>> {
        let contract_id_bytes = contract_id.to_vec();
        let network = app_context.network.to_string();

        // Query the contract by ID
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT contract, alias FROM contract WHERE contract_id = ? AND network = ?",
        )?;

        let result = stmt.query_row(params![contract_id_bytes, network], |row| {
            let contract_bytes: Vec<u8> = row.get(0)?;
            let alias: Option<String> = row.get(1)?; // Assuming `alias` can be NULL
            Ok((contract_bytes, alias))
        });

        match result {
            Ok((bytes, alias)) => {
                // Deserialize the DataContract
                match DataContract::versioned_deserialize(
                    &bytes,
                    false,
                    app_context.platform_version(),
                ) {
                    Ok(contract) => {
                        // Construct the QualifiedContract
                        let qualified_contract = QualifiedContract { contract, alias };
                        Ok(Some(qualified_contract))
                    }
                    Err(e) => {
                        // Handle deserialization errors
                        tracing::warn!("Deserialization error: {}", e);
                        Ok(None)
                    }
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn get_unqualified_contract_by_id(
        &self,
        contract_id: Identifier,
        app_context: &AppContext,
    ) -> Result<Option<DataContract>> {
        let contract_id_bytes = contract_id.to_vec();
        let network = app_context.network.to_string();

        // Query the contract by ID
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT contract FROM contract WHERE contract_id = ? AND network = ?")?;

        let result = stmt.query_row(params![contract_id_bytes, network], |row| {
            let contract_bytes: Vec<u8> = row.get(0)?;
            Ok(contract_bytes)
        });

        match result {
            Ok(bytes) => {
                // Deserialize the DataContract
                match DataContract::versioned_deserialize(
                    &bytes,
                    false,
                    app_context.platform_version(),
                ) {
                    Ok(contract) => Ok(Some(contract)),
                    Err(e) => {
                        // Handle deserialization errors
                        tracing::warn!("Deserialization error: {}", e);
                        Ok(None)
                    }
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn replace_contract(
        &self,
        contract_id: Identifier,
        data_contract: &DataContract,
        app_context: &AppContext,
    ) -> Result<()> {
        let contract_bytes = data_contract
            .serialize_to_bytes_with_platform_version(app_context.platform_version())
            .expect("expected to serialize contract");
        let network = app_context.network.to_string();

        // Get the existing contract alias (if any)
        let existing_alias = {
            let conn = self.conn.lock().unwrap();
            conn.query_row(
                "SELECT alias FROM contract WHERE contract_id = ? AND network = ?",
                params![contract_id.to_vec(), network.clone()],
                |row| row.get::<_, Option<String>>(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok::<Option<String>, rusqlite::Error>(None),
                other => Err(other),
            })?
        };

        // Replace the contract
        self.execute(
            "REPLACE INTO contract (contract_id, contract, alias, network) VALUES (?, ?, ?, ?)",
            params![
                contract_id.to_vec(),
                contract_bytes,
                existing_alias,
                network
            ],
        )?;

        Ok(())
    }

    #[allow(dead_code)] // May be used for contract lookup by user-friendly names
    pub fn get_contract_by_alias(
        &self,
        alias: &str,
        app_context: &AppContext,
    ) -> Result<Option<QualifiedContract>> {
        let network = app_context.network.to_string();

        // Query the contract by alias and network
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT contract, alias FROM contract WHERE alias = ? AND network = ?")?;

        let result = stmt.query_row(params![alias, network], |row| {
            let contract_bytes: Vec<u8> = row.get(0)?;
            let contract_alias: Option<String> = row.get(1)?; // Handle potential null values
            Ok((contract_bytes, contract_alias))
        });

        match result {
            Ok((bytes, alias)) => {
                // Deserialize the DataContract
                match DataContract::versioned_deserialize(
                    &bytes,
                    false,
                    app_context.platform_version(),
                ) {
                    Ok(contract) => {
                        // Construct the QualifiedContract
                        let qualified_contract = QualifiedContract { contract, alias };
                        Ok(Some(qualified_contract))
                    }
                    Err(e) => {
                        // Handle deserialization errors
                        tracing::warn!("Deserialization error: {}", e);
                        Ok(None)
                    }
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn get_contracts(
        &self,
        app_context: &AppContext,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<QualifiedContract>> {
        let network = app_context.network.to_string();

        let excluded_contract_ids = app_context
            .system_contract_ids()
            .map(|contract_id| contract_id.to_vec());
        let excluded_contract_placeholders = vec!["?"; excluded_contract_ids.len()].join(", ");

        // Build the SQL query with optional limit and offset. A stable
        // `ORDER BY contract_id` is required: without it, SQLite is free to
        // return rows in any order, so LIMIT/OFFSET pagination would not be
        // deterministic across calls and pages could overlap or skip rows.
        let mut query = format!(
            "SELECT contract, alias FROM contract WHERE network = ? AND contract_id NOT IN ({excluded_contract_placeholders}) ORDER BY contract_id"
        );
        if limit.is_some() {
            query.push_str(" LIMIT ?");
        } else if offset.is_some() {
            query.push_str(" LIMIT -1");
        }
        if offset.is_some() {
            query.push_str(" OFFSET ?");
        }

        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&query)?;

        // Store the limit and offset in variables to extend their lifetimes
        let limit_value;
        let offset_value;

        // Collect parameters for query execution
        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&network];
        for contract_id in &excluded_contract_ids {
            params.push(contract_id);
        }
        if let Some(l) = limit {
            limit_value = l;
            params.push(&limit_value); // Now `limit_value` lives long enough
        }
        if let Some(o) = offset {
            offset_value = o;
            params.push(&offset_value); // Now `offset_value` lives long enough
        }

        let mut rows = stmt.query(params_from_iter(params))?;

        // Collect the results into a Vec<QualifiedContract>
        let mut contracts = Vec::new();
        while let Some(row) = rows.next()? {
            let contract_bytes: Vec<u8> = row.get(0)?;
            let alias: Option<String> = row.get(1)?;

            // Deserialize the DataContract
            match DataContract::versioned_deserialize(
                &contract_bytes,
                false,
                app_context.platform_version(),
            ) {
                Ok(contract) => {
                    contracts.push(QualifiedContract { contract, alias });
                }
                Err(e) => {
                    tracing::error!("Deserialization error: {}", e);
                    // Optionally skip this entry instead of returning an error
                    continue;
                }
            }
        }

        Ok(contracts)
    }

    /// Returns the contract IDs of every contract row persisted on the given
    /// network whose contract blob successfully deserializes. Rows with
    /// malformed contract IDs or unparseable contract bytes are skipped, so
    /// the result is consistent with the visible contract set surfaced by
    /// [`Self::get_contracts`] and [`Self::get_contract_by_id`] (both of
    /// which silently drop malformed rows on `versioned_deserialize`).
    pub fn get_contract_ids_for_network(
        &self,
        app_context: &AppContext,
    ) -> rusqlite::Result<Vec<Identifier>> {
        let network = app_context.network.to_string();
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT contract_id, contract FROM contract WHERE network = ?")?;
        let mut rows = stmt.query(params![network])?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next()? {
            let id_bytes: Vec<u8> = row.get(0)?;
            let contract_bytes: Vec<u8> = row.get(1)?;
            let id = match Identifier::from_bytes(&id_bytes) {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!("Skipping malformed contract id row: {}", e);
                    continue;
                }
            };
            // Only return IDs of rows that round-trip through the same
            // deserialization path as the listing helpers. Rows with valid
            // 32-byte keys but corrupted blobs would otherwise show as
            // "loaded" while never appearing in the visible contract list.
            match DataContract::versioned_deserialize(
                &contract_bytes,
                false,
                app_context.platform_version(),
            ) {
                Ok(_) => ids.push(id),
                Err(e) => {
                    tracing::warn!(?id, "Skipping contract row with malformed blob: {}", e);
                    continue;
                }
            }
        }
        Ok(ids)
    }

    pub fn remove_contract(
        &self,
        contract_id: &[u8],
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        self.remove_contract_for_network(contract_id, &app_context.network)
    }

    /// Network-only variant of [`Self::remove_contract`] for callers (e.g. tests
    /// and free helpers in `context::contract_token_db`) that do not have a
    /// full [`AppContext`] available. Production paths should keep using
    /// [`Self::remove_contract`].
    pub(crate) fn remove_contract_for_network(
        &self,
        contract_id: &[u8],
        network: &Network,
    ) -> rusqlite::Result<()> {
        let network = network.to_string();
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;

        tx.execute(
            "DELETE FROM identity_token_balances
             WHERE network = ?
               AND token_id IN (
                   SELECT id FROM token WHERE data_contract_id = ? AND network = ?
               )",
            params![&network, contract_id, &network],
        )?;
        // `token_order` has no `network` column on purpose — see the
        // invariant on `Database::initialize_token_order_table`. Token
        // metadata/order are single-network-at-a-time surfaces, so this
        // cleanup follows the current network recorded on the token row.
        tx.execute(
            "DELETE FROM token_order
             WHERE token_id IN (
                 SELECT id FROM token WHERE data_contract_id = ? AND network = ?
             )",
            params![contract_id, &network],
        )?;
        tx.execute(
            "DELETE FROM token WHERE data_contract_id = ? AND network = ?",
            params![contract_id, &network],
        )?;
        tx.execute(
            "DELETE FROM contract
             WHERE contract_id = ? AND network = ?",
            params![contract_id, &network],
        )?;

        tx.commit()
    }

    /// Deletes all contracts in Devnet variants and Regtest.
    pub fn remove_all_contracts_in_all_devnets_and_regtest(
        &self,
        conn: &Connection,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "DELETE FROM contract WHERE network LIKE 'devnet%' OR network = 'regtest'",
            [],
        )?;

        Ok(())
    }

    /// Deletes all local tokens and related entries (identity_token_balances, token_order) in Devnet.
    pub fn remove_all_contracts_in_devnet(&self, app_context: &AppContext) -> rusqlite::Result<()> {
        if app_context.network != Network::Devnet {
            return Ok(());
        }
        let network = app_context.network.to_string();

        let conn = self.conn.lock().unwrap();

        // Delete tokens and cascade deletions in related tables due to foreign keys
        conn.execute("DELETE FROM contract WHERE network = ?", params![network])?;

        Ok(())
    }

    /// Updates the alias of a specified contract.
    pub fn set_contract_alias(
        &self,
        identifier: &Identifier,
        new_alias: Option<&str>,
    ) -> rusqlite::Result<()> {
        let id = identifier.to_vec();
        let conn = self.conn.lock().unwrap();

        let rows_updated = conn.execute(
            "UPDATE contract SET alias = ? WHERE contract_id = ?",
            params![new_alias, id],
        )?;

        if rows_updated == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }

        Ok(())
    }

    #[allow(dead_code)] // May be used for retrieving user-friendly contract names
    pub fn get_contract_alias(&self, identifier: &Identifier) -> rusqlite::Result<Option<String>> {
        let id = identifier.to_vec();
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare("SELECT alias FROM contract WHERE contract_id = ?")?;
        let alias: Option<String> = stmt.query_row(params![id], |row| row.get(0)).ok();

        Ok(alias)
    }

    /// Perform the database migration to change the field "name" to "alias" in the contract table.
    pub fn change_contract_name_to_alias(&self, conn: &Connection) -> rusqlite::Result<()> {
        // Check if the column "name" exists
        let mut stmt = conn.prepare("PRAGMA table_info(contract)")?;
        let mut columns = stmt.query([])?;

        let mut name_column_exists = false;
        while let Some(row) = columns.next()? {
            let column_name: String = row.get(1)?;
            if column_name == "name" {
                name_column_exists = true;
                break;
            }
        }

        // If the column "name" exists, rename it to "alias"
        if name_column_exists {
            conn.execute("ALTER TABLE contract RENAME COLUMN name TO alias", [])?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::database::test_helpers::create_test_database;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::platform::Identifier;
    use rusqlite::params;

    fn id(byte: u8) -> Identifier {
        Identifier::from_bytes(&[byte; 32]).expect("32 bytes is a valid identifier")
    }

    fn count_rows(conn: &rusqlite::Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> i64 {
        conn.query_row(sql, rusqlite::params_from_iter(params), |row| row.get(0))
            .expect("count query")
    }

    /// Removing a non-system (user) contract via `remove_contract_for_network`
    /// must cascade to every dependent row (tokens, token_order, identity
    /// token balances) for that contract on the same network in a single
    /// transaction. Production behaviour for unrelated rows on the same
    /// network must be preserved.
    #[test]
    fn remove_contract_cascades_dependent_rows() {
        let db = create_test_database().expect("create in-memory db");
        let network = Network::Testnet;
        let network_str = network.to_string();

        let contract_id = id(0xC1);
        let other_contract_id = id(0xC2);
        let token_id = id(0xD1);
        let other_token_id = id(0xD2);
        let identity_id = id(0xA1);

        // Direct SQL inserts — we don't need real serialized contracts for
        // this test because we only verify the cascade DELETE behaviour.
        // The test DB enables foreign keys, so the rows we reference must
        // exist (e.g. an `identity` row backing the FK in
        // `identity_token_balances`).
        let conn = db.shared_connection();
        let conn = conn.lock().unwrap();

        // Identity row to satisfy the FK from identity_token_balances.
        conn.execute(
            "INSERT INTO identity (id, is_local, network) VALUES (?, ?, ?)",
            params![identity_id.to_vec(), 1i64, &network_str],
        )
        .unwrap();

        // Two user-contract rows on the same network.
        for cid in [&contract_id, &other_contract_id] {
            conn.execute(
                "INSERT INTO contract (contract_id, contract, alias, network) VALUES (?, ?, ?, ?)",
                params![
                    cid.to_vec(),
                    b"placeholder".to_vec(),
                    Option::<String>::None,
                    &network_str
                ],
            )
            .unwrap();
        }

        // Token rows: one belonging to the contract under test, one to the
        // unrelated contract on the same network.
        conn.execute(
            "INSERT INTO token (id, token_alias, token_config, data_contract_id, token_position, network)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                token_id.to_vec(),
                "alias",
                b"cfg".to_vec(),
                contract_id.to_vec(),
                0u16,
                &network_str
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO token (id, token_alias, token_config, data_contract_id, token_position, network)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                other_token_id.to_vec(),
                "alias-other",
                b"cfg".to_vec(),
                other_contract_id.to_vec(),
                0u16,
                &network_str
            ],
        )
        .unwrap();

        // identity_token_balances rows for both tokens.
        conn.execute(
            "INSERT INTO identity_token_balances (token_id, identity_id, balance, network)
             VALUES (?, ?, ?, ?)",
            params![token_id.to_vec(), identity_id.to_vec(), 0u64, &network_str],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO identity_token_balances (token_id, identity_id, balance, network)
             VALUES (?, ?, ?, ?)",
            params![
                other_token_id.to_vec(),
                identity_id.to_vec(),
                0u64,
                &network_str
            ],
        )
        .unwrap();

        // token_order rows for both tokens.
        conn.execute(
            "INSERT INTO token_order (pos, token_id, identity_id) VALUES (?, ?, ?)",
            params![0i64, token_id.to_vec(), identity_id.to_vec()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO token_order (pos, token_id, identity_id) VALUES (?, ?, ?)",
            params![1i64, other_token_id.to_vec(), identity_id.to_vec()],
        )
        .unwrap();

        // Sanity: every row exists before removal.
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM contract WHERE contract_id = ? AND network = ?",
                &[&contract_id.to_vec(), &network_str]
            ),
            1
        );
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM token WHERE data_contract_id = ? AND network = ?",
                &[&contract_id.to_vec(), &network_str]
            ),
            1
        );
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM identity_token_balances WHERE token_id = ?",
                &[&token_id.to_vec()]
            ),
            1
        );
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM token_order WHERE token_id = ?",
                &[&token_id.to_vec()]
            ),
            1
        );

        // Drop the lock so `remove_contract_for_network` can re-acquire it.
        drop(conn);

        db.remove_contract_for_network(contract_id.as_bytes(), &network)
            .expect("remove_contract_for_network succeeds for user contract");

        let conn = db.shared_connection();
        let conn = conn.lock().unwrap();

        // The targeted contract and all its dependents are gone.
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM contract WHERE contract_id = ? AND network = ?",
                &[&contract_id.to_vec(), &network_str]
            ),
            0,
            "contract row should be deleted"
        );
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM token WHERE data_contract_id = ? AND network = ?",
                &[&contract_id.to_vec(), &network_str]
            ),
            0,
            "token rows tied to the contract should be deleted"
        );
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM identity_token_balances WHERE token_id = ?",
                &[&token_id.to_vec()]
            ),
            0,
            "identity_token_balances rows for the removed token should be deleted"
        );
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM token_order WHERE token_id = ?",
                &[&token_id.to_vec()]
            ),
            0,
            "token_order rows for the removed token should be deleted"
        );

        // Unrelated rows on the same network must remain untouched.
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM contract WHERE contract_id = ? AND network = ?",
                &[&other_contract_id.to_vec(), &network_str]
            ),
            1,
            "unrelated contract row must remain"
        );
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM token WHERE data_contract_id = ? AND network = ?",
                &[&other_contract_id.to_vec(), &network_str]
            ),
            1,
            "unrelated token row must remain"
        );
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM identity_token_balances WHERE token_id = ?",
                &[&other_token_id.to_vec()]
            ),
            1,
            "unrelated identity_token_balances row must remain"
        );
        assert_eq!(
            count_rows(
                &conn,
                "SELECT COUNT(*) FROM token_order WHERE token_id = ?",
                &[&other_token_id.to_vec()]
            ),
            1,
            "unrelated token_order row must remain"
        );
    }
}
