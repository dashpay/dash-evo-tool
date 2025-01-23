use dash_sdk::{
    dpp::identity::{accessors::IdentityGettersV0, KeyType, Purpose, SecurityLevel},
    platform::{transition::put_contract::PutContract, DataContract},
    Sdk,
};

use crate::{context::AppContext, model::qualified_identity::QualifiedIdentity};

use super::BackendTaskSuccessResult;

impl AppContext {
    pub async fn register_data_contract(
        data_contract: DataContract,
        identity: QualifiedIdentity,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        let public_key = identity
            .identity
            .get_first_public_key_matching(
                // is it correct purpose and security level? should move this to the screen and only allow identities that contain the right key
                Purpose::AUTHENTICATION,
                [SecurityLevel::MASTER, SecurityLevel::CRITICAL].into(),
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
            Ok(_returned_contract) => Ok(BackendTaskSuccessResult::Message(
                "DataContract successfully registered".to_string(),
            )),
            Err(e) => Err(format!("Failed to register DataContract: {:?}", e)),
        }
    }
}
