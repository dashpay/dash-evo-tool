use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::GroupStateTransitionInfoStatus;
use dash_sdk::dpp::group::group_action_status::GroupActionStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::tokens::builders::mint::TokenMintTransitionBuilder;
use dash_sdk::platform::tokens::transitions::MintResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::{Error, Sdk};
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
        sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let builder = TokenMintTransitionBuilder::new(
            data_contract.clone(),
            token_position,
            sending_identity.identity.id(),
            amount,
        );

        let mut builder = if let Some(recipient_id) = optional_recipient {
            builder.issued_to_identity_id(recipient_id)
        } else {
            builder
        };

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
            .token_mint(builder, &signing_key, sending_identity)
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
                        "Error broadcasting Mint Tokens transition: {}, proof error logged",
                        proof_error
                    )
                }
                e => format!("Error broadcasting Mint Tokens transition: {}", e),
            })?;

        // Using the result, update the balance of the recipient identity
        if let Some(token_id) = data_contract.token_id(token_position) {
            match result {
                // Standard mint result - direct balance update
                MintResult::TokenBalance(identity_id, amount) => {
                    if let Err(e) =
                        self.insert_token_identity_balance(&token_id, &identity_id, amount)
                    {
                        eprintln!("Failed to update token balance: {}", e);
                    }
                }

                // Historical document - extract recipient and amount from document
                MintResult::HistoricalDocument(document) => {
                    if let (Some(recipient_value), Some(amount_value)) =
                        (document.get("recipientId"), document.get("amount"))
                    {
                        if let (Value::Identifier(recipient_bytes), Value::U64(amount)) =
                            (recipient_value, amount_value)
                        {
                            if let Ok(recipient_id) = Identifier::from_bytes(recipient_bytes) {
                                if let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &recipient_id,
                                    *amount,
                                ) {
                                    eprintln!(
                                        "Failed to update token balance from historical document: {}",
                                        e
                                    );
                                }
                            }
                        }
                    }
                }

                // Group action with document - assume completed if document exists
                MintResult::GroupActionWithDocument(_, Some(document)) => {
                    if let (Some(recipient_value), Some(amount_value)) =
                        (document.get("recipientId"), document.get("amount"))
                    {
                        if let (Value::Identifier(recipient_bytes), Value::U64(amount)) =
                            (recipient_value, amount_value)
                        {
                            if let Ok(recipient_id) = Identifier::from_bytes(recipient_bytes) {
                                if let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &recipient_id,
                                    *amount,
                                ) {
                                    eprintln!(
                                        "Failed to update token balance from group action document: {}",
                                        e
                                    );
                                }
                            }
                        }
                    }
                }

                // Group action with balance - only update if action is closed
                MintResult::GroupActionWithBalance(_, status, Some(amount)) => {
                    if matches!(status, GroupActionStatus::ActionClosed) {
                        // Get the recipient identity (either optional_recipient or sending_identity)
                        let recipient_id =
                            optional_recipient.unwrap_or_else(|| sending_identity.identity.id());
                        if let Err(e) =
                            self.insert_token_identity_balance(&token_id, &recipient_id, amount)
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
        Ok(BackendTaskSuccessResult::Message("MintTokens".to_string()))
    }
}
