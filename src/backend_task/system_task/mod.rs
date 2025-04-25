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
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            SystemTask::WipePlatformData => self.wipe_devnet(),
        }
    }

    pub fn wipe_devnet(self: &Arc<Self>) -> Result<BackendTaskSuccessResult, String> {
        self.db
            .delete_all_local_qualified_identities_in_devnet(self)
            .map_err(|e| e.to_string())?;

        self.db
            .delete_all_local_tokens_in_devnet(self)
            .map_err(|e| e.to_string())?;

        self.db
            .remove_all_asset_locks_identity_id_for_devnet(self)
            .map_err(|e| e.to_string())?;

        self.db
            .remove_all_contracts_in_devnet(self)
            .map_err(|e| e.to_string())?;

        Ok(BackendTaskSuccessResult::Refresh)
    }
}
