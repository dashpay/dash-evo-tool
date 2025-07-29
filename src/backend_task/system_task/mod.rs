use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
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
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            SystemTask::WipePlatformData => self.wipe_devnet(),
            SystemTask::UpdateThemePreference(theme_mode) => {
                self.update_theme_preference(theme_mode)
            }
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

    pub fn update_theme_preference(
        self: &Arc<Self>,
        theme_mode: ThemeMode,
    ) -> Result<BackendTaskSuccessResult, String> {
        let _guard = self.invalidate_settings_cache();
        
        self.db
            .update_theme_preference(theme_mode)
            .map_err(|e| e.to_string())?;

        Ok(BackendTaskSuccessResult::UpdatedThemePreference(theme_mode))
    }
}
