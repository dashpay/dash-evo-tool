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

/// Returns the first identifier in `identifiers` that appears earlier in the
/// same slice, or `None` if every entry is unique.
///
/// Used as a task-boundary duplicate guard for [`ContractTask::FetchContracts`].
fn first_duplicate_id(identifiers: &[Identifier]) -> Option<Identifier> {
    identifiers
        .iter()
        .enumerate()
        .find_map(|(index, id)| identifiers[..index].contains(id).then_some(*id))
}

/// Returns the first identifier in `requested` that is also present in
/// `existing`, or `None` when none of the requested IDs are already loaded.
fn first_already_loaded_id(
    requested: &[Identifier],
    existing: &[Identifier],
) -> Option<Identifier> {
    requested
        .iter()
        .find(|identifier| existing.contains(identifier))
        .copied()
}

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
    ) -> Result<BackendTaskSuccessResult, crate::backend_task::error::TaskError> {
        match task {
            ContractTask::FetchContracts(identifiers) => {
                // Task-boundary duplicate / already-loaded enforcement.
                //
                // The `AddContractsScreen` UI also performs these checks, but
                // duplicating them here protects non-UI callers (e.g. MCP tools)
                // and closes the TOCTOU window between the screen check and
                // the network fetch / persistence below.
                if let Some(duplicate) = first_duplicate_id(&identifiers) {
                    return Err(
                        crate::backend_task::error::TaskError::DuplicateContractInRequest {
                            contract_id: duplicate,
                        },
                    );
                }
                let existing_ids = self.loaded_contract_ids()?;
                if let Some(already_loaded) = first_already_loaded_id(&identifiers, &existing_ids) {
                    return Err(
                        crate::backend_task::error::TaskError::ContractAlreadyLoaded {
                            contract_id: already_loaded,
                        },
                    );
                }

                match DataContract::fetch_many(sdk, identifiers).await {
                    Ok(data_contracts) => {
                        let mut results = vec![];
                        for data_contract in data_contracts {
                            if let Some(contract) = &data_contract.1 {
                                self.db.insert_contract_if_not_exists(
                                    contract,
                                    None,
                                    NoTokensShouldBeAdded,
                                    self,
                                )?;
                                results.push(Some(contract.clone()));
                            } else {
                                results.push(None);
                            }
                        }
                        Ok(BackendTaskSuccessResult::FetchedContracts(results))
                    }
                    Err(e) => Err(crate::backend_task::error::TaskError::from(e)),
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
                                    .map_err(crate::backend_task::error::TaskError::from)?;

                                let mut token_infos = vec![];
                                for token in contract.tokens() {
                                    let token_configuration = match contract
                                        .expected_token_configuration(*token.0)
                                    {
                                        Ok(config) => config,
                                        Err(e) => {
                                            tracing::warn!(
                                                "Skipping token at position {} in contract {}: {}",
                                                token.0,
                                                contract.id(),
                                                e
                                            );
                                            continue;
                                        }
                                    };
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
                    Err(e) => Err(crate::backend_task::error::TaskError::from(e)),
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
                        .map_err(crate::backend_task::error::TaskError::from)?;

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
            }
            ContractTask::RemoveContract(identifier) => self
                .remove_contract(&identifier)
                .map(|_| BackendTaskSuccessResult::RemovedContract),
            ContractTask::SaveDataContract(data_contract, alias, insert_tokens_too) => {
                // Task-boundary enforcement: the local DB layer silently
                // skips system contract IDs and `INSERT OR IGNORE` makes a
                // duplicate user-contract insert a no-op, so without this
                // check non-UI callers could appear to "save" a contract
                // when the database state did not actually change.
                let contract_id = data_contract.id();
                if self.is_system_contract_id(&contract_id) {
                    return Err(
                        crate::backend_task::error::TaskError::SystemContractImmutable {
                            contract_id,
                        },
                    );
                }
                let existing_ids = self.loaded_contract_ids()?;
                if existing_ids.contains(&contract_id) {
                    return Err(
                        crate::backend_task::error::TaskError::ContractAlreadyLoaded {
                            contract_id,
                        },
                    );
                }

                self.db.insert_contract_if_not_exists(
                    &data_contract,
                    alias.as_deref(),
                    insert_tokens_too,
                    self,
                )?;
                Ok(BackendTaskSuccessResult::SavedContract)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(byte: u8) -> Identifier {
        Identifier::from_bytes(&[byte; 32]).expect("32 bytes is a valid identifier")
    }

    #[test]
    fn first_duplicate_id_returns_none_for_unique_inputs() {
        assert!(first_duplicate_id(&[id(1), id(2), id(3)]).is_none());
    }

    #[test]
    fn first_duplicate_id_returns_first_repeat() {
        assert_eq!(
            first_duplicate_id(&[id(1), id(2), id(1), id(3)]),
            Some(id(1))
        );
    }

    #[test]
    fn first_already_loaded_id_returns_first_match_in_request_order() {
        let requested = [id(7), id(2), id(9)];
        let existing = [id(9), id(2)];
        assert_eq!(first_already_loaded_id(&requested, &existing), Some(id(2)));
    }

    #[test]
    fn first_already_loaded_id_returns_none_when_no_overlap() {
        assert!(first_already_loaded_id(&[id(1), id(2)], &[id(3), id(4)]).is_none());
    }
}
