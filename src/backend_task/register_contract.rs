use dash_sdk::{
    dpp::{
        data_contract::accessors::v0::DataContractV0Getters,
        identity::{accessors::IdentityGettersV0, KeyType, Purpose, SecurityLevel},
    },
    platform::{transition::put_contract::PutContract, DataContract, IdentityPublicKey},
    Sdk,
};

use crate::{context::AppContext, model::qualified_identity::QualifiedIdentity};

use super::BackendTaskSuccessResult;

impl AppContext {
    pub async fn register_data_contract(
        &self,
        data_contract: DataContract,
        alias: String,
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        match data_contract
            .put_to_platform_and_wait_for_response(&sdk, signing_key.clone(), &identity, None)
            .await
        {
            Ok(returned_contract) => {
                let optional_alias = match alias.is_empty() {
                    true => None,
                    false => Some(alias),
                };
                self.db
                    .insert_contract_if_not_exists(
                        &returned_contract,
                        optional_alias.as_deref(),
                        self,
                    )
                    .map_err(|e| {
                        format!(
                            "Error inserting contract into the database: {}",
                            e.to_string()
                        )
                    })?;
                Ok(BackendTaskSuccessResult::Message(
                    "DataContract successfully registered".to_string(),
                ))
            }
            Err(e) => Err(format!("Failed to register DataContract: {:?}", e)),
        }
    }
}
