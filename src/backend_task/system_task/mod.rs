use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SystemTask {
    WipePlatformData,
}

impl AppContext {
    pub async fn run_system_task(
        self: &Arc<Self>,
        task: SystemTask,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            SystemTask::WipePlatformData => self.wipe_devnet(),
        }
    }

    pub fn wipe_devnet(self: &Arc<Self>) -> Result<BackendTaskSuccessResult, String> {
        self.db
            .delete_all_local_qualified_identity_in_devnet(self)
            .map_err(|e| e.to_string())?;

        Ok(BackendTaskSuccessResult::Refresh)
    }
}
