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
        // Serialize the contract
        let contract_bytes = data_contract
            .serialize_to_bytes_with_platform_version(app_context.platform_version())
            .expect("expected to serialize contract");
        let contract_id = data_contract.id().to_vec();
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
                if let Some(token_id) = data_contract.token_id(token_contract_position) {
                    if let Ok(token_configuration) =
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
                        eprintln!("Deserialization error: {}", e);
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
                        eprintln!("Deserialization error: {}", e);
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
                        eprintln!("Deserialization error: {}", e);
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

        // Build the SQL query with optional limit and offset
        let mut query = String::from("SELECT contract, alias FROM contract WHERE network = ?");
        if limit.is_some() {
            query.push_str(" LIMIT ?");
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
                    eprintln!("Deserialization error: {}", e);
                    // Optionally skip this entry instead of returning an error
                    continue;
                }
            }
        }

        Ok(contracts)
    }

    pub fn remove_contract(
        &self,
        contract_id: &[u8],
        app_context: &AppContext,
    ) -> rusqlite::Result<()> {
        let network = app_context.network.to_string();

        // 1) remove the contract itself
        self.execute(
            "DELETE FROM contract
         WHERE contract_id = ? AND network = ?",
            params![contract_id, network],
        )?;

        Ok(())
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
