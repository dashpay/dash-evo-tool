use dash_sdk::platform::transition::vote::PutVote;

use crate::context::AppContext;
use crate::platform::contract::ContractTask;
use crate::platform::BackendTask;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::DataContract;
use dash_sdk::dpp::{
    data_contract::{accessors::v0::DataContractV0Getters, document_type::DocumentType},
    identifier::Identifier,
    platform_value::{string_encoding::Encoding, Value},
    voting::{
        contender_structs::ContenderWithSerializedDocument,
        vote_choices::resource_vote_choice::ResourceVoteChoice,
        vote_polls::{
            contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll, VotePoll,
        },
        votes::{resource_vote::ResourceVote, Vote},
    },
};
use dash_sdk::drive::query::{
    vote_poll_vote_state_query::{
        ContestedDocumentVotePollDriveQuery, ContestedDocumentVotePollDriveQueryResultType,
    },
    vote_polls_by_document_type_query::VotePollsByDocumentTypeQuery,
    VotePollsByEndDateDriveQuery,
};
use dash_sdk::{
    platform::{DocumentQuery, FetchMany},
    query_types::ContestedResource,
    Sdk,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContestedResourceTask {
    QueryDPNSContestedResources,
    // QueryVoteContenders(String, Vec<Value>, String, Identifier),
    // VoteOnContestedResource(VotePoll, ResourceVoteChoice),
}

impl AppContext {
    pub async fn run_contested_resource_task(
        &self,
        task: ContestedResourceTask,
        sdk: &Sdk,
    ) -> Result<(), String> {
        match &task {
            ContestedResourceTask::QueryDPNSContestedResources => {
                if self.dpns_contract.is_none() {
                    self.run_contract_task(ContractTask::FetchDPNSContract, sdk)
                        .await?;
                }
                let Some(data_contract) = self.dpns_contract.as_ref() else {
                    return Err("DPNS contract not found".to_string());
                };
                let document_type = data_contract
                    .document_type_for_name("domain")
                    .expect("expected document type");
                if let Some(contested_index) = document_type.find_contested_index() {
                    let query = VotePollsByDocumentTypeQuery {
                        contract_id: data_contract.id(),
                        document_type_name: document_type.name().to_string(),
                        index_name: contested_index.name.clone(),
                        start_at_value: None,
                        start_index_values: vec!["dash".into()], // hardcoded for dpns
                        end_index_values: vec![],
                        limit: None,
                        order_ascending: true,
                    };

                    let contested_resources = ContestedResource::fetch_many(&sdk, query).await.map_err(|e| e.to_string())?;

                    let contested_resources_as_strings : Vec<String> = contested_resources.0.into_iter().map(|contested_resource| contested_resource.0.as_str().expect("expected str").to_string()).collect();

                    self.db.insert_name_contests_as_normalized_names(contested_resources_as_strings, &self).map_err(|e| e.to_string())?;

                    Ok(())
                } else {
                    Err("No contested index on dpns domains".to_string())
                }
            } //     ContestedResourceTask::QueryVoteContenders(
              //         index_name,
              //         index_values,
              //         document_type_name,
              //         contract_id,
              //     ) => {
              //         let vote_poll = ContestedDocumentResourceVotePoll {
              //             index_name: index_name.to_string(),
              //             index_values: index_values.to_vec(),
              //             document_type_name: document_type_name.to_string(),
              //             contract_id: *contract_id,
              //         };
              //
              //         let contenders_query = ContestedDocumentVotePollDriveQuery {
              //             limit: None,
              //             offset: None,
              //             start_at: None,
              //             vote_poll: vote_poll.clone(),
              //             allow_include_locked_and_abstaining_vote_tally: true,
              //             result_type:
              //             ContestedDocumentVotePollDriveQueryResultType::DocumentsAndVoteTally,
              //         };
              //
              //         let contenders = match ContenderWithSerializedDocument::fetch_many(
              //             sdk,
              //             contenders_query.clone(),
              //         )
              //             .await
              //         {
              //             Ok(contenders) => {
              //                 // TODO: Insert contenders into the database
              //                 BackendEvent::TaskCompleted {
              //                     task: Task::Document(task),
              //                     execution_result: Ok(CompletedTaskPayload::ContestedResourceContenders(
              //                         contenders_query.vote_poll,
              //                         contenders,
              //                         None,
              //                     )),
              //                 }
              //             }
              //             Err(e) => {
              //                 BackendEvent::TaskCompleted {
              //                     task: Task::Document(task),
              //                     execution_result: Err(format!("{e}")),
              //                 }
              //             }
              //         };
              //     }
              //     ContestedResourceTask::VoteOnContestedResource(vote_poll, vote_choice) => {
              //         let mut vote = Vote::default();
              //         let identity_private_keys_lock = self.known_identities_private_keys.lock().await;
              //         let loaded_identity_lock = match self.loaded_identity.lock().await.clone() {
              //             Some(identity) => identity,
              //             None => {
              //                 return BackendEvent::TaskCompleted {
              //                     task: Task::Document(task),
              //                     execution_result: Err(
              //                         "No loaded identity for signing vote transaction".to_string(),
              //                     ),
              //                 };
              //             }
              //         };
              //
              //         let mut signer = SimpleSigner::default();
              //         let Identity::V0(identity_v0) = &loaded_identity_lock;
              //         for (key_id, public_key) in &identity_v0.public_keys {
              //             let identity_key_tuple = (identity_v0.id, *key_id);
              //             if let Some(private_key_bytes) =
              //                 identity_private_keys_lock.get(&identity_key_tuple)
              //             {
              //                 signer
              //                     .private_keys
              //                     .insert(public_key.clone(), private_key_bytes.clone());
              //             }
              //         }
              //
              //         let voting_public_key = match loaded_identity_lock.get_first_public_key_matching(
              //             Purpose::VOTING,
              //             HashSet::from(SecurityLevel::full_range()),
              //             HashSet::from(KeyType::all_key_types()),
              //             false,
              //         ) {
              //             Some(voting_key) => voting_key,
              //             None => {
              //                 return BackendEvent::TaskCompleted {
              //                     task: Task::Document(task),
              //                     execution_result: Err(
              //                         "No voting key in the loaded identity. Are you sure it's a masternode identity?".to_string()
              //                     ),
              //                 };
              //             }
              //         };
              //
              //         match vote {
              //             Vote::ResourceVote(ref mut resource_vote) => match resource_vote {
              //                 ResourceVote::V0(ref mut resource_vote_v0) => {
              //                     resource_vote_v0.vote_poll = vote_poll.clone();
              //                     resource_vote_v0.resource_vote_choice = *vote_choice;
              //                     let pro_tx_hash = self
              //                         .loaded_identity_pro_tx_hash
              //                         .lock()
              //                         .await
              //                         .expect("Expected a proTxHash in AppState");
              //                     match vote
              //                         .put_to_platform_and_wait_for_response(
              //                             pro_tx_hash,
              //                             voting_public_key,
              //                             sdk,
              //                             &signer,
              //                             None,
              //                         )
              //                         .await
              //                     {
              //                         Ok(_) => {
              //                             // TODO: Insert vote result into the database
              //                             BackendEvent::TaskCompleted {
              //                                 task: Task::Document(task),
              //                                 execution_result: Ok(CompletedTaskPayload::String(
              //                                     "Vote cast successfully".to_string(),
              //                                 )),
              //                             }
              //                         }
              //                         Err(e) => BackendEvent::TaskCompleted {
              //                             task: Task::Document(task),
              //                             execution_result: Err(e.to_string()),
              //                         },
              //                     }
              //                 }
              //             },
              //         }
              //     }
        }
    }
}
