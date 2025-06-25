use super::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::ui::tokens::tokens_screen::IdentityTokenInfo;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::tokens::builders::config_update::TokenConfigUpdateTransitionBuilder;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::{DataContract, Fetch, IdentityPublicKey};
use dash_sdk::{
    dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem,
    Error, Sdk,
};
use std::sync::Arc;

impl AppContext {
    pub async fn update_token_config(
        &self,
        identity_token_info: IdentityTokenInfo,
        change_item: TokenConfigurationChangeItem,
        signing_key: &IdentityPublicKey,
        public_note: Option<String>,
        group_info: Option<GroupStateTransitionInfoStatus>,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        tracing::trace!(
            ?group_info,
            ?identity_token_info,
            ?change_item,
            "Updating token config for a token",
        );
        // Get the existing contract and identity for building the state transition
        // First, fetch the contract from the local database
        let existing_data_contract = &self
            .get_contract_by_id(&identity_token_info.data_contract.contract.id())
            .map_err(|e| {
                format!(
                    "Error getting contract by ID {}: {}",
                    identity_token_info.data_contract.contract.id(),
                    e
                )
            })?
            .ok_or_else(|| {
                format!(
                    "Contract with ID {} not found",
                    identity_token_info.data_contract.contract.id()
                )
            })?
            .contract;

        // Then, fetch the identity from the local database
        let identity = self
            .get_identity_by_id(&identity_token_info.identity.identity.id())
            .map_err(|e| {
                format!(
                    "Error getting identity by ID {}: {}",
                    identity_token_info.identity.identity.id(),
                    e
                )
            })?
            .ok_or_else(|| {
                format!(
                    "Identity with ID {} not found",
                    identity_token_info.identity.identity.id()
                )
            })?;

        let data_contract_arc = Arc::new(existing_data_contract.clone());

        // Create the TokenConfigUpdateTransition
        let mut builder = TokenConfigUpdateTransitionBuilder::new(
            data_contract_arc,
            identity_token_info.token_position,
            identity_token_info.identity.identity.id(),
            change_item.clone(),
        );

        // Add the optional public note
        if let Some(public_note) = &public_note {
            builder = builder.with_public_note(public_note.clone());
        }

        if let Some(group_info) = group_info {
            builder = builder.with_using_group_info(group_info);
        }

        if let Some(options) = self.state_transition_options() {
            builder = builder.with_state_transition_creation_options(options);
        }

        // Sign the state transition
        let state_transition = builder
            .sign(sdk, signing_key, &identity, self.platform_version())
            .await
            .map_err(|e| format!("Error signing Token Config Update transition: {}", e))?;

        // Broadcast the state transition
        let _proof_result = state_transition
            .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
            .await
            .map_err(|e| match e {
                Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                    self.db
                        .insert_proof_log_item(ProofLogItem {
                            request_type: RequestType::BroadcastStateTransition,
                            request_bytes: vec![],
                            verification_path_query_bytes: vec![],
                            height: block_info.height,
                            time_ms: block_info.time_ms,
                            proof_bytes,
                            error: Some(proof_error.to_string()),
                        })
                        .ok();
                    format!(
                        "Error broadcasting Update token config transition: {}, proof error logged",
                        proof_error
                    )
                }
                e => format!("Error broadcasting Update token config transition: {}", e),
            })?;

        // Now update the data contract in the local database
        // First, fetch the updated contract from the platform
        let data_contract =
            DataContract::fetch(sdk, identity_token_info.data_contract.contract.id())
                .await
                .map_err(|e| format!("Error fetching contract from platform: {}", e))?
                .ok_or_else(|| {
                    format!(
                        "Contract with ID {} not found on platform",
                        identity_token_info.data_contract.contract.id()
                    )
                })?;

        let token = data_contract
            .tokens()
            .get(&identity_token_info.token_position)
            .ok_or_else(|| {
                format!(
                    "Token with position {} not found in contract",
                    identity_token_info.token_position
                )
            })?;

        // Then replace the contract in the local database
        self.replace_contract(
            identity_token_info.data_contract.contract.id(),
            &data_contract,
        )
        .map_err(|e| format!("Error replacing contract in local database: {}", e))?;

        self.remove_token(&identity_token_info.token_id)
            .map_err(|e| format!("Error removing token from local database: {}", e))?;

        self.insert_token(
            &identity_token_info.token_id,
            &identity_token_info.token_alias,
            token.clone(),
            &identity_token_info.data_contract.contract.id(),
            identity_token_info.token_position,
        )
        .map_err(|e| format!("Error inserting token into local database: {}", e))?;

        // Return success
        Ok(BackendTaskSuccessResult::Message(format!(
            "Successfully updated token config item: {}",
            change_item
        )))
    }
}
