use super::BackendTaskSuccessResult;
use crate::{context::AppContext, model::qualified_identity::QualifiedIdentity};
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
    Sdk,
};
use std::time::Duration;
use tokio::time::sleep;

impl AppContext {
    pub async fn update_data_contract(
        &self,
        data_contract: &mut DataContract,
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        data_contract.increment_version();

        let identity_contract_nonce = sdk
            .get_identity_contract_nonce(identity.identity.id(), data_contract.id(), true, None)
            .await
            .map_err(|_| format!("Failed to get nonce"))?;

        let contract_update_transition: DataContractUpdateTransition =
            (data_contract.clone(), identity_contract_nonce)
                .try_into_platform_versioned(sdk.version())
                .expect("expected to get transition");

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
                e.to_string()
            )
        })?;

        match state_transition.broadcast_and_wait(sdk, None).await {
            Ok(returned_contract) => {
                self.db
                    .replace_contract(data_contract.id(), &returned_contract, self)
                    .map_err(|e| {
                        format!(
                            "Error inserting contract into the database: {}",
                            e.to_string()
                        )
                    })?;
                Ok(BackendTaskSuccessResult::Message(
                    "DataContract successfully updated".to_string(),
                ))
            }
            Err(e) => {
                // If the error is a Proof error, fetch the contract from Platform state and add to local database
                if e.to_string().contains("proof") {
                    println!("Received proof error when updateing contract. Attempting to fetch contract from Platform state and add to local database");
                    match self.network {
                        Network::Regtest => sleep(Duration::from_secs(3)).await,
                        _ => sleep(Duration::from_secs(10)).await,
                    }
                    let error_string = e.to_string();
                    let id_str = error_string
                        .split(" ")
                        .last()
                        .ok_or("Failed to get contract ID from proof error")?;
                    let id = Identifier::from_string(id_str, Encoding::Base58).map_err(|e| {
                        format!(
                            "Failed to convert contract ID from string to Identifier: {}",
                            e.to_string()
                        )
                    })?;
                    let maybe_contract = match DataContract::fetch(sdk, id).await {
                        Ok(contract) => contract,
                        Err(e) => {
                            return Err(format!(
                                "Failed to fetch contract from Platform state: {}",
                                e.to_string()
                            ))
                        }
                    };
                    if let Some(contract) = maybe_contract {
                        self.db
                            .replace_contract(contract.id(), &contract, self)
                            .map_err(|e| {
                                format!(
                                    "Error inserting contract into the database: {}",
                                    e.to_string()
                                )
                            })?;
                        println!("DataContract successfully updated but the proof was wrong. Please report to Dash Core Group. Error: {}", e.to_string());
                        Ok(BackendTaskSuccessResult::Message(
                            "DataContract successfully updated".to_string(),
                        ))
                    } else {
                        Err(format!("Failed to update DataContract: {}", e.to_string()))
                    }
                } else {
                    Err(format!("Failed to update DataContract: {}", e.to_string()))
                }
            }
        }
    }
}
