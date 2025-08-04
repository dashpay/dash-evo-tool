use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::ProtocolError;
use dash_sdk::dpp::consensus::ConsensusError;
use dash_sdk::dpp::consensus::state::state_error::StateError;
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
    /// Claim all pending token claims matching the provided parameters.
    ///
    /// This method iterates until all tokens are claimed or no more claims are available.
    ///
    /// If no tokens are available to claim, it returns `TokensClaimed(0)`
    /// (in contrary to the [AppContext::claim_token()] method which returns error).
    #[allow(clippy::too_many_arguments)]
    pub async fn claim_all_tokens(
        &self,
        data_contract: Arc<DataContract>,
        token_position: u16,
        actor_identity: &QualifiedIdentity,
        distribution_type: TokenDistributionType,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        let mut total_claimed = 0;
        loop {
            let result = self
                .claim_token(
                    data_contract.clone(),
                    token_position,
                    actor_identity,
                    distribution_type,
                    signing_key.clone(),
                    public_note.clone(),
                    sdk,
                )
                .await;

            match result {
                Ok(BackendTaskSuccessResult::TokensClaimed(0)) => {
                    // If no tokens were claimed, we can exit the loop
                    break;
                }
                Ok(BackendTaskSuccessResult::TokensClaimed(amount)) => {
                    total_claimed += amount;
                    // Continue to check for more tokens to claim
                }
                Err(dash_sdk::Error::Protocol(ProtocolError::ConsensusError(ce)))
                    if matches!(
                        *ce,
                        ConsensusError::StateError(StateError::InvalidTokenClaimNoCurrentRewards(
                            _
                        )),
                    ) =>
                {
                    // No more rewards available, exit the loop
                    break;
                }
                // any other result we propagate
                Ok(x) => return Ok(x),
                Err(e) => {
                    return Err(format!(
                        "Error claiming tokens: {}; claimed so far: {}",
                        e, total_claimed
                    ));
                }
            }
        }

        // Return the total claimed amount.
        Ok(BackendTaskSuccessResult::TokensClaimed(total_claimed))
    }

    /// Execute single token claim.
    ///
    /// This method will return error if no tokens were available to claim.
    #[allow(clippy::too_many_arguments)]
    pub async fn claim_token(
        &self,
        data_contract: Arc<DataContract>,
        token_position: u16,
        actor_identity: &QualifiedIdentity,
        distribution_type: TokenDistributionType,
        signing_key: IdentityPublicKey,
        public_note: Option<String>,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, dash_sdk::Error> {
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
            .inspect_err(|e|{ match e {
                Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                    self.db
                        .insert_proof_log_item(ProofLogItem {
                            request_type: RequestType::BroadcastStateTransition,
                            request_bytes: vec![],
                            verification_path_query_bytes: vec![],
                            height: block_info.height,
                            time_ms: block_info.time_ms,
                            proof_bytes: proof_bytes.clone(),
                            error: Some(proof_error.to_string()),
                        })
                        .ok();
                    tracing::error!(error=?proof_error, "Error broadcasting ClaimTokens transition, proof error logged");
                }
                e => tracing::error!(error=?e, "Error broadcasting ClaimTokens transition"),
            };
        })?;

        // Using the result, update the balance of the claimer identity
        let mut claimed_amount = 0;
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
                            claimed_amount = *amount;
                            if let Ok(claimer_id) = Identifier::from_bytes(claimer_bytes) {
                                if let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &claimer_id,
                                    *amount,
                                ) {
                                    tracing::error!(error=?e, identity=?claimer_id, token_id=?token_id,
                                        "Failed to update token balance from claim document",
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
                            claimed_amount = *amount;
                            if let Ok(claimer_id) = Identifier::from_bytes(claimer_bytes) {
                                if let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &claimer_id,
                                    *amount,
                                ) {
                                    tracing::error!(error=?e, identity=?claimer_id, token_id=?token_id,
                                        "Failed to update token balance from group action document",
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // Return success
        Ok(BackendTaskSuccessResult::TokensClaimed(claimed_amount))
    }
}
