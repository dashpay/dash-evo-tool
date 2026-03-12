use std::time::Duration;

use dash_sdk::{
    Error, Sdk,
    dpp::{dashcore::Network, data_contract::accessors::v0::DataContractV0Getters},
    platform::{DataContract, Fetch, IdentityPublicKey, transition::put_contract::PutContract},
};
use tokio::time::sleep;

use super::{BackendTaskSuccessResult, FeeResult};
use crate::backend_task::error::TaskError;
use crate::backend_task::update_data_contract::extract_contract_id_from_error;
use crate::model::fee_estimation::PlatformFeeEstimator;
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
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Estimate fee for contract creation
        let estimated_fee = PlatformFeeEstimator::new().estimate_contract_create_base();

        match data_contract
            .put_to_platform_and_wait_for_response(sdk, signing_key.clone(), &identity, None)
            .await
        {
            Ok(returned_contract) => {
                let optional_alias = match alias.is_empty() {
                    true => None,
                    false => Some(alias),
                };
                self.db.insert_contract_if_not_exists(
                    &returned_contract,
                    optional_alias.as_deref(),
                    AllTokensShouldBeAdded,
                    self,
                )?;
                let fee_result = FeeResult::new(estimated_fee, estimated_fee);
                Ok(BackendTaskSuccessResult::RegisteredContract(fee_result))
            }
            Err(e) => match e {
                Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                    // Log the proof error first, before any other operations
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

                    sender
                        .send(TaskResult::Success(Box::new(
                            BackendTaskSuccessResult::ProofErrorLogged,
                        )))
                        .await
                        .map_err(|_| TaskError::InternalSendError)?;

                    // Try to extract contract ID and fetch the contract if it exists
                    // This handles the case where the contract was actually created despite the proof error
                    if let Ok(id) = extract_contract_id_from_error(proof_error.to_string().as_str())
                    {
                        match self.network {
                            Network::Regtest => sleep(Duration::from_secs(3)).await,
                            _ => sleep(Duration::from_secs(10)).await,
                        }
                        if let Ok(Some(contract)) = DataContract::fetch(sdk, id).await {
                            let optional_alias = self
                                .get_contract_by_id(&contract.id())
                                .ok()
                                .flatten()
                                .and_then(|c| c.alias);

                            self.db
                                .insert_contract_if_not_exists(
                                    &contract,
                                    optional_alias.as_deref(),
                                    AllTokensShouldBeAdded,
                                    self,
                                )
                                .ok();

                            return Err(TaskError::Generic(format!(
                                "Error broadcasting Register Contract transition: {}, proof error logged, contract inserted into the database",
                                proof_error
                            )));
                        }
                    }

                    Err(TaskError::Generic(format!(
                        "Error broadcasting Register Contract transition: {}, proof error logged",
                        proof_error
                    )))
                }
                e => Err(TaskError::from(e)),
            },
        }
    }
}
