use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::ui::theme::ThemeMode;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum SystemTask {
    ClearNetworkDatabase,
    WipePlatformData,
    UpdateThemePreference(ThemeMode),
}

impl AppContext {
    pub async fn run_system_task(
        self: &Arc<Self>,
        task: SystemTask,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            SystemTask::ClearNetworkDatabase => {
                self.clear_network_database().await?;
                Ok(BackendTaskSuccessResult::NetworkDatabaseCleared {
                    network: self.network,
                })
            }
            SystemTask::WipePlatformData => self.wipe_devnet(),
            SystemTask::UpdateThemePreference(theme_mode) => {
                self.handle_update_theme_preference(theme_mode)
            }
        }
    }

    pub fn wipe_devnet(self: &Arc<Self>) -> Result<BackendTaskSuccessResult, TaskError> {
        self.delete_all_local_qualified_identities_in_devnet()?;
        self.delete_all_local_tokens_in_devnet()?;

        // Asset-lock state lives in the upstream `AssetLockManager`; the
        // legacy `asset_lock_transaction` DET table and its module were
        // deleted, so there is no DET-side mirror to clear here.

        self.clear_user_contracts()?;

        Ok(BackendTaskSuccessResult::Refresh)
    }

    /// Backend-task handler for `SystemTask::UpdateThemePreference`.
    /// Wraps [`AppContext::update_theme_preference`] (the k/v writer) in
    /// the `BackendTaskSuccessResult` envelope the dispatcher expects.
    pub fn handle_update_theme_preference(
        self: &Arc<Self>,
        theme_mode: ThemeMode,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.update_theme_preference(theme_mode)
            .map_err(|source| TaskError::AppSettingsWrite { source })?;

        Ok(BackendTaskSuccessResult::UpdatedThemePreference(theme_mode))
    }
}
