use crate::context::AppContext;
use crate::platform::contested_names::ContestedResourceTask;
use crate::platform::contract::ContractTask;

pub mod contested_names;
mod contract;

#[derive(Debug, Clone)]
pub(crate) enum BackendTask {
    ContractTask(ContractTask),
    ContestedResourceTask(ContestedResourceTask),
}

impl AppContext {
    pub async fn run_backend_tasks(&self, tasks: Vec<BackendTask>) -> Result<(), String> {
        for task in tasks {
            self.run_backend_task(task).await?;
        }
        Ok(())
    }

    pub async fn run_backend_task(&self, task: BackendTask) -> Result<(), String> {
        match task {
            BackendTask::ContractTask(contract_task) => self.run_contract_task(contract_task).await,
            BackendTask::ContestedResourceTask(contested_resource_task) => {
                self.run_contested_resource_task(contested_resource_task)
                    .await
            }
        }
    }
}
