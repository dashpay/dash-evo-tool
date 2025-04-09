use std::collections::BTreeMap;

use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::tokens::tokens_screen::{ContractDescriptionInfo, TokenInfo};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{
    DataContract, Document, DocumentQuery, Fetch, FetchMany, Identifier, IdentityPublicKey,
};
use dash_sdk::Sdk;

use super::BackendTaskSuccessResult;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContractTask {
    FetchContracts(Vec<Identifier>),
    FetchContractsWithDescriptions(Vec<Identifier>),
    RemoveContract(Identifier),
    RegisterDataContract(DataContract, String, QualifiedIdentity, IdentityPublicKey),
}

impl AppContext {
    pub async fn run_contract_task(
        &self,
        task: ContractTask,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            ContractTask::FetchContracts(identifiers) => {
                match DataContract::fetch_many(sdk, identifiers).await {
                    Ok(data_contracts) => {
                        let mut results = vec![];
                        for data_contract in data_contracts {
                            if let Some(contract) = &data_contract.1 {
                                self.db
                                    .insert_contract_if_not_exists(contract, None, self)
                                    .map_err(|e| {
                                        format!(
                                            "Error inserting contract into the database: {}",
                                            e.to_string()
                                        )
                                    })?;
                                results.push(Some(contract.clone()));
                            } else {
                                results.push(None);
                            }
                        }
                        Ok(BackendTaskSuccessResult::FetchedContracts(results))
                    }
                    Err(e) => Err(format!("Error fetching contracts: {}", e.to_string())),
                }
            }
            ContractTask::FetchContractsWithDescriptions(identifiers) => {
                // For each identifier, fetch the contract as in FetchContracts
                // and then if successful, fetch the contract description from the Search Contract's "fullDescription" document type
                match DataContract::fetch_many(sdk, identifiers).await {
                    Ok(data_contracts) => {
                        let mut results = BTreeMap::new();
                        for data_contract in data_contracts {
                            if let Some(contract) = &data_contract.1 {
                                // Fetch the contract description from the Search Contract
                                let search_contract = &self.keyword_search_contract;
                                let document_query = DocumentQuery {
                                    data_contract: search_contract.clone(),
                                    document_type_name: "fullDescription".to_string(),
                                    limit: 1,
                                    start: None,
                                    where_clauses: vec![WhereClause {
                                        field: "contractId".to_string(),
                                        operator: WhereOperator::Equal,
                                        value: Value::Identifier(contract.id().into()),
                                    }],
                                    order_by_clauses: vec![],
                                };
                                let document_option = Document::fetch(sdk, document_query)
                                    .await
                                    .map_err(|e| format!("Error fetching description: {}", e))?;

                                let mut token_infos = vec![];
                                for token in contract.tokens() {
                                    let token_name = {
                                        let token_configuration = contract
                                            .expected_token_configuration(*token.0)
                                            .expect("Expected to get token configuration")
                                            .as_cow_v0();
                                        let conventions = match &token_configuration.conventions {
                                            TokenConfigurationConvention::V0(conventions) => {
                                                conventions
                                            }
                                        };
                                        conventions
                                            .plural_form_by_language_code_or_default("en")
                                            .to_string()
                                    };

                                    let token_info = TokenInfo {
                                        token_identifier: contract
                                            .token_id(*token.0)
                                            .unwrap_or_default(),
                                        token_name,
                                        data_contract_id: contract.id(),
                                        token_position: *token.0,
                                        description: token.1.description(),
                                    };

                                    token_infos.push(token_info);
                                }

                                if let Some(document) = document_option {
                                    let contract_description_info = ContractDescriptionInfo {
                                        data_contract_id: contract.id(),
                                        description: document
                                            .get("description")
                                            .and_then(|v| v.as_text())
                                            .unwrap_or_default()
                                            .to_string(),
                                    };

                                    results.insert(
                                        contract.id(),
                                        (Some(contract_description_info), token_infos),
                                    );
                                }
                            }
                        }
                        Ok(BackendTaskSuccessResult::ContractsWithDescriptions(results))
                    }
                    Err(e) => Err(format!("Error fetching contracts: {}", e.to_string())),
                }
            }
            ContractTask::RegisterDataContract(data_contract, alias, identity, signing_key) => {
                AppContext::register_data_contract(
                    &self,
                    data_contract,
                    alias,
                    identity,
                    signing_key,
                    sdk,
                )
                .await
                .map(|_| {
                    BackendTaskSuccessResult::Message(
                        "Successfully registered contract".to_string(),
                    )
                })
                .map_err(|e| format!("Error registering contract: {}", e.to_string()))
            }
            ContractTask::RemoveContract(identifier) => self
                .remove_contract(&identifier)
                .map(|_| {
                    BackendTaskSuccessResult::Message("Successfully removed contract".to_string())
                })
                .map_err(|e| format!("Error removing contract: {}", e.to_string())),
        }
    }
}
