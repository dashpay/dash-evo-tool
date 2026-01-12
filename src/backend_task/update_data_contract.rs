use super::{BackendTaskSuccessResult, FeeResult};
use crate::{
    app::TaskResult,
    context::AppContext,
    model::{
        fee_estimation::PlatformFeeEstimator,
        proof_log_item::{ProofLogItem, RequestType},
        qualified_identity::QualifiedIdentity,
    },
};
use dash_sdk::{
    Error, Sdk,
    dpp::{
        dashcore::Network,
        data_contract::accessors::v0::{DataContractV0Getters, DataContractV0Setters},
        identity::{SecurityLevel, accessors::IdentityGettersV0},
        platform_value::string_encoding::Encoding,
        state_transition::{
            StateTransition, StateTransitionSigningOptions,
            data_contract_update_transition::DataContractUpdateTransition,
        },
        version::TryIntoPlatformVersioned,
    },
    platform::{
        DataContract, Fetch, Identifier, IdentityPublicKey,
        transition::broadcast::BroadcastStateTransition,
    },
};
use std::time::Duration;
use tokio::time::sleep;

/// Extracts the contract ID from a formatted error message string that contains:
/// "... with id <contract_id>: ..."
pub fn extract_contract_id_from_error(error: &str) -> Result<Identifier, String> {
    // Find the start of "with id "
    let prefix = "with id ";
    let start_index = error
        .find(prefix)
        .ok_or("Missing 'with id ' prefix in error message")?
        + prefix.len();

    // Slice from after "with id " and find the next colon
    let rest = &error[start_index..];
    let end_index = rest.find(':').ok_or("Missing ':' after contract ID")?;

    let id_str = &rest[..end_index].trim();

    Identifier::from_string(id_str, Encoding::Base58).map_err(|e| {
        format!(
            "Failed to convert contract ID from string to Identifier: {}",
            e
        )
    })
}

impl AppContext {
    pub async fn update_data_contract(
        &self,
        data_contract: &mut DataContract,
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Estimate fee for contract update
        let estimated_fee = PlatformFeeEstimator::new().estimate_contract_update();

        // Increment the version of the data contract
        data_contract.increment_version();

        // Fetch the identity contract nonce
        let identity_contract_nonce = sdk
            .get_identity_contract_nonce(identity.identity.id(), data_contract.id(), true, None)
            .await
            .map_err(|_| "Failed to get nonce".to_string())?;

        // Update UI
        sender
            .send(TaskResult::Success(Box::new(
                BackendTaskSuccessResult::FetchedNonce,
            )))
            .await
            .map_err(|e| format!("Failed to send message: {}", e))?;

        let contract_update_transition: DataContractUpdateTransition =
            (data_contract.clone(), identity_contract_nonce)
                .try_into_platform_versioned(sdk.version())
                .map_err(|e: dash_sdk::dpp::ProtocolError| {
                    format!(
                        "Failed to convert data contract to DataContractUpdateTransition: {}",
                        e
                    )
                })?;

        let mut state_transition = StateTransition::DataContractUpdate(contract_update_transition);

        state_transition.sign_external_with_options(
            &signing_key,
            &identity,
            None::<fn(Identifier, String) -> Result<SecurityLevel, dash_sdk::dpp::ProtocolError>>,
            StateTransitionSigningOptions {
                allow_signing_with_any_security_level: false,
                allow_signing_with_any_purpose: false,
            },
        ).map_err(|e| {
            format!(
                "Failed to sign state transition: {}",
                e
            )
        })?;

        match state_transition.broadcast_and_wait(sdk, None).await {
            Ok(returned_contract) => {
                self.db
                    .replace_contract(data_contract.id(), &returned_contract, self)
                    .map_err(|e| format!("Error inserting contract into the database: {}", e))?;
                let fee_result = FeeResult::new(estimated_fee, estimated_fee);
                Ok(BackendTaskSuccessResult::UpdatedContract(fee_result))
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
                        .map_err(|e| format!("Failed to send message: {}", e))?;

                    // Try to extract contract ID and fetch the contract if it exists
                    // This handles the case where the contract was actually updated despite the proof error
                    if let Ok(id) =
                        extract_contract_id_from_error(proof_error.to_string().as_str())
                    {
                        match self.network {
                            Network::Regtest => sleep(Duration::from_secs(3)).await,
                            _ => sleep(Duration::from_secs(10)).await,
                        }
                        if let Ok(Some(contract)) = DataContract::fetch(sdk, id).await {
                            self.db
                                .replace_contract(contract.id(), &contract, self)
                                .ok();

                            return Err(format!(
                                "Error broadcasting Contract Update transition: {}, proof error logged, contract inserted into the database",
                                proof_error
                            ));
                        }
                    }

                    Err(format!(
                        "Error broadcasting Contract Update transition: {}, proof error logged",
                        proof_error
                    ))
                }
                e => Err(format!(
                    "Error broadcasting Contract Update transition: {}",
                    e
                )),
            },
        }
    }
}
