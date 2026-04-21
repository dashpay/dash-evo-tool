use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::util::strings::convert_to_homograph_safe_chars;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::dpp::voting::vote_polls::contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll;
use dash_sdk::dpp::voting::votes::Vote;
use dash_sdk::dpp::voting::votes::resource_vote::ResourceVote;
use dash_sdk::dpp::voting::votes::resource_vote::v0::ResourceVoteV0;
use dash_sdk::drive::query::vote_polls_by_document_type_query::VotePollsByDocumentTypeQuery;
use dash_sdk::platform::FetchMany;
use dash_sdk::platform::transition::vote::PutVote;
use dash_sdk::query_types::ContestedResource;
use std::sync::Arc;

/// Build `[Value::from("dash"), Value::Text(normalized_label)]` for a DPNS vote poll.
///
/// Label is normalized via `convert_to_homograph_safe_chars` (`alice` → `a11ce`);
/// Platform indexes polls under the normalized form.
fn dpns_vote_poll_index_values(name: &str) -> Vec<Value> {
    vec![
        Value::from("dash"),
        Value::Text(convert_to_homograph_safe_chars(name)),
    ]
}

impl AppContext {
    pub(super) async fn vote_on_dpns_name(
        self: &Arc<Self>,
        name: &str,
        vote_choice: ResourceVoteChoice,
        voters: &[QualifiedIdentity],
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|_| TaskError::InternalSendError)?;

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
        let index_values = dpns_vote_poll_index_values(name);

        let vote_poll = ContestedDocumentResourceVotePoll {
            index_name: contested_index.name.clone(),
            index_values,
            document_type_name: document_type.name().to_string(),
            contract_id: data_contract.id(),
        };

        // Pre-flight: confirm Platform has an open poll for this label before
        // broadcasting — avoids the ~70 s retry chain on a missing poll.
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

        let mut vote_results = vec![];

        for qualified_identity in voters.iter() {
            if let Some((_, public_key)) = &qualified_identity.associated_voter_identity {
                let resource_vote = ResourceVoteV0 {
                    vote_poll: vote_poll.clone().into(),
                    resource_vote_choice: vote_choice,
                };
                let vote = Vote::ResourceVote(ResourceVote::V0(resource_vote));

                let result = vote
                    .put_to_platform_and_wait_for_response(
                        qualified_identity.identity.id(),
                        public_key,
                        sdk,
                        qualified_identity,
                        None,
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| Arc::new(TaskError::from(e)));

                vote_results.push((name.to_owned(), vote_choice, result));
            } else {
                return Err(TaskError::NoVotingIdentity {
                    identity_id: qualified_identity.identity.id().to_string(Encoding::Base58),
                });
            }
        }

        Ok(BackendTaskSuccessResult::DPNSVoteResults(vote_results))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_values_normalizes_homograph_chars() {
        // Given: a label containing i/l/o homograph characters.
        let label = "alice";

        // When: constructing the vote poll index values.
        let values = dpns_vote_poll_index_values(label);

        // Then: first element is always the `"dash"` parent, second is the
        // normalized label (i/l → 1, o → 0).
        assert_eq!(values.len(), 2);
        assert_eq!(values[0], Value::from("dash"));
        assert_eq!(values[1], Value::Text("a11ce".to_owned()));
    }

    #[test]
    fn index_values_preserve_non_homograph_label() {
        // Given: a label with no homograph characters (e.g. `bar22`).
        let label = "bar22";

        // When: constructing the vote poll index values.
        let values = dpns_vote_poll_index_values(label);

        // Then: normalization leaves it unchanged.
        assert_eq!(values[1], Value::Text("bar22".to_owned()));
    }
}
