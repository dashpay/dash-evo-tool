//! Typed side channel for signing failures that must cross the SDK boundary.
//!
//! The upstream `Signer` trait returns `ProtocolError`, whose only free-form
//! variants carry a `String`, so a typed [`TaskError`] raised while resolving a
//! signing key would be flattened into `ProtocolError::Generic` and surface as a
//! generic SDK error. The signer records the typed error here; the
//! `From<SdkError>` conversion takes it back out when the SDK hands that generic
//! error up. The slot is scoped to one backend task (a `tokio` task-local), so
//! concurrent tasks never see each other's failures, and code running outside a
//! [`scope`] behaves exactly as if this module did not exist.

use std::cell::RefCell;
use std::future::Future;

use super::error::TaskError;

tokio::task_local! {
    static SIGNING_FAILURE: RefCell<Option<TaskError>>;
}

/// Run `future` with an empty signing-failure slot of its own.
pub(crate) async fn scope<F: Future>(future: F) -> F::Output {
    SIGNING_FAILURE.scope(RefCell::new(None), future).await
}

/// Remember `error` as the cause of the signing failure about to be flattened
/// into a `ProtocolError`. Last write wins; a no-op outside a [`scope`].
pub(crate) fn record(error: TaskError) {
    let _ = SIGNING_FAILURE.try_with(|slot| *slot.borrow_mut() = Some(error));
}

/// Take the recorded signing failure, leaving the slot empty.
pub(crate) fn take() -> Option<TaskError> {
    SIGNING_FAILURE
        .try_with(|slot| slot.borrow_mut().take())
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_recorded_failure_is_taken_once() {
        let taken = scope(async {
            record(TaskError::SecretSeamMissing);
            (take(), take())
        })
        .await;
        assert!(matches!(taken.0, Some(TaskError::SecretSeamMissing)));
        assert!(taken.1.is_none(), "the slot empties on take");
    }

    #[tokio::test]
    async fn outside_a_scope_nothing_is_recorded() {
        record(TaskError::SecretSeamMissing);
        assert!(take().is_none());
    }

    #[tokio::test]
    async fn scopes_do_not_share_a_slot() {
        scope(async { record(TaskError::SecretSeamMissing) }).await;
        assert!(scope(async { take() }).await.is_none());
    }
}
