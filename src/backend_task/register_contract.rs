use dash_sdk::{
    dpp::identity::{accessors::IdentityGettersV0, KeyType, Purpose, SecurityLevel},
    platform::{transition::put_contract::PutContract, DataContract},
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
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        let public_key = identity
            .identity
            .get_first_public_key_matching(
                Purpose::AUTHENTICATION,
                [SecurityLevel::CRITICAL, SecurityLevel::HIGH].into(),
                KeyType::all_key_types().into(),
                false,
            )
            .ok_or_else(|| {
                "No public key found for the given identity that can register contracts".to_string()
            })?;

        match data_contract
            .put_to_platform_and_wait_for_response(&sdk, public_key.clone(), &identity)
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
