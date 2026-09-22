use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::request_type::RequestType;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::voting::contender_structs::ContenderWithSerializedDocument;
use dash_sdk::dpp::voting::vote_polls::contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll;
use dash_sdk::drive::query::vote_poll_vote_state_query::{
    ContestedDocumentVotePollDriveQuery, ContestedDocumentVotePollDriveQueryResultType,
};
use dash_sdk::platform::FetchMany;

impl AppContext {
    pub(super) async fn query_dpns_vote_contenders(
        &self,
        name: &str,
        sdk: &Sdk,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<(), TaskError> {
        let data_contract = self.dpns_contract.as_ref();
        let document_type = data_contract
            .document_type_for_name("domain")
            .map_err(|_| TaskError::DataContractNotFound)?;
        let Some(contested_index) = document_type.find_contested_index() else {
            return Err(TaskError::PlatformFetchError {
                source: Box::new(dash_sdk::Error::Generic(
                    "No contested index on dpns domains".to_string(),
                )),
            });
        };
        let index_values = [Value::from("dash"), Value::Text(name.to_owned())]; // hardcoded for dpns

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

        const MAX_RETRIES: usize = 3;
        let mut retries = 0;

        loop {
            match ContenderWithSerializedDocument::fetch_many(sdk, contenders_query.clone()).await {
                Ok(contenders) => {
                    return self.insert_or_update_contenders(name, &contenders, document_type);
                }
                Err(e) => {
                    tracing::error!("Error fetching vote contenders: {}", e);
                    super::log_contested_proof_error(
                        &e,
                        RequestType::GetContestedResourceIdentityVotes,
                    );
                    // TODO(#875): replace substring match once the SDK exposes a
                    // structural "contract not found" variant.
                    if matches!(e, dash_sdk::Error::StaleNode(_))
                        || e.to_string().contains(
                            "contract not found when querying from value with contract info",
                        )
                    {
                        retries += 1;
                        if retries > MAX_RETRIES {
                            tracing::error!(
                                "Max retries reached for query_dpns_vote_contenders: {}",
                                e
                            );
                            return Err(TaskError::from(e));
                        } else {
                            continue;
                        }
                    } else {
                        return Err(TaskError::from(e));
                    }
                }
            }
        }
    }
}
