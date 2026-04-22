//! Transfer tokens from one identity to another

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::proof_log_item::RequestType;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::tokens::builders::transfer::TokenTransferTransitionBuilder;
use dash_sdk::platform::tokens::transitions::TransferResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use std::sync::Arc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn transfer_tokens(
        &self,
        sending_identity: &QualifiedIdentity,
        recipient_id: Identifier,
        amount: u64,
        data_contract: Arc<DataContract>,
        token_position: u16,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let mut builder = TokenTransferTransitionBuilder::new(
            data_contract.clone(),
            token_position,
            sending_identity.identity.id(),
            recipient_id,
            amount,
        );

        if let Some(note) = public_note {
            builder = builder.with_public_note(note);
        }

        let maybe_options = self.state_transition_options();
        if let Some(options) = maybe_options {
            builder = builder.with_state_transition_creation_options(options);
        }

        let result = sdk
            .token_transfer(builder, &signing_key, sending_identity)
            .await
            .map_err(|e| self.log_drive_proof_error(e, RequestType::BroadcastStateTransition))?;

        // Using the result, update the balance of both sender and recipient identities
        if let Some(token_id) = data_contract.token_id(token_position) {
            match result {
                // Standard transfer result - update balances from map
                TransferResult::IdentitiesBalances(balances_map) => {
                    for (identity_id, balance) in balances_map {
                        if let Err(e) =
                            self.insert_token_identity_balance(&token_id, &identity_id, balance)
                        {
                            tracing::error!(
                                "Failed to update token balance for identity {}: {}",
                                identity_id,
                                e
                            );
                        }
                    }
                }

                // Historical document - extract sender, recipient and amounts from document
                TransferResult::HistoricalDocument(document) => {
                    if let (
                        Some(sender_value),
                        Some(sender_amount_value),
                        Some(recipient_value),
                        Some(recipient_amount_value),
                    ) = (
                        document.get("senderId"),
                        document.get("senderAmount"),
                        document.get("recipientId"),
                        document.get("recipientAmount"),
                    ) && let (
                        Value::Identifier(sender_bytes),
                        Value::U64(sender_amount),
                        Value::Identifier(recipient_bytes),
                        Value::U64(recipient_amount),
                    ) = (
                        sender_value,
                        sender_amount_value,
                        recipient_value,
                        recipient_amount_value,
                    ) && let (Ok(sender_id), Ok(recipient_id)) = (
                        Identifier::from_bytes(sender_bytes),
                        Identifier::from_bytes(recipient_bytes),
                    ) {
                        if let Err(e) = self.insert_token_identity_balance(
                            &token_id,
                            &sender_id,
                            *sender_amount,
                        ) {
                            tracing::error!(
                                "Failed to update sender token balance from historical document: {}",
                                e
                            );
                        }
                        if let Err(e) = self.insert_token_identity_balance(
                            &token_id,
                            &recipient_id,
                            *recipient_amount,
                        ) {
                            tracing::error!(
                                "Failed to update recipient token balance from historical document: {}",
                                e
                            );
                        }
                    }
                }

                // Group action with document - assume completed if document exists
                TransferResult::GroupActionWithDocument(_, Some(document)) => {
                    if let (
                        Some(sender_value),
                        Some(sender_amount_value),
                        Some(recipient_value),
                        Some(recipient_amount_value),
                    ) = (
                        document.get("senderId"),
                        document.get("senderAmount"),
                        document.get("recipientId"),
                        document.get("recipientAmount"),
                    ) && let (
                        Value::Identifier(sender_bytes),
                        Value::U64(sender_amount),
                        Value::Identifier(recipient_bytes),
                        Value::U64(recipient_amount),
                    ) = (
                        sender_value,
                        sender_amount_value,
                        recipient_value,
                        recipient_amount_value,
                    ) && let (Ok(sender_id), Ok(recipient_id)) = (
                        Identifier::from_bytes(sender_bytes),
                        Identifier::from_bytes(recipient_bytes),
                    ) {
                        if let Err(e) = self.insert_token_identity_balance(
                            &token_id,
                            &sender_id,
                            *sender_amount,
                        ) {
                            tracing::error!(
                                "Failed to update sender token balance from group action document: {}",
                                e
                            );
                        }
                        if let Err(e) = self.insert_token_identity_balance(
                            &token_id,
                            &recipient_id,
                            *recipient_amount,
                        ) {
                            tracing::error!(
                                "Failed to update recipient token balance from group action document: {}",
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
        use crate::backend_task::FeeResult;
        use crate::model::fee_estimation::PlatformFeeEstimator;
        let estimated_fee = PlatformFeeEstimator::new().estimate_document_batch(1);
        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::TransferredTokens(fee_result))
    }
}
