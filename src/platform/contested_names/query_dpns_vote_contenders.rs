use crate::context::AppContext;
use crate::platform::contract::ContractTask;
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

impl AppContext {
    pub(super) async fn query_dpns_vote_contenders(
        &self,
        name: &String,
        sdk: Sdk,
    ) -> Result<(), String> {
        let data_contract = self.dpns_contract.as_ref();
        let document_type = data_contract
            .document_type_for_name("domain")
            .expect("expected document type");
        let Some(contested_index) = document_type.find_contested_index() else {
            return Err("No contested index on dpns domains".to_string());
        };
        let index_values = vec![Value::from("dash"), Value::Text(name.clone())]; // hardcoded for dpns

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

        let contenders =
            ContenderWithSerializedDocument::fetch_many(&sdk, contenders_query.clone())
                .await
                .map_err(|e| {
                    tracing::error!("error fetching contested resources: {}", e);
                    format!("error fetching contested resources: {}", e.to_string())
                })?;
        self.db
            .insert_or_update_contenders(name, &contenders, document_type, self)
            .map_err(|e| e.to_string())
    }
}
