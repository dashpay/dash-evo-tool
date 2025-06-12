use std::time::Duration;

use dash_sdk::{
    dpp::{dashcore::Network, data_contract::accessors::v0::DataContractV0Getters},
    platform::{transition::put_contract::PutContract, DataContract, Fetch, IdentityPublicKey},
    Error, Sdk,
};
use tokio::time::sleep;

use super::BackendTaskSuccessResult;
use crate::backend_task::update_data_contract::extract_contract_id_from_error;
use crate::{
    app::TaskResult,
    context::AppContext,
    model::{proof_log_item::RequestType, qualified_identity::QualifiedIdentity},
};
use crate::{
    database::contracts::InsertTokensToo::AllTokensShouldBeAdded,
    model::proof_log_item::ProofLogItem,
};

impl AppContext {
    pub async fn register_data_contract(
        &self,
        data_contract: DataContract,
        alias: String,
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match data_contract
            .put_to_platform_and_wait_for_response(sdk, signing_key.clone(), &identity, None)
            .await
        {
            Ok(returned_contract) => {
                let optional_alias = match alias.is_empty() {
                    true => None,
                    false => Some(alias),
                };
                self.db
                    .insert_contract_if_not_exists(
                        &returned_contract,
                        optional_alias.as_deref(),
                        AllTokensShouldBeAdded,
                        self,
                    )
                    .map_err(|e| format!("Error inserting contract into the database: {}", e))?;
                Ok(BackendTaskSuccessResult::Message(
                    "DataContract successfully registered".to_string(),
                ))
            }
            Err(e) => match e {
                Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                    sender
                        .send(TaskResult::Success(Box::new(
                            BackendTaskSuccessResult::Message(
                                "Transaction returned proof error".to_string(),
                            ),
                        )))
                        .await
                        .map_err(|e| format!("Failed to send message: {}", e))?;
                    match self.network {
                        Network::Regtest => sleep(Duration::from_secs(3)).await,
                        _ => sleep(Duration::from_secs(10)).await,
                    }
                    let id = match extract_contract_id_from_error(proof_error.to_string().as_str())
                    {
                        Ok(id) => id,
                        Err(e) => {
                            return Err(format!("Failed to extract id from error message: {}", e))
                        }
                    };
                    let maybe_contract = match DataContract::fetch(sdk, id).await {
                        Ok(contract) => contract,
                        Err(e) => {
                            return Err(format!(
                                "Failed to fetch contract from Platform state: {}",
                                e
                            ))
                        }
                    };
                    if let Some(contract) = maybe_contract {
                        let optional_alias = self
                            .get_contract_by_id(&contract.id())
                            .map(|contract| {
                                if let Some(contract) = contract {
                                    contract.alias
                                } else {
                                    None
                                }
                            })
                            .map_err(|e| {
                                format!("Failed to get contract by ID from database: {}", e)
                            })?;

                        self.db
                            .insert_contract_if_not_exists(
                                &contract,
                                optional_alias.as_deref(),
                                AllTokensShouldBeAdded,
                                self,
                            )
                            .map_err(|e| {
                                format!("Error inserting contract into the database: {}", e)
                            })?;
                    }
                    self.db
                        .insert_proof_log_item(ProofLogItem {
                            request_type: RequestType::BroadcastStateTransition,
                            request_bytes: vec![],
                            verification_path_query_bytes: vec![],
                            height: block_info.height,
                            time_ms: block_info.time_ms,
                            proof_bytes,
                            error: Some(proof_error.to_string()),
                        })
                        .ok();
                    Err(format!(
                        "Error broadcasting Register Contract transition: {}, proof error logged, contract inserted into the database",
                        proof_error
                    ))
                }
                e => Err(format!(
                    "Error broadcasting Register Contract transition: {}",
                    e
                )),
            },
        }
    }
}
