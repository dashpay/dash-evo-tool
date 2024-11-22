use crate::app::TaskResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::voting::contender_structs::ContenderWithSerializedDocument;
use dash_sdk::dpp::voting::vote_polls::contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll;
use dash_sdk::drive::query::vote_poll_vote_state_query::{
    ContestedDocumentVotePollDriveQuery, ContestedDocumentVotePollDriveQueryResultType,
};
use dash_sdk::platform::FetchMany;
use dash_sdk::Sdk;
use tokio::sync::mpsc;

impl AppContext {
    pub(super) async fn query_dpns_vote_contenders(
        &self,
        name: &String,
        sdk: &Sdk,
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<(), String> {
        let data_contract = self.dpns_contract.as_ref();
        let document_type = data_contract
            .document_type_for_name("domain")
            .expect("expected document type");
        let Some(contested_index) = document_type.find_contested_index() else {
            return Err("No contested index on dpns domains".to_string());
        };
        let index_values = [Value::from("dash"), Value::Text(name.clone())]; // hardcoded for dpns

        let vote_poll = ContestedDocumentResourceVotePoll {
            index_name: contested_index.name.clone(),
            index_values: index_values.to_vec(),
            document_type_name: document_type.name().to_string(),
            contract_id: data_contract.id(),
        };

        let contenders_query = ContestedDocumentVotePollDriveQuery {
            limit: None,
            offset: None,
            start_at: None,
            vote_poll: vote_poll.clone(),
            allow_include_locked_and_abstaining_vote_tally: true,
            result_type: ContestedDocumentVotePollDriveQueryResultType::DocumentsAndVoteTally,
        };

        // Define retries
        const MAX_RETRIES: usize = 3;
        let mut retries = 0;

        loop {
            match ContenderWithSerializedDocument::fetch_many(&sdk, contenders_query.clone()).await
            {
                Ok(contenders) => {
                    // If successful, proceed to insert/update contenders
                    return self
                        .db
                        .insert_or_update_contenders(name, &contenders, document_type, self)
                        .map_err(|e| e.to_string());
                }
                Err(e) => {
                    tracing::error!("Error fetching vote contenders: {}", e);
                    if let dash_sdk::Error::Proof(
                        dash_sdk::ProofVerifierError::GroveDBProofVerificationError {
                            proof_bytes,
                            path_query,
                            height,
                            time_ms,
                            error,
                        },
                    ) = &e
                    {
                        // Encode the query using bincode
                        let encoded_query = match bincode::encode_to_vec(
                            &contenders_query,
                            bincode::config::standard(),
                        )
                        .map_err(|encode_err| {
                            tracing::error!("Error encoding query: {}", encode_err);
                            format!("Error encoding query: {}", encode_err)
                        }) {
                            Ok(encoded_query) => encoded_query,
                            Err(e) => return Err(e),
                        };

                        // Encode the path_query using bincode
                        let verification_path_query_bytes =
                            match bincode::encode_to_vec(&path_query, bincode::config::standard())
                                .map_err(|encode_err| {
                                    tracing::error!("Error encoding path_query: {}", encode_err);
                                    format!("Error encoding path_query: {}", encode_err)
                                }) {
                                Ok(encoded_path_query) => encoded_path_query,
                                Err(e) => return Err(e),
                            };

                        if let Err(e) = self
                            .db
                            .insert_proof_log_item(ProofLogItem {
                                request_type: RequestType::GetContestedResourceIdentityVotes,
                                request_bytes: encoded_query,
                                verification_path_query_bytes,
                                height: *height,
                                time_ms: *time_ms,
                                proof_bytes: proof_bytes.clone(),
                                error: Some(error.clone()),
                            })
                            .map_err(|e| e.to_string())
                        {
                            return Err(e);
                        }
                    }
                    let error_str = e.to_string();
                    if error_str.contains("try another server")
                        || error_str.contains(
                            "contract not found when querying from value with contract info",
                        )
                    {
                        retries += 1;
                        if retries > MAX_RETRIES {
                            tracing::error!(
                                "Max retries reached for query_dpns_vote_contenders: {}",
                                e
                            );
                            return Err(format!(
                                "Error fetching vote contenders after retries: {}",
                                e
                            ));
                        } else {
                            continue;
                        }
                    } else {
                        // For other errors, return immediately
                        return Err(format!("Error fetching vote contenders: {}", e));
                    }
                }
            }
        }
    }
}
