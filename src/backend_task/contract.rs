use std::collections::BTreeMap;

use super::BackendTaskSuccessResult;
use crate::app::TaskResult;
use crate::context::AppContext;
use crate::database::contracts::InsertTokensToo;
use crate::database::contracts::InsertTokensToo::NoTokensShouldBeAdded;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::tokens::tokens_screen::{ContractDescriptionInfo, TokenInfo};
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::group::group_action::GroupAction;
use dash_sdk::dpp::group::group_action_status::GroupActionStatus;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::group_actions::GroupActionsQuery;
use dash_sdk::platform::{
    DataContract, Document, DocumentQuery, Fetch, FetchMany, Identifier, IdentityPublicKey,
};
use dash_sdk::query_types::IndexMap;

#[derive(Debug, Clone, PartialEq)]
pub enum ContractTask {
    FetchContracts(Vec<Identifier>),
    FetchContractsWithDescriptions(Vec<Identifier>),
    FetchActiveGroupActions(QualifiedContract, QualifiedIdentity),
    RemoveContract(Identifier),
    RegisterDataContract(DataContract, String, QualifiedIdentity, IdentityPublicKey), // contract, alias, identity, signing_key
    UpdateDataContract(DataContract, QualifiedIdentity, IdentityPublicKey), // contract, identity, signing_key
    SaveDataContract(DataContract, Option<String>, InsertTokensToo),
}

impl AppContext {
    pub async fn run_contract_task(
        &self,
        task: ContractTask,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            ContractTask::FetchContracts(identifiers) => {
                match DataContract::fetch_many(sdk, identifiers).await {
                    Ok(data_contracts) => {
                        let mut results = vec![];
                        for data_contract in data_contracts {
                            if let Some(contract) = &data_contract.1 {
                                self.db
                                    .insert_contract_if_not_exists(
                                        contract,
                                        None,
                                        NoTokensShouldBeAdded,
                                        self,
                                    )
                                    .map_err(|e| {
                                        format!("Error inserting contract into the database: {}", e)
                                    })?;
                                results.push(Some(contract.clone()));
                            } else {
                                results.push(None);
                            }
                        }
                        Ok(BackendTaskSuccessResult::FetchedContracts(results))
                    }
                    Err(e) => Err(format!("Error fetching contracts: {}", e)),
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
                                    let token_configuration = contract
                                        .expected_token_configuration(*token.0)
                                        .expect("Expected to get token configuration");
                                    let token_name = {
                                        let TokenConfigurationConvention::V0(conventions) =
                                            &token_configuration.conventions();
                                        conventions
                                            .singular_form_by_language_code_or_default("en")
                                            .to_string()
                                    };

                                    let token_info = TokenInfo {
                                        token_id: contract.token_id(*token.0).unwrap_or_default(),
                                        token_name,
                                        data_contract_id: contract.id(),
                                        token_position: *token.0,
                                        token_configuration: token_configuration.clone(),
                                        description: token.1.description().clone(),
                                    };

                                    token_infos.push(token_info);
                                }

                                let contract_description_info =
                                    document_option.map(|document| ContractDescriptionInfo {
                                        data_contract_id: contract.id(),
                                        description: document
                                            .get("description")
                                            .and_then(|v| v.as_text())
                                            .unwrap_or_default()
                                            .to_string(),
                                    });

                                results.insert(
                                    contract.id(),
                                    (contract_description_info, token_infos),
                                );
                            }
                        }
                        Ok(BackendTaskSuccessResult::ContractsWithDescriptions(results))
                    }
                    Err(e) => Err(format!("Error fetching contracts: {}", e)),
                }
            }
            ContractTask::FetchActiveGroupActions(contract, identity) => {
                let mut actions = IndexMap::new();

                let mut group_positions = vec![];
                for group in contract.contract.groups() {
                    if group.1.members().contains_key(&identity.identity.id()) {
                        group_positions.push(group.0);
                    }
                }

                for group_position in group_positions {
                    let query = GroupActionsQuery {
                        contract_id: contract.contract.id(),
                        group_contract_position: *group_position,
                        status: GroupActionStatus::ActionActive,
                        start_at_action_id: None,
                        limit: None,
                    };

                    let group_actions = GroupAction::fetch_many(sdk, query)
                        .await
                        .map_err(|e| format!("Error fetching group actions: {}", e))?;

                    for group_action in group_actions {
                        if let Some(action) = &group_action.1 {
                            actions.insert(group_action.0, action.clone());
                        }
                    }
                }

                Ok(BackendTaskSuccessResult::ActiveGroupActions(actions))
            }
            ContractTask::RegisterDataContract(data_contract, alias, identity, signing_key) => {
                AppContext::register_data_contract(
                    self,
                    data_contract,
                    alias,
                    identity,
                    signing_key,
                    sdk,
                    sender,
                )
                .await
                .map(|_| {
                    BackendTaskSuccessResult::Message(
                        "Successfully registered contract".to_string(),
                    )
                })
                .map_err(|e| format!("Error registering contract: {}", e))
            }
            ContractTask::UpdateDataContract(mut data_contract, identity, signing_key) => {
                AppContext::update_data_contract(
                    self,
                    &mut data_contract,
                    identity,
                    signing_key,
                    sdk,
                    sender,
                )
                .await
                .map(|_| {
                    BackendTaskSuccessResult::Message("Successfully updated contract".to_string())
                })
                .map_err(|e| format!("Error updating contract: {}", e))
            }
            ContractTask::RemoveContract(identifier) => self
                .remove_contract(&identifier)
                .map(|_| {
                    BackendTaskSuccessResult::Message("Successfully removed contract".to_string())
                })
                .map_err(|e| format!("Error removing contract: {}", e)),
            ContractTask::SaveDataContract(data_contract, alias, insert_tokens_too) => {
                self.db
                    .insert_contract_if_not_exists(
                        &data_contract,
                        alias.as_deref(),
                        insert_tokens_too,
                        self,
                    )
                    .map_err(|e| format!("Error inserting contract into the database: {}", e))?;
                Ok(BackendTaskSuccessResult::Message(
                    "DataContract successfully saved".to_string(),
                ))
            }
        }
    }
}
