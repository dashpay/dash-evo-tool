use crate::app::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::{AppContext, BackupPruneReport};
use crate::model::backup_retention::BackupRetention;
use crate::ui::theme::ThemeMode;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum SystemTask {
    ClearNetworkDatabase,
    WipePlatformData,
    UpdateThemePreference(ThemeMode),
    /// Persist the upgrade-backup retention policy and apply it right away.
    UpdateBackupRetention(BackupRetention),
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
            SystemTask::UpdateBackupRetention(retention) => {
                self.handle_update_backup_retention(retention)
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

    /// Backend-task handler for `SystemTask::UpdateBackupRetention`.
    ///
    /// Only the save can fail the task. Pruning under the new policy is
    /// best-effort and is retried on every start.
    pub fn handle_update_backup_retention(
        self: &Arc<Self>,
        retention: BackupRetention,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let retention = retention.sanitized();
        self.set_backup_retention(retention)
            .map_err(|source| TaskError::AppSettingsWrite { source })?;
        // Only the databases this process has already opened successfully are safe
        // to prune; before the wallet backend is up, the next start applies it.
        let report = if self.wallet_backend().is_ok() {
            self.prune_expired_upgrade_backups_best_effort()
        } else {
            BackupPruneReport::default()
        };
        Ok(BackendTaskSuccessResult::UpdatedBackupRetention {
            retention,
            deleted: report.deleted,
            cleanup_failure: report.failure.map(Arc::new),
        })
    }
}
