use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::dpp::voting::vote_polls::contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll;
use dash_sdk::dpp::voting::votes::resource_vote::v0::ResourceVoteV0;
use dash_sdk::dpp::voting::votes::resource_vote::ResourceVote;
use dash_sdk::dpp::voting::votes::Vote;
use dash_sdk::platform::transition::vote::PutVote;
use dash_sdk::Sdk;
use std::sync::Arc;
use tokio::sync::mpsc;

impl AppContext {
    pub(super) async fn vote_on_dpns_name(
        self: &Arc<Self>,
        name: &String,
        vote_choice: ResourceVoteChoice,
        voters: &Vec<QualifiedIdentity>,
        sdk: &Sdk,
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Fetch DPNS contract and document type information
        let data_contract = self.dpns_contract.as_ref();
        let document_type = data_contract
            .document_type_for_name("domain")
            .expect("expected document type");

        let Some(contested_index) = document_type.find_contested_index() else {
            return Err("No contested index on dpns domains".to_string());
        };

        // Hardcoded values for DPNS
        let index_values = [Value::from("dash"), Value::Text(name.clone())];

        // Create the vote poll to use in the vote
        let vote_poll = ContestedDocumentResourceVotePoll {
            index_name: contested_index.name.clone(),
            index_values: index_values.to_vec(),
            document_type_name: document_type.name().to_string(),
            contract_id: data_contract.id(),
        };

        let mut vote_results = vec![];
        let mut strength = 0;

        // Iterate over the provided voters (QualifiedIdentity)
        for qualified_identity in voters.iter() {
            if let Some((_, public_key)) = &qualified_identity.associated_voter_identity {
                // Create the resource vote
                let resource_vote = ResourceVoteV0 {
                    vote_poll: vote_poll.clone().into(),
                    resource_vote_choice: vote_choice,
                };
                let vote = Vote::ResourceVote(ResourceVote::V0(resource_vote));

                // Submit the vote to the platform and await a response
                let result = vote
                    .put_to_platform_and_wait_for_response(
                        qualified_identity.identity.id(),
                        public_key,
                        &sdk,
                        qualified_identity,
                        None,
                    )
                    .await
                    .map_err(|e| format!("Error voting: {}", e))?;

                strength += qualified_identity.identity_type.vote_strength();
                vote_results.push(result);
            } else {
                return Err(format!(
                    "No associated voter identity for qualified identity: {:?}",
                    qualified_identity.identity.id()
                ));
            }
        }

        self.db
            .update_vote_count(
                name,
                self.network.to_string().as_str(),
                strength,
                vote_choice,
            )
            .map_err(|e| format!("error updating ending time: {}", e))?;

        Ok(BackendTaskSuccessResult::SuccessfulVotes(vote_results))
    }
}
