//! Lifecycle registry for identity loads.
//!
//! A dispatched load is invisible to the screen that started it: task results
//! reach only the *visible* screen, and a load has no cooperative cancellation.
//! A screen that navigates away therefore cannot learn from a callback whether
//! its load is still running, succeeded, or failed — and neither proxy for that
//! answer holds up:
//!
//! - **"the task holds a claim"** does not mean "running": a load is outstanding
//!   from the moment it is dispatched, and only claims its identity once the task
//!   starts and validates its input.
//! - **"the node is in the store"** does not mean "succeeded":
//!   `load_identity` inserts the node before sealing its keys, so a seal failure
//!   leaves the node persisted by a load that errored.
//!
//! So the load reports its own phase here, at every transition, and the registry
//! is the single source of truth for it. The same records give the load exclusive
//! use of its identity for the whole check → fetch → insert → seal span, which is
//! what makes `IdentityLoadMode::RejectIfExists` safe: those steps are not one
//! atomic operation, so two overlapping loads of one identity would otherwise
//! both pass the duplicate check and clobber each other's insert.
//!
//! Records outlive their load and are overwritten by the next load of that
//! identity — the map holds at most one entry per identity loaded this session.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dash_sdk::platform::Identifier;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;

/// How far a dispatched identity load has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityLoadPhase {
    /// Dispatched. Its task has not started, or has not reached its claim yet.
    Submitted,
    /// Running: the task holds this identity's exclusive claim.
    Running,
    /// Finished, fully applied — the node is stored, with its keys as requested.
    Loaded,
    /// Finished with an error. The node may still have been persisted: the insert
    /// precedes the key seal, and a failed seal leaves the insert behind. Anything
    /// resuming from this phase must offer a retry, not report success.
    Failed,
}

impl IdentityLoadPhase {
    /// Whether the load can still change the store, and so must not be resubmitted
    /// or reported as finished.
    pub fn is_outstanding(self) -> bool {
        matches!(self, Self::Submitted | Self::Running)
    }
}

/// The phase of the most recent load of each identity.
pub(crate) type IdentityLoadPhases = Arc<Mutex<HashMap<Identifier, IdentityLoadPhase>>>;

/// A load's exclusive claim on one identity. Records the load's terminal phase
/// when dropped: [`Loaded`](IdentityLoadPhase::Loaded) if the task called
/// [`loaded`](Self::loaded) after its last fallible step, otherwise
/// [`Failed`](IdentityLoadPhase::Failed) — so every `?`, and a panic, still leaves
/// a truthful record.
#[derive(Debug)]
pub struct IdentityLoadGuard {
    phases: IdentityLoadPhases,
    identity_id: Identifier,
    loaded: bool,
}

impl IdentityLoadGuard {
    /// Record that the load fully applied. Call it after the last step that can
    /// fail — anything left undone after this point is reported as a success.
    pub fn loaded(mut self) {
        self.loaded = true;
    }
}

impl Drop for IdentityLoadGuard {
    fn drop(&mut self) {
        let mut phases = self.phases.lock().unwrap_or_else(|e| e.into_inner());
        // A `Running` record for this identity can only be this guard's own claim:
        // `begin_identity_load` refuses a second one. Any other phase means a newer
        // load superseded this record, and its outcome is not this guard's to write.
        if phases.get(&self.identity_id) != Some(&IdentityLoadPhase::Running) {
            return;
        }
        let phase = if self.loaded {
            IdentityLoadPhase::Loaded
        } else {
            IdentityLoadPhase::Failed
        };
        phases.insert(self.identity_id, phase);
    }
}

impl AppContext {
    /// Record that a load of `identity_id` has been dispatched, before its task
    /// runs. Callers that gate on the load must mark it here: until the task
    /// claims the identity there is nothing else that says the load is
    /// outstanding.
    pub fn mark_identity_load_submitted(&self, identity_id: Identifier) {
        self.identity_load_phases
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(identity_id, IdentityLoadPhase::Submitted);
    }

    /// Claim `identity_id` for a load, excluding any concurrent load of the same
    /// identity until the returned guard drops.
    ///
    /// # Errors
    ///
    /// Returns [`TaskError::IdentityLoadInProgress`] when a load of this identity
    /// is already running.
    pub fn begin_identity_load(
        &self,
        identity_id: Identifier,
    ) -> Result<IdentityLoadGuard, TaskError> {
        let mut phases = self
            .identity_load_phases
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        if phases.get(&identity_id) == Some(&IdentityLoadPhase::Running) {
            return Err(TaskError::IdentityLoadInProgress { identity_id });
        }
        phases.insert(identity_id, IdentityLoadPhase::Running);
        drop(phases);

        Ok(IdentityLoadGuard {
            phases: self.identity_load_phases.clone(),
            identity_id,
            loaded: false,
        })
    }

    /// How far this session's most recent load of `identity_id` got, or `None`
    /// when no load of it was dispatched. Cheap enough to call from the frame
    /// loop.
    pub fn identity_load_phase(&self, identity_id: &Identifier) -> Option<IdentityLoadPhase> {
        self.identity_load_phases
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(identity_id)
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;

    #[test]
    fn a_claim_excludes_a_concurrent_load_of_the_same_identity() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let id = Identifier::from([0x01; 32]);
        let other = Identifier::from([0x02; 32]);

        assert_eq!(ctx.identity_load_phase(&id), None);

        let guard = ctx.begin_identity_load(id).expect("first claim");
        assert_eq!(
            ctx.identity_load_phase(&id),
            Some(IdentityLoadPhase::Running)
        );
        assert!(
            matches!(
                ctx.begin_identity_load(id),
                Err(TaskError::IdentityLoadInProgress { identity_id }) if identity_id == id
            ),
            "a second load of the same identity must be rejected, not raced"
        );

        // A different identity is unrelated — it may load concurrently.
        let other_guard = ctx.begin_identity_load(other).expect("unrelated claim");
        drop(other_guard);
        assert_eq!(
            ctx.identity_load_phase(&other),
            Some(IdentityLoadPhase::Failed)
        );

        // Releasing the claim re-opens the identity for another load.
        drop(guard);
        ctx.begin_identity_load(id)
            .expect("a finished identity can be loaded again");
    }

    /// The phases a load moves through, and what each one licenses. Only a task
    /// that reports `loaded` after its last fallible step records success — a
    /// guard dropped on any error path records `Failed`, whether or not the load
    /// had already persisted the node.
    #[test]
    fn a_load_reports_its_own_terminal_phase() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let id = Identifier::from([0x03; 32]);

        ctx.mark_identity_load_submitted(id);
        let phase = ctx.identity_load_phase(&id).expect("submitted");
        assert_eq!(phase, IdentityLoadPhase::Submitted);
        assert!(
            phase.is_outstanding(),
            "a dispatched load is outstanding before its task claims it"
        );

        let guard = ctx.begin_identity_load(id).expect("claim");
        assert!(
            ctx.identity_load_phase(&id)
                .expect("running")
                .is_outstanding()
        );

        // Failure path: the guard drops without a `loaded` report.
        drop(guard);
        let phase = ctx.identity_load_phase(&id).expect("failed");
        assert_eq!(phase, IdentityLoadPhase::Failed);
        assert!(!phase.is_outstanding(), "a failed load is finished");

        // Success path: the task reports after its last fallible step.
        ctx.mark_identity_load_submitted(id);
        ctx.begin_identity_load(id).expect("re-claim").loaded();
        let phase = ctx.identity_load_phase(&id).expect("loaded");
        assert_eq!(phase, IdentityLoadPhase::Loaded);
        assert!(!phase.is_outstanding(), "a loaded identity is finished");
    }

    /// A guard whose record was superseded by a newer load must not overwrite that
    /// newer load's phase with its own outcome.
    #[test]
    fn a_superseded_guard_does_not_report_over_the_load_that_replaced_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let id = Identifier::from([0x04; 32]);

        let stale = ctx.begin_identity_load(id).expect("claim");
        ctx.mark_identity_load_submitted(id);
        drop(stale);

        assert_eq!(
            ctx.identity_load_phase(&id),
            Some(IdentityLoadPhase::Submitted),
            "the newer load's phase must survive the older guard's drop"
        );
    }
}
