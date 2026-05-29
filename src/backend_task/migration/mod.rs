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

pub mod finish_unwire;

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
    /// re-poll affected screens once the migration finishes.
    pub async fn run_migration_task(
        self: &Arc<Self>,
        task: MigrationTask,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            MigrationTask::FinishUnwire => {
                finish_unwire::run(self).await?;
                Ok(BackendTaskSuccessResult::Refresh)
            }
        }
    }
}
