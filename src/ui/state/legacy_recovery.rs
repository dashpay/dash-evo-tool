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
    /// The previous version's saved data holds no identity of this network, so
    /// there is nothing to detect — ever. A fresh install never leaves this
    /// state; an upgraded one never enters it.
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
    /// The offer for `identity_id`, armed only when the previous version's data
    /// actually holds identities for this network.
    ///
    /// The gate is on rows, not on the file: every install has a `data.db`,
    /// because a fresh one creates an empty compatibility database at first
    /// boot. Only rows can tell an upgrade from a first run, and the answer is
    /// cached on the context, so this costs nothing after the first screen.
    pub fn new(app_context: &AppContext, identity_id: Identifier) -> Self {
        let state = match app_context.has_legacy_identities() {
            true => FetchState::NotRequested,
            false => FetchState::Unavailable,
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

    /// Re-arm detection for a finished restore of `identity_id`, reporting
    /// whether that restore was this offer's.
    ///
    /// The attribution rule for a completion, and the twin of [`Self::offered`]:
    /// a restore answers on a channel that reaches whichever screen is visible
    /// when the answer arrives, not the screen that dispatched it. A completion
    /// for another identity must leave this offer's state — and any banner or
    /// reload a caller hangs off it — untouched.
    pub fn completed_for(&mut self, identity_id: Identifier) -> bool {
        if identity_id != self.identity_id {
            return false;
        }
        self.completed();
        true
    }

    /// Re-arm detection, so the offer recomputes from the store and disappears
    /// once nothing is left stranded. Called whenever a screen is arrived at
    /// again, since another screen's restore may have landed meanwhile;
    /// [`Self::completed_for`] is the attributed form for a restore result.
    ///
    /// An install with no previous-version data stays unarmed: there is nothing
    /// to detect there, ever.
    pub fn completed(&mut self) {
        if !matches!(self.state, FetchState::Unavailable) {
            self.state = FetchState::NotRequested;
        }
    }

    /// Record that this offer's own check or restore of `identity_id` failed,
    /// reporting whether the failure was in fact this offer's.
    ///
    /// The attribution rule for a failure, and the twin of [`Self::offered`]:
    /// errors reach whichever screen is visible when they arrive, so an
    /// unrelated task's failure must leave this offer alone. Ending a restore
    /// it never started would re-enable the Restore button while the original
    /// task still holds the identity, and pressing it again would only report
    /// that a load is already in progress.
    ///
    /// A failed restore returns to its offer, so a mistyped identity password
    /// can be corrected and Restore pressed again. A failed detection is
    /// terminal: retrying it would re-read the same unreadable row every frame.
    pub fn failed_for(&mut self, identity_id: Identifier) -> bool {
        if identity_id != self.identity_id {
            return false;
        }
        self.state = match std::mem::replace(&mut self.state, FetchState::Failed) {
            FetchState::Restoring(plan) => FetchState::Offered(plan),
            FetchState::Checking => FetchState::Failed,
            other => other,
        };
        true
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
    use crate::context::test_support::test_app_context;
    use crate::database::test_helpers::LegacyIdentityFixture;
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

    /// A testnet [`AppContext`] on a throwaway data dir whose legacy `identity`
    /// table already carries `rows` — the shape of an upgraded install, where
    /// the previous version's rows predate anything this session does. Only row
    /// existence is the gate's business, so each blob is a placeholder rather
    /// than an encoded record.
    fn context_carrying_legacy_rows(
        dir: &std::path::Path,
        rows: &[(u8, &str)],
    ) -> std::sync::Arc<AppContext> {
        {
            let db = crate::database::Database::new(dir.join("data.db")).expect("legacy db");
            db.create_tables(true).expect("legacy schema");
            db.set_default_version().expect("legacy version");
            for (id, network) in rows {
                LegacyIdentityFixture::new([*id; 32], Some(vec![0x01]), *network)
                    .insert(&db.locked_conn())
                    .expect("stage a legacy identity row");
            }
        }
        test_app_context(dir)
    }

    /// The gate is on rows, not on the file: every install has a `data.db`
    /// (a fresh one creates an empty compatibility database at first boot), so
    /// a file-existence check would arm detection for every user forever.
    /// Rows for *this* network are the only thing that tells an upgrade from a
    /// first run.
    #[test]
    fn only_a_legacy_identity_row_for_this_network_arms_the_check() {
        // One data dir each: the app k/v store takes an exclusive lock, so two
        // contexts cannot share a directory.
        let fresh_dir = tempfile::tempdir().expect("tempdir");
        let mainnet_dir = tempfile::tempdir().expect("tempdir");
        let upgraded_dir = tempfile::tempdir().expect("tempdir");

        let fresh = context_carrying_legacy_rows(fresh_dir.path(), &[]);
        assert!(
            fresh.db.db_file_path().is_some_and(|path| path.exists()),
            "the premise: the file this gate used to key on is there regardless",
        );
        assert!(
            LegacyRecoveryState::new(&fresh, identity(0x11))
                .ensure_checked()
                .is_none(),
            "an install whose legacy table holds nothing must never ask",
        );

        let other_network = context_carrying_legacy_rows(mainnet_dir.path(), &[(0x12, "dash")]);
        assert!(
            LegacyRecoveryState::new(&other_network, identity(0x12))
                .ensure_checked()
                .is_none(),
            "a mainnet row must not arm a testnet context",
        );

        let upgraded = context_carrying_legacy_rows(upgraded_dir.path(), &[(0x13, "testnet")]);
        assert!(
            LegacyRecoveryState::new(&upgraded, identity(0x13))
                .ensure_checked()
                .is_some_and(|task| is_check(&task)),
            "an install carrying this network's legacy identities must check",
        );
    }

    /// An install with no previous-version data never asks at all.
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

        assert!(restoring.failed_for(identity(0x07)));

        assert!(!restoring.is_restoring());
        assert!(
            restoring.plan().is_some(),
            "the offer survives so the restore can be retried",
        );

        let mut checking = LegacyRecoveryState::armed(identity(0x08));
        checking.ensure_checked().expect("dispatch");

        assert!(checking.failed_for(identity(0x08)));

        assert!(checking.plan().is_none());
        assert!(
            checking.ensure_checked().is_none(),
            "a failed check must not re-read the same row every frame",
        );
    }

    /// Regression: any failing task routed to the visible screen used to end the
    /// restore. Re-enabling Restore while the original task still holds the
    /// identity only buys the user an "already in progress" error, so a failure
    /// that is not this offer's own operation must change nothing.
    #[test]
    fn an_unrelated_failure_leaves_a_running_restore_alone() {
        let mut state = LegacyRecoveryState::armed(identity(0x09));
        state.ensure_checked().expect("dispatch");
        state.offered(identity(0x09), plan());
        state.restore(vec![]).expect("restore");

        assert!(
            !state.failed_for(identity(0x0A)),
            "another identity's failure is not this offer's to adopt",
        );
        assert!(
            state.is_restoring(),
            "the restore is still in flight, so the button must stay disabled",
        );
        assert!(
            state.restore(vec![]).is_none(),
            "and it must not be dispatchable a second time",
        );
    }
}
