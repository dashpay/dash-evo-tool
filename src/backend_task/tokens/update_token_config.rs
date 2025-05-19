use super::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::ui::tokens::tokens_screen::IdentityTokenBalance;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::{DataContract, Fetch, IdentityPublicKey};
use dash_sdk::{
    dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem,
    platform::transition::fungible_tokens::config_update::TokenConfigUpdateTransitionBuilder,
    Error, Sdk,
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
        );

        // Add the optional public note
        if let Some(public_note) = &public_note {
            builder = builder.with_public_note(public_note.clone());
        }

        let options = self.state_transition_options();

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

        // Now update the token in the local database
        // First, fetch the updated contract from Platform
        let data_contract = DataContract::fetch(sdk, identity_token_balance.data_contract_id)
            .await
            .map_err(|e| format!("Error fetching contract from platform: {}", e.to_string()))?
            .ok_or_else(|| {
                format!(
                    "Contract with ID {} not found on platform",
                    identity_token_balance.data_contract_id
                )
            })?;

        let token = data_contract
            .tokens()
            .get(&identity_token_balance.token_position)
            .ok_or_else(|| {
                format!(
                    "Token with position {} not found in contract",
                    identity_token_balance.token_position
                )
            })?;

        // Then replace the contract in the local database
        self.replace_contract(identity_token_balance.data_contract_id, &data_contract)
            .map_err(|e| {
                format!(
                    "Error replacing contract in local database: {}",
                    e.to_string()
                )
            })?;

        self.remove_token(&identity_token_balance.token_id)
            .map_err(|e| {
                format!(
                    "Error removing token from local database: {}",
                    e.to_string()
                )
            })?;

        self.insert_token(
            &identity_token_balance.token_id,
            &identity_token_balance.token_alias,
            token.clone(),
            &identity_token_balance.data_contract_id,
            identity_token_balance.token_position,
        )
        .map_err(|e| {
            format!(
                "Error inserting token into local database: {}",
                e.to_string()
            )
        })?;

        // Return success
        Ok(BackendTaskSuccessResult::Message(format!(
            "Successfully updated token config item: {}",
            change_item.to_string()
        )))
    }
}
