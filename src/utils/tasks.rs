use std::sync::{Arc, atomic::AtomicUsize};
use tokio::time::{Duration, timeout};
use tokio_util::sync::CancellationToken;

/// Timeout duration for graceful shutdown.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct TaskManager {
    pub cancellation_token: CancellationToken, // Cancellation token for graceful shutdown
    tasks: Arc<tokio::sync::Mutex<tokio::task::JoinSet<()>>>, // Subtasks for graceful shutdown
}

/// TaskManager tracks spawned subtasks and allows for graceful shutdown of all tasks.
impl TaskManager {
    pub fn new() -> Self {
        let cancellation_token = CancellationToken::new();
        let subtasks = Arc::new(tokio::sync::Mutex::new(tokio::task::JoinSet::new()));

        TaskManager {
            cancellation_token,
            tasks: subtasks,
        }
    }

    // Spawn a new future as a subtask, to beu used in asynchronous context.
    // #[inline(always)]
    // pub async fn spawn_async<F>(&self, future: F)
    // where
    //     F: std::future::Future<Output = ()> + Send + 'static,
    //     F::Output: Send + 'static,
    // {
    //     spawn_subtask(self.tasks.clone(), future).await
    // }

    /// Spawn a new future as a subtask, to be used in synchronous context.
    ///
    /// Right now only used to manage dash-qt process.
    ///
    /// Note we don't correctly cleanup results of the spawned tasks, causing
    /// resource leaks. Before using this function in more places,
    /// we must implement a proper cleanup mechanism.
    #[inline(always)]
    pub fn spawn_sync<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
        F::Output: Send + 'static,
    {
        let subtasks = self.tasks.clone();
        tokio::spawn(spawn_subtask(subtasks, future));
    }

    /// Shutdown all subtasks gracefully.
    ///
    /// Wait for all subtasks to finish within a specified timeout, and then abort them.
    ///
    /// This is an equivalent of `Runtime::shutdown_timeout` but for subtasks.
    pub fn shutdown(&self) -> Result<(), String> {
        let cancel = self.cancellation_token.clone();
        let subtasks = self.tasks.clone();

        // a bit naive synchronization to wait for shutdown
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        // counter for logging
        let completed = Arc::new(AtomicUsize::new(0));

        let counter = completed.clone();
        let counter_for_timeout = completed.clone();
        // we need to run this task in separate task to avoid cancelling it during shutdown
        tokio::task::spawn(async move {
            // Cancel all background tasks
            tracing::trace!("shutdown: cancelling all tasks");
            cancel.cancel();

            // Wait for all subtasks to finish within SHUTDOWN_TIMEOUT

            let tasks_list = subtasks.clone();
            let timed_out = timeout(SHUTDOWN_TIMEOUT, async move {
                let mut tasks = tasks_list.lock().await;
                let total = tasks.len();
                tracing::trace!(total, "shutdown: joining tasks");
                let start = std::time::Instant::now();
                while let Some(handle) = tasks.join_next().await {
                    let i = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    match &handle {
                        Ok(()) => tracing::trace!(
                            task_num = i,
                            total,
                            elapsed_ms = start.elapsed().as_millis() as u64,
                            "shutdown: task joined OK"
                        ),
                        Err(e) => tracing::trace!(
                            task_num = i,
                            total,
                            elapsed_ms = start.elapsed().as_millis() as u64,
                            error = %e,
                            "shutdown: task joined with error"
                        ),
                    }
                }
            })
            .await;

            if timed_out.is_err() {
                let done = counter_for_timeout.load(std::sync::atomic::Ordering::Relaxed);
                tracing::trace!(
                    completed = done,
                    remaining = subtasks.try_lock().map(|s| Some(s.len())).ok().flatten(),
                    "shutdown: timed out waiting for tasks, aborting remaining"
                );
            }

            // now abort all tasks
            subtasks.lock().await.shutdown().await;

            // notify that shutdown is complete
            if tx.send(()).is_err() {
                tracing::error!("Failed to send shutdown completion signal");
            }
        });

        // wait for the shutdown task to finish
        const WAIT_TIME: Duration = Duration::from_millis(100);
        for _ in 0..SHUTDOWN_TIMEOUT.as_millis() / WAIT_TIME.as_millis() {
            if rx.try_recv().is_ok() {
                break;
            }
            // wait for a short time to avoid busy waiting
            std::thread::sleep(WAIT_TIME);
        }

        tracing::debug!(
            "Shutdown complete, {} subtasks finished cleanly",
            completed.load(std::sync::atomic::Ordering::Relaxed)
        );

        Ok(())
    }
}

#[inline(always)]
async fn spawn_subtask<F>(subtasks: Arc<tokio::sync::Mutex<tokio::task::JoinSet<()>>>, future: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
    F::Output: Send + 'static,
{
    let mut subtasks_lock = subtasks.lock().await;
    subtasks_lock.spawn(future);
}

impl Default for TaskManager {
    fn default() -> Self {
        TaskManager::new()
    }
}
