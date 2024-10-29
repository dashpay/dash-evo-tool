use crate::app::TaskResult;
use crate::context::AppContext;
use crate::platform::contested_names::ContestedResourceTask;
use crate::platform::core::{CoreItem, CoreTask};
use crate::platform::identity::IdentityTask;
use dash_sdk::dpp::voting::votes::Vote;
use std::sync::Arc;
use tokio::sync::mpsc;

pub mod contested_names;
pub mod core;
pub mod identity;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BackendTask {
    IdentityTask(IdentityTask),
    ContestedResourceTask(ContestedResourceTask),
    CoreTask(CoreTask),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BackendTaskSuccessResult {
    None,
    CoreItem(CoreItem),
    SuccessfulVotes(Vec<Vote>),
}

impl BackendTaskSuccessResult {}

impl AppContext {
    pub async fn run_backend_task(
        self: &Arc<Self>,
        task: BackendTask,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let sdk = self.sdk.clone();
        match task {
            BackendTask::ContestedResourceTask(contested_resource_task) => {
                self.run_contested_resource_task(contested_resource_task, &sdk, sender)
                    .await
            }
            BackendTask::IdentityTask(identity_task) => self
                .run_identity_task(identity_task, &sdk, sender)
                .await
                .map(|_| BackendTaskSuccessResult::None),
            BackendTask::CoreTask(core_task) => self.run_core_task(core_task).await,
        }
    }
}
