//! Dispatch backend tasks from MCP tool handlers.

use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use std::sync::Arc;

/// Run a single backend task and return its result.
///
/// Creates a throwaway channel (the receiver is never read).
/// Same pattern as `tests/backend-e2e/framework/task_runner.rs`.
pub(crate) async fn dispatch_task(
    app_context: &Arc<AppContext>,
    task: BackendTask,
) -> Result<BackendTaskSuccessResult, TaskError> {
    let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
    let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, egui::Context::default());
    app_context.run_backend_task(task, sender).await
}
