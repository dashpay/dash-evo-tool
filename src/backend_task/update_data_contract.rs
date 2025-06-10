use super::BackendTaskSuccessResult;
use crate::{
    app::TaskResult,
    context::AppContext,
    model::{
        proof_log_item::{ProofLogItem, RequestType},
        qualified_identity::QualifiedIdentity,
    },
};
use dash_sdk::{
    dpp::{
        dashcore::Network,
        data_contract::accessors::v0::{DataContractV0Getters, DataContractV0Setters},
        identity::{accessors::IdentityGettersV0, SecurityLevel},
        platform_value::string_encoding::Encoding,
        state_transition::{
            data_contract_update_transition::DataContractUpdateTransition, StateTransition,
            StateTransitionSigningOptions,
        },
        version::TryIntoPlatformVersioned,
    },
    platform::{
        transition::broadcast::BroadcastStateTransition, DataContract, Fetch, Identifier,
        IdentityPublicKey,
    },
    Error, Sdk,
};
use std::time::Duration;
use tokio::{sync::mpsc, time::sleep};

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
                BackendTaskSuccessResult::Message("Nonce fetched successfully".to_string()),
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
                Ok(BackendTaskSuccessResult::Message(
                    "DataContract successfully updated".to_string(),
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
                        self.db
                            .replace_contract(contract.id(), &contract, self)
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
                        "Error broadcasting Contract Update transition: {}, proof error logged, contract inserted into the database",
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
