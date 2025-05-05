use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::IdentityPublicKey;
use dash_sdk::{
    dpp::data_contract::associated_token::token_configuration_item::TokenConfigurationChangeItem,
    platform::transition::fungible_tokens::config_update::TokenConfigUpdateTransitionBuilder, Sdk,
};
use tokio::sync::mpsc;

use super::BackendTaskSuccessResult;
use crate::app::TaskResult;
use crate::context::AppContext;
use crate::ui::tokens::tokens_screen::IdentityTokenBalance;

impl AppContext {
    pub async fn update_token_config(
        &self,
        identity_token_balance: IdentityTokenBalance,
        change_items: Vec<TokenConfigurationChangeItem>,
        signing_key: &IdentityPublicKey,
        public_note: Option<String>,
        sdk: &Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let data_contract = &self
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

        for change_item in change_items.iter() {
            let mut builder = TokenConfigUpdateTransitionBuilder::new(
                data_contract,
                identity_token_balance.token_position,
                identity_token_balance.identity_id,
                change_item.clone(),
                None,
            );

            if let Some(public_note) = &public_note {
                builder = builder.with_public_note(public_note.clone());
            }

            let state_transition = builder
                .sign(sdk, &signing_key, &identity, self.platform_version)
                .await
                .map_err(|e| {
                    format!(
                        "Error signing Token Config Update transition: {}",
                        e.to_string()
                    )
                })?;

            let _proof_result = state_transition
                .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
                .await
                .map_err(|e| format!("Error broadcasting Token Config Update transition: {}", e))?;

            let _ = sender
                .send(TaskResult::Success(BackendTaskSuccessResult::Message(
                    format!("Successfully updated {:?}", change_item),
                )))
                .await;
        }

        Ok(BackendTaskSuccessResult::Message(
            "Successfully updated all token config items".to_string(),
        ))
    }
}
