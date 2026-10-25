use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::request_type::RequestType;
use crate::utils::time::now_ms;
use dash_sdk::Sdk;
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

        let token_id = data_contract.token_id(token_position);
        let claimer_id = actor_identity.identity.id();
        // The block time the proved claim document carries, when it does.
        let mut proved_claim_time: Option<u64> = None;
        let result = self
            .execute_token_op(
            async {
                sdk.token_claim(builder, &signing_key, actor_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })
            },
            |result| {
                // Using the result, update the balance of the claimer identity
                proved_claim_time = match &result {
                    ClaimResult::Document(document)
                    | ClaimResult::GroupActionWithDocument(_, document) => document.created_at(),
                };
                if let Some(token_id) = token_id {
                    match result {
                        // Standard claim result - extract claimer and amount from document
                        ClaimResult::Document(document) => {
                            if let (Some(claimer_value), Some(amount_value)) =
                                (document.get("claimerId"), document.get("amount"))
                                && let (Value::Identifier(claimer_bytes), Value::U64(amount)) =
                                    (claimer_value, amount_value)
                                && let Ok(claimer_id) = Identifier::from_bytes(claimer_bytes)
                                && let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &claimer_id,
                                    *amount,
                                )
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
                                && let Err(e) = self.insert_token_identity_balance(
                                    &token_id,
                                    &claimer_id,
                                    *amount,
                                )
                            {
                                tracing::error!(
                                    "Failed to update token balance from group action document: {}",
                                    e
                                );
                            }
                        }
                    }
                }
            },
            BackendTaskSuccessResult::ClaimedTokens,
        )
        .await;

        match token_id {
            Some(token_id) if distribution_type == TokenDistributionType::OncePerIdentity => {
                settle_once_per_identity_claim(result, proved_claim_time, now_ms(), |at| {
                    self.record_once_per_identity_claim(&token_id, &claimer_id, at)
                })
            }
            _ => result,
        }
    }
}

/// The outcome of a once-per-identity claim attempt, after recording a
/// successful claim with `record` at its claim time.
///
/// The claim cannot be undone, and the record is what keeps the screen from
/// offering it again, so a failed record turns the success into
/// [`TaskError::TokensClaimedButClaimNotRecorded`]: the user learns the claim
/// was made and is not invited to try it again.
fn settle_once_per_identity_claim(
    result: Result<BackendTaskSuccessResult, TaskError>,
    proved_claim_time: Option<u64>,
    now_ms: u64,
    record: impl FnOnce(u64) -> Result<(), TaskError>,
) -> Result<BackendTaskSuccessResult, TaskError> {
    match once_per_identity_claim_time(&result, proved_claim_time, now_ms) {
        Some(claimed_at_ms) => match record(claimed_at_ms) {
            Ok(()) => result,
            Err(source) => Err(TaskError::TokensClaimedButClaimNotRecorded {
                source: Box::new(source),
            }),
        },
        None => result,
    }
}

/// When to record a once-per-identity claim hint for this claim attempt:
/// only after a successful claim, whose result is proof-backed, at the block
/// time of the proved claim document (the local clock when it carries none).
///
/// A refusal as "already claimed" records nothing: that consensus error comes
/// from the node DET talked to, unproven, and upstream offers no proved query
/// for the claim status, so trusting it would let one hostile node mark an
/// entitlement as spent on this device.
fn once_per_identity_claim_time(
    result: &Result<BackendTaskSuccessResult, TaskError>,
    proved_claim_time: Option<u64>,
    now_ms: u64,
) -> Option<u64> {
    result
        .as_ref()
        .ok()
        .map(|_| proved_claim_time.unwrap_or(now_ms))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::FeeResult;

    fn claimed() -> Result<BackendTaskSuccessResult, TaskError> {
        Ok(BackendTaskSuccessResult::ClaimedTokens(
            FeeResult::estimated_only(1),
        ))
    }

    #[test]
    fn a_successful_claim_is_recorded_at_its_proved_block_time() {
        assert_eq!(
            once_per_identity_claim_time(&claimed(), Some(7), 9),
            Some(7)
        );
        assert_eq!(
            once_per_identity_claim_time(&claimed(), None, 9),
            Some(9),
            "the local clock when the document carries no time"
        );
    }

    /// A claim that succeeded but could not be recorded is reported as made
    /// and not recorded, not as a plain success the device then forgets.
    #[test]
    fn a_claim_that_cannot_be_recorded_says_so() {
        let mut recorded_at = None;
        let settled = settle_once_per_identity_claim(claimed(), Some(7), 9, |at| {
            recorded_at = Some(at);
            Err(TaskError::IdentityNotFoundLocally)
        });

        assert_eq!(recorded_at, Some(7));
        assert!(
            matches!(
                settled,
                Err(TaskError::TokensClaimedButClaimNotRecorded { .. })
            ),
            "got {settled:?}"
        );
    }

    /// A recorded claim keeps its success; a failed claim is passed on and
    /// records nothing.
    #[test]
    fn a_recorded_claim_or_a_failed_claim_is_passed_on() {
        let settled = settle_once_per_identity_claim(claimed(), Some(7), 9, |_| Ok(()));
        assert!(matches!(
            settled,
            Ok(BackendTaskSuccessResult::ClaimedTokens(_))
        ));

        let settled =
            settle_once_per_identity_claim(Err(TaskError::MasterKeyNotFound), Some(7), 9, |_| {
                panic!("a failed claim records nothing")
            });
        assert!(matches!(settled, Err(TaskError::MasterKeyNotFound)));
    }

    /// An "already claimed" refusal is unproven node output: never recorded,
    /// whichever identity it names.
    #[test]
    fn a_refusal_or_any_other_error_is_not_recorded() {
        for identity_id in [Identifier::random(), Identifier::random()] {
            let refused = Err(TaskError::TokenOncePerIdentityAlreadyClaimed {
                token_id: Identifier::random(),
                identity_id,
                claimed_at_ms: 5,
                source_error: Box::new(dash_sdk::Error::Generic("claimed".to_string())),
            });
            assert_eq!(once_per_identity_claim_time(&refused, Some(7), 9), None);
        }
        assert_eq!(
            once_per_identity_claim_time(&Err(TaskError::MasterKeyNotFound), Some(7), 9),
            None
        );
    }
}
