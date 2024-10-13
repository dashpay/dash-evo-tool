use crate::app::TaskResult;
use crate::context::AppContext;
use crate::platform::contested_names::ContestedResourceTask;
use crate::platform::contract::ContractTask;
use crate::platform::identity::IdentityTask;
use std::sync::Arc;
use tokio::sync::mpsc;
use crate::platform::document::DocumentTask;

pub mod contested_names;
pub mod contract;
pub mod identity;
mod document;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BackendTask {
    IdentityTask(IdentityTask),
    DocumentTask(DocumentTask),
    ContractTask(ContractTask),
    ContestedResourceTask(ContestedResourceTask),
}

impl AppContext {
    pub async fn run_backend_tasks(
        self: &Arc<Self>,
        tasks: Vec<BackendTask>,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<(), String> {
        for task in tasks {
            self.run_backend_task(task, sender.clone()).await?;
        }
        Ok(())
    }

    pub async fn run_backend_task(
        self: &Arc<Self>,
        task: BackendTask,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<(), String> {
        let sdk = self.sdk.clone();
        match task {
            BackendTask::ContractTask(contract_task) => {
                self.run_contract_task(contract_task, &sdk).await
            }
            BackendTask::ContestedResourceTask(contested_resource_task) => {
                self.run_contested_resource_task(contested_resource_task, &sdk, sender)
                    .await
            }
            BackendTask::IdentityTask(identity_task) => {
                self.run_identity_task(identity_task, &sdk).await
            }
            BackendTask::DocumentTask(document_task) => {
                self.run_document_task(document_task, &sdk).await
            }
        }
    }
}
