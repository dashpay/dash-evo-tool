use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::vote_polls_by_document_type_query::VotePollsByDocumentTypeQuery;
use dash_sdk::platform::FetchMany;
use dash_sdk::query_types::ContestedResource;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

impl AppContext {
    pub(super) async fn query_dpns_contested_resources(
        self: &Arc<Self>,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<(), TaskError> {
        let data_contract = self.dpns_contract.as_ref();
        let document_type = data_contract
            .document_type_for_name("domain")
            .map_err(|_| TaskError::DataContractNotFound)?;
        let Some(contested_index) = document_type.find_contested_index() else {
            return Err(TaskError::PlatformFetchError {
                source: Box::new(dash_sdk::Error::Generic(
                    "No contested index on dpns domains".to_string(),
                )),
            });
        };
        const MAX_RETRIES: usize = 3;
        let mut start_at_value = None;
        let mut names_to_be_updated = Vec::new();
        loop {
            let query = VotePollsByDocumentTypeQuery {
                contract_id: data_contract.id(),
                document_type_name: document_type.name().to_string(),
                index_name: contested_index.name.clone(),
                start_at_value: start_at_value.clone(),
                start_index_values: vec!["dash".into()], // hardcoded for dpns
                end_index_values: vec![],
                limit: Some(100),
                order_ascending: true,
            };

            let mut retries = 0;

            let contested_resources = match ContestedResource::fetch_many(sdk, query.clone()).await
            {
                Ok(contested_resources) => contested_resources,
                Err(e) => {
                    tracing::error!("Error fetching contested resources: {}", e);
                    if let dash_sdk::Error::Proof(dash_sdk::ProofVerifierError::GroveDBError {
                        proof_bytes,
                        path_query,
                        height,
                        time_ms,
                        error,
                    }) = &e
                    {
                        let encoded_query =
                            bincode::encode_to_vec(&query, bincode::config::standard()).map_err(
                                |encode_err| {
                                    tracing::error!("Error encoding query: {}", encode_err);
                                    TaskError::SerializationError {
                                        detail: format!("Error encoding query: {}", encode_err),
                                    }
                                },
                            )?;

                        let verification_path_query_bytes =
                            bincode::encode_to_vec(path_query, bincode::config::standard())
                                .map_err(|encode_err| {
                                    tracing::error!("Error encoding path_query: {}", encode_err);
                                    TaskError::SerializationError {
                                        detail: format!(
                                            "Error encoding path_query: {}",
                                            encode_err
                                        ),
                                    }
                                })?;

                        if let Err(e) = self.db.insert_proof_log_item(ProofLogItem {
                            request_type: RequestType::GetContestedResources,
                            request_bytes: encoded_query,
                            verification_path_query_bytes,
                            height: *height,
                            time_ms: *time_ms,
                            proof_bytes: proof_bytes.clone(),
                            error: Some(error.clone()),
                        }) {
                            return Err(TaskError::Database { source: e });
                        }
                    }
                    // TODO: Replace the "contract not found" string match with a
                    // structural SDK variant when one is available.
                    if matches!(e, dash_sdk::Error::StaleNode(_))
                        || e.to_string().contains(
                            "contract not found when querying from value with contract info",
                        )
                    {
                        retries += 1;
                        if retries > MAX_RETRIES {
                            tracing::error!("Max retries reached for query: {}", e);
                            return Err(TaskError::from(e));
                        } else {
                            continue;
                        }
                    } else {
                        return Err(TaskError::from(e));
                    }
                }
            };
            let contested_resources_len = contested_resources.0.len();

            if contested_resources_len == 0 {
                break;
            }

            let contested_resources_as_strings: Vec<String> = contested_resources
                .0
                .into_iter()
                .filter_map(|contested_resource| match contested_resource.0.as_str() {
                    Some(s) => Some(s.to_string()),
                    None => {
                        tracing::warn!(
                            "Contested resource value is not a string: {:?}",
                            contested_resource.0
                        );
                        None
                    }
                })
                .collect();

            let Some(last_found_name) = contested_resources_as_strings.last().cloned() else {
                break;
            };

            let new_names_to_be_updated = self
                .db
                .insert_name_contests_as_normalized_names(contested_resources_as_strings, self)
                .map_err(|e| TaskError::Database { source: e })?;

            names_to_be_updated.extend(new_names_to_be_updated);

            sender
                .send(TaskResult::Refresh)
                .await
                .map_err(|_| TaskError::InternalSendError)?;

            if contested_resources_len < 100 {
                break;
            }
            start_at_value = Some((Value::Text(last_found_name), false))
        }

        let semaphore = Arc::new(Semaphore::new(24));

        let mut handles = Vec::new();

        let handle = {
            let semaphore = semaphore.clone();
            let sdk = sdk.clone();
            let sender = sender.clone();
            let self_ref = self.clone();

            tokio::spawn(async move {
                let _permit: OwnedSemaphorePermit = match semaphore.acquire_owned().await {
                    Ok(permit) => permit,
                    Err(e) => {
                        tracing::error!("Semaphore closed while querying dpns end times: {}", e);
                        return;
                    }
                };

                match self_ref.query_dpns_ending_times(sdk, sender.clone()).await {
                    Ok(_) => {
                        if let Err(e) = sender.send(TaskResult::Refresh).await {
                            tracing::warn!(
                                "Failed to send refresh after dpns end times query: {}",
                                e
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("Error querying dpns end times: {}", e);
                        if let Err(send_err) = sender.send(TaskResult::Error(e)).await {
                            tracing::warn!(
                                "Failed to send error for dpns end times query: {}",
                                send_err
                            );
                        }
                    }
                }
            })
        };

        handles.push(handle);

        for name in names_to_be_updated {
            let semaphore = semaphore.clone();
            let sdk = sdk.clone();
            let sender = sender.clone();
            let self_ref = self.clone();

            let handle = tokio::spawn(async move {
                let _permit: OwnedSemaphorePermit = match semaphore.acquire_owned().await {
                    Ok(permit) => permit,
                    Err(e) => {
                        tracing::error!(
                            "Semaphore closed while querying vote contenders for {}: {}",
                            name,
                            e
                        );
                        return;
                    }
                };

                match self_ref
                    .query_dpns_vote_contenders(&name, &sdk, sender.clone())
                    .await
                {
                    Ok(_) => {
                        if let Err(e) = sender.send(TaskResult::Refresh).await {
                            tracing::warn!(
                                "Failed to send refresh after vote contenders query for {}: {}",
                                name,
                                e
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("Error querying dpns vote contenders for {}: {}", name, e);
                        if let Err(send_err) = sender.send(TaskResult::Error(e)).await {
                            tracing::warn!(
                                "Failed to send error for vote contenders query for {}: {}",
                                name,
                                send_err
                            );
                        }
                    }
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            if let Err(e) = handle.await {
                tracing::error!("Task failed: {:?}", e);
            }
        }

        sender
            .send(TaskResult::Success(Box::new(
                BackendTaskSuccessResult::RefreshedDpnsContests,
            )))
            .await
            .map_err(|_| TaskError::InternalSendError)?;
        Ok(())
    }
}
