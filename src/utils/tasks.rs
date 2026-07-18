use futures::FutureExt;
use std::sync::{Arc, Mutex};
use tokio::time::{Duration, timeout};
use tokio_util::sync::CancellationToken;

/// Timeout duration for graceful shutdown.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

type BlockingTaskCompletion = futures::future::Shared<
    futures::future::BoxFuture<'static, Arc<Mutex<Option<Result<(), tokio::task::JoinError>>>>>,
>;

#[derive(Clone)]
struct TrackedBlockingTask {
    name: &'static str,
    completion: BlockingTaskCompletion,
}

impl std::fmt::Debug for TrackedBlockingTask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("TrackedBlockingTask")
            .field(&self.name)
            .finish()
    }
}

/// Terminal state of managed task shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskShutdownOutcome {
    /// Ordinary and blocking tasks reached their bounded shutdown points.
    Complete,
    /// Blocking work was still running when its shutdown wait expired.
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
    tasks: tokio::task::JoinSet<&'static str>,
    blocking_tasks: Vec<TrackedBlockingTask>,
}

#[derive(Debug)]
struct ShutdownTasks {
    tasks: tokio::task::JoinSet<&'static str>,
    blocking_tasks: Vec<TrackedBlockingTask>,
}

/// TaskManager tracks spawned subtasks and allows for graceful shutdown of all tasks.
impl TaskManager {
    pub fn new() -> Self {
        let cancellation_token = CancellationToken::new();

        TaskManager {
            cancellation_token,
            task_state: Arc::new(Mutex::new(TaskState {
                accepting: true,
                tasks: tokio::task::JoinSet::new(),
                blocking_tasks: Vec::new(),
            })),
            active_names: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Spawn a named future as a subtask, to be used in synchronous context.
    ///
    /// The `name` label is logged during shutdown to identify slow tasks.
    #[inline(always)]
    pub fn spawn_sync<F>(&self, name: &'static str, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
        F::Output: Send + 'static,
    {
        let mut state = self.task_state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.accepting {
            tracing::debug!(task = name, "Rejected task registration during shutdown");
            return;
        }
        state.tasks.spawn(async move {
            future.await;
            name
        });
        self.active_names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(name);
    }

    /// Spawn blocking work and retain its real task handle through shutdown.
    ///
    /// The async join observer remains an ordinary abortable subtask, while a
    /// separate completion handle lets shutdown await non-cancellable blocking work.
    pub fn spawn_blocking_sync<F, C, Fut>(&self, name: &'static str, task: F, on_join: C)
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
            return;
        }

        let join_handle = tokio::task::spawn_blocking(task);
        let completion = async move { Arc::new(Mutex::new(Some(join_handle.await))) }
            .boxed()
            .shared();
        state.blocking_tasks.push(TrackedBlockingTask {
            name,
            completion: completion.clone(),
        });
        state.tasks.spawn(async move {
            let result = completion
                .await
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take();
            if let Some(result) = result {
                on_join(result).await;
            }
            name
        });
        self.active_names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(name);
    }

    /// Start an asynchronous graceful shutdown of all subtasks.
    ///
    /// Cancels ordinary tasks and returns a receiver that resolves after both
    /// ordinary and blocking task waits reach a bounded outcome. This does
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

    /// Shutdown all subtasks gracefully (blocking).
    ///
    /// Ordinary tasks are aborted after [`SHUTDOWN_TIMEOUT`]. Blocking work is
    /// then awaited for the same bound and returns an error if still running.
    /// Blocks the calling thread. Prefer [`shutdown_async`] for a responsive UI.
    ///
    /// This is an equivalent of `Runtime::shutdown_timeout` but for subtasks.
    pub fn shutdown(&self) -> Result<(), String> {
        let tasks = self.begin_shutdown();
        let active_names = self.active_names.clone();

        // a bit naive synchronization to wait for shutdown
        let (tx, mut rx) = tokio::sync::oneshot::channel::<(usize, TaskShutdownOutcome)>();

        // we need to run this task in separate task to avoid cancelling it during shutdown
        tokio::task::spawn(async move {
            let result = shutdown_all_inner(
                tasks,
                &active_names,
                "blocking",
                SHUTDOWN_TIMEOUT,
                SHUTDOWN_TIMEOUT,
            )
            .await;

            // notify that shutdown is complete
            if tx.send(result).is_err() {
                tracing::error!("Failed to send shutdown completion signal");
            }
        });

        // wait for the shutdown task to finish
        const WAIT_TIME: Duration = Duration::from_millis(100);
        let mut completed = 0;
        let mut outcome = TaskShutdownOutcome::BackendTasksTimedOut;
        for _ in 0..(2 * SHUTDOWN_TIMEOUT.as_millis()) / WAIT_TIME.as_millis() {
            if let Ok((count, shutdown_outcome)) = rx.try_recv() {
                completed = count;
                outcome = shutdown_outcome;
                break;
            }
            // wait for a short time to avoid busy waiting
            std::thread::sleep(WAIT_TIME);
        }

        tracing::debug!("Shutdown complete, {} subtasks finished cleanly", completed);

        match outcome {
            TaskShutdownOutcome::Complete => Ok(()),
            TaskShutdownOutcome::BackendTasksTimedOut => {
                Err("backend task blocking work timed out during shutdown".to_owned())
            }
        }
    }

    fn begin_shutdown(&self) -> ShutdownTasks {
        let (tasks, blocking_tasks) = {
            let mut state = self.task_state.lock().unwrap_or_else(|e| e.into_inner());
            state.accepting = false;
            (
                std::mem::take(&mut state.tasks),
                std::mem::take(&mut state.blocking_tasks),
            )
        };
        self.cancellation_token.cancel();
        ShutdownTasks {
            tasks,
            blocking_tasks,
        }
    }
}

async fn shutdown_all_inner(
    tasks: ShutdownTasks,
    active_names: &Arc<Mutex<Vec<&'static str>>>,
    label: &str,
    task_timeout: Duration,
    blocking_task_timeout: Duration,
) -> (usize, TaskShutdownOutcome) {
    let completed = shutdown_inner(tasks.tasks, active_names, label, task_timeout).await;
    let blocking_tasks_completed =
        shutdown_blocking_tasks(tasks.blocking_tasks, label, blocking_task_timeout).await;
    let outcome = if blocking_tasks_completed {
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
    let names_for_join = active_names.clone();
    let timed_out = timeout(shutdown_timeout, async {
        let total = tasks.len();
        tracing::trace!(total, "{label}: joining tasks");
        let start = std::time::Instant::now();
        while let Some(handle) = tasks.join_next().await {
            completed += 1;
            match &handle {
                Ok(name) => {
                    // Remove one instance of this name from active list
                    if let Ok(mut names) = names_for_join.lock()
                        && let Some(pos) = names.iter().position(|n| *n == *name)
                    {
                        names.swap_remove(pos);
                    }
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
                    error = %e,
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

async fn shutdown_blocking_tasks(
    tasks: Vec<TrackedBlockingTask>,
    label: &str,
    shutdown_timeout: Duration,
) -> bool {
    let total = tasks.len();
    if total == 0 {
        return true;
    }

    tracing::trace!(total, "{label}: joining backend task blocking work");
    let joined = timeout(
        shutdown_timeout,
        futures::future::join_all(tasks.into_iter().map(|task| async move {
            task.completion.await;
            task.name
        })),
    )
    .await;

    if joined.is_err() {
        tracing::warn!(
            total,
            timeout_secs = shutdown_timeout.as_secs(),
            "Backend task blocking work exceeded shutdown wait; continuing with degraded teardown"
        );
        false
    } else {
        tracing::trace!(total, "{label}: backend task blocking work joined");
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

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_rejects_tasks_submitted_after_the_barrier() {
        let manager = TaskManager::new();
        let accepted_task_ran = Arc::new(AtomicBool::new(false));
        let ran = Arc::clone(&accepted_task_ran);
        manager.spawn_sync("accepted-task", async move {
            ran.store(true, Ordering::Release);
        });

        let shutdown = manager.shutdown_async();
        let late_task_ran = Arc::new(AtomicBool::new(false));
        let ran = Arc::clone(&late_task_ran);

        manager.spawn_sync("late-task", async move {
            ran.store(true, Ordering::Release);
        });

        shutdown.await.expect("shutdown completion");
        tokio::task::yield_now().await;
        assert!(accepted_task_ran.load(Ordering::Acquire));
        assert!(!late_task_ran.load(Ordering::Acquire));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutdown_waits_for_blocking_work_past_abort_timeout() {
        let manager = TaskManager::new();
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        manager.spawn_blocking_sync(
            "backend-task",
            move || {
                started_tx.send(()).expect("report task start");
                release_rx.recv().expect("wait for task release");
            },
            |_| async {},
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
        manager.spawn_blocking_sync(
            "stuck-backend-task",
            move || {
                started_tx.send(()).expect("report task start");
                release_rx.recv().expect("wait for task release");
            },
            |_| async {},
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
}
