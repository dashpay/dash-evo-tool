use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::group_action_status::GroupActionStatus;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::tokens::builders::burn::TokenBurnTransitionBuilder;
use dash_sdk::platform::tokens::transitions::BurnResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::{Error, Sdk};
use std::sync::Arc;
use tokio::sync::mpsc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn burn_tokens(
        &self,
        owner_identity: &QualifiedIdentity,
        data_contract: &DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        amount: u64,
        group_info: Option<GroupStateTransitionInfoStatus>,
        sdk: &Sdk,
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let data_contract_arc = Arc::new(data_contract.clone());

        let mut builder = TokenBurnTransitionBuilder::new(
            data_contract_arc,
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
                        "Error broadcasting Burn Tokens transition: {}, proof error logged",
                        proof_error
                    )
                }
                e => format!("Error broadcasting Burn Tokens transition: {}", e),
            })?;

        // Using the result, update the balance of the owner identity
        if let Some(token_id) = data_contract.token_id(token_position) {
            match result {
                // Standard burn result - direct balance update
                BurnResult::TokenBalance(identity_id, amount) => {
                    if let Err(e) =
                        self.insert_token_identity_balance(&token_id, &identity_id, amount)
                    {
                        eprintln!("Failed to update token balance: {}", e);
                    }
                }

                // Historical document - extract owner and amount from document
                BurnResult::HistoricalDocument(document) => {
                    if let (Some(owner_value), Some(amount_value)) =
                        (document.get("ownerId"), document.get("amount"))
                    {
                        if let (Value::Identifier(owner_bytes), Value::U64(amount)) =
                            (owner_value, amount_value)
                        {
                            if let Ok(owner_id) = Identifier::from_bytes(owner_bytes) {
                                if let Err(e) = self
                                    .insert_token_identity_balance(&token_id, &owner_id, *amount)
                                {
                                    eprintln!("Failed to update token balance from historical document: {}", e);
                                }
                            }
                        }
                    }
                }

                // Group action with document - assume completed if document exists
                BurnResult::GroupActionWithDocument(_, Some(document)) => {
                    if let (Some(owner_value), Some(amount_value)) =
                        (document.get("ownerId"), document.get("amount"))
                    {
                        if let (Value::Identifier(owner_bytes), Value::U64(amount)) =
                            (owner_value, amount_value)
                        {
                            if let Ok(owner_id) = Identifier::from_bytes(owner_bytes) {
                                if let Err(e) = self
                                    .insert_token_identity_balance(&token_id, &owner_id, *amount)
                                {
                                    eprintln!("Failed to update token balance from group action document: {}", e);
                                }
                            }
                        }
                    }
                }

                // Group action with balance - only update if action is closed
                BurnResult::GroupActionWithBalance(_, status, Some(amount)) => {
                    if matches!(status, GroupActionStatus::ActionClosed) {
                        let owner_id = owner_identity.identity.id();
                        if let Err(e) =
                            self.insert_token_identity_balance(&token_id, &owner_id, amount)
                        {
                            eprintln!("Failed to update token balance from group action: {}", e);
                        }
                    }
                }

                // Other variants don't require balance updates
                _ => {}
            }
        }

        // Return success
        Ok(BackendTaskSuccessResult::Message("BurnTokens".to_string()))
    }
}
