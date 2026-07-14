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
use crate::context::migration_status::MigrationState;

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
    /// Retire the "some scheduled votes could not be read" warning for the
    /// active network. The warning is re-raised on every launch until the user
    /// acknowledges it, so this is the one gesture that stops it — the legacy
    /// vote rows themselves are never touched.
    AcknowledgeUnreadableVotes,
    /// Retire the "some identities could not be read" warning for the active
    /// network. Same contract as [`Self::AcknowledgeUnreadableVotes`]: the
    /// warning is re-raised on every launch until the user acknowledges it, and
    /// the legacy identity rows themselves are never touched.
    AcknowledgeUnreadableIdentities,
    /// Retire both warnings at once — the acknowledgement of the single combined
    /// banner that names both problems. Retiring only one half would re-raise the
    /// other on the next launch, as a notice the user has already read and acted
    /// on.
    AcknowledgeUnreadableIdentitiesAndVotes,
}

impl AppContext {
    /// Dispatch a [`MigrationTask`]. Always returns
    /// [`BackendTaskSuccessResult::Refresh`] on success so the UI can
    /// re-poll affected screens once the migration finishes. On
    /// failure, publishes [`MigrationState::Failed`] so the per-frame
    /// banner reconciliation in `AppState` can surface the error
    /// variant with a "Retry now" action — without it the banner
    /// would be stuck in `Running` forever, and `run_backend_task`
    /// would keep rejecting every wallet-touching task with
    /// `WalletStorageNotReady` until the app is restarted.
    pub async fn run_migration_task(
        self: &Arc<Self>,
        task: MigrationTask,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            MigrationTask::FinishUnwire => match finish_unwire::run(self).await {
                // Every `Ok` path of `finish_unwire::run` publishes its own
                // terminal state — including the one that reports a *failure*
                // the user must still act on
                // (`FailedWithUnreadableIdentities`). Re-publishing here would
                // overwrite it.
                Ok(_did_work) => Ok(BackendTaskSuccessResult::Refresh),
                Err(task_error) => {
                    // Publish `Failed` for *every* error, carrying the typed
                    // `MigrationError` chain so the banner can `Display::fmt` it
                    // at render time and surface the source in the details panel
                    // — no stringification on the writer side. Coercing rather
                    // than matching one variant is what makes this total: an
                    // error that published nothing would leave the status on
                    // `Running` and wedge the whole wallet surface.
                    let source = finish_unwire::migration_error_chain(task_error);
                    // `Arc::clone` is a cheap refcount bump — both the returned
                    // `Err` and the published `Failed` state observe the same
                    // typed error chain.
                    self.migration_status().set_state(MigrationState::Failed {
                        error: Arc::clone(&source),
                    });
                    Err(TaskError::MigrationFailed { source })
                }
            },
            MigrationTask::AcknowledgeUnreadableVotes => {
                finish_unwire::acknowledge_unreadable_votes(self)?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
            MigrationTask::AcknowledgeUnreadableIdentities => {
                finish_unwire::acknowledge_unreadable_identities(self)?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
            MigrationTask::AcknowledgeUnreadableIdentitiesAndVotes => {
                finish_unwire::acknowledge_unreadable_votes(self)?;
                finish_unwire::acknowledge_unreadable_identities(self)?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
        }
    }
}
