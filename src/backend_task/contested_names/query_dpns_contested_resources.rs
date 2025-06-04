use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::vote_polls_by_document_type_query::VotePollsByDocumentTypeQuery;
use dash_sdk::platform::FetchMany;
use dash_sdk::query_types::ContestedResource;
use dash_sdk::Sdk;
use std::sync::Arc;
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

impl AppContext {
    pub(super) async fn query_dpns_contested_resources(
        self: &Arc<Self>,
        sdk: &Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<(), String> {
        let data_contract = self.dpns_contract.as_ref();
        let document_type = data_contract
            .document_type_for_name("domain")
            .expect("expected document type");
        let Some(contested_index) = document_type.find_contested_index() else {
            return Err(
                "Contested resource query failed: No contested index on dpns domains.".to_string(),
            );
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

            // Initialize retry counter
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
                        // Encode the query using bincode
                        let encoded_query =
                            match bincode::encode_to_vec(&query, bincode::config::standard())
                                .map_err(|encode_err| {
                                    tracing::error!("Error encoding query: {}", encode_err);
                                    format!("Error encoding query: {}", encode_err)
                                }) {
                                Ok(encoded_query) => encoded_query,
                                Err(e) => return Err(e),
                            };

                        // Encode the path_query using bincode
                        let verification_path_query_bytes =
                            match bincode::encode_to_vec(path_query, bincode::config::standard())
                                .map_err(|encode_err| {
                                    tracing::error!("Error encoding path_query: {}", encode_err);
                                    format!("Error encoding path_query: {}", encode_err)
                                }) {
                                Ok(encoded_path_query) => encoded_path_query,
                                Err(e) => {
                                    return Err(format!("Contested resource query failed: {}", e))
                                }
                            };

                        if let Err(e) = self
                            .db
                            .insert_proof_log_item(ProofLogItem {
                                request_type: RequestType::GetContestedResources,
                                request_bytes: encoded_query,
                                verification_path_query_bytes,
                                height: *height,
                                time_ms: *time_ms,
                                proof_bytes: proof_bytes.clone(),
                                error: Some(error.clone()),
                            })
                            .map_err(|e| e.to_string())
                        {
                            return Err(format!("Contested resource query failed: {}", e));
                        }
                    }
                    if e.to_string().contains("try another server")
                        || e.to_string().contains(
                            "contract not found when querying from value with contract info",
                        )
                    {
                        retries += 1;
                        if retries > MAX_RETRIES {
                            tracing::error!("Max retries reached for query: {}", e);
                            return Err(format!(
                                "Contested resource query failed after retries: {}",
                                e
                            ));
                        } else {
                            // Retry
                            continue;
                        }
                    } else {
                        return Err(format!("Contested resource query failed: {}", e));
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
                .map(|contested_resource| {
                    contested_resource
                        .0
                        .as_str()
                        .expect("expected str")
                        .to_string()
                })
                .collect();

            let last_found_name = contested_resources_as_strings.last().unwrap().clone();

            let new_names_to_be_updated = self
                .db
                .insert_name_contests_as_normalized_names(contested_resources_as_strings, self)
                .map_err(|e| format!("Contested resource query failed. Failed to insert name contests into database: {}", e))?;

            names_to_be_updated.extend(new_names_to_be_updated);

            sender.send(TaskResult::Refresh).await.map_err(|e| {
                format!(
                    "Contested resource query failed. Sender failed to send TaskResult: {}",
                    e
                )
            })?;

            if contested_resources_len < 100 {
                break;
            }
            start_at_value = Some((Value::Text(last_found_name), false))
        }

        // Create a semaphore with 15 permits
        let semaphore = Arc::new(Semaphore::new(24));

        let mut handles = Vec::new();

        let handle = {
            let semaphore = semaphore.clone();
            let sdk = sdk.clone();
            let sender = sender.clone();
            let self_ref = self.clone();

            tokio::spawn(async move {
                // Acquire a permit from the semaphore
                let _permit: OwnedSemaphorePermit = semaphore.acquire_owned().await.unwrap();

                match self_ref.query_dpns_ending_times(sdk, sender.clone()).await {
                    Ok(_) => {
                        // Send a refresh message if the query succeeded
                        sender
                            .send(TaskResult::Refresh)
                            .await
                            .expect("expected to send refresh");
                    }
                    Err(e) => {
                        tracing::error!("Error querying dpns end times: {}", e);
                        sender
                            .send(TaskResult::Error(e))
                            .await
                            .expect("expected to send error");
                    }
                }
            })
        };

        handles.push(handle);

        for name in names_to_be_updated {
            // Clone the semaphore, sdk, and sender for each task
            let semaphore = semaphore.clone();
            let sdk = sdk.clone();
            let sender = sender.clone();
            let self_ref = self.clone(); // Assuming self is cloneable

            // Spawn each task with a permit from the semaphore
            let handle = tokio::spawn(async move {
                // Acquire a permit from the semaphore
                let _permit: OwnedSemaphorePermit = semaphore.acquire_owned().await.unwrap();

                // Perform the query
                match self_ref
                    .query_dpns_vote_contenders(&name, &sdk, sender.clone())
                    .await
                {
                    Ok(_) => {
                        // Send a refresh message if the query succeeded
                        sender
                            .send(TaskResult::Refresh)
                            .await
                            .expect("expected to send refresh");
                    }
                    Err(e) => {
                        tracing::error!("Error querying dpns vote contenders for {}: {}", name, e);
                        sender
                            .send(TaskResult::Error(e))
                            .await
                            .expect("expected to send error");
                    }
                }
            });

            // Collect all task handles
            handles.push(handle);
        }

        // Await all tasks
        for handle in handles {
            if let Err(e) = handle.await {
                tracing::error!("Task failed: {:?}", e);
            }
        }

        sender
            .send(TaskResult::Success(Box::new(
                BackendTaskSuccessResult::Message(
                    "Successfully refreshed DPNS contests".to_string(),
                ),
            )))
            .await
            .map_err(|e| {
                format!(
                    "Successfully refreshed DPNS contests but sender failed to send TaskResult: {}",
                    e
                )
            })?;
        Ok(())
    }
}
