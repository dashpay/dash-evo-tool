//! Whether a masternode's stored voting key is still its current one, and
//! the user's removal of an old one.
//!
//! The node page owns one of these per opened node. The check runs as a
//! backend task (`IdentityTask::CheckRetiredVotingKey`) because the answer
//! comes from the masternode list; nothing here reads it or touches storage.

use crate::app::AppAction;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::ui::state::PendingRequest;
use dash_sdk::platform::Identifier;

/// What the last check said about the node's stored voting key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Not checked yet, or the stored key changed since the last check.
    Unchecked,
    /// The check is out.
    Checking,
    /// Current, nothing stored, or the list could not tell.
    Current,
    /// The masternode list names another voting key.
    Retired,
}

/// The retired-voting-key notice of one node's page.
#[derive(Debug)]
pub struct RetiredVotingKeyState {
    identity_id: Identifier,
    status: Status,
    /// The user chose to keep the old key; hidden until the page is opened
    /// again.
    dismissed: bool,
    check: PendingRequest,
    removal: PendingRequest,
}

impl RetiredVotingKeyState {
    pub fn new(identity_id: Identifier) -> Self {
        Self {
            identity_id,
            status: Status::Unchecked,
            dismissed: false,
            check: PendingRequest::default(),
            removal: PendingRequest::default(),
        }
    }

    /// The check to dispatch, or `None` when one is out or has answered for
    /// the stored key. Marks the check out, so the caller dispatches the
    /// returned action.
    pub fn ensure_checked(&mut self) -> Option<AppAction> {
        if self.status != Status::Unchecked {
            return None;
        }
        self.status = Status::Checking;
        Some(self.check.dispatch(BackendTask::IdentityTask(
            IdentityTask::CheckRetiredVotingKey {
                identity_id: self.identity_id,
            },
        )))
    }

    /// Check again on the next frame: the stored key or the list may have
    /// changed. An answer still out for the old state is ignored.
    pub fn recheck(&mut self) {
        self.check.clear();
        self.status = Status::Unchecked;
    }

    /// Whether to show the notice.
    pub fn shows_notice(&self) -> bool {
        self.status == Status::Retired && !self.dismissed
    }

    /// Whether the removal the user confirmed is still running.
    pub fn is_removing(&self) -> bool {
        self.removal.is_pending()
    }

    /// Hide the notice until the page is opened again.
    pub fn dismiss(&mut self) {
        self.dismissed = true;
    }

    /// The removal the user confirmed.
    pub fn remove(&mut self) -> AppAction {
        self.removal.dispatch(BackendTask::IdentityTask(
            IdentityTask::RemoveRetiredVotingKey {
                identity_id: self.identity_id,
            },
        ))
    }

    /// Take in a result this page's own check or removal produced. Results
    /// of other dispatches, including an older check, are ignored.
    pub fn absorb_result(
        &mut self,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) {
        match result {
            BackendTaskSuccessResult::RetiredVotingKeyStatus {
                identity_id,
                retired,
            } if *identity_id == self.identity_id && self.check.settle(context).is_some() => {
                self.status = if *retired {
                    Status::Retired
                } else {
                    Status::Current
                };
            }
            BackendTaskSuccessResult::RetiredVotingKeyRemoved { identity_id }
            | BackendTaskSuccessResult::RetiredVotingKeyKept { identity_id }
                if *identity_id == self.identity_id && self.removal.settle(context).is_some() =>
            {
                // Whatever the removal found, the old key no longer needs the
                // user: it is gone, or it is current again.
                self.check.clear();
                self.status = Status::Current;
            }
            _ => {}
        }
    }

    /// Take in an error. Only this page's own check or removal is settled:
    /// a failed check shows no notice, a failed removal offers it again.
    pub fn absorb_error(&mut self, context: &BackendTaskContext) {
        if self.check.settle(context).is_some() {
            self.status = Status::Current;
        }
        self.removal.settle(context);
    }
}

#[cfg(test)]
impl RetiredVotingKeyState {
    pub(crate) fn check_context_for_test(&self) -> Option<BackendTaskContext> {
        self.check.context().cloned()
    }

    pub(crate) fn removal_context_for_test(&self) -> Option<BackendTaskContext> {
        self.removal.context().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> Identifier {
        Identifier::new([0x4D; 32])
    }

    fn dispatched_task(action: AppAction) -> BackendTask {
        match action {
            AppAction::BackendTaskWithContext { task, .. } => task,
            other => panic!("expected a dispatched task, got {other:?}"),
        }
    }

    fn status(retired: bool) -> BackendTaskSuccessResult {
        BackendTaskSuccessResult::RetiredVotingKeyStatus {
            identity_id: node(),
            retired,
        }
    }

    /// Checked once; a retired answer shows the notice.
    #[test]
    fn a_retired_answer_shows_the_notice() {
        let mut state = RetiredVotingKeyState::new(node());

        let task = dispatched_task(state.ensure_checked().expect("first check"));
        assert!(matches!(
            task,
            BackendTask::IdentityTask(IdentityTask::CheckRetiredVotingKey { identity_id }) if identity_id == node()
        ));
        assert!(state.ensure_checked().is_none(), "one check at a time");
        let context = state.check_context_for_test().unwrap();
        state.absorb_result(&context, &status(true));

        assert!(state.shows_notice());
        assert!(state.ensure_checked().is_none(), "answered");
    }

    /// The answer to a check superseded by a recheck is ignored.
    #[test]
    fn an_answer_for_an_older_check_is_ignored() {
        let mut state = RetiredVotingKeyState::new(node());
        state.ensure_checked();
        let old = state.check_context_for_test().unwrap();
        state.recheck();
        state.ensure_checked();

        state.absorb_result(&old, &status(true));

        assert!(!state.shows_notice());
        assert!(state.ensure_checked().is_none(), "the recheck is still out");
    }

    /// "Keep it" hides the notice; a failed check never shows it.
    #[test]
    fn keep_hides_the_notice_and_a_failed_check_shows_none() {
        let mut state = RetiredVotingKeyState::new(node());
        state.ensure_checked();
        let context = state.check_context_for_test().unwrap();
        state.absorb_result(&context, &status(true));
        state.dismiss();
        assert!(!state.shows_notice());

        let mut failed = RetiredVotingKeyState::new(node());
        failed.ensure_checked();
        let context = failed.check_context_for_test().unwrap();
        failed.absorb_error(&context);
        assert!(!failed.shows_notice());
        assert!(failed.ensure_checked().is_none(), "no retry loop");
    }

    /// The removal runs until its own result; then the notice goes, whether
    /// the key was removed or found current again. A failure offers it again.
    #[test]
    fn the_removal_settles_on_its_own_outcome() {
        for outcome in [
            BackendTaskSuccessResult::RetiredVotingKeyRemoved {
                identity_id: node(),
            },
            BackendTaskSuccessResult::RetiredVotingKeyKept {
                identity_id: node(),
            },
        ] {
            let mut state = RetiredVotingKeyState::new(node());
            state.ensure_checked();
            let check = state.check_context_for_test().unwrap();
            state.absorb_result(&check, &status(true));

            let task = dispatched_task(state.remove());
            assert!(matches!(
                task,
                BackendTask::IdentityTask(IdentityTask::RemoveRetiredVotingKey { identity_id }) if identity_id == node()
            ));
            assert!(state.is_removing());
            let removal = state.removal_context_for_test().unwrap();
            state.absorb_result(&removal, &outcome);

            assert!(!state.is_removing(), "{outcome:?}");
            assert!(!state.shows_notice(), "{outcome:?}");
        }

        let mut failed = RetiredVotingKeyState::new(node());
        failed.ensure_checked();
        let check = failed.check_context_for_test().unwrap();
        failed.absorb_result(&check, &status(true));
        failed.remove();
        let removal = failed.removal_context_for_test().unwrap();
        failed.absorb_error(&removal);
        assert!(!failed.is_removing());
        assert!(failed.shows_notice(), "a failed removal offers it again");
    }
}
