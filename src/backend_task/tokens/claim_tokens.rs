use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::model::proof_log_item::RequestType;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Error as SdkError;
use dash_sdk::Sdk;
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
use std::sync::Arc;

impl AppContext {
    /// Claim tokens for a single distribution cycle.
    ///
    /// On success returns [`BackendTaskSuccessResult::ClaimedTokens`] with fee info.
    ///
    /// If the platform reports that the identity has no current rewards to
    /// claim ([`StateError::InvalidTokenClaimNoCurrentRewards`]), this returns
    /// [`TaskError::NoTokensToClaim`] so callers (like [`Self::claim_all_tokens`])
    /// can match on it structurally.
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
    ) -> Result<BackendTaskSuccessResult, TaskError> {
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
            .map_err(|e| {
                if is_no_current_rewards_error(&e) {
                    TaskError::NoTokensToClaim
                } else {
                    self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                }
            })?;

        // Using the result, update the balance of the claimer identity
        if let Some(token_id) = data_contract.token_id(token_position) {
            match result {
                // Standard claim result - extract claimer and amount from document
                ClaimResult::Document(document) => {
                    if let (Some(claimer_value), Some(amount_value)) =
                        (document.get("claimerId"), document.get("amount"))
                        && let (Value::Identifier(claimer_bytes), Value::U64(amount)) =
                            (claimer_value, amount_value)
                        && let Ok(claimer_id) = Identifier::from_bytes(claimer_bytes)
                        && let Err(e) =
                            self.insert_token_identity_balance(&token_id, &claimer_id, *amount)
                    {
                        tracing::error!(
                            "Failed to update token balance from claim document: {}",
                            e
                        );
                    }
                }

                // Group action with document - assume completed if document exists
                ClaimResult::GroupActionWithDocument(_, document) => {
                    if let (Some(claimer_value), Some(amount_value)) =
                        (document.get("claimerId"), document.get("amount"))
                        && let (Value::Identifier(claimer_bytes), Value::U64(amount)) =
                            (claimer_value, amount_value)
                        && let Ok(claimer_id) = Identifier::from_bytes(claimer_bytes)
                        && let Err(e) =
                            self.insert_token_identity_balance(&token_id, &claimer_id, *amount)
                    {
                        tracing::error!(
                            "Failed to update token balance from group action document: {}",
                            e
                        );
                    }
                }
            }
        }

        // Return success with fee result
        let estimated_fee = PlatformFeeEstimator::new().estimate_document_batch(1);
        let fee_result = FeeResult::new(estimated_fee, estimated_fee);
        Ok(BackendTaskSuccessResult::ClaimedTokens(fee_result))
    }

    /// Repeatedly claim tokens until the distribution has no rewards left.
    ///
    /// Calls [`Self::claim_tokens`] in a loop, accumulating fees across
    /// iterations. Stops cleanly when the platform reports
    /// [`TaskError::NoTokensToClaim`]. If no iteration succeeded (the very
    /// first call reported nothing to claim), the error is propagated so the
    /// user gets a proper "nothing to claim" banner instead of a silent
    /// success.
    ///
    /// Any other error short-circuits the loop and is returned as-is.
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
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let mut iterations: usize = 0;
        loop {
            let outcome = self
                .claim_tokens(
                    data_contract.clone(),
                    token_position,
                    actor_identity,
                    distribution_type,
                    signing_key.clone(),
                    public_note.clone(),
                    sdk,
                )
                .await;

            match outcome {
                Ok(BackendTaskSuccessResult::ClaimedTokens(_)) => {
                    iterations = iterations.saturating_add(1);
                }
                // Any other success variant is unexpected — propagate verbatim.
                Ok(other) => return Ok(other),
                Err(TaskError::NoTokensToClaim) => {
                    if iterations == 0 {
                        return Err(TaskError::NoTokensToClaim);
                    }
                    break;
                }
                Err(e) => return Err(e),
            }
        }

        let total_fee = PlatformFeeEstimator::new().estimate_document_batch(iterations);
        Ok(BackendTaskSuccessResult::ClaimedTokens(FeeResult::new(
            total_fee, total_fee,
        )))
    }
}

/// Returns `true` when the SDK error is the platform's
/// `InvalidTokenClaimNoCurrentRewards` consensus state error.
fn is_no_current_rewards_error(e: &SdkError) -> bool {
    matches!(
        e,
        SdkError::Protocol(ProtocolError::ConsensusError(ce))
            if matches!(
                ce.as_ref(),
                ConsensusError::StateError(StateError::InvalidTokenClaimNoCurrentRewards(_))
            )
    )
}
