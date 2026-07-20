use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dpns_voting::{DpnsVoteOperationId, DpnsVoteTargetKey};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::consensus::ConsensusError;
use dash_sdk::dpp::consensus::basic::BasicError;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
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
use dash_sdk::dpp::voting::votes::resource_vote::v0::ResourceVoteV0;
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

/// Build `[Value::from("dash"), Value::Text(normalized_label.to_owned())]` for a DPNS vote poll.
///
/// Caller must pre-normalize the label via `convert_to_homograph_safe_chars`
/// (`alice` → `a11ce`); Platform indexes polls under the normalized form.
fn dpns_vote_poll_index_values(normalized_label: &str) -> Vec<Value> {
    vec![
        Value::from("dash"),
        Value::Text(normalized_label.to_owned()),
    ]
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
        dash_sdk::Error::StateTransitionBroadcastError(broadcast_error) => {
            let rejected = broadcast_error.cause.is_some();
            let error = TaskError::from(error);
            if rejected {
                DpnsVoteAttempt::Rejected(error)
            } else {
                DpnsVoteAttempt::Unconfirmed(error)
            }
        }
        _ => DpnsVoteAttempt::FailedBeforeSubmission(TaskError::from(error)),
    }
}

fn classify_broadcast_journal_result(result: Result<(), TaskError>) -> Result<(), DpnsVoteAttempt> {
    result.map_err(DpnsVoteAttempt::Unconfirmed)
}

impl AppContext {
    pub(super) async fn submit_dpns_vote(
        self: &Arc<Self>,
        operation_id: DpnsVoteOperationId,
        key: &DpnsVoteTargetKey,
        name: &str,
        vote_choice: ResourceVoteChoice,
        qualified_identity: &QualifiedIdentity,
        sdk: &Sdk,
    ) -> Result<DpnsVoteAttempt, TaskError> {
        let data_contract = self.dpns_contract.as_ref();
        let document_type = data_contract
            .document_type_for_name("domain")
            .map_err(|_| TaskError::DataContractNotFound)?;

        let Some(contested_index) = document_type.find_contested_index() else {
            return Err(TaskError::ContractSchemaMismatch {
                detail: "DPNS domain document type has no contested index",
            });
        };

        let normalized_label = convert_to_homograph_safe_chars(name);
        let index_values = dpns_vote_poll_index_values(&normalized_label);

        let vote_poll = ContestedDocumentResourceVotePoll {
            index_name: contested_index.name.clone(),
            index_values,
            document_type_name: document_type.name().to_string(),
            contract_id: data_contract.id(),
        };

        // Pre-flight: confirm Platform has an open poll for this label before
        // broadcasting — fails fast with VotePollNotFound if it doesn't.
        let existence_query = VotePollsByDocumentTypeQuery {
            contract_id: data_contract.id(),
            document_type_name: document_type.name().to_string(),
            index_name: contested_index.name.clone(),
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
        if let Err(error) = state_transition.broadcast(sdk, Some(settings)).await {
            return Ok(classify_broadcast_error(error));
        }
        if let Err(attempt) =
            classify_broadcast_journal_result(self.mark_dpns_vote_broadcast(operation_id, key))
        {
            return Ok(attempt);
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
    fn index_values_uses_the_given_normalized_label() {
        // Given: a pre-normalized DPNS label (homographs already substituted).
        let normalized = "a11ce";

        // When: constructing the vote poll index values.
        let values = dpns_vote_poll_index_values(normalized);

        // Then: first element is the `"dash"` parent, second is the label as-given.
        assert_eq!(values.len(), 2);
        assert_eq!(values[0], Value::from("dash"));
        assert_eq!(values[1], Value::Text("a11ce".to_owned()));
    }

    #[test]
    fn index_values_do_not_renormalize_the_label() {
        // Given: a label that still contains homograph characters.
        let not_yet_normalized = "alice";

        // When: passing it directly to the helper (violating the contract).
        let values = dpns_vote_poll_index_values(not_yet_normalized);

        // Then: the helper does NOT renormalize — the raw label is returned as-is.
        // (Caller is responsible for normalizing before calling.)
        assert_eq!(values[1], Value::Text("alice".to_owned()));
    }

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
    fn broadcast_transport_error_fails_before_submission() {
        let attempt =
            classify_broadcast_error(dash_sdk::Error::Generic("connection refused".to_owned()));

        assert!(matches!(
            attempt,
            DpnsVoteAttempt::FailedBeforeSubmission(_)
        ));
    }

    #[test]
    fn journal_failure_after_broadcast_is_unconfirmed() {
        let attempt = classify_broadcast_journal_result(Err(TaskError::DpnsVoteTargetBusy))
            .expect_err("a failed journal mark must stop the confirmation wait");

        assert!(matches!(attempt, DpnsVoteAttempt::Unconfirmed(_)));
    }
}
