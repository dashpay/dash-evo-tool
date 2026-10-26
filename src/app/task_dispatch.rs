//! One execution boundary for single requests and ordered or concurrent batches.

use super::BackendTasksExecutionMode;
use crate::backend_task::error::{BackendTaskJoinError, TaskError};
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult, TaskResult};
use crate::context::AppContext;
use crate::utils::{egui_mpsc::SenderAsync, tasks::TaskManager};
use dash_sdk::platform::Identifier;
use futures::{FutureExt, future::join_all};
use std::{
    future::Future,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
};

fn request_error(request_id: Option<Identifier>, error: TaskError) -> TaskError {
    match request_id {
        Some(request_id) => TaskError::DashPayContactRequestActionFailed {
            request_id,
            source: Box::new(error),
        },
        None => error,
    }
}

pub(super) async fn run_batch<F, T>(
    requests: Vec<F>,
    mode: BackendTasksExecutionMode,
) -> Vec<Result<T, TaskError>>
where
    F: Future<Output = Result<T, TaskError>>,
{
    let guarded = requests.into_iter().map(|request| async move {
        AssertUnwindSafe(request)
            .catch_unwind()
            .await
            .unwrap_or_else(|_| Err(TaskError::BackendTaskPanicked))
    });
    match mode {
        BackendTasksExecutionMode::Concurrent => join_all(guarded).await,
        BackendTasksExecutionMode::Sequential => {
            let mut stopped = false;
            let mut results = Vec::new();
            for request in guarded {
                let result = if stopped {
                    Err(TaskError::BackendTaskBatchAborted)
                } else {
                    request.await
                };
                stopped |= matches!(result, Err(TaskError::BackendTaskPanicked));
                results.push(result);
            }
            results
        }
    }
}

/// Requests still awaiting an outcome, with the request id a paid DashPay
/// action reports under.
type PendingRequests = Arc<Mutex<Vec<(BackendTaskContext, Option<Identifier>)>>>;

/// Settle every request a stopped worker left without an outcome. One join
/// covers the whole batch, so each unfinished request hears of the failure
/// under its own context; the screen waiting on it is released instead of
/// loading for good.
async fn report_interrupted(
    joined: Result<(), tokio::task::JoinError>,
    pending: PendingRequests,
    sender: SenderAsync<TaskResult>,
) {
    let Err(source) = joined else {
        return;
    };
    let source = BackendTaskJoinError::from(source);
    let unfinished = std::mem::take(&mut *pending.lock().unwrap_or_else(|e| e.into_inner()));
    for (context, request_id) in unfinished {
        let error = request_error(
            request_id,
            TaskError::BackendTaskFailed {
                source: source.clone(),
            },
        );
        if let Err(error) = sender.send(TaskResult::Error { context, error }).await {
            tracing::error!(%error, "Could not deliver interrupted backend request");
        }
    }
}

pub(super) fn spawn(
    app_context: Arc<AppContext>,
    manager: &TaskManager,
    sender: SenderAsync<TaskResult>,
    tasks: Vec<(BackendTask, BackendTaskContext)>,
    mode: BackendTasksExecutionMode,
) -> Vec<TaskResult> {
    let pending: PendingRequests = Arc::new(Mutex::new(
        tasks
            .iter()
            .map(|(task, context)| {
                (
                    context.clone(),
                    crate::backend_task::dashpay_request_id(task),
                )
            })
            .collect::<Vec<_>>(),
    ));
    let worker_pending = pending.clone();
    let join_pending = pending.clone();
    let worker_sender = sender.clone();
    let join_sender = sender.clone();
    let handle = tokio::runtime::Handle::current();
    let registration = manager.spawn_blocking_sync(
        "backend_requests",
        move || {
            handle.block_on(async move {
                let requests = tasks
                    .into_iter()
                    .map(|(task, _)| {
                        let app_context = app_context.clone();
                        let sender = worker_sender.clone();
                        async move { app_context.run_backend_task(task, sender).await }
                    })
                    .collect();
                let results: Vec<Result<BackendTaskSuccessResult, TaskError>> =
                    run_batch(requests, mode).await;
                for result in results {
                    let (context, request_id) = worker_pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(0);
                    let result = result.map_err(|error| match error {
                        TaskError::BackendTaskPanicked | TaskError::BackendTaskBatchAborted => {
                            request_error(request_id, error)
                        }
                        other => other,
                    });
                    if let Err(error) = worker_sender
                        .send(TaskResult::from_backend_task_result(context, result))
                        .await
                    {
                        tracing::error!(%error, "Could not deliver backend request completion");
                    }
                }
            });
        },
        move |joined| report_interrupted(joined, join_pending, join_sender),
    );
    if registration.is_err() {
        let unfinished = std::mem::take(&mut *pending.lock().unwrap_or_else(|e| e.into_inner()));
        return unfinished
            .into_iter()
            .map(|(context, request_id)| {
                let error = request_error(request_id, TaskError::TaskManagerShuttingDown);
                TaskResult::Error { context, error }
            })
            .collect();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn sequential_panic_stops_later_work_but_reports_every_item() {
        let ran = AtomicUsize::new(0);
        let requests = (0..3)
            .map(|index| {
                let ran = &ran;
                async move {
                    ran.fetch_add(1, Ordering::SeqCst);
                    assert_ne!(index, 1, "test panic");
                    Ok(index)
                }
            })
            .collect();
        let results = run_batch(requests, BackendTasksExecutionMode::Sequential).await;
        assert_eq!(ran.load(Ordering::SeqCst), 2);
        assert!(matches!(
            results.as_slice(),
            [
                Ok(0),
                Err(TaskError::BackendTaskPanicked),
                Err(TaskError::BackendTaskBatchAborted)
            ]
        ));
    }

    #[tokio::test]
    async fn concurrent_panic_preserves_sibling_outcomes() {
        let requests = (0..3)
            .map(|index| async move {
                assert_ne!(index, 1, "test panic");
                Ok(index)
            })
            .collect();
        let results = run_batch(requests, BackendTasksExecutionMode::Concurrent).await;
        assert!(matches!(
            results.as_slice(),
            [Ok(0), Err(TaskError::BackendTaskPanicked), Ok(2)]
        ));
    }

    /// A worker that stops part-way settles each request it had not finished
    /// under that request's own context, so every screen waiting on one of
    /// them, such as Home's contact reads, is released.
    #[tokio::test]
    async fn stopped_worker_settles_each_unfinished_request() {
        use crate::model::dashpay::ContactsLoadKind;
        use dash_sdk::dpp::dashcore::Network;
        let read = |dispatch_id, kind| BackendTaskContext::Dispatched {
            dispatch_id,
            operation: Box::new(BackendTaskContext::DashPayContactsLoad {
                network: Network::Testnet,
                identity_id: Identifier::from([0x45; 32]),
                kind,
            }),
        };
        let contacts = read(u64::MAX - 1, ContactsLoadKind::Contacts);
        let requests = read(u64::MAX, ContactsLoadKind::Requests);
        for contexts in [
            vec![contacts.clone(), requests.clone()],
            vec![contacts.clone(), BackendTaskContext::Other],
            vec![BackendTaskContext::Other, BackendTaskContext::Other],
        ] {
            let pending: PendingRequests = Arc::new(Mutex::new(
                contexts
                    .iter()
                    .cloned()
                    .map(|context| (context, None))
                    .collect(),
            ));
            let (tx, mut rx) = tokio::sync::mpsc::channel(4);
            let sender = SenderAsync::new(tx, egui::Context::default());
            let joined = tokio::task::spawn_blocking(|| panic!("batch child panic")).await;

            report_interrupted(joined, pending.clone(), sender).await;

            let mut reported = Vec::new();
            while let Ok(result) = rx.try_recv() {
                let TaskResult::Error {
                    context,
                    error: TaskError::BackendTaskFailed { .. },
                } = result
                else {
                    panic!("expected a typed backend task failure, got {result:?}");
                };
                reported.push(context);
            }
            assert_eq!(reported, contexts);
            assert!(pending.lock().unwrap().is_empty());
        }
    }
}
