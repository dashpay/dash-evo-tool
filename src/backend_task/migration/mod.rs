//! Backend tasks that drain DET's legacy `data.db` into the upstream
//! `platform-wallet-storage` k/v store and `SecretStore`.
//!
//! Today the only variant is [`MigrationTask::FinishUnwire`], which
//! orchestrates the post-PR-#860 cold-start migration. The orchestrator
//! detects whether legacy rows are still present, walks the per-domain
//! adapters (filled in by T-SK-02 / T-SH-02), and writes a completion
//! sentinel so subsequent launches short-circuit. See
//! [`finish_unwire`] for the orchestrator body and
//! [`MigrationError`](finish_unwire::MigrationError) for failure shapes.

use std::sync::Arc;

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::context::migration_status::MigrationState;

pub mod finish_unwire;
pub mod single_key_restore;

pub use finish_unwire::MigrationError;

/// Migration orchestrator dispatch enum. Cheap to clone — every
/// payload is plain data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationTask {
    /// Run the post-unwire `data.db` drain. Idempotent: once the
    /// completion sentinel exists in `det-app.sqlite`, subsequent calls
    /// return `Success` immediately without touching the legacy file.
    FinishUnwire,
}

impl AppContext {
    /// Dispatch a [`MigrationTask`]. Always returns
    /// [`BackendTaskSuccessResult::Refresh`] on success so the UI can
    /// re-poll affected screens once the migration finishes. On
    /// failure, publishes [`MigrationState::Failed`] so the per-frame
    /// banner reconciliation in `AppState` can surface the error
    /// variant with a "Retry now" action — without it the banner
    /// would be stuck in `Running` forever.
    pub async fn run_migration_task(
        self: &Arc<Self>,
        task: MigrationTask,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            MigrationTask::FinishUnwire => match finish_unwire::run(self).await {
                // `finish_unwire::run` already publishes the terminal state
                // (`Success` only when it moved data, `Idle` for a no-op
                // launch), so the banner is correct without anything here.
                Ok(_did_work) => Ok(BackendTaskSuccessResult::Refresh),
                Err(task_error) => {
                    // Publish a `Failed` state carrying the typed
                    // `MigrationError` chain so the UI banner can call
                    // `Display::fmt` at render time and surface the
                    // wrapped source via the details panel — no
                    // stringification on the writer side.
                    if let TaskError::MigrationFailed { source } = &task_error {
                        // `Arc::clone` is a cheap refcount bump — both
                        // the returned `Err` and the published `Failed`
                        // state observe the same typed error chain.
                        self.migration_status().set_state(MigrationState::Failed {
                            error: Arc::clone(source),
                        });
                    }
                    Err(task_error)
                }
            },
        }
    }
}
