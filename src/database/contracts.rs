use crate::context::AppContext;
use crate::database::Database;
use crate::model::qualified_contract::QualifiedContract;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::serialization::{
    PlatformDeserializableWithPotentialValidationFromVersionedStructure,
    PlatformSerializableWithPlatformVersion,
};
use rusqlite::{params, params_from_iter, Result};

impl Database {
    pub fn insert_contract_if_not_exists(
        &self,
        data_contract: &DataContract,
        contract_name: Option<&str>,
        app_context: &AppContext,
    ) -> Result<()> {
        // Serialize the contract
        let contract_bytes = data_contract
            .serialize_to_bytes_with_platform_version(app_context.platform_version)
            .expect("expected to serialize contract");
        let contract_id = data_contract.id().to_vec();
        let network = app_context.network_string();

        // Insert the contract if it does not exist
        self.execute(
            "INSERT OR IGNORE INTO contract (contract_id, contract, name, network) VALUES (?, ?, ?, ?)",
            params![contract_id, contract_bytes, contract_name, network],
        )?;

        // Next, if the contract has tokens, add the tokens to identity_token_balances
        if !data_contract.tokens().is_empty() {
            for token in data_contract.tokens().iter() {
                let wallets = app_context.wallets.read().unwrap();
                let identities = self.get_local_qualified_identities(app_context, &wallets)?;
                if let Some(token_id) = data_contract.token_id(*token.0) {
                    for identity in identities {
                        let balance = if data_contract.owner_id() == identity.identity.id() {
                            token.1.base_supply()
                        } else {
                            0
                        };
                        let _ = self
                            .insert_identity_token_balance(
                                &token_id,
                                &identity.identity.id(),
                                balance,
                                &data_contract.id(),
                                *token.0,
                                app_context,
                            )
                            .map_err(|e| {
                                format!(
                                    "Failed to insert token balance into local database: {:?}",
                                    e
                                )
                            });
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
        let network = app_context.network_string();

        // Query the contract by ID
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT contract, name FROM contract WHERE contract_id = ? AND network = ?")?;

        let result = stmt.query_row(params![contract_id_bytes, network], |row| {
            let contract_bytes: Vec<u8> = row.get(0)?;
            let name: Option<String> = row.get(1)?; // Assuming `name` can be NULL
            Ok((contract_bytes, name))
        });

        match result {
            Ok((bytes, name)) => {
                // Deserialize the DataContract
                match DataContract::versioned_deserialize(
                    &bytes,
                    false,
                    app_context.platform_version,
                ) {
                    Ok(contract) => {
                        // Construct the QualifiedContract
                        let qualified_contract = QualifiedContract {
                            contract,
                            alias: name,
                        };
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
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_contract_by_name(
        &self,
        name: &str,
        app_context: &AppContext,
    ) -> Result<Option<QualifiedContract>> {
        let network = app_context.network_string();

        // Query the contract by name and network
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT contract, name FROM contract WHERE name = ? AND network = ?")?;

        let result = stmt.query_row(params![name, network], |row| {
            let contract_bytes: Vec<u8> = row.get(0)?;
            let contract_name: Option<String> = row.get(1)?; // Handle potential null values
            Ok((contract_bytes, contract_name))
        });

        match result {
            Ok((bytes, alias)) => {
                // Deserialize the DataContract
                match DataContract::versioned_deserialize(
                    &bytes,
                    false,
                    app_context.platform_version,
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
            Err(e) => Err(e.into()),
        }
    }

    pub fn get_contracts(
        &self,
        app_context: &AppContext,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<QualifiedContract>> {
        let network = app_context.network_string();

        // Build the SQL query with optional limit and offset
        let mut query = String::from("SELECT contract, name FROM contract WHERE network = ?");
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
                app_context.platform_version,
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
        let network = app_context.network_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM contract WHERE contract_id = ? AND network = ?",
            rusqlite::params![contract_id, network],
        )?;
        Ok(())
    }
}
