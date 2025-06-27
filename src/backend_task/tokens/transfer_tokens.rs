//! Transfer tokens from one identity to another

use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::tokens::builders::transfer::TokenTransferTransitionBuilder;
use dash_sdk::platform::tokens::transitions::TransferResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::{Error, Sdk};
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
    ) -> Result<BackendTaskSuccessResult, String> {
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
                        "Error broadcasting Transfer Tokens transition: {}, proof error logged",
                        proof_error
                    )
                }
                e => format!("Error broadcasting Transfer Tokens transition: {}", e),
            })?;

        // Using the result, update the balance of both sender and recipient identities
        if let Some(token_id) = data_contract.token_id(token_position) {
            match result {
                // Standard transfer result - update balances from map
                TransferResult::IdentitiesBalances(balances_map) => {
                    for (identity_id, balance) in balances_map {
                        if let Err(e) =
                            self.insert_token_identity_balance(&token_id, &identity_id, balance)
                        {
                            eprintln!(
                                "Failed to update token balance for identity {}: {}",
                                identity_id, e
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
                    ) {
                        if let (
                            Value::Identifier(sender_bytes),
                            Value::U64(sender_amount),
                            Value::Identifier(recipient_bytes),
                            Value::U64(recipient_amount),
                        ) = (
                            sender_value,
                            sender_amount_value,
                            recipient_value,
                            recipient_amount_value,
                        ) {
                            if let (Ok(sender_id), Ok(recipient_id)) = (
                                Identifier::from_bytes(sender_bytes),
                                Identifier::from_bytes(recipient_bytes),
                            ) {
                                if let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &sender_id,
                                    *sender_amount,
                                ) {
                                    eprintln!("Failed to update sender token balance from historical document: {}", e);
                                }
                                if let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &recipient_id,
                                    *recipient_amount,
                                ) {
                                    eprintln!("Failed to update recipient token balance from historical document: {}", e);
                                }
                            }
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
                    ) {
                        if let (
                            Value::Identifier(sender_bytes),
                            Value::U64(sender_amount),
                            Value::Identifier(recipient_bytes),
                            Value::U64(recipient_amount),
                        ) = (
                            sender_value,
                            sender_amount_value,
                            recipient_value,
                            recipient_amount_value,
                        ) {
                            if let (Ok(sender_id), Ok(recipient_id)) = (
                                Identifier::from_bytes(sender_bytes),
                                Identifier::from_bytes(recipient_bytes),
                            ) {
                                if let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &sender_id,
                                    *sender_amount,
                                ) {
                                    eprintln!("Failed to update sender token balance from group action document: {}", e);
                                }
                                if let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &recipient_id,
                                    *recipient_amount,
                                ) {
                                    eprintln!("Failed to update recipient token balance from group action document: {}", e);
                                }
                            }
                        }
                    }
                }

                // Other variants don't require balance updates
                _ => {}
            }
        }

        Ok(BackendTaskSuccessResult::Message(
            "TransferTokens".to_string(),
        ))
    }
}
