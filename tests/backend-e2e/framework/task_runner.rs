//! Thin wrapper around `AppContext::run_backend_task` for tests.

use dash_evo_tool::app::TaskResult;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::utils::egui_mpsc::SenderAsync;
use std::sync::Arc;
use std::time::Duration;

/// Run a single backend task and return its result.
///
/// Creates a throwaway `SenderAsync` channel (the receiver is never read).
/// This is appropriate for tests that only care about the return value.
pub async fn run_task(
    app_context: &Arc<AppContext>,
    task: BackendTask,
) -> Result<BackendTaskSuccessResult, TaskError> {
    // INTENTIONAL(CMT-017): Bounded channel capacity 32 with dropped receiver.
    // Backend tasks send 0-1 results; blocking risk is negligible for test usage.
    let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
    let sender = SenderAsync::new(tx, egui::Context::default());
    app_context.run_backend_task(task, sender).await
}

#[allow(dead_code)]
/// Run a backend task with automatic retry on identity nonce conflicts.
///
/// Retries up to 3 times with a 2s delay on `IdentityNonceOverflow` or
/// `IdentityNonceNotFound`. All other errors are returned immediately.
/// Since `BackendTask: Clone`, the task is cloned for each attempt.
pub async fn run_task_with_nonce_retry(
    app_context: &Arc<AppContext>,
    task: BackendTask,
) -> Result<BackendTaskSuccessResult, TaskError> {
    const MAX_ATTEMPTS: u32 = 3;
    const RETRY_DELAY: Duration = Duration::from_secs(2);

    let mut last_err = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match run_task(app_context, task.clone()).await {
            Ok(result) => return Ok(result),
            Err(e @ TaskError::IdentityNonceOverflow { .. })
            | Err(e @ TaskError::IdentityNonceNotFound { .. }) => {
                tracing::warn!(
                    attempt,
                    max_attempts = MAX_ATTEMPTS,
                    error = %e,
                    "Identity nonce conflict — retrying after {}s",
                    RETRY_DELAY.as_secs(),
                );
                last_err = Some(e);
                tokio::time::sleep(RETRY_DELAY).await;
            }
            Err(e) => return Err(e),
        }
    }

    Err(last_err.expect("loop always sets last_err before exhausting attempts"))
}

// NOTE: `run_task_with_retry` was removed because retrying ConfirmationTimeout
// is a workaround for the IS lock relay bug. Tests should fail clearly when
// confirmation times out — that surfaces the real issue instead of hiding it.
// Use `run_task` (no retry) or `run_task_with_nonce_retry` (nonce conflicts
// only, which are legitimate under parallel execution).
