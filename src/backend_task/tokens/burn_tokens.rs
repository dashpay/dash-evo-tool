use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::group::group_action_status::GroupActionStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::tokens::builders::burn::TokenBurnTransitionBuilder;
use dash_sdk::platform::tokens::transitions::BurnResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::Sdk;
use std::sync::Arc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn burn_tokens(
        &self,
        owner_identity: &QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        amount: u64,
        group_info: Option<GroupStateTransitionInfoStatus>,
        sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let mut builder = TokenBurnTransitionBuilder::new(
            data_contract.clone(),
            token_position,
            owner_identity.identity.id(),
            amount,
        );

        if let Some(note) = public_note {
            builder = builder.with_public_note(note);
        }

        if let Some(group_info) = group_info {
            builder = builder.with_using_group_info(group_info);
        }

        let maybe_options = self.state_transition_options();
        if let Some(options) = maybe_options {
            builder = builder.with_state_transition_creation_options(options);
        }

        let result = sdk
            .token_burn(builder, &signing_key, owner_identity)
            .await
            .map_err(|e| self.log_drive_proof_error(e))?;

        // Using the result, update the balance of the owner identity
        if let Some(token_id) = data_contract.token_id(token_position) {
            match result {
                // Standard burn result - direct balance update
                BurnResult::TokenBalance(identity_id, amount) => {
                    if let Err(e) =
                        self.insert_token_identity_balance(&token_id, &identity_id, amount)
                    {
                        tracing::error!("Failed to update token balance: {}", e);
                    }
                }

                // Historical document - extract owner and amount from document
                BurnResult::HistoricalDocument(document) => {
                    if let (Some(owner_value), Some(amount_value)) =
                        (document.get("ownerId"), document.get("amount"))
                        && let (Value::Identifier(owner_bytes), Value::U64(amount)) =
                            (owner_value, amount_value)
                        && let Ok(owner_id) = Identifier::from_bytes(owner_bytes)
                        && let Err(e) =
                            self.insert_token_identity_balance(&token_id, &owner_id, *amount)
                    {
                        tracing::error!(
                            "Failed to update token balance from historical document: {}",
                            e
                        );
                    }
                }

                // Group action with document - assume completed if document exists
                BurnResult::GroupActionWithDocument(_, Some(document)) => {
                    if let (Some(owner_value), Some(amount_value)) =
                        (document.get("ownerId"), document.get("amount"))
                        && let (Value::Identifier(owner_bytes), Value::U64(amount)) =
                            (owner_value, amount_value)
                        && let Ok(owner_id) = Identifier::from_bytes(owner_bytes)
                        && let Err(e) =
                            self.insert_token_identity_balance(&token_id, &owner_id, *amount)
                    {
                        tracing::error!(
                            "Failed to update token balance from group action document: {}",
                            e
                        );
                    }
                }

                // Group action with balance - only update if action is closed
                BurnResult::GroupActionWithBalance(_, status, Some(amount)) => {
                    if matches!(status, GroupActionStatus::ActionClosed) {
                        let owner_id = owner_identity.identity.id();
                        if let Err(e) =
                            self.insert_token_identity_balance(&token_id, &owner_id, amount)
                        {
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

        // Return success with fee result
        // For token operations, we use the estimated fee as a placeholder
        // TODO: Add proper fee tracking when SDK provides this information
        use crate::backend_task::FeeResult;
        use crate::model::fee_estimation::PlatformFeeEstimator;
        let estimated_fee = PlatformFeeEstimator::new().estimate_document_batch(1);
        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::BurnedTokens(fee_result))
    }
}
