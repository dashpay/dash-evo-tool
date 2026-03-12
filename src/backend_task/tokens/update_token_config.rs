use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
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
    Sdk,
    dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem,
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
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        tracing::trace!(
            ?group_info,
            ?identity_token_info,
            ?change_item,
            "Updating token config for a token",
        );
        // Get the existing contract and identity for building the state transition
        // First, fetch the contract from the local database
        let existing_data_contract = &self
            .get_contract_by_id(&identity_token_info.data_contract.contract.id())?
            .ok_or(TaskError::DocumentNotFound)?
            .contract;

        // Then, fetch the identity from the local database
        let identity = self
            .get_identity_by_id(&identity_token_info.identity.identity.id())?
            .ok_or(TaskError::IdentityNotFoundLocally)?;

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
            .map_err(TaskError::from)?;

        // Broadcast the state transition
        let proof_result = state_transition
            .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
            .await
            .map_err(|e| self.log_drive_proof_error(e))?;

        // Log proof result for audit trail
        tracing::info!("TokenConfigUpdate proof result: {}", proof_result);

        // Now update the data contract in the local database
        // The proof result contains an action document, not the updated contract,
        // so we need to fetch the updated contract from the platform
        let data_contract =
            DataContract::fetch(sdk, identity_token_info.data_contract.contract.id())
                .await
                .map_err(TaskError::from)?
                .ok_or(TaskError::DocumentNotFound)?;

        let token = data_contract
            .tokens()
            .get(&identity_token_info.token_position)
            .ok_or(TaskError::TokenPositionNotFound {
                position: identity_token_info.token_position,
            })?;

        // Then replace the contract in the local database
        self.replace_contract(
            identity_token_info.data_contract.contract.id(),
            &data_contract,
        )?;

        self.remove_token(&identity_token_info.token_id)?;

        self.insert_token(
            &identity_token_info.token_id,
            &identity_token_info.token_alias,
            token.clone(),
            &identity_token_info.data_contract.contract.id(),
            identity_token_info.token_position,
        )?;

        // Return success with fee result
        use crate::backend_task::FeeResult;
        use crate::model::fee_estimation::PlatformFeeEstimator;
        let estimated_fee = PlatformFeeEstimator::new().estimate_document_batch(1);
        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::UpdatedTokenConfig(
            change_item.to_string(),
            fee_result,
        ))
    }
}
