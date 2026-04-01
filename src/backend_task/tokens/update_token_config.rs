use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::proof_log_item::RequestType;
use crate::ui::tokens::tokens_screen::IdentityTokenInfo;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
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
            .ok_or(TaskError::DataContractNotFound)?
            .contract;

        // Then, fetch the identity from the local database
        let identity = self
            .get_identity_by_id(&identity_token_info.identity.identity.id())?
            .ok_or(TaskError::IdentityNotFoundLocally)?;

        let data_contract_arc = Arc::new(existing_data_contract.clone());

        let platform_wallet = self.platform_wallet_for_identity(&identity)?;
        let token_wallet = platform_wallet.tokens();

        let _result = token_wallet
            .update_config_with_signer(
                data_contract_arc,
                identity_token_info.token_position,
                identity_token_info.identity.identity.id(),
                change_item.clone(),
                signing_key,
                &identity,
                public_note,
                group_info,
                self.state_transition_options(),
            )
            .await
            .map_err(|e| self.log_drive_proof_error(e, RequestType::BroadcastStateTransition))?;

        // Log proof result for audit trail
        tracing::info!("TokenConfigUpdate completed successfully");

        // Now update the data contract in the local database
        // The proof result contains an action document, not the updated contract,
        // so we need to fetch the updated contract from the platform
        let data_contract =
            DataContract::fetch(sdk, identity_token_info.data_contract.contract.id())
                .await
                .map_err(TaskError::from)?
                .ok_or(TaskError::DataContractNotFound)?;

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

        // insert_token uses upsert (ON CONFLICT DO UPDATE), so a separate
        // remove_token is not needed and avoids a window where the token is
        // missing if insert_token were to fail after remove_token.
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
