use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::ui::theme::ThemeMode;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum SystemTask {
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
            SystemTask::WipePlatformData => self.wipe_devnet(),
            SystemTask::UpdateThemePreference(theme_mode) => {
                self.handle_update_theme_preference(theme_mode)
            }
        }
    }

    pub fn wipe_devnet(self: &Arc<Self>) -> Result<BackendTaskSuccessResult, TaskError> {
        self.db
            .delete_all_local_qualified_identities_in_devnet(self)?;

        self.db.delete_all_local_tokens_in_devnet(self)?;

        self.db
            .remove_all_asset_locks_identity_id_for_devnet(self)?;

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
