use std::time::Duration;

use dash_sdk::{
    dpp::{dashcore::Network, platform_value::string_encoding::Encoding},
    platform::{
        transition::put_contract::PutContract, DataContract, Fetch, Identifier, IdentityPublicKey,
    },
    Sdk,
};
use tokio::time::sleep;

use crate::{context::AppContext, model::qualified_identity::QualifiedIdentity};

use super::BackendTaskSuccessResult;

impl AppContext {
    pub async fn register_data_contract(
        &self,
        data_contract: DataContract,
        alias: String,
        identity: QualifiedIdentity,
        signing_key: IdentityPublicKey,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        match data_contract
            .put_to_platform_and_wait_for_response(&sdk, signing_key.clone(), &identity, None)
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
                        self,
                    )
                    .map_err(|e| {
                        format!(
                            "Error inserting contract into the database: {}",
                            e.to_string()
                        )
                    })?;
                Ok(BackendTaskSuccessResult::Message(
                    "DataContract successfully registered".to_string(),
                ))
            }
            Err(e) => {
                // If the error is a Proof error, fetch the contract from Platform state and add to local database
                if e.to_string().contains("proof") {
                    println!("Received proof error when registering contract. Attempting to fetch contract from Platform state and add to local database");
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
                            "Failed to convert contract ID from string to Identifier: {:?}",
                            e
                        )
                    })?;
                    let maybe_contract = match DataContract::fetch(sdk, id).await {
                        Ok(contract) => contract,
                        Err(e) => {
                            return Err(format!(
                                "Failed to fetch contract from Platform state: {:?}",
                                e
                            ))
                        }
                    };
                    if let Some(contract) = maybe_contract {
                        let optional_alias = match alias.is_empty() {
                            true => None,
                            false => Some(alias),
                        };

                        self.db
                            .insert_contract_if_not_exists(
                                &contract,
                                optional_alias.as_deref(),
                                self,
                            )
                            .map_err(|e| {
                                format!(
                                    "Error inserting contract into the database: {}",
                                    e.to_string()
                                )
                            })?;
                        println!("DataContract successfully registered but the proof was wrong. Please report to Dash Core Group. Error: {:?}", e);
                        Ok(BackendTaskSuccessResult::Message(format!(
                            "DataContract successfully registered"
                        )))
                    } else {
                        Err(format!("Failed to register DataContract: {:?}", e))
                    }
                } else {
                    Err(format!("Failed to register DataContract: {:?}", e))
                }
            }
        }
    }
}
