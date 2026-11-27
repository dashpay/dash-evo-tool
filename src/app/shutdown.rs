//! Whole-application shutdown, including late-created network contexts.
//!
//! The UI owns close intent and viewport presentation. This module owns the
//! task-drain and wallet-teardown ordering shared by both exit adapters.

use super::TaskResult;
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::utils::tasks::{TaskManager, TaskShutdownOutcome};
use crate::wallet_backend::WalletBackend;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc as tokiompsc, oneshot};

const WALLET_BACKEND_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_DEADLINE_MARGIN: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ShutdownOutcome {
    Complete,
    TaskManagerFailed,
    BackendTasksTimedOut,
    WalletBackendTimedOut,
}

pub(super) fn shutdown_hard_deadline() -> Duration {
    TaskManager::graceful_shutdown_budget()
        + WALLET_BACKEND_SHUTDOWN_TIMEOUT
        + SHUTDOWN_DEADLINE_MARGIN
}

trait ShutdownWalletBackend: Send + Sync {
    fn forget_all_secrets(&self);
    fn shutdown(&self) -> futures::future::BoxFuture<'_, ()>;
}

impl ShutdownWalletBackend for WalletBackend {
    fn forget_all_secrets(&self) {
        WalletBackend::forget_all_secrets(self);
    }

    fn shutdown(&self) -> futures::future::BoxFuture<'_, ()> {
        Box::pin(WalletBackend::shutdown(self))
    }
}

/// Takes ownership of result drainage once, before either exit adapter waits.
pub(super) struct ShutdownSession {
    contexts: Vec<Arc<AppContext>>,
    task_shutdown: oneshot::Receiver<TaskShutdownOutcome>,
    task_result_receiver: tokiompsc::Receiver<TaskResult>,
    #[cfg(feature = "mcp")]
    mcp_app_context: Option<Arc<arc_swap::ArcSwap<AppContext>>>,
}

impl ShutdownSession {
    pub(super) fn begin(
        contexts: impl IntoIterator<Item = Arc<AppContext>>,
        subtasks: &TaskManager,
        task_result_receiver: &mut tokiompsc::Receiver<TaskResult>,
        #[cfg(feature = "mcp")] mcp_app_context: Option<Arc<arc_swap::ArcSwap<AppContext>>>,
    ) -> Self {
        let mut unique_contexts = Vec::new();
        for context in contexts {
            push_unique_context(&mut unique_contexts, context);
        }
        let task_shutdown = subtasks.shutdown_async();
        let (_empty_sender, empty_receiver) = tokiompsc::channel(1);
        Self {
            contexts: unique_contexts,
            task_shutdown,
            task_result_receiver: std::mem::replace(task_result_receiver, empty_receiver),
            #[cfg(feature = "mcp")]
            mcp_app_context,
        }
    }

    pub(super) fn start(self) -> oneshot::Receiver<ShutdownOutcome> {
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let mut session = self;
            let outcome = loop {
                tokio::select! {
                    result = &mut session.task_shutdown => break result.ok(),
                    task_result = session.task_result_receiver.recv() => {
                        let Some(task_result) = task_result else {
                            break (&mut session.task_shutdown).await.ok();
                        };
                        collect_created_context(&mut session.contexts, task_result);
                    }
                }
            };
            let _ = tx.send(session.finish(outcome).await);
        });
        rx
    }

    pub(super) fn run_blocking(mut self) {
        const POLL_INTERVAL: Duration = Duration::from_millis(10);
        let task_shutdown_outcome = loop {
            while let Ok(task_result) = self.task_result_receiver.try_recv() {
                collect_created_context(&mut self.contexts, task_result);
            }
            match self.task_shutdown.try_recv() {
                Ok(outcome) => break Some(outcome),
                Err(oneshot::error::TryRecvError::Closed) => break None,
                Err(oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(POLL_INTERVAL);
                }
            }
        };
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        tokio::spawn(async move {
            let _ = tx.send(self.finish(task_shutdown_outcome).await);
        });
        match rx.recv_timeout(WALLET_BACKEND_SHUTDOWN_TIMEOUT + SHUTDOWN_DEADLINE_MARGIN) {
            Ok(ShutdownOutcome::Complete)
                if task_shutdown_outcome == Some(TaskShutdownOutcome::Complete) => {}
            Ok(outcome) => tracing::warn!(
                ?outcome,
                ?task_shutdown_outcome,
                "Blocking shutdown fallback completed with degraded teardown"
            ),
            Err(error) => tracing::warn!(
                ?error,
                "Blocking wallet backend shutdown did not report a terminal outcome"
            ),
        }
    }

    async fn finish(self, outcome: Option<TaskShutdownOutcome>) -> ShutdownOutcome {
        finish_wallet_shutdown(
            outcome,
            self.contexts,
            self.task_result_receiver,
            #[cfg(feature = "mcp")]
            self.mcp_app_context,
        )
        .await
    }
}

fn push_unique_context(contexts: &mut Vec<Arc<AppContext>>, context: Arc<AppContext>) {
    if !contexts
        .iter()
        .any(|existing| Arc::ptr_eq(existing, &context))
    {
        contexts.push(context);
    }
}

fn collect_created_context(contexts: &mut Vec<Arc<AppContext>>, task_result: TaskResult) {
    if let TaskResult::Success { result, .. } = task_result {
        match *result {
            BackendTaskSuccessResult::NetworkContextRegistered { context, .. }
            | BackendTaskSuccessResult::NetworkContextCreated { context, .. } => {
                push_unique_context(contexts, context);
            }
            _ => {}
        }
    }
}

async fn shutdown_wallet_backend_instances<B: ShutdownWalletBackend>(
    wallet_backends: &[Arc<B>],
    shutdown_timeout: Duration,
) -> ShutdownOutcome {
    forget_wallet_backend_secrets(wallet_backends);

    let shutdowns = futures::future::join_all(
        wallet_backends
            .iter()
            .map(|wallet_backend| wallet_backend.shutdown()),
    );
    let outcome = if tokio::time::timeout(shutdown_timeout, shutdowns)
        .await
        .is_err()
    {
        tracing::warn!(
            timeout_secs = shutdown_timeout.as_secs(),
            "Wallet backend shutdown timed out; closing with degraded teardown"
        );
        ShutdownOutcome::WalletBackendTimedOut
    } else {
        ShutdownOutcome::Complete
    };

    forget_wallet_backend_secrets(wallet_backends);

    outcome
}

fn forget_wallet_backend_secrets<B: ShutdownWalletBackend>(wallet_backends: &[Arc<B>]) {
    for backend in wallet_backends {
        backend.forget_all_secrets();
    }
}

async fn shutdown_wallet_backends(
    task_shutdown_outcome: Option<TaskShutdownOutcome>,
    contexts: Vec<Arc<AppContext>>,
) -> ShutdownOutcome {
    let mut wallet_backends = Vec::new();
    for context in contexts {
        if let Ok(backend) = context.wallet_backend()
            && !wallet_backends
                .iter()
                .any(|existing| Arc::ptr_eq(existing, &backend))
        {
            wallet_backends.push(backend);
        }
    }

    finish_shutdown_after_tasks(
        task_shutdown_outcome,
        &wallet_backends,
        shutdown_wallet_backend_instances(&wallet_backends, WALLET_BACKEND_SHUTDOWN_TIMEOUT),
    )
    .await
}

async fn finish_wallet_shutdown(
    task_shutdown_outcome: Option<TaskShutdownOutcome>,
    mut contexts: Vec<Arc<AppContext>>,
    mut task_result_receiver: tokiompsc::Receiver<TaskResult>,
    #[cfg(feature = "mcp")] mcp_app_context: Option<Arc<arc_swap::ArcSwap<AppContext>>>,
) -> ShutdownOutcome {
    while let Ok(task_result) = task_result_receiver.try_recv() {
        collect_created_context(&mut contexts, task_result);
    }

    #[cfg(feature = "mcp")]
    if let Some(mcp_app_context) = mcp_app_context {
        push_unique_context(&mut contexts, mcp_app_context.load_full());
    }

    shutdown_wallet_backends(task_shutdown_outcome, contexts).await
}

async fn finish_shutdown_after_tasks<B, F>(
    task_shutdown_outcome: Option<TaskShutdownOutcome>,
    wallet_backends: &[Arc<B>],
    wallet_shutdown: F,
) -> ShutdownOutcome
where
    B: ShutdownWalletBackend,
    F: std::future::Future<Output = ShutdownOutcome>,
{
    forget_wallet_backend_secrets(wallet_backends);

    match task_shutdown_outcome {
        Some(TaskShutdownOutcome::Complete) => wallet_shutdown.await,
        Some(TaskShutdownOutcome::BackendTasksTimedOut) => ShutdownOutcome::BackendTasksTimedOut,
        None => ShutdownOutcome::TaskManagerFailed,
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use crate::app::AppState;
    use crate::backend_task::error::TaskError;
    use dash_sdk::dpp::dashcore::Network;
    use eframe::egui;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct MockShutdownBackend {
        forget_calls: AtomicUsize,
        secret_cached: AtomicBool,
        shutdown_completes: bool,
    }

    impl ShutdownWalletBackend for MockShutdownBackend {
        fn forget_all_secrets(&self) {
            self.forget_calls.fetch_add(1, Ordering::Relaxed);
            self.secret_cached.store(false, Ordering::Release);
        }

        fn shutdown(&self) -> futures::future::BoxFuture<'_, ()> {
            self.secret_cached.store(true, Ordering::Release);
            if self.shutdown_completes {
                Box::pin(async {})
            } else {
                Box::pin(std::future::pending())
            }
        }
    }

    struct TestDataDir {
        prior: Option<String>,
        _temp_dir: tempfile::TempDir,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TestDataDir {
        fn enter() -> Self {
            let lock = crate::test_support::DASH_EVO_DATA_DIR_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let temp_dir = tempfile::tempdir().expect("temporary app data directory");
            let prior = std::env::var("DASH_EVO_DATA_DIR").ok();
            // Safety: the process-global test lock serializes this environment override.
            unsafe { std::env::set_var("DASH_EVO_DATA_DIR", temp_dir.path()) };
            Self {
                prior,
                _temp_dir: temp_dir,
                _lock: lock,
            }
        }
    }

    impl Drop for TestDataDir {
        fn drop(&mut self) {
            // Safety: `_lock` remains held until after the prior value is restored.
            unsafe {
                match &self.prior {
                    Some(value) => std::env::set_var("DASH_EVO_DATA_DIR", value),
                    None => std::env::remove_var("DASH_EVO_DATA_DIR"),
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn second_exit_after_async_attempt_does_not_repeat_wallet_teardown() {
        use crate::wallet_backend::{RememberPolicy, SecretPlaintext, SecretScope};
        use zeroize::Zeroizing;
        let seed_24 = crate::test_support::keys::seed();

        let _data_dir = TestDataDir::enter();
        let mut app = AppState::new(egui::Context::default()).expect("test AppState");
        let context = app.current_app_context().clone();
        context
            .ensure_wallet_backend(app.task_result_sender.clone())
            .await
            .expect("wire test wallet backend");
        let backend = context.wallet_backend().expect("wired wallet backend");
        let secret_access = backend.secret_access();
        let scope = SecretScope::HdSeed {
            seed_hash: [0x42; 32],
        };
        let secret = Zeroizing::new(seed_24);
        secret_access.remember_session(
            &scope,
            SecretPlaintext::HdSeed(&secret),
            RememberPolicy::UntilAppClose,
        );

        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        assert!(
            app.subtasks
                .spawn_blocking_sync(
                    "slow-shutdown-regression-task",
                    move || {
                        started_tx.send(()).expect("report slow task start");
                        release_rx.recv().expect("wait for slow task release");
                    },
                    |_| async {},
                )
                .is_ok(),
            "slow shutdown task is accepted"
        );
        started_rx.recv().expect("slow task started");

        app.shutdown_started = Some(std::time::Instant::now());
        let mut shutdown = app.start_async_shutdown();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut shutdown)
                .await
                .is_err(),
            "wallet teardown waits for in-flight blocking work"
        );
        release_tx.send(()).expect("release slow task");
        let outcome = tokio::time::timeout(Duration::from_secs(5), &mut shutdown)
            .await
            .expect("async shutdown stays within its budget")
            .expect("async shutdown task reports an outcome");
        assert_eq!(outcome, ShutdownOutcome::Complete);
        assert!(!secret_access.is_session_cached(&scope));

        secret_access.remember_session(
            &scope,
            SecretPlaintext::HdSeed(&secret),
            RememberPolicy::UntilAppClose,
        );
        app.shutdown_finished = false;
        app.shutdown_receiver = None;
        eframe::App::on_exit(&mut app);

        assert!(
            secret_access.is_session_cached(&scope),
            "on_exit must not invoke wallet teardown again after any async attempt"
        );
        secret_access.forget_all();
    }

    #[tokio::test]
    async fn wallet_shutdown_clears_secrets_again_after_teardown_wait() {
        let backend = Arc::new(MockShutdownBackend {
            forget_calls: AtomicUsize::new(0),
            secret_cached: AtomicBool::new(true),
            shutdown_completes: true,
        });

        let outcome =
            shutdown_wallet_backend_instances(&[Arc::clone(&backend)], Duration::from_secs(1))
                .await;

        assert_eq!(outcome, ShutdownOutcome::Complete);
        assert_eq!(backend.forget_calls.load(Ordering::Relaxed), 2);
        assert!(
            !backend.secret_cached.load(Ordering::Acquire),
            "a secret re-cached during shutdown is cleared by the final pass"
        );
    }

    #[tokio::test]
    async fn wallet_shutdown_clears_secrets_again_after_teardown_timeout() {
        let backend = Arc::new(MockShutdownBackend {
            forget_calls: AtomicUsize::new(0),
            secret_cached: AtomicBool::new(true),
            shutdown_completes: false,
        });

        let outcome =
            shutdown_wallet_backend_instances(&[Arc::clone(&backend)], Duration::from_millis(10))
                .await;

        assert_eq!(outcome, ShutdownOutcome::WalletBackendTimedOut);
        assert_eq!(backend.forget_calls.load(Ordering::Relaxed), 2);
        assert!(
            !backend.secret_cached.load(Ordering::Acquire),
            "the final clear still runs when backend shutdown exceeds its timeout"
        );
    }

    #[tokio::test]
    async fn backend_task_timeout_forgets_secrets_without_wallet_teardown() {
        let backend = Arc::new(MockShutdownBackend {
            forget_calls: AtomicUsize::new(0),
            secret_cached: AtomicBool::new(true),
            shutdown_completes: true,
        });
        let teardown_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&teardown_calls);

        let outcome = finish_shutdown_after_tasks(
            Some(TaskShutdownOutcome::BackendTasksTimedOut),
            &[Arc::clone(&backend)],
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                ShutdownOutcome::Complete
            },
        )
        .await;

        assert_eq!(outcome, ShutdownOutcome::BackendTasksTimedOut);
        assert_eq!(backend.forget_calls.load(Ordering::Relaxed), 1);
        assert!(!backend.secret_cached.load(Ordering::Acquire));
        assert_eq!(teardown_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn task_manager_failure_forgets_secrets_without_wallet_teardown() {
        let backend = Arc::new(MockShutdownBackend {
            forget_calls: AtomicUsize::new(0),
            secret_cached: AtomicBool::new(true),
            shutdown_completes: true,
        });
        let teardown_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&teardown_calls);

        let outcome = finish_shutdown_after_tasks(None, &[Arc::clone(&backend)], async move {
            calls.fetch_add(1, Ordering::Relaxed);
            ShutdownOutcome::Complete
        })
        .await;

        assert_eq!(outcome, ShutdownOutcome::TaskManagerFailed);
        assert_eq!(backend.forget_calls.load(Ordering::Relaxed), 1);
        assert!(!backend.secret_cached.load(Ordering::Acquire));
        assert_eq!(teardown_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn complete_task_shutdown_forgets_secrets_and_runs_wallet_teardown_once() {
        let backend = Arc::new(MockShutdownBackend {
            forget_calls: AtomicUsize::new(0),
            secret_cached: AtomicBool::new(true),
            shutdown_completes: true,
        });
        let teardown_calls = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&teardown_calls);

        let outcome = finish_shutdown_after_tasks(
            Some(TaskShutdownOutcome::Complete),
            &[Arc::clone(&backend)],
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                ShutdownOutcome::Complete
            },
        )
        .await;

        assert_eq!(outcome, ShutdownOutcome::Complete);
        assert_eq!(backend.forget_calls.load(Ordering::Relaxed), 1);
        assert!(!backend.secret_cached.load(Ordering::Acquire));
        assert_eq!(teardown_calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn collect_created_context_only_keeps_network_context_results() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let context = crate::context::test_support::test_app_context(temp_dir.path());
        let mut contexts = Vec::new();

        collect_created_context(
            &mut contexts,
            TaskResult::unattributed_success(BackendTaskSuccessResult::NetworkContextRegistered {
                network: Network::Testnet,
                context: Arc::clone(&context),
            }),
        );
        assert_eq!(contexts.len(), 1);
        assert!(Arc::ptr_eq(&contexts[0], &context));

        collect_created_context(
            &mut contexts,
            TaskResult::unattributed_success(BackendTaskSuccessResult::NetworkContextCreated {
                network: Network::Testnet,
                context: Arc::clone(&context),
                spv_started: false,
            }),
        );
        assert_eq!(contexts.len(), 1);
        assert!(Arc::ptr_eq(&contexts[0], &context));

        collect_created_context(
            &mut contexts,
            TaskResult::unattributed_success(BackendTaskSuccessResult::NetworkContextCreated {
                network: Network::Testnet,
                context: Arc::clone(&context),
                spv_started: false,
            }),
        );
        assert_eq!(contexts.len(), 1);

        collect_created_context(
            &mut contexts,
            TaskResult::unattributed_success(BackendTaskSuccessResult::None),
        );
        assert_eq!(contexts.len(), 1);

        collect_created_context(
            &mut contexts,
            TaskResult::unattributed_error(TaskError::NoIdentitiesFound),
        );
        assert_eq!(contexts.len(), 1);
    }

    #[test]
    fn viewport_deadline_is_derived_from_shutdown_phase_budgets() {
        assert_eq!(
            shutdown_hard_deadline(),
            TaskManager::graceful_shutdown_budget()
                + WALLET_BACKEND_SHUTDOWN_TIMEOUT
                + SHUTDOWN_DEADLINE_MARGIN
        );
    }
}
