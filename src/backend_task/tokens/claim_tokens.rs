use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::tokens::builders::claim::TokenClaimTransitionBuilder;
use dash_sdk::platform::tokens::transitions::ClaimResult;
use dash_sdk::platform::{DataContract, Identifier, IdentityPublicKey};
use dash_sdk::{Error, Sdk};
use std::sync::Arc;

impl AppContext {
    #[allow(clippy::too_many_arguments)]
    pub async fn claim_tokens(
        &self,
        data_contract: Arc<DataContract>,
        token_position: u16,
        actor_identity: &QualifiedIdentity,
        distribution_type: TokenDistributionType,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Build
        let mut builder = TokenClaimTransitionBuilder::new(
            data_contract.clone(),
            token_position,
            actor_identity.identity.id(),
            distribution_type,
        );

        if let Some(note) = public_note {
            builder = builder.with_public_note(note);
        }

        let maybe_options = self.state_transition_options();
        if let Some(options) = maybe_options {
            builder = builder.with_state_transition_creation_options(options);
        }

        let result = sdk
            .token_claim(builder, &signing_key, actor_identity)
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
                        "Error broadcasting ClaimTokens transition: {}, proof error logged",
                        proof_error
                    )
                }
                e => format!("Error broadcasting ClaimTokens transition: {}", e),
            })?;

        // Using the result, update the balance of the claimer identity
        if let Some(token_id) = data_contract.token_id(token_position) {
            match result {
                // Standard claim result - extract claimer and amount from document
                ClaimResult::Document(document) => {
                    if let (Some(claimer_value), Some(amount_value)) =
                        (document.get("claimerId"), document.get("amount"))
                    {
                        if let (Value::Identifier(claimer_bytes), Value::U64(amount)) =
                            (claimer_value, amount_value)
                        {
                            if let Ok(claimer_id) = Identifier::from_bytes(claimer_bytes) {
                                if let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &claimer_id,
                                    *amount,
                                ) {
                                    eprintln!(
                                        "Failed to update token balance from claim document: {}",
                                        e
                                    );
                                }
                            }
                        }
                    }
                }

                // Group action with document - assume completed if document exists
                ClaimResult::GroupActionWithDocument(_, document) => {
                    if let (Some(claimer_value), Some(amount_value)) =
                        (document.get("claimerId"), document.get("amount"))
                    {
                        if let (Value::Identifier(claimer_bytes), Value::U64(amount)) =
                            (claimer_value, amount_value)
                        {
                            if let Ok(claimer_id) = Identifier::from_bytes(claimer_bytes) {
                                if let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &claimer_id,
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
            }
        }

        // Return success
        Ok(BackendTaskSuccessResult::Message("ClaimTokens".to_string()))
    }
}
