//! Per-identity fetch state for the offer to restore keys stranded in the
//! previous version's saved data (issue #889).
//!
//! Detection reads the preserved legacy database, so it is a backend task, not
//! a frame-loop read. A screen owns one of these per identity it shows, renders
//! from the cached plan, and dispatches through it. Renders nothing itself —
//! [`LegacyRecoverySection`](crate::ui::components::legacy_recovery_section::LegacyRecoverySection)
//! is the widget.
//!
//! The offer holds no durable state and self-extinguishes: eligibility is
//! recomputed from the two stores on every check, so once nothing is left
//! stranded the plan comes back empty and the section disappears.

use dash_sdk::platform::Identifier;

use crate::backend_task::BackendTask;
use crate::backend_task::identity::IdentityTask;
use crate::context::AppContext;
use crate::model::legacy_recovery::{RecoveryItem, RecoveryPlan};

/// Where one identity's recovery offer currently stands.
enum FetchState {
    /// This install has no previous-version database, so there is nothing to
    /// detect — ever. A fresh install never leaves this state.
    Unavailable,
    /// Nothing dispatched yet. The only state a detection task starts from, so
    /// neither an empty plan nor a failure re-dispatches every frame.
    NotRequested,
    /// A detection task is in flight.
    Checking,
    /// Detection answered. An empty plan means nothing is offered.
    Offered(RecoveryPlan),
    /// A restore of this plan is in flight.
    Restoring(RecoveryPlan),
    /// Detection failed. The typed error already reached the user through the
    /// global banner; the offer stays hidden rather than re-asking.
    Failed,
}

/// The recovery offer for one identity, as a screen sees it.
pub struct LegacyRecoveryState {
    identity_id: Identifier,
    state: FetchState,
}

impl LegacyRecoveryState {
    /// The offer for `identity_id`, armed only when this install actually has a
    /// previous-version database to read. That check is one path probe at
    /// construction, never per frame.
    pub fn new(app_context: &AppContext, identity_id: Identifier) -> Self {
        let state = match app_context.db.db_file_path() {
            Some(path) if path.exists() => FetchState::NotRequested,
            _ => FetchState::Unavailable,
        };
        Self { identity_id, state }
    }

    /// The detection task to dispatch, or `None` when one already went out, has
    /// answered, or there is nothing to detect. Marks the check in flight, so a
    /// caller that dispatches the returned task fires it exactly once.
    pub fn ensure_checked(&mut self) -> Option<BackendTask> {
        if !matches!(self.state, FetchState::NotRequested) {
            return None;
        }
        self.state = FetchState::Checking;
        Some(BackendTask::IdentityTask(
            IdentityTask::CheckLegacyRecovery {
                identity_id: self.identity_id,
            },
        ))
    }

    /// The plan to render, or `None` while detection is outstanding, failed, or
    /// unavailable. An empty plan is still `Some` — the caller decides that
    /// nothing is worth showing.
    pub fn plan(&self) -> Option<&RecoveryPlan> {
        match &self.state {
            FetchState::Offered(plan) | FetchState::Restoring(plan) => Some(plan),
            _ => None,
        }
    }

    /// Whether a restore is in flight, so the section shows progress instead of
    /// a button that would dispatch the same restore twice.
    pub fn is_restoring(&self) -> bool {
        matches!(self.state, FetchState::Restoring(_))
    }

    /// The restore task for `approved`, marking it in flight. Returns `None`
    /// unless there is a plan on offer and no restore already running.
    pub fn restore(&mut self, approved: Vec<RecoveryItem>) -> Option<BackendTask> {
        let FetchState::Offered(plan) = &self.state else {
            return None;
        };
        self.state = FetchState::Restoring(plan.clone());
        Some(BackendTask::IdentityTask(
            IdentityTask::RecoverLegacyIdentityData {
                identity_id: self.identity_id,
                approved,
            },
        ))
    }

    /// Record the plan detection returned for `identity_id`. Ignores a result
    /// for any other identity, since a screen may show one identity while
    /// another's check is still in flight.
    pub fn offered(&mut self, identity_id: Identifier, plan: RecoveryPlan) {
        if identity_id == self.identity_id {
            self.state = FetchState::Offered(plan);
        }
    }

    /// Re-arm detection, so the offer recomputes from the store and disappears
    /// once nothing is left stranded. Called when a restore finishes and
    /// whenever a screen is arrived at again, since another screen's restore
    /// may have landed meanwhile.
    ///
    /// An install with no previous-version database stays unarmed: there is
    /// nothing to detect there, ever.
    pub fn completed(&mut self) {
        if !matches!(self.state, FetchState::Unavailable) {
            self.state = FetchState::NotRequested;
        }
    }

    /// Record that an in-flight operation failed.
    ///
    /// A failed restore returns to its offer, so a mistyped identity password
    /// can be corrected and Restore pressed again. A failed detection is
    /// terminal: retrying it would re-read the same unreadable row every frame.
    /// Attribution is coarse — an unrelated task's error arriving while a check
    /// is outstanding lands here too, which only hides an offer until the
    /// identity is opened again.
    pub fn failed(&mut self) {
        self.state = match std::mem::replace(&mut self.state, FetchState::Failed) {
            FetchState::Restoring(plan) => FetchState::Offered(plan),
            FetchState::Checking => FetchState::Failed,
            other => other,
        };
    }
}

#[cfg(test)]
impl LegacyRecoveryState {
    /// A state as it stands on an install that does have a previous-version
    /// database, without needing an `AppContext` to probe for one.
    fn armed(identity_id: Identifier) -> Self {
        Self {
            identity_id,
            state: FetchState::NotRequested,
        }
    }

    /// A state on an install with nothing to read.
    fn unavailable(identity_id: Identifier) -> Self {
        Self {
            identity_id,
            state: FetchState::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::legacy_recovery::{RecoveryItem, RecoveryItemDescriptor};
    use crate::model::qualified_identity::PrivateKeyTarget;

    fn identity(id: u8) -> Identifier {
        Identifier::from([id; 32])
    }

    /// A plan with one restorable key, so `Offered` carries something.
    fn plan() -> RecoveryPlan {
        RecoveryPlan {
            items: vec![RecoveryItemDescriptor {
                item: RecoveryItem::Key {
                    target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
                    key_id: 1,
                },
                purpose: None,
            }],
            excluded: vec![],
        }
    }

    fn is_check(task: &BackendTask) -> bool {
        matches!(
            task,
            BackendTask::IdentityTask(IdentityTask::CheckLegacyRecovery { .. })
        )
    }

    /// Detection is dispatched from the render loop, so the latch is the only
    /// thing standing between one check and one per frame.
    #[test]
    fn detection_is_dispatched_once_while_it_is_outstanding() {
        let mut state = LegacyRecoveryState::armed(identity(0x01));

        let first = state.ensure_checked().expect("the first frame dispatches");
        assert!(is_check(&first));
        assert!(
            state.ensure_checked().is_none(),
            "a check already in flight must not be dispatched again",
        );

        state.offered(identity(0x01), plan());
        assert!(
            state.ensure_checked().is_none(),
            "an answered check must not be re-dispatched either",
        );
    }

    /// An install with no previous-version database never asks at all.
    #[test]
    fn nothing_is_dispatched_without_previous_version_data() {
        let mut state = LegacyRecoveryState::unavailable(identity(0x02));

        assert!(state.ensure_checked().is_none());
        assert!(state.plan().is_none());
        state.completed();
        assert!(
            state.ensure_checked().is_none(),
            "there is nothing to re-arm on an install with nothing to read",
        );
    }

    /// A screen can show one identity while another's check is still in flight,
    /// so a result must only be adopted by the state that asked for it.
    #[test]
    fn a_result_for_another_identity_is_ignored() {
        let mut state = LegacyRecoveryState::armed(identity(0x03));
        state.ensure_checked().expect("dispatch");

        state.offered(identity(0x04), plan());
        assert!(
            state.plan().is_none(),
            "that plan belongs to another screen"
        );

        state.offered(identity(0x03), plan());
        assert!(state.plan().is_some());
    }

    /// Restore is only ever the answer to an offer on screen, and it cannot be
    /// dispatched twice for the same offer.
    #[test]
    fn a_restore_needs_an_offer_and_dispatches_once() {
        let mut state = LegacyRecoveryState::armed(identity(0x05));
        assert!(
            state.restore(vec![]).is_none(),
            "there is no offer to restore yet",
        );

        state.ensure_checked().expect("dispatch");
        state.offered(identity(0x05), plan());
        let task = state.restore(vec![]).expect("an offer can be restored");
        assert!(matches!(
            task,
            BackendTask::IdentityTask(IdentityTask::RecoverLegacyIdentityData { .. })
        ));
        assert!(state.is_restoring());
        assert!(
            state.restore(vec![]).is_none(),
            "a restore in flight must not be dispatched again",
        );
    }

    /// The offer's whole promise is that it retires itself: a finished restore
    /// re-arms detection, which recomputes from the store and comes back empty
    /// once nothing is left stranded.
    #[test]
    fn a_finished_restore_re_arms_detection() {
        let mut state = LegacyRecoveryState::armed(identity(0x06));
        state.ensure_checked().expect("dispatch");
        state.offered(identity(0x06), plan());
        state.restore(vec![]).expect("restore");

        state.completed();

        assert!(state.plan().is_none(), "the finished offer is gone");
        assert!(!state.is_restoring());
        assert!(
            state.ensure_checked().is_some_and(|task| is_check(&task)),
            "the next frame re-checks, so the offer disappears on its own",
        );
    }

    /// A failed restore keeps its offer so the user can correct a mistyped
    /// password and press Restore again; a failed check gives up instead of
    /// re-reading an unreadable row every frame.
    #[test]
    fn a_failure_returns_a_restore_to_its_offer_but_ends_a_check() {
        let mut restoring = LegacyRecoveryState::armed(identity(0x07));
        restoring.ensure_checked().expect("dispatch");
        restoring.offered(identity(0x07), plan());
        restoring.restore(vec![]).expect("restore");

        restoring.failed();

        assert!(!restoring.is_restoring());
        assert!(
            restoring.plan().is_some(),
            "the offer survives so the restore can be retried",
        );

        let mut checking = LegacyRecoveryState::armed(identity(0x08));
        checking.ensure_checked().expect("dispatch");

        checking.failed();

        assert!(checking.plan().is_none());
        assert!(
            checking.ensure_checked().is_none(),
            "a failed check must not re-read the same row every frame",
        );
    }
}
