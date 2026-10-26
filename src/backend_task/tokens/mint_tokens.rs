use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::token::MintRecipient;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::group::group_action_status::GroupActionStatus;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::tokens::transitions::MintResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use std::sync::Arc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn mint_tokens(
        &self,
        sending_identity: &QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        amount: u64,
        optional_recipient: Option<Identifier>,
        group_info: Option<GroupStateTransitionInfoStatus>,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // The identity the tokens go to: the chosen one when the token allows
        // choosing, otherwise its configured destination.
        let mint_recipient = data_contract
            .expected_token_configuration(token_position)
            .ok()
            .and_then(|config| {
                MintRecipient::resolve(config.distribution_rules(), optional_recipient).identity()
            });
        let actor = self.token_actor(sending_identity).await?;
        let common = self.token_action_common(data_contract.clone(), token_position, &signing_key);

        self.execute_token_op(
            actor.mint(common, amount, optional_recipient, public_note, group_info),
            |result| {
                // Using the result, update the balance of the recipient identity
                if let Some(token_id) = data_contract.token_id(token_position) {
                    match result {
                        // Standard mint result - direct balance update
                        MintResult::TokenBalance(identity_id, amount) => {
                            if let Err(e) =
                                self.insert_token_identity_balance(&token_id, &identity_id, amount)
                            {
                                tracing::error!("Failed to update token balance: {}", e);
                            }
                        }

                        // Historical document - extract recipient and amount from document
                        MintResult::HistoricalDocument(document) => {
                            if let (Some(recipient_value), Some(amount_value)) =
                                (document.get("recipientId"), document.get("amount"))
                                && let (Value::Identifier(recipient_bytes), Value::U64(amount)) =
                                    (recipient_value, amount_value)
                                && let Ok(recipient_id) = Identifier::from_bytes(recipient_bytes)
                                && let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &recipient_id,
                                    *amount,
                                )
                            {
                                tracing::error!(
                                    "Failed to update token balance from historical document: {}",
                                    e
                                );
                            }
                        }

                        // Group action with document - assume completed if document exists
                        MintResult::GroupActionWithDocument(_, Some(document)) => {
                            if let (Some(recipient_value), Some(amount_value)) =
                                (document.get("recipientId"), document.get("amount"))
                                && let (Value::Identifier(recipient_bytes), Value::U64(amount)) =
                                    (recipient_value, amount_value)
                                && let Ok(recipient_id) = Identifier::from_bytes(recipient_bytes)
                                && let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &recipient_id,
                                    *amount,
                                )
                            {
                                tracing::error!(
                                    "Failed to update token balance from group action document: {}",
                                    e
                                );
                            }
                        }

                        // Group action with balance - only update if action is closed
                        MintResult::GroupActionWithBalance(_, status, Some(amount)) => {
                            if matches!(status, GroupActionStatus::ActionClosed)
                                && let Some(recipient_id) = mint_recipient
                            {
                                if let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &recipient_id,
                                    amount,
                                ) {
                                    tracing::error!(
                                        "Failed to update token balance from group action: {}",
                                        e
                                    );
                                }
                            }
                        }

                        // Other variants don't require balance updates
                        _ => {}
                    }
                }
            },
            BackendTaskSuccessResult::MintedTokens,
        )
        .await
    }
}
