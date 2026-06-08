//! Thin wrapper around `AppContext::run_backend_task` for tests.

use dash_evo_tool::app::TaskResult;
use dash_evo_tool::backend_task::error::TaskError;
use dash_evo_tool::backend_task::{BackendTask, BackendTaskSuccessResult};
use dash_evo_tool::context::AppContext;
use dash_evo_tool::utils::egui_mpsc::SenderAsync;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Thread stack size for the SDK-bearing runtime (worker and blocking threads).
///
/// SDK proof verification recurses deeply via upstream `futures::executor::block_on`
/// (grovedb / drive-proof-verifier), overflowing the default 8 MB test-thread stack;
/// 32 MB is proven safe (16 MB is the floor).
const SDK_THREAD_STACK_SIZE: usize = 32 * 1024 * 1024;

/// Dedicated multi-thread runtime whose threads carry a large stack, so the deep
/// SDK `block_on` runs on a guaranteed-large stack regardless of `RUST_MIN_STACK`.
/// `thread_stack_size` applies to both worker and blocking threads.
fn sdk_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_stack_size(SDK_THREAD_STACK_SIZE)
            .thread_name("e2e-sdk")
            .build()
            .expect("failed to build SDK-bearing test runtime")
    })
}

/// Run a single backend task and return its result.
///
/// Creates a throwaway `SenderAsync` channel (the receiver is never read).
/// This is appropriate for tests that only care about the return value.
///
/// The task is driven via [`drive_on_large_stack`], so deep SDK proof
/// verification cannot overflow the default test-thread stack.
pub async fn run_task(
    app_context: &Arc<AppContext>,
    task: BackendTask,
) -> Result<BackendTaskSuccessResult, TaskError> {
    // Bounded channel (cap 32) with a dropped receiver: backend tasks send 0-1
    // results, so the sender never blocks in test usage.
    let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
    let sender = SenderAsync::new(tx, egui::Context::default());
    let app_context = Arc::clone(app_context);
    drive_on_large_stack(move || async move { app_context.run_backend_task(task, sender).await })
        .await
}

/// Drive an owned future to completion on the large-stack [`sdk_runtime`].
///
/// The future is built and `block_on`-driven on a 32 MB blocking thread of
/// `sdk_runtime`, so the deep synchronous SDK `block_on` runs on a guaranteed
/// large stack regardless of `RUST_MIN_STACK`. Constructing the future via
/// `make_fut` *on* the target thread means no future crosses a thread boundary,
/// sidestepping the higher-ranked `Send` bound of `tokio::spawn` (the backend
/// future holds lifetime-bound borrows that are not `Send` for all lifetimes).
/// The async caller awaits the result via a oneshot without blocking the shared
/// runtime. Tasks requiring `tokio::task::block_in_place` are not driven through
/// this path; the SDK proof-verify path that overflows does not use it.
async fn drive_on_large_stack<F, Fut, T>(make_fut: F) -> T
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = T>,
    T: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    sdk_runtime().spawn_blocking(move || {
        let out = tokio::runtime::Handle::current().block_on(make_fut());
        let _ = tx.send(out);
    });
    rx.await
        .expect("large-stack task panicked or was cancelled")
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

// Not `#[cfg(test)]`: in an integration-test crate, `cfg(test)` is false, so a
// gated module is compiled out entirely. A bare module keeps the `#[test]`
// collectable in the `backend-e2e` binary.
mod stack_smoke {
    use super::*;
    use std::hint::black_box;

    /// Per-frame stack array (16 KiB). `black_box` on a reference keeps the array
    /// live without the whole-array copy a `black_box(buf)` by-value would add.
    const FRAME_BYTES: usize = 16 * 1024;
    /// ~768 frames × 16 KiB ≈ 12 MiB of arrays — well above the 8 MiB default
    /// stack (so it could not run there) yet within the 32 MiB SDK stack.
    const RECURSION_DEPTH: usize = 768;

    /// Recurse `depth` frames, touching a stack array each frame so the optimizer
    /// cannot elide it. `black_box` defeats tail-call elimination.
    fn deep_recurse(depth: usize, acc: u64) -> u64 {
        let mut buf = [0u8; FRAME_BYTES];
        buf[depth % buf.len()] = (acc & 0xff) as u8;
        let sum = acc.wrapping_add(black_box(&buf)[0] as u64);
        if depth == 0 {
            sum
        } else {
            deep_recurse(depth - 1, sum)
        }
    }

    /// Drives a recursion whose stack footprint (~12 MiB) exceeds the default
    /// 8 MiB stack through the exact [`drive_on_large_stack`] path used by
    /// `run_task`, proving the large-stack mechanism is load-bearing without
    /// `RUST_MIN_STACK`. Cheap, deterministic, no network.
    #[test]
    fn large_stack_path_survives_deep_recursion() {
        let driver = sdk_runtime();
        let result = driver.block_on(drive_on_large_stack(|| async {
            deep_recurse(black_box(RECURSION_DEPTH), 0)
        }));
        assert_eq!(result, black_box(result));
    }
}
