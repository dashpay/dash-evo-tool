use crate::app::TaskResult;
use crate::context::AppContext;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::drive::query::vote_polls_by_document_type_query::VotePollsByDocumentTypeQuery;
use dash_sdk::platform::FetchMany;
use dash_sdk::query_types::ContestedResource;
use dash_sdk::Sdk;
use std::sync::Arc;
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};

impl AppContext {
    pub(super) async fn query_dpns_contested_resources(
        self: &Arc<Self>,
        sdk: Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<(), String> {
        let data_contract = self.dpns_contract.as_ref();
        let document_type = data_contract
            .document_type_for_name("domain")
            .expect("expected document type");
        let Some(contested_index) = document_type.find_contested_index() else {
            return Err("No contested index on dpns domains".to_string());
        };
        let query = VotePollsByDocumentTypeQuery {
            contract_id: data_contract.id(),
            document_type_name: document_type.name().to_string(),
            index_name: contested_index.name.clone(),
            start_at_value: None,
            start_index_values: vec!["dash".into()], // hardcoded for dpns
            end_index_values: vec![],
            limit: None,
            order_ascending: true,
        };

        let contested_resources =
            ContestedResource::fetch_many(&sdk, query)
                .await
                .map_err(|e| {
                    tracing::error!("error fetching contested resources: {}", e);
                    format!("error fetching contested resources: {}", e)
                })?;

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

        let names_to_be_updated = self
            .db
            .insert_name_contests_as_normalized_names(contested_resources_as_strings, &self)
            .map_err(|e| e.to_string())?;

        sender
            .send(TaskResult::Refresh)
            .await
            .expect("expected to send refresh");

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
                        tracing::error!("error querying dpns end times: {}", e);
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
                    .query_dpns_vote_contenders(&name, sdk, sender.clone())
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
                        tracing::error!("error querying dpns vote contenders for {}: {}", name, e);
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

        Ok(())
    }
}
