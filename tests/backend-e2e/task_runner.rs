//! Thin wrapper around `AppContext::run_backend_task` for tests.

use dash_evo_tool::app::TaskResult;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::utils::egui_mpsc::SenderAsync;
use std::sync::Arc;

/// Run a single backend task and return its result.
///
/// Creates a throwaway `SenderAsync` channel (the receiver is never read).
/// This is appropriate for tests that only care about the return value.
pub async fn run_task(
    app_context: &Arc<AppContext>,
    task: BackendTask,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
    let sender = SenderAsync::new(tx, egui::Context::default());
    app_context.run_backend_task(task, sender).await
}
