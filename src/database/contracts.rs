use crate::context::AppContext;
use crate::database::Database;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::serialization::{
    PlatformDeserializableWithPotentialValidationFromVersionedStructure,
    PlatformSerializableWithPlatformVersion,
};
use rusqlite::{params, Result};
impl Database {
    pub fn insert_contract_if_not_exists(
        &self,
        data_contract: &DataContract,
        contract_name: Option<&str>,
        app_context: &AppContext,
    ) -> Result<()> {
        // Serialize the contract
        let contract_bytes = data_contract
            .serialize_to_bytes_with_platform_version(&app_context.platform_version)
            .expect("expected to serialize contract");
        let contract_id = data_contract.id().to_vec();
        let network = app_context.network_string();

        // Insert the contract if it does not exist
        self.conn.execute(
            "INSERT OR IGNORE INTO contract (contract_id, contract, name, network) VALUES (?, ?, ?, ?)",
            params![contract_id, contract_bytes, contract_name, network],
        )?;
        Ok(())
    }

    pub fn get_contract_by_id(
        &self,
        contract_id: Identifier,
        app_context: &AppContext,
    ) -> Result<Option<DataContract>> {
        let contract_id_bytes = contract_id.to_vec();
        let network = app_context.network_string();

        // Query the contract by ID
        let mut stmt = self
            .conn
            .prepare("SELECT contract FROM contract WHERE contract_id = ? AND network = ?")?;

        let contract_bytes: Result<Vec<u8>> =
            stmt.query_row(params![contract_id_bytes, network], |row| row.get(0));

        match contract_bytes {
            Ok(bytes) => {
                // Deserialize and return the contract
                match DataContract::versioned_deserialize(
                    &bytes,
                    false,
                    &app_context.platform_version,
                ) {
                    Ok(contract) => Ok(Some(contract)),
                    Err(e) => {
                        // Log the deserialization error if needed
                        eprintln!("Deserialization error: {}", e);
                        Ok(None)
                    }
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn get_contract_by_name(
        &self,
        name: &str,
        app_context: &AppContext,
    ) -> Result<Option<DataContract>> {
        let network = app_context.network_string();

        // Query the contract by name
        let mut stmt = self
            .conn
            .prepare("SELECT contract FROM contract WHERE name = ? AND network = ?")?;

        let contract_bytes: Result<Vec<u8>> =
            stmt.query_row(params![name, network], |row| row.get(0));

        match contract_bytes {
            Ok(bytes) => {
                // Deserialize and return the contract
                match DataContract::versioned_deserialize(
                    &bytes,
                    false,
                    &app_context.platform_version,
                ) {
                    Ok(contract) => Ok(Some(contract)),
                    Err(e) => {
                        // Log the deserialization error if needed
                        eprintln!("Deserialization error: {}", e);
                        Ok(None)
                    }
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
