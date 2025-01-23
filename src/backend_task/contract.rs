use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::system_data_contracts::dpns_contract;
use dash_sdk::platform::{DataContract, Fetch, FetchMany, Identifier};
use dash_sdk::Sdk;

use super::BackendTaskSuccessResult;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContractTask {
    FetchDPNSContract,
    FetchContract(Identifier, Option<String>),
    FetchContracts(Vec<Identifier>),
    RemoveContract(Identifier),
    RegisterDataContract(DataContract, QualifiedIdentity),
}

impl AppContext {
    pub async fn run_contract_task(
        &self,
        task: ContractTask,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            ContractTask::FetchContract(identifier, name) => {
                match DataContract::fetch(sdk, identifier).await {
                    Ok(Some(data_contract)) => self
                        .db
                        .insert_contract_if_not_exists(&data_contract, name.as_deref(), self)
                        .map(|_| BackendTaskSuccessResult::FetchedContract(data_contract))
                        .map_err(|e| {
                            format!(
                                "Error inserting contract into the database: {}",
                                e.to_string()
                            )
                        }),
                    Ok(None) => Err("Contract not found".to_string()),
                    Err(e) => Err(format!("Error fetching contract: {}", e.to_string())),
                }
            }
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
            ContractTask::FetchDPNSContract => {
                match DataContract::fetch(sdk, Into::<Identifier>::into(dpns_contract::ID_BYTES))
                    .await
                {
                    Ok(Some(data_contract)) => self
                        .db
                        .insert_contract_if_not_exists(&data_contract, Some("dpns"), self)
                        .map(|_| BackendTaskSuccessResult::FetchedContract(data_contract))
                        .map_err(|e| e.to_string()),
                    Ok(None) => Err("No DPNS contract found".to_string()),
                    Err(e) => Err(format!("Error fetching DPNS contract: {}", e.to_string())),
                }
            }
            ContractTask::RegisterDataContract(data_contract, identity) => {
                AppContext::register_data_contract(data_contract, identity, sdk)
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
