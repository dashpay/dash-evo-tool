use futures::FutureExt;
use std::sync::{Arc, Mutex};
use tokio::time::{Duration, timeout};
use tokio_util::sync::CancellationToken;

/// Timeout duration for graceful shutdown.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

// `Shared` needs a cloneable output; the mutex-wrapped option lets one observer take `JoinError`.
type TaskCompletion = futures::future::Shared<
    futures::future::BoxFuture<'static, Arc<Mutex<Option<Result<(), tokio::task::JoinError>>>>>,
>;

struct ActiveTaskGuard {
    active_names: Arc<Mutex<Vec<&'static str>>>,
    name: &'static str,
}

impl ActiveTaskGuard {
    fn new(active_names: Arc<Mutex<Vec<&'static str>>>, name: &'static str) -> Self {
        Self { active_names, name }
    }
}

impl Drop for ActiveTaskGuard {
    fn drop(&mut self) {
        let mut names = self
            .active_names
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(position) = names.iter().position(|name| *name == self.name) {
            names.swap_remove(position);
        }
    }
}

#[derive(Clone)]
struct TrackedTask {
    name: &'static str,
    completion: TaskCompletion,
}

impl std::fmt::Debug for TrackedTask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("TrackedTask")
            .field(&self.name)
            .finish()
    }
}

/// Terminal state of managed task shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskShutdownOutcome {
    /// Ordinary tasks and tracked joins reached their bounded shutdown points.
    Complete,
    /// Tracked work was still running when its shutdown wait expired.
    BackendTasksTimedOut,
}

#[derive(Debug, Clone)]
pub struct TaskManager {
    pub cancellation_token: CancellationToken, // Cancellation token for graceful shutdown
    task_state: Arc<Mutex<TaskState>>,         // Subtasks and their registration barrier
    active_names: Arc<Mutex<Vec<&'static str>>>, // Names of currently running tasks
}

#[derive(Debug)]
struct TaskState {
    accepting: bool,
    tracking_joins: bool,
    tasks: tokio::task::JoinSet<&'static str>,
    tracked_tasks: Vec<TrackedTask>,
}

#[derive(Debug)]
struct ShutdownTasks {
    tasks: tokio::task::JoinSet<&'static str>,
    task_state: Arc<Mutex<TaskState>>,
}

/// TaskManager tracks spawned subtasks and allows for graceful shutdown of all tasks.
impl TaskManager {
    pub fn new() -> Self {
        let cancellation_token = CancellationToken::new();

        TaskManager {
            cancellation_token,
            task_state: Arc::new(Mutex::new(TaskState {
                accepting: true,
                tracking_joins: true,
                tasks: tokio::task::JoinSet::new(),
                tracked_tasks: Vec::new(),
            })),
            active_names: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Spawn a named future as a subtask, to be used in synchronous context.
    ///
    /// The `name` label is logged during shutdown to identify slow tasks.
    /// Returns the original future when the shutdown admission barrier is closed.
    #[inline(always)]
    pub fn spawn_sync<F>(&self, name: &'static str, future: F) -> Result<(), F>
    where
        F: std::future::Future<Output = ()> + Send + 'static,
        F::Output: Send + 'static,
    {
        let mut state = self.task_state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.accepting {
            tracing::debug!(task = name, "Rejected task registration during shutdown");
            return Err(future);
        }
        self.active_names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(name);
        let active_names = Arc::clone(&self.active_names);
        state.tasks.spawn(async move {
            let _active_task = ActiveTaskGuard::new(active_names, name);
            future.await;
            name
        });
        Ok(())
    }

    /// Spawn blocking work and retain its real task handle through shutdown.
    ///
    /// The async join observer remains an ordinary abortable subtask, while a
    /// separate completion handle lets shutdown await non-cancellable blocking work.
    /// Returns the work and callback when shutdown has stopped accepting tasks.
    pub fn spawn_blocking_sync<F, C, Fut>(
        &self,
        name: &'static str,
        task: F,
        on_join: C,
    ) -> Result<(), (F, C)>
    where
        F: FnOnce() + Send + 'static,
        C: FnOnce(Result<(), tokio::task::JoinError>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let mut state = self.task_state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.accepting {
            tracing::debug!(
                task = name,
                "Rejected blocking task registration during shutdown"
            );
            return Err((task, on_join));
        }

        let join_handle = tokio::task::spawn_blocking(task);
        let completion = task_completion(join_handle);
        self.register_tracked_completion(&mut state, name, completion, on_join);
        Ok(())
    }

    /// Spawn async work whose join must survive ordinary-task shutdown.
    pub fn spawn_tracked_sync<F, T, C, Fut>(
        &self,
        name: &'static str,
        task: F,
        on_join: C,
    ) -> Result<(), (F, C)>
    where
        F: std::future::Future<Output = T> + Send + 'static,
        T: Send + 'static,
        C: FnOnce(Result<(), tokio::task::JoinError>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let mut state = self.task_state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.tracking_joins {
            tracing::debug!(task = name, "Rejected task join tracking after shutdown");
            return Err((task, on_join));
        }

        let completion = task_completion(tokio::spawn(task));
        self.register_tracked_completion(&mut state, name, completion, on_join);
        Ok(())
    }

    fn register_tracked_completion<C, Fut>(
        &self,
        state: &mut TaskState,
        name: &'static str,
        completion: TaskCompletion,
        on_join: C,
    ) where
        C: FnOnce(Result<(), tokio::task::JoinError>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let callback_completion = task_completion(tokio::spawn(async move {
            let result = completion
                .await
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            if let Some(result) = result {
                on_join(result).await;
            }
        }));
        state.tracked_tasks.retain(|task| {
            task.completion.peek().is_none_or(|completion| {
                completion
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .is_some()
            })
        });
        state.tracked_tasks.push(TrackedTask {
            name,
            completion: callback_completion.clone(),
        });
        if !state.accepting {
            return;
        }
        self.active_names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(name);
        let active_names = Arc::clone(&self.active_names);
        state.tasks.spawn(async move {
            let _active_task = ActiveTaskGuard::new(active_names, name);
            let result = callback_completion
                .await
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            if let Some(Err(error)) = result {
                tracing::warn!(
                    task = name,
                    cancelled = error.is_cancelled(),
                    panicked = error.is_panic(),
                    "Tracked task completion callback stopped unexpectedly"
                );
            }
            name
        });
    }

    /// Maximum time used by the ordinary-task and tracked-join shutdown phases.
    pub const fn graceful_shutdown_budget() -> Duration {
        SHUTDOWN_TIMEOUT.saturating_add(SHUTDOWN_TIMEOUT)
    }

    /// Start an asynchronous graceful shutdown of all subtasks.
    ///
    /// Cancels ordinary tasks and returns a receiver that resolves after both
    /// ordinary-task and tracked-join waits reach a bounded outcome. This does
    /// **not** block the calling thread, so the UI can keep repainting.
    pub fn shutdown_async(&self) -> tokio::sync::oneshot::Receiver<TaskShutdownOutcome> {
        let tasks = self.begin_shutdown();
        let active_names = self.active_names.clone();

        let (tx, rx) = tokio::sync::oneshot::channel::<TaskShutdownOutcome>();

        tokio::task::spawn(async move {
            let (completed, outcome) = shutdown_all_inner(
                tasks,
                &active_names,
                "async",
                SHUTDOWN_TIMEOUT,
                SHUTDOWN_TIMEOUT,
            )
            .await;

            tracing::debug!(
                ?outcome,
                "Async shutdown complete, {} subtasks finished cleanly",
                completed
            );

            let _ = tx.send(outcome);
        });

        rx
    }

    fn begin_shutdown(&self) -> ShutdownTasks {
        let tasks = {
            let mut state = self.task_state.lock().unwrap_or_else(|e| e.into_inner());
            state.accepting = false;
            std::mem::take(&mut state.tasks)
        };
        self.cancellation_token.cancel();
        ShutdownTasks {
            tasks,
            task_state: Arc::clone(&self.task_state),
        }
    }
}

fn task_completion<T>(task: tokio::task::JoinHandle<T>) -> TaskCompletion
where
    T: Send + 'static,
{
    async move { Arc::new(Mutex::new(Some(task.await.map(|_| ())))) }
        .boxed()
        .shared()
}

async fn shutdown_all_inner(
    tasks: ShutdownTasks,
    active_names: &Arc<Mutex<Vec<&'static str>>>,
    label: &str,
    task_timeout: Duration,
    blocking_task_timeout: Duration,
) -> (usize, TaskShutdownOutcome) {
    let completed = shutdown_inner(tasks.tasks, active_names, label, task_timeout).await;
    let tracked_tasks_completed =
        shutdown_tracked_tasks(tasks.task_state, label, blocking_task_timeout).await;
    let outcome = if tracked_tasks_completed {
        TaskShutdownOutcome::Complete
    } else {
        TaskShutdownOutcome::BackendTasksTimedOut
    };
    (completed, outcome)
}

/// Join the tasks captured by the registration barrier, aborting on timeout.
///
/// Returns the number of tasks that completed cleanly within the timeout.
/// The `label` is used in log messages to distinguish async vs blocking callers.
async fn shutdown_inner(
    mut tasks: tokio::task::JoinSet<&'static str>,
    active_names: &Arc<Mutex<Vec<&'static str>>>,
    label: &str,
    shutdown_timeout: Duration,
) -> usize {
    let mut completed = 0;
    let timed_out = timeout(shutdown_timeout, async {
        let total = tasks.len();
        tracing::trace!(total, "{label}: joining tasks");
        let start = std::time::Instant::now();
        while let Some(handle) = tasks.join_next().await {
            completed += 1;
            match &handle {
                Ok(name) => {
                    tracing::trace!(
                        task = name,
                        task_num = completed,
                        total,
                        elapsed_ms = start.elapsed().as_millis() as u64,
                        "{label}: task joined OK"
                    );
                }
                Err(e) => tracing::trace!(
                    task_num = completed,
                    total,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    cancelled = e.is_cancelled(),
                    panicked = e.is_panic(),
                    "{label}: task joined with error"
                ),
            }
        }
    })
    .await;

    if timed_out.is_err() {
        let remaining: Vec<&str> = active_names.lock().map(|n| n.clone()).unwrap_or_default();
        tracing::trace!(
            completed,
            remaining_count = remaining.len(),
            remaining_tasks = ?remaining,
            "{label}: timed out waiting for tasks, aborting remaining"
        );

        #[cfg(tokio_unstable)]
        {
            let handle = tokio::runtime::Handle::current();
            let dump = handle.dump().await;
            for (i, task) in dump.tasks().iter().enumerate() {
                tracing::trace!(
                    task_num = i,
                    trace = %task.trace(),
                    "{label}: active tokio task"
                );
            }
        }
    }

    // Abort all remaining tasks
    tasks.shutdown().await;

    completed
}

async fn shutdown_tracked_tasks(
    task_state: Arc<Mutex<TaskState>>,
    label: &str,
    shutdown_timeout: Duration,
) -> bool {
    let mut total = 0;
    let joined = timeout(shutdown_timeout, async {
        loop {
            let tasks = {
                let mut state = task_state.lock().unwrap_or_else(|error| error.into_inner());
                if state.tracked_tasks.is_empty() {
                    state.tracking_joins = false;
                    break;
                }
                std::mem::take(&mut state.tracked_tasks)
            };
            total += tasks.len();
            tracing::trace!(
                batch = tasks.len(),
                total,
                "{label}: joining tracked task work"
            );
            let completions = futures::future::join_all(
                tasks
                    .into_iter()
                    .map(|task| async move { (task.name, task.completion.await) }),
            )
            .await;
            for (name, completion) in completions {
                let result = completion
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .take();
                if let Some(Err(error)) = result {
                    tracing::warn!(
                        task = name,
                        cancelled = error.is_cancelled(),
                        panicked = error.is_panic(),
                        "Tracked task completion callback stopped unexpectedly"
                    );
                }
            }
        }
    })
    .await;

    if joined.is_err() {
        task_state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .tracking_joins = false;
        tracing::warn!(
            total,
            timeout_secs = shutdown_timeout.as_secs(),
            "Tracked backend work exceeded shutdown wait; continuing with degraded teardown"
        );
        false
    } else {
        tracing::trace!(total, "{label}: tracked task work joined");
        true
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        TaskManager::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn completed_blocking_entry_is_pruned_on_next_registration() {
        let manager = TaskManager::new();
        let (joined_tx, joined_rx) = tokio::sync::oneshot::channel();
        assert!(
            manager
                .spawn_blocking_sync(
                    "completed-backend-task",
                    || {},
                    move |_| async move {
                        let _ = joined_tx.send(());
                    },
                )
                .is_ok(),
            "completed task is accepted"
        );
        joined_rx.await.expect("completed task callback ran");
        tokio::time::timeout(Duration::from_secs(1), async {
            while !manager
                .active_names
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("completed task observer ran");

        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        assert!(
            manager
                .spawn_blocking_sync(
                    "pending-backend-task",
                    move || {
                        release_rx.recv().expect("wait for task release");
                    },
                    |_| async {},
                )
                .is_ok(),
            "pending task is accepted"
        );

        let tracked = manager
            .task_state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .tracked_tasks
            .len();
        release_tx.send(()).expect("release pending task");

        assert_eq!(tracked, 1, "only the pending blocking task stays tracked");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn panicking_on_join_callback_is_removed_from_active_names() {
        let manager = TaskManager::new();
        assert!(
            manager
                .spawn_blocking_sync(
                    "panicking-on-join",
                    || {},
                    |_| async { panic!("on_join panic for regression coverage") },
                )
                .is_ok(),
            "panicking callback task is accepted"
        );

        let _ = shutdown_all_inner(
            manager.begin_shutdown(),
            &manager.active_names,
            "test",
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await;

        assert!(
            manager
                .active_names
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty(),
            "completed work is not reported as active when its callback panics"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_rejects_tasks_submitted_after_the_barrier() {
        let manager = TaskManager::new();
        let accepted_task_ran = Arc::new(AtomicBool::new(false));
        let ran = Arc::clone(&accepted_task_ran);
        assert!(
            manager
                .spawn_sync("accepted-task", async move {
                    ran.store(true, Ordering::Release);
                })
                .is_ok(),
            "task is accepted before shutdown"
        );

        let shutdown = manager.shutdown_async();
        let late_task_ran = Arc::new(AtomicBool::new(false));
        let ran = Arc::clone(&late_task_ran);

        let late_task = manager.spawn_sync("late-task", async move {
            ran.store(true, Ordering::Release);
        });

        shutdown.await.expect("shutdown completion");
        tokio::task::yield_now().await;
        assert!(accepted_task_ran.load(Ordering::Acquire));
        assert!(!late_task_ran.load(Ordering::Acquire));
        late_task
            .expect_err("late task is returned to the caller")
            .await;
        assert!(late_task_ran.load(Ordering::Acquire));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_waits_for_blocking_work_past_abort_timeout() {
        let manager = TaskManager::new();
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        assert!(
            manager
                .spawn_blocking_sync(
                    "backend-task",
                    move || {
                        started_tx.send(()).expect("report task start");
                        release_rx.recv().expect("wait for task release");
                    },
                    |_| async {},
                )
                .is_ok(),
            "backend task is accepted"
        );
        started_rx.recv().expect("blocking task started");

        let teardown_started = Arc::new(AtomicBool::new(false));
        let teardown_flag = Arc::clone(&teardown_started);
        let shutdown = manager.shutdown_async();
        let gated_teardown = tokio::spawn(async move {
            let outcome = shutdown.await.expect("shutdown completion");
            teardown_flag.store(true, Ordering::Release);
            outcome
        });
        tokio::time::sleep(SHUTDOWN_TIMEOUT + Duration::from_millis(100)).await;
        let teardown_started_while_blocking = teardown_started.load(Ordering::Acquire);

        release_tx.send(()).expect("release blocking task");
        let outcome = tokio::time::timeout(Duration::from_secs(1), gated_teardown)
            .await
            .expect("shutdown after blocking task release")
            .expect("gated teardown task");

        assert!(
            !teardown_started_while_blocking,
            "wallet teardown must stay gated while blocking work is running"
        );
        assert_eq!(outcome, TaskShutdownOutcome::Complete);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_reports_backend_task_timeout_as_degraded() {
        let manager = TaskManager::new();
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        assert!(
            manager
                .spawn_blocking_sync(
                    "stuck-backend-task",
                    move || {
                        started_tx.send(()).expect("report task start");
                        release_rx.recv().expect("wait for task release");
                    },
                    |_| async {},
                )
                .is_ok(),
            "stuck backend task is accepted"
        );
        started_rx.recv().expect("blocking task started");

        let outcome = shutdown_all_inner(
            manager.begin_shutdown(),
            &manager.active_names,
            "test",
            Duration::from_millis(10),
            Duration::from_millis(20),
        )
        .await
        .1;
        release_tx.send(()).expect("release blocking task");

        assert_eq!(outcome, TaskShutdownOutcome::BackendTasksTimedOut);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn request_is_tracked_before_ordinary_shutdown_aborts_its_caller() {
        let manager = Arc::new(TaskManager::new());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (_release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let task_manager = Arc::clone(&manager);
        assert!(
            manager
                .spawn_sync("managed-request", async move {
                    let result = crate::backend_task::await_managed_network_request_with_timeout(
                        task_manager,
                        "request-reaper",
                        Duration::from_secs(1),
                        async move {
                            let _ = started_tx.send(());
                            let _ = release_rx.await;
                        },
                        |source| {
                            crate::backend_task::error::TaskError::TokenBalanceRefreshTimeout {
                                source,
                            }
                        },
                    )
                    .await;
                    let _ = result_tx.send(result);
                })
                .is_ok(),
            "managed request is accepted"
        );
        started_rx.await.expect("request started");

        let outcome = shutdown_all_inner(
            manager.begin_shutdown(),
            &manager.active_names,
            "test",
            Duration::from_millis(10),
            Duration::from_millis(25),
        )
        .await
        .1;

        assert!(
            result_rx.await.is_err(),
            "ordinary shutdown aborts the request caller before its timeout"
        );
        assert_eq!(outcome, TaskShutdownOutcome::BackendTasksTimedOut);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accepted_timeout_reaper_cannot_yield_complete_while_request_runs() {
        let manager = Arc::new(TaskManager::new());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (_release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let result = crate::backend_task::await_managed_network_request_with_timeout(
            Arc::clone(&manager),
            "request-reaper",
            Duration::from_millis(10),
            async move {
                let _ = started_tx.send(());
                let _ = release_rx.await;
            },
            |source| crate::backend_task::error::TaskError::TokenBalanceRefreshTimeout { source },
        );
        let (_, result) = tokio::join!(started_rx, result);
        assert!(result.is_err());

        let outcome = shutdown_all_inner(
            manager.begin_shutdown(),
            &manager.active_names,
            "test",
            Duration::from_millis(10),
            Duration::from_millis(25),
        )
        .await
        .1;

        assert_eq!(outcome, TaskShutdownOutcome::BackendTasksTimedOut);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_consumes_late_blocking_join_error() {
        let manager = TaskManager::new();
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        let join_error_observed = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&join_error_observed);
        assert!(
            manager
                .spawn_blocking_sync(
                    "late-panic",
                    move || {
                        started_tx.send(()).expect("report task start");
                        release_rx.recv().expect("wait for task release");
                        panic!("synthetic panic without secret material");
                    },
                    move |result| async move {
                        observed.store(result.is_err(), Ordering::Release);
                    },
                )
                .is_ok(),
            "late panic task is accepted"
        );
        started_rx.recv().expect("blocking task started");
        let tasks = manager.begin_shutdown();
        let completion = tasks
            .task_state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .tracked_tasks[0]
            .completion
            .clone();

        shutdown_inner(
            tasks.tasks,
            &manager.active_names,
            "test",
            Duration::from_millis(10),
        )
        .await;
        release_tx.send(()).expect("release blocking task");

        assert!(
            manager
                .spawn_tracked_sync("later-tracked-task", async {}, |_| async {})
                .is_ok(),
            "tracked registration remains open during the join phase"
        );

        assert!(shutdown_tracked_tasks(tasks.task_state, "test", Duration::from_secs(1)).await);
        assert!(
            join_error_observed.load(Ordering::Acquire),
            "the tracked callback consumes the late join error"
        );

        let stored = completion
            .peek()
            .expect("blocking completion")
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert!(
            stored.is_none(),
            "shutdown consumes the callback completion"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_finishes_join_callback_consumed_before_ordinary_abort() {
        let manager = TaskManager::new();
        let (callback_started_tx, callback_started_rx) = tokio::sync::oneshot::channel();
        let (release_callback_tx, release_callback_rx) = tokio::sync::oneshot::channel();
        let callback_finished = Arc::new(AtomicBool::new(false));
        let finished = Arc::clone(&callback_finished);
        assert!(
            manager
                .spawn_tracked_sync(
                    "callback-in-progress",
                    async { panic!("synthetic panic without secret material") },
                    move |result| async move {
                        assert!(result.is_err(), "task panic reaches its join callback");
                        let _ = callback_started_tx.send(());
                        let _ = release_callback_rx.await;
                        finished.store(true, Ordering::Release);
                    },
                )
                .is_ok(),
            "tracked task is accepted"
        );

        callback_started_rx.await.expect("join callback started");
        let tasks = manager.begin_shutdown();
        shutdown_inner(tasks.tasks, &manager.active_names, "test", Duration::ZERO).await;
        let _ = release_callback_tx.send(());

        assert!(
            shutdown_tracked_tasks(tasks.task_state, "test", Duration::from_secs(1)).await,
            "tracked callback finishes within its shutdown phase"
        );
        assert!(
            callback_finished.load(Ordering::Acquire),
            "tracked shutdown finishes a callback after ordinary observers are aborted"
        );
    }
}
