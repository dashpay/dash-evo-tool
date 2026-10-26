//! Transfer tokens from one identity to another

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::Value;
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
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let actor = self.token_actor(sending_identity).await?;
        let common = self.token_action_common(data_contract.clone(), token_position, &signing_key);

        self.execute_token_op(
            actor.transfer(common, recipient_id, amount, public_note),
            |result| {
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
            },
            BackendTaskSuccessResult::TransferredTokens,
        )
        .await
    }
}
