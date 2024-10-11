use crate::context::AppContext;
use dash_sdk::dpp::system_data_contracts::dpns_contract;
use dash_sdk::platform::{DataContract, Fetch, Identifier};
use dash_sdk::Sdk;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ContractTask {
    FetchDPNSContract,
    FetchContract(Identifier, Option<String>),
}

impl AppContext {
    pub async fn run_contract_task(&self, task: ContractTask, sdk: &Sdk) -> Result<(), String> {
        match task {
            ContractTask::FetchContract(identifier, name) => {
                match DataContract::fetch(sdk, identifier).await {
                    Ok(Some(data_contract)) => self
                        .db
                        .insert_contract_if_not_exists(&data_contract, name.as_deref(), self)
                        .map_err(|e| e.to_string()),
                    Ok(None) => Ok(()),
                    Err(e) => Err(e.to_string()),
                }
            }
            ContractTask::FetchDPNSContract => {
                match DataContract::fetch(sdk, Into::<Identifier>::into(dpns_contract::ID_BYTES))
                    .await
                {
                    Ok(Some(data_contract)) => self
                        .db
                        .insert_contract_if_not_exists(&data_contract, Some("dpns"), self)
                        .map_err(|e| e.to_string()),
                    Ok(None) => Err("No DPNS contract found".to_string()),
                    Err(e) => Err(e.to_string()),
                }
            }
        }
    }
}
