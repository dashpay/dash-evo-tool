use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::platform::{DataContract, FetchMany, Identifier, IdentityPublicKey};
use dash_sdk::Sdk;

use super::BackendTaskSuccessResult;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContractTask {
    FetchContracts(Vec<Identifier>),
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
