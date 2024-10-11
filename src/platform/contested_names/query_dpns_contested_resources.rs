use crate::context::AppContext;
use crate::platform::contract::ContractTask;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::drive::query::vote_polls_by_document_type_query::VotePollsByDocumentTypeQuery;
use dash_sdk::platform::FetchMany;
use dash_sdk::query_types::ContestedResource;
use dash_sdk::Sdk;

impl AppContext {
    pub(super) async fn query_dpns_contested_resources(&self, sdk: Sdk) -> Result<(), String> {
        if self.dpns_contract.is_none() {
            self.run_contract_task(ContractTask::FetchDPNSContract, &sdk)
                .await?;
        }
        let Some(data_contract) = self.dpns_contract.as_ref() else {
            return Err("DPNS contract not found".to_string());
        };
        let document_type = data_contract
            .document_type_for_name("domain")
            .expect("expected document type");
        let Some(contested_index) = document_type.find_contested_index() else {
            return Err("No contested index on dpns domains".to_string());
        };
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

        let contested_resources =
            ContestedResource::fetch_many(&sdk, query)
                .await
                .map_err(|e| {
                    tracing::error!("error fetching contested resources: {}", e);
                    format!("error fetching contested resources: {}", e.to_string())
                })?;

        let contested_resources_as_strings: Vec<String> = contested_resources
            .0
            .into_iter()
            .map(|contested_resource| {
                contested_resource
                    .0
                    .as_str()
                    .expect("expected str")
                    .to_string()
            })
            .collect();

        let names_to_be_updated = self
            .db
            .insert_name_contests_as_normalized_names(contested_resources_as_strings, &self)
            .map_err(|e| e.to_string())?;

        for name in names_to_be_updated {
            self.query_dpns_vote_contenders(&name, sdk.clone()).await?;
        }

        Ok(())
    }
}
