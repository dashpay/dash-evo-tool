use super::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::ui::tokens::tokens_screen::IdentityTokenBalance;
use dash_sdk::dpp::state_transition::batch_transition::methods::StateTransitionCreationOptions;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::dpp::state_transition::StateTransitionSigningOptions;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::{DataContract, Fetch, IdentityPublicKey};
use dash_sdk::{
    dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem,
    platform::transition::fungible_tokens::config_update::TokenConfigUpdateTransitionBuilder, Sdk,
};

impl AppContext {
    pub async fn update_token_config(
        &self,
        identity_token_balance: IdentityTokenBalance,
        change_item: TokenConfigurationChangeItem,
        signing_key: &IdentityPublicKey,
        public_note: Option<String>,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Get the existing contract and identity for building the state transition
        // First, fetch the contract from the local database
        let existing_data_contract = &self
            .get_contract_by_id(&identity_token_balance.data_contract_id)
            .map_err(|e| {
                format!(
                    "Error getting contract by ID {}: {}",
                    identity_token_balance.data_contract_id, e
                )
            })?
            .ok_or_else(|| {
                format!(
                    "Contract with ID {} not found",
                    identity_token_balance.data_contract_id
                )
            })?
            .contract;

        // Then, fetch the identity from the local database
        let identity = self
            .get_identity_by_id(&identity_token_balance.identity_id)
            .map_err(|e| {
                format!(
                    "Error getting identity by ID {}: {}",
                    identity_token_balance.identity_id, e
                )
            })?
            .ok_or_else(|| {
                format!(
                    "Identity with ID {} not found",
                    identity_token_balance.identity_id
                )
            })?;

        // Create the TokenConfigUpdateTransition
        let mut builder = TokenConfigUpdateTransitionBuilder::new(
            existing_data_contract,
            identity_token_balance.token_position,
            identity_token_balance.identity_id,
            change_item.clone(),
            None,
        );

        // Add the optional public note
        if let Some(public_note) = &public_note {
            builder = builder.with_public_note(public_note.clone());
        }

        let options = if self.developer_mode {
            Some(StateTransitionCreationOptions {
                signing_options: StateTransitionSigningOptions {
                    allow_signing_with_any_security_level: true,
                    allow_signing_with_any_purpose: true,
                },
                batch_feature_version: None,
                method_feature_version: None,
                base_feature_version: None,
            })
        } else {
            None
        };

        // Sign the state transition
        let state_transition = builder
            .sign(sdk, &signing_key, &identity, self.platform_version, options)
            .await
            .map_err(|e| {
                format!(
                    "Error signing Token Config Update transition: {}",
                    e.to_string()
                )
            })?;

        // Broadcast the state transition
        let _proof_result = state_transition
            .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
            .await
            .map_err(|e| format!("Error broadcasting Token Config Update transition: {}", e))?;

        // Now update the data contract in the local database
        // First, fetch the updated contract from the platform
        let data_contract = DataContract::fetch(sdk, identity_token_balance.data_contract_id)
            .await
            .map_err(|e| format!("Error fetching contract from platform: {}", e.to_string()))?
            .ok_or_else(|| {
                format!(
                    "Contract with ID {} not found on platform",
                    identity_token_balance.data_contract_id
                )
            })?;

        // Then replace the contract in the local database
        self.replace_contract(identity_token_balance.data_contract_id, &data_contract)
            .map_err(|e| format!("Error replacing contract in local database: {}", e))?;

        // Return success
        Ok(BackendTaskSuccessResult::Message(format!(
            "Successfully updated token config item: {}",
            change_item.to_string()
        )))
    }
}
