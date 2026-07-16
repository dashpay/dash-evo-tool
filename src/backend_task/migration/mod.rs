//! Backend tasks that drain DET's legacy `data.db` into the upstream
//! `platform-wallet-storage` k/v store and `SecretStore`.
//!
//! Today the only variant is [`MigrationTask::FinishUnwire`], which
//! orchestrates the post-PR-#860 cold-start migration. The orchestrator
//! detects whether legacy rows are still present, drains single-key rows,
//! wallet seeds and wallet metadata into the upstream store, registers the
//! migrated wallets, and writes a completion sentinel so subsequent
//! launches short-circuit. See [`finish_unwire`] for the orchestrator body
//! and [`MigrationError`](finish_unwire::MigrationError) for failure shapes.

use std::sync::Arc;

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;

pub mod finish_unwire;
pub mod legacy_settings;
pub mod single_key_restore;

/// Cross-subsystem upgrade regression from a real v0.9.3 `data.db`: schema
/// ladder, boot settings import and wallet drain, composed in production order.
#[cfg(test)]
mod v093_upgrade;

pub use finish_unwire::MigrationError;

/// Migration orchestrator dispatch enum. Cheap to clone — every
/// payload is plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationTask {
    /// Run the post-unwire `data.db` drain. Idempotent: once the
    /// completion sentinel exists in `det-app.sqlite`, subsequent calls
    /// return `Success` immediately without touching the legacy file.
    FinishUnwire,
    /// Retire warnings for unreadable scheduled-vote and top-up rows on the
    /// active network. The legacy rows themselves are never touched.
    AcknowledgeUnreadableAppData,
    /// Retire the "some identities could not be read" warning for the active
    /// network. Same contract as [`Self::AcknowledgeUnreadableAppData`]: the
    /// warning is re-raised on every launch until the user acknowledges it, and
    /// the legacy identity rows themselves are never touched.
    AcknowledgeUnreadableIdentities,
    /// Retire identity and app-data warnings together after acknowledging the
    /// single banner that names every affected data type.
    AcknowledgeUnreadableData,
}

impl AppContext {
    /// Dispatch a [`MigrationTask`]. Always returns
    /// [`BackendTaskSuccessResult::Refresh`] on success so the UI can
    /// re-poll affected screens once the migration finishes. On
    /// failure, publishes [`MigrationState::Failed`] so the per-frame
    /// banner reconciliation in `AppState` can surface the error
    /// variant with its recovery action when retrying can help — without it
    /// the banner would be stuck in `Running` forever, and `run_backend_task`
    /// would keep rejecting every wallet-touching task with
    /// `WalletStorageNotReady` until the app is restarted.
    pub async fn run_migration_task(
        self: &Arc<Self>,
        task: MigrationTask,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            MigrationTask::FinishUnwire => {
                finish_unwire::run(self).await?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
            MigrationTask::AcknowledgeUnreadableAppData => {
                finish_unwire::acknowledge_unreadable_app_data(self)?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
            MigrationTask::AcknowledgeUnreadableIdentities => {
                finish_unwire::acknowledge_unreadable_identities(self)?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
            MigrationTask::AcknowledgeUnreadableData => {
                finish_unwire::acknowledge_unreadable_app_data(self)?;
                finish_unwire::acknowledge_unreadable_identities(self)?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
        }
    }
}

pub(crate) fn migration_task_error(source: Arc<MigrationError>) -> TaskError {
    match source.as_ref() {
        MigrationError::InteractivePromptUnavailable => {
            TaskError::StorageUpdateNeedsDesktop { source }
        }
        MigrationError::LegacyDataTooOld { .. } => TaskError::SavedDataTooOld { source },
        MigrationError::LegacyDataTooNew { .. } => TaskError::SavedDataTooNew { source },
        _ => TaskError::MigrationFailed { source },
    }
}
