use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dpns_voting::{DpnsVoteOperationId, DpnsVoteTargetKey, dpns_vote_poll};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::consensus::ConsensusError;
use dash_sdk::dpp::consensus::basic::BasicError;
use dash_sdk::dpp::identifier::MasternodeIdentifiers;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::state_transition::masternode_vote_transition::MasternodeVoteTransition;
use dash_sdk::dpp::state_transition::masternode_vote_transition::methods::MasternodeVoteTransitionMethodsV0;
use dash_sdk::dpp::state_transition::{StateTransition, StateTransitionStructureValidation};
use dash_sdk::dpp::util::strings::convert_to_homograph_safe_chars;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::dpp::voting::vote_polls::contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll;
use dash_sdk::dpp::voting::votes::Vote;
use dash_sdk::dpp::voting::votes::resource_vote::ResourceVote;
use dash_sdk::dpp::voting::votes::resource_vote::accessors::v0::ResourceVoteGettersV0;
use dash_sdk::dpp::voting::votes::resource_vote::v0::ResourceVoteV0;
use dash_sdk::drive::query::contested_resource_votes_given_by_identity_query::ContestedResourceVotesGivenByIdentityQuery;
use dash_sdk::drive::query::vote_polls_by_document_type_query::VotePollsByDocumentTypeQuery;
use dash_sdk::platform::FetchMany;
use dash_sdk::platform::Identifier;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::broadcast_request::BroadcastRequestForStateTransition;
use dash_sdk::platform::transition::put_settings::PutSettings;
use dash_sdk::platform::transition::waitable::Waitable;
use dash_sdk::query_types::ContestedResource;
use std::sync::Arc;

#[derive(Debug)]
pub(super) enum DpnsVoteAttempt {
    Confirmed,
    Unconfirmed(TaskError),
    Rejected(TaskError),
    FailedBeforeSubmission(TaskError),
}

fn ensure_valid_vote_transition_structure(
    state_transition: &StateTransition,
    sdk: &Sdk,
) -> Result<(), TaskError> {
    let validation_result = state_transition.validate_structure(sdk.version());
    if validation_result.is_valid()
        || validation_result.errors.iter().all(|error| {
            matches!(
                error,
                ConsensusError::BasicError(BasicError::UnsupportedFeatureError(_))
            )
        })
    {
        Ok(())
    } else {
        Err(TaskError::from(dash_sdk::Error::from(validation_result)))
    }
}

fn classify_post_broadcast_error(error: dash_sdk::Error) -> DpnsVoteAttempt {
    let rejected = matches!(
        &error,
        dash_sdk::Error::StateTransitionBroadcastError(broadcast_error)
            if broadcast_error.cause.is_some()
    );
    let error = TaskError::from(error);
    if rejected {
        DpnsVoteAttempt::Rejected(error)
    } else {
        DpnsVoteAttempt::Unconfirmed(error)
    }
}

fn classify_broadcast_error(error: dash_sdk::Error) -> DpnsVoteAttempt {
    match &error {
        dash_sdk::Error::Protocol(dash_sdk::dpp::ProtocolError::ConsensusError(_)) => {
            DpnsVoteAttempt::Rejected(TaskError::from(error))
        }
        dash_sdk::Error::StateTransitionBroadcastError(broadcast_error) => {
            let rejected = broadcast_error.cause.is_some();
            let error = TaskError::from(error);
            if rejected {
                DpnsVoteAttempt::Rejected(error)
            } else {
                DpnsVoteAttempt::Unconfirmed(error)
            }
        }
        _ => DpnsVoteAttempt::Unconfirmed(TaskError::from(error)),
    }
}

fn classify_broadcast_journal_result(
    result: Result<bool, TaskError>,
) -> Result<(), DpnsVoteAttempt> {
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(DpnsVoteAttempt::FailedBeforeSubmission(
            TaskError::DpnsVoteBroadcastPhaseNotMarked,
        )),
        Err(error) => Err(DpnsVoteAttempt::FailedBeforeSubmission(error)),
    }
}

/// Read one node's proved current vote on one contest.
pub(super) async fn proved_dpns_vote_choice(
    key: &DpnsVoteTargetKey,
    sdk: &Sdk,
) -> Result<Option<ResourceVoteChoice>, dash_sdk::Error> {
    let query = ContestedResourceVotesGivenByIdentityQuery {
        identity_id: key.voter_id,
        offset: None,
        limit: Some(1),
        start_at: Some((key.vote_poll_id.to_buffer(), true)),
        order_ascending: true,
    };
    let votes = ResourceVote::fetch_many(sdk, query).await?;
    Ok(votes
        .get(&key.vote_poll_id)
        .and_then(Option::as_ref)
        .map(ResourceVoteGettersV0::resource_vote_choice))
}

/// Settle a rejection that `broadcast` reported.
///
/// `broadcast` retries internally and reports only its last attempt, so the
/// rejection can follow an earlier attempt that Platform applied (a repeated
/// transition is refused once the first one is in a block). A proved current
/// vote equal to the requested choice shows the vote went in. Anything else,
/// including a failed read, leaves the rejection standing: the next vote on
/// the target starts from a fresh proved read anyway.
fn settle_broadcast_rejection(
    rejection: TaskError,
    requested: ResourceVoteChoice,
    proved_current: Result<Option<ResourceVoteChoice>, dash_sdk::Error>,
) -> DpnsVoteAttempt {
    match proved_current {
        Ok(Some(current)) if current == requested => DpnsVoteAttempt::Confirmed,
        Ok(_) => DpnsVoteAttempt::Rejected(rejection),
        Err(read_error) => {
            tracing::debug!(
                ?read_error,
                "Could not read the current vote after a rejected DPNS vote broadcast"
            );
            DpnsVoteAttempt::Rejected(rejection)
        }
    }
}

async fn broadcast_after_journal_mark<Mark, Broadcast, BroadcastFuture>(
    mark_broadcast: Mark,
    broadcast: Broadcast,
) -> Result<(), DpnsVoteAttempt>
where
    Mark: FnOnce() -> Result<bool, TaskError>,
    Broadcast: FnOnce() -> BroadcastFuture,
    BroadcastFuture: std::future::Future<Output = Result<(), dash_sdk::Error>>,
{
    classify_broadcast_journal_result(mark_broadcast())?;
    broadcast().await.map_err(classify_broadcast_error)
}

impl AppContext {
    /// Build a DPNS poll only after proving that it currently exists.
    pub(super) async fn checked_dpns_vote_poll(
        &self,
        name: &str,
        sdk: &Sdk,
    ) -> Result<ContestedDocumentResourceVotePoll, TaskError> {
        let vote_poll = dpns_vote_poll(&self.dpns_contract, name)?;
        let normalized_label = convert_to_homograph_safe_chars(name);

        // Pre-flight: confirm Platform has an open poll for this label before
        // broadcasting — fails fast with VotePollNotFound if it doesn't.
        let existence_query = VotePollsByDocumentTypeQuery {
            contract_id: vote_poll.contract_id,
            document_type_name: vote_poll.document_type_name.clone(),
            index_name: vote_poll.index_name.clone(),
            start_index_values: vec![Value::from("dash")],
            end_index_values: vec![],
            // Start exactly at our normalized label (inclusive) — a single
            // row is enough to confirm the poll exists.
            start_at_value: Some((Value::Text(normalized_label.clone()), true)),
            limit: Some(1),
            order_ascending: true,
        };

        let resources = ContestedResource::fetch_many(sdk, existence_query)
            .await
            .map_err(TaskError::from)?;
        let poll_exists = resources
            .0
            .iter()
            .any(|r| r.0.as_str() == Some(normalized_label.as_str()));
        if !poll_exists {
            return Err(TaskError::VotePollNotFound {
                name: name.to_owned(),
            });
        }
        Ok(vote_poll)
    }

    pub(super) async fn submit_dpns_vote(
        self: &Arc<Self>,
        operation_id: DpnsVoteOperationId,
        key: &DpnsVoteTargetKey,
        name: &str,
        vote_choice: ResourceVoteChoice,
        qualified_identity: &QualifiedIdentity,
        sdk: &Sdk,
    ) -> Result<DpnsVoteAttempt, TaskError> {
        let vote_poll = self.checked_dpns_vote_poll(name, sdk).await?;
        let Some((_, public_key)) = &qualified_identity.associated_voter_identity else {
            return Err(TaskError::NoVotingIdentity {
                identity_id: qualified_identity.identity.id().to_string(Encoding::Base58),
            });
        };
        let resource_vote = ResourceVoteV0 {
            vote_poll: vote_poll.into(),
            resource_vote_choice: vote_choice,
        };
        let vote = Vote::ResourceVote(ResourceVote::V0(resource_vote));
        let voter_pro_tx_hash = qualified_identity.identity.id();
        let voting_public_key_hash = public_key
            .public_key_hash()
            .map_err(dash_sdk::Error::from)?;
        let voting_identity_id = Identifier::create_voter_identifier(
            voter_pro_tx_hash.as_bytes(),
            &voting_public_key_hash,
        );
        let settings = PutSettings::default();
        let nonce = sdk
            .get_identity_nonce(voting_identity_id, true, Some(settings))
            .await
            .map_err(TaskError::from)?;
        let state_transition = MasternodeVoteTransition::try_from_vote_with_signer(
            vote,
            qualified_identity,
            voter_pro_tx_hash,
            public_key,
            nonce,
            sdk.version(),
            None,
        )
        .await
        .map_err(dash_sdk::Error::from)?;
        ensure_valid_vote_transition_structure(&state_transition, sdk)?;
        state_transition.broadcast_request_for_state_transition()?;
        if let Err(attempt) = broadcast_after_journal_mark(
            || self.mark_dpns_vote_broadcast(operation_id, key),
            || state_transition.broadcast(sdk, Some(settings)),
        )
        .await
        {
            return Ok(match attempt {
                DpnsVoteAttempt::Rejected(rejection) => settle_broadcast_rejection(
                    rejection,
                    vote_choice,
                    proved_dpns_vote_choice(key, sdk).await,
                ),
                attempt => attempt,
            });
        }

        match Vote::wait_for_response(sdk, state_transition, Some(settings)).await {
            Ok(_) => Ok(DpnsVoteAttempt::Confirmed),
            Err(error) => Ok(classify_post_broadcast_error(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convert_to_homograph_safe_chars_maps_alice_to_a11ce() {
        // Given: the canonical DPNS homograph substitutions (i/l → 1, o → 0).
        // When: normalizing a label with i/l/o.
        let normalized = convert_to_homograph_safe_chars("alice");

        // Then: the result matches the constant used by the vote poll tests.
        assert_eq!(normalized, "a11ce");
    }

    #[test]
    fn non_broadcast_wait_error_is_unconfirmed() {
        let attempt =
            classify_post_broadcast_error(dash_sdk::Error::Generic("wait failed".to_owned()));

        assert!(matches!(attempt, DpnsVoteAttempt::Unconfirmed(_)));
    }

    #[test]
    fn broadcast_transport_error_after_phase_boundary_is_unconfirmed() {
        let attempt =
            classify_broadcast_error(dash_sdk::Error::Generic("connection refused".to_owned()));

        assert!(matches!(attempt, DpnsVoteAttempt::Unconfirmed(_)));
    }

    #[test]
    fn followup_cause_less_broadcast_error_remains_unconfirmed() {
        let attempt = classify_broadcast_error(
            dash_sdk::error::StateTransitionBroadcastError {
                code: 1,
                message: "broadcast rejected".to_owned(),
                cause: None,
            }
            .into(),
        );

        assert!(matches!(attempt, DpnsVoteAttempt::Unconfirmed(_)));
    }

    #[test]
    fn broadcast_consensus_error_is_rejected() {
        let attempt = classify_broadcast_error(dash_sdk::Error::Protocol(
            dash_sdk::dpp::ProtocolError::ConsensusError(Box::new(ConsensusError::DefaultError)),
        ));

        assert!(matches!(attempt, DpnsVoteAttempt::Rejected(_)));
    }

    /// A rejection reported by a retried broadcast does not prove the vote
    /// was not applied by an earlier attempt; the proved current vote does.
    #[test]
    fn a_rejected_broadcast_is_confirmed_when_platform_shows_the_requested_vote() {
        let lock = ResourceVoteChoice::Lock;
        let rejection = || TaskError::DpnsVoteTargetBusy;

        assert!(matches!(
            settle_broadcast_rejection(rejection(), lock, Ok(Some(lock))),
            DpnsVoteAttempt::Confirmed
        ));
        for proved_current in [
            Ok(None),
            Ok(Some(ResourceVoteChoice::Abstain)),
            Err(dash_sdk::Error::Generic("unreachable".to_owned())),
        ] {
            assert!(matches!(
                settle_broadcast_rejection(rejection(), lock, proved_current),
                DpnsVoteAttempt::Rejected(TaskError::DpnsVoteTargetBusy)
            ));
        }
    }

    #[test]
    fn journal_failure_before_broadcast_fails_before_submission() {
        let attempt = classify_broadcast_journal_result(Err(TaskError::DpnsVoteTargetBusy))
            .expect_err("a failed journal mark must prevent the broadcast");

        assert!(matches!(
            attempt,
            DpnsVoteAttempt::FailedBeforeSubmission(_)
        ));
    }

    #[test]
    fn journal_no_op_before_broadcast_fails_before_submission() {
        let attempt = classify_broadcast_journal_result(Ok(false))
            .expect_err("a no-op journal mark must prevent the broadcast");

        assert!(matches!(
            attempt,
            DpnsVoteAttempt::FailedBeforeSubmission(_)
        ));
    }

    #[tokio::test]
    async fn durable_phase_boundary_precedes_broadcast() {
        let marked = std::cell::Cell::new(false);
        let broadcast_called = std::cell::Cell::new(false);

        broadcast_after_journal_mark(
            || {
                marked.set(true);
                Ok(true)
            },
            || async {
                assert!(marked.get(), "journal must be marked before broadcasting");
                broadcast_called.set(true);
                Ok(())
            },
        )
        .await
        .expect("journal mark and broadcast succeed");

        assert!(broadcast_called.get());
    }

    #[tokio::test]
    async fn failed_phase_boundaries_prevent_broadcast() {
        let broadcast_called = std::cell::Cell::new(false);

        let attempt = broadcast_after_journal_mark(
            || Ok(false),
            || async {
                broadcast_called.set(true);
                Ok(())
            },
        )
        .await
        .expect_err("a no-op journal mark must stop submission");

        assert!(matches!(
            attempt,
            DpnsVoteAttempt::FailedBeforeSubmission(_)
        ));
        assert!(!broadcast_called.get());

        let attempt = broadcast_after_journal_mark(
            || Err(TaskError::DpnsVoteTargetBusy),
            || async {
                broadcast_called.set(true);
                Ok(())
            },
        )
        .await
        .expect_err("a failed journal write must stop submission");

        assert!(matches!(
            attempt,
            DpnsVoteAttempt::FailedBeforeSubmission(_)
        ));
        assert!(!broadcast_called.get());
    }
}
