//! Dispatch backend tasks from MCP tool handlers.

use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use std::sync::Arc;

async fn run_blocking_dispatch<T, F>(
    task_manager: Arc<crate::utils::tasks::TaskManager>,
    task: F,
) -> Result<T, TaskError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let reply = Arc::new(std::sync::Mutex::new(Some(reply_tx)));
    let task_reply = Arc::clone(&reply);
    let join_reply = Arc::clone(&reply);
    let registration = task_manager.spawn_blocking_sync(
        "mcp_backend_task",
        move || {
            let result = task();
            if let Some(reply) = task_reply
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
            {
                let _ = reply.send(Ok(result));
            }
        },
        move |join_result| async move {
            if let Err(source) = join_result
                && let Some(reply) = join_reply
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .take()
            {
                let _ = reply.send(Err(TaskError::BackendTaskFailed {
                    source: source.into(),
                }));
            }
        },
    );
    if registration.is_err() {
        return Err(TaskError::TaskManagerShuttingDown);
    }
    drop(reply);

    reply_rx
        .await
        .unwrap_or(Err(TaskError::TaskManagerShuttingDown))
}

/// Run a single backend task and return its result.
///
/// Creates a throwaway channel because `run_backend_task` requires a sender
/// for the GUI event loop. In MCP/CLI mode there is no GUI consuming the
/// receiver, so it is intentionally dropped. The task result is returned
/// directly from the async call rather than through the channel.
///
/// Uses TaskManager-tracked blocking work plus `block_on` to avoid `Send`
/// bound issues with platform SDK types across await points. This matches
/// `AppState::handle_backend_task` and participates in the same shutdown barrier.
pub(crate) async fn dispatch_task(
    app_context: &Arc<AppContext>,
    task: BackendTask,
) -> Result<BackendTaskSuccessResult, TaskError> {
    dispatch_task_with(app_context, task, std::future::ready).await
}

/// Run a backend task and keep its lifecycle tail inside the shutdown barrier.
pub(crate) async fn dispatch_task_with<T, F, Fut>(
    app_context: &Arc<AppContext>,
    task: BackendTask,
    lifecycle_tail: F,
) -> Result<T, TaskError>
where
    T: Send + 'static,
    F: FnOnce(BackendTaskSuccessResult) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = T>,
{
    let app_context = app_context.clone();
    let handle = tokio::runtime::Handle::current();
    run_blocking_dispatch(app_context.subtasks.clone(), move || {
        handle.block_on(async move {
            let (tx, _) = tokio::sync::mpsc::channel::<TaskResult>(32);
            let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, egui::Context::default());
            let result = app_context.run_backend_task(task, sender).await?;
            Ok(lifecycle_tail(result).await)
        })
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::tasks::{TaskManager, TaskShutdownOutcome};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_waits_for_in_flight_mcp_dispatch() {
        let task_manager = Arc::new(TaskManager::new());
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        let dispatch_manager = Arc::clone(&task_manager);
        let dispatch = tokio::spawn(async move {
            run_blocking_dispatch(dispatch_manager, move || {
                started_tx.send(()).expect("report dispatch start");
                release_rx.recv().expect("wait for dispatch release");
                42
            })
            .await
        });
        started_rx.recv().expect("dispatch started");

        let mut shutdown = task_manager.shutdown_async();
        let shutdown_stayed_pending =
            tokio::time::timeout(Duration::from_millis(50), &mut shutdown)
                .await
                .is_err();

        release_tx.send(()).expect("release dispatch");
        assert_eq!(
            dispatch.await.expect("dispatch task").expect("dispatch"),
            42
        );
        assert!(
            shutdown_stayed_pending,
            "shutdown must remain pending while MCP dispatch is running"
        );
        assert_eq!(
            shutdown.await.expect("shutdown outcome"),
            TaskShutdownOutcome::Complete
        );
    }

    #[tokio::test]
    async fn dispatch_after_shutdown_is_rejected_without_running() {
        let task_manager = Arc::new(TaskManager::new());
        assert_eq!(
            task_manager
                .shutdown_async()
                .await
                .expect("shutdown outcome"),
            TaskShutdownOutcome::Complete
        );
        let ran = Arc::new(AtomicBool::new(false));
        let task_ran = Arc::clone(&ran);

        let result = run_blocking_dispatch(task_manager, move || {
            task_ran.store(true, Ordering::Release);
        })
        .await;

        assert!(matches!(result, Err(TaskError::TaskManagerShuttingDown)));
        assert!(!ran.load(Ordering::Acquire));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_waits_for_post_dispatch_lifecycle_tail() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let app_context = crate::context::test_support::test_app_context(temp_dir.path());
        let task_manager = Arc::clone(&app_context.subtasks);
        let dispatch_context = Arc::clone(&app_context);
        let (tail_started_tx, tail_started_rx) = tokio::sync::oneshot::channel();
        let (release_tail_tx, release_tail_rx) = tokio::sync::oneshot::channel();
        let dispatch = tokio::spawn(async move {
            dispatch_task_with(
                &dispatch_context,
                BackendTask::None,
                move |result| async move {
                    let _ = tail_started_tx.send(());
                    let _ = release_tail_rx.await;
                    result
                },
            )
            .await
        });
        tail_started_rx.await.expect("lifecycle tail started");

        let mut shutdown = task_manager.shutdown_async();
        let shutdown_stayed_pending =
            tokio::time::timeout(Duration::from_millis(50), &mut shutdown)
                .await
                .is_err();

        release_tail_tx.send(()).expect("release lifecycle tail");
        assert!(matches!(
            dispatch.await.expect("dispatch task").expect("dispatch"),
            BackendTaskSuccessResult::None
        ));
        assert!(
            shutdown_stayed_pending,
            "shutdown must remain pending until post-dispatch lifecycle work finishes"
        );
        assert_eq!(
            shutdown.await.expect("shutdown outcome"),
            TaskShutdownOutcome::Complete
        );
    }
}
