use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::tokens::builders::purchase::TokenDirectPurchaseTransitionBuilder;
use dash_sdk::platform::tokens::transitions::DirectPurchaseResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::{Error, Sdk};
use std::sync::Arc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn purchase_tokens(
        &self,
        sending_identity: &QualifiedIdentity,
        data_contract: Arc<DataContract>,
        token_position: u16,
        signing_key: IdentityPublicKey,
        amount: TokenAmount,
        total_agreed_price: Credits,
        sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let mut builder = TokenDirectPurchaseTransitionBuilder::new(
            data_contract.clone(),
            token_position,
            sending_identity.identity.id(),
            amount,
            total_agreed_price,
        );

        if let Some(options) = self.state_transition_options() {
            builder = builder.with_state_transition_creation_options(options);
        }

        let result = sdk
            .token_purchase(builder, &signing_key, sending_identity)
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
                        "Error broadcasting Purchase Tokens transition: {}, proof error logged",
                        proof_error
                    )
                }
                e => format!("Error broadcasting Purchase Tokens transition: {}", e),
            })?;

        // Update token balance from the proof-verified result
        if let Some(token_id) = data_contract.token_id(token_position) {
            match result {
                // Standard purchase result - update purchaser's balance
                DirectPurchaseResult::TokenBalance(identity_id, balance) => {
                    tracing::info!(
                        "PurchaseTokens: identity {} new balance {}",
                        identity_id,
                        balance
                    );
                    if let Err(e) =
                        self.insert_token_identity_balance(&token_id, &identity_id, balance)
                    {
                        tracing::warn!("Failed to update token balance: {}", e);
                    }
                }

                // Historical document - extract purchaser and balance from document
                DirectPurchaseResult::HistoricalDocument(document) => {
                    tracing::info!("PurchaseTokens: historical document id={}", document.id());
                    if let (Some(purchaser_value), Some(balance_value)) =
                        (document.get("purchaserId"), document.get("balance"))
                        && let (Value::Identifier(purchaser_bytes), Value::U64(balance)) =
                            (purchaser_value, balance_value)
                        && let Ok(purchaser_id) = Identifier::from_bytes(purchaser_bytes)
                        && let Err(e) =
                            self.insert_token_identity_balance(&token_id, &purchaser_id, *balance)
                    {
                        tracing::warn!(
                            "Failed to update token balance from historical document: {}",
                            e
                        );
                    }
                }

                // Group action with document
                DirectPurchaseResult::GroupActionWithDocument(power, Some(document)) => {
                    tracing::info!(
                        "PurchaseTokens: group action power={}, doc_id={}",
                        power,
                        document.id()
                    );
                    if let (Some(purchaser_value), Some(balance_value)) =
                        (document.get("purchaserId"), document.get("balance"))
                        && let (Value::Identifier(purchaser_bytes), Value::U64(balance)) =
                            (purchaser_value, balance_value)
                        && let Ok(purchaser_id) = Identifier::from_bytes(purchaser_bytes)
                        && let Err(e) =
                            self.insert_token_identity_balance(&token_id, &purchaser_id, *balance)
                    {
                        tracing::warn!(
                            "Failed to update token balance from group action document: {}",
                            e
                        );
                    }
                }

                // Group action without document - no balance to update
                DirectPurchaseResult::GroupActionWithDocument(power, None) => {
                    tracing::info!("PurchaseTokens: group action power={}, no document", power);
                }
            }
        }

        // Return success
        Ok(BackendTaskSuccessResult::Message(
            "PurchaseTokens".to_string(),
        ))
    }
}
