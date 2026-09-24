//! Lifecycle registry for identity loads.
//!
//! A dispatched load is invisible to the screen that started it: task results
//! reach only the *visible* screen, and a load has no cooperative cancellation.
//! A screen that navigates away therefore cannot learn from a callback whether
//! its load is still running, succeeded, or failed — and neither proxy for that
//! answer holds up:
//!
//! - **"the task holds a claim"** does not mean "running": a load is outstanding
//!   from the moment it is dispatched, and only claims its identity once its task
//!   starts.
//! - **"the node is in the store"** does not mean "succeeded": `load_identity`
//!   inserts the node before sealing its keys, so a failed seal leaves the node
//!   persisted by a load that errored.
//!
//! So each load reports its own phase here, and the registry is the single source
//! of truth for it. Loads are dispatched from several places — the Masternodes
//! form, Add Existing, the detail screen, MCP tools — so a record is stamped with
//! a [`IdentityLoadToken`] identifying the one load it belongs to, and every write
//! checks that stamp first. Without it a later submission could erase a running
//! load's record, and the two loads would then race each other's storage writes,
//! or publish each other's outcome.
//!
//! The records also give a load exclusive use of its identity for its whole
//! check → fetch → insert → seal span, which is what makes
//! `IdentityLoadMode::RejectIfExists` safe: those steps are not one atomic
//! operation, so two overlapping loads of one identity would otherwise both pass
//! the duplicate check and clobber each other's insert.
//!
//! A record outlives its load and is replaced by the next load of that identity —
//! the map holds at most one entry per identity loaded this session.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dash_sdk::platform::Identifier;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;

/// How far a dispatched identity load has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityLoadPhase {
    /// Dispatched. Its task has not started, or has not claimed the identity yet.
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

/// Identifies one load of one identity. Every registry write is checked against
/// it, so a load can only ever report its own phase — never overwrite the record
/// of a load that superseded it, and never be overwritten by a later submission
/// while it is still outstanding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityLoadToken(u64);

/// The current load of one identity.
#[derive(Debug, Clone, Copy)]
struct LoadRecord {
    token: IdentityLoadToken,
    phase: IdentityLoadPhase,
}

/// The most recent load of each identity, and the source of load tokens.
#[derive(Debug, Default)]
pub(crate) struct LoadRegistry {
    records: HashMap<Identifier, LoadRecord>,
    next_token: u64,
}

impl LoadRegistry {
    fn mint(&mut self) -> IdentityLoadToken {
        self.next_token = self.next_token.wrapping_add(1);
        IdentityLoadToken(self.next_token)
    }
}

pub(crate) type SharedLoadRegistry = Arc<Mutex<LoadRegistry>>;

/// A load's exclusive claim on one identity. Records the load's terminal phase
/// when dropped: [`Loaded`](IdentityLoadPhase::Loaded) if the task called
/// [`loaded`](Self::loaded) after its last fallible step, otherwise
/// [`Failed`](IdentityLoadPhase::Failed) — so every `?`, and a panic, still leaves
/// a truthful record.
#[derive(Debug)]
pub struct IdentityLoadGuard {
    registry: SharedLoadRegistry,
    identity_id: Identifier,
    token: IdentityLoadToken,
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
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        // Report only on this load's own record. A record carrying another token
        // belongs to a load that superseded this one, and its outcome is not this
        // guard's to publish.
        let Some(record) = registry.records.get_mut(&self.identity_id) else {
            return;
        };
        if record.token != self.token {
            return;
        }
        record.phase = if self.loaded {
            IdentityLoadPhase::Loaded
        } else {
            IdentityLoadPhase::Failed
        };
    }
}

/// Backstop for one dispatched load, held for as long as its task runs. Records
/// [`Failed`](IdentityLoadPhase::Failed) when dropped over a record still
/// [`Submitted`](IdentityLoadPhase::Submitted) under this load's token — the task
/// returned without ever claiming the identity, so no [`IdentityLoadGuard`] was
/// ever taken, and nothing else would close that record out.
///
/// A task can end before its claim in ways the load itself never sees: a gate in
/// `run_backend_task` short-circuits it, the runtime drops it, it panics. Without
/// this the load would stay outstanding for the rest of the session, and the
/// screen gating on it stuck on "Loading…".
///
/// A record that has moved past `Submitted` belongs to the load's own guard, which
/// reports the outcome it actually reached — the backstop leaves it alone.
#[derive(Debug)]
pub(crate) struct IdentityLoadDispatch {
    registry: SharedLoadRegistry,
    identity_id: Identifier,
    token: IdentityLoadToken,
}

impl Drop for IdentityLoadDispatch {
    fn drop(&mut self) {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let Some(record) = registry.records.get_mut(&self.identity_id) else {
            return;
        };
        if record.token != self.token || record.phase != IdentityLoadPhase::Submitted {
            return;
        }
        tracing::debug!(
            identity_id = %self.identity_id,
            "Identity load ended before it claimed its identity — recording it failed"
        );
        record.phase = IdentityLoadPhase::Failed;
    }
}

impl AppContext {
    /// Record that a load of `identity_id` has been dispatched, before its task
    /// runs, and return the token identifying it. Callers that gate on a load must
    /// mark it here: until its task claims the identity, nothing else records that
    /// the load is outstanding.
    ///
    /// Returns `None` when a load of this identity is already outstanding — that
    /// load owns the record, and its task will reject the new one. Never disturbs
    /// it.
    pub fn mark_identity_load_submitted(
        &self,
        identity_id: Identifier,
    ) -> Option<IdentityLoadToken> {
        let mut registry = self
            .identity_loads
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        if registry
            .records
            .get(&identity_id)
            .is_some_and(|record| record.phase.is_outstanding())
        {
            return None;
        }
        let token = registry.mint();
        registry.records.insert(
            identity_id,
            LoadRecord {
                token,
                phase: IdentityLoadPhase::Submitted,
            },
        );
        Some(token)
    }

    /// Claim `identity_id` for the load `token` names, excluding any concurrent
    /// load of the same identity until the returned guard drops. Adopts the record
    /// of its own submission — the one stamped with `token` — so the screen waiting
    /// on that token follows this task through to its outcome. A load dispatched
    /// without a prior submission (Add Existing, the detail screen, an MCP tool)
    /// passes `None` and opens a record of its own.
    ///
    /// # Errors
    ///
    /// Returns [`TaskError::IdentityLoadInProgress`] when another load of this
    /// identity is outstanding — running, or submitted and not yet started.
    /// Adopting a record this load was not dispatched under would publish its
    /// outcome under the other load's token, and race that load's storage writes.
    pub fn begin_identity_load(
        &self,
        identity_id: Identifier,
        token: Option<IdentityLoadToken>,
    ) -> Result<IdentityLoadGuard, TaskError> {
        let mut registry = self
            .identity_loads
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let token = match registry.records.get(&identity_id) {
            Some(record)
                if record.phase == IdentityLoadPhase::Submitted && Some(record.token) == token =>
            {
                record.token
            }
            Some(record) if record.phase.is_outstanding() => {
                return Err(TaskError::IdentityLoadInProgress { identity_id });
            }
            _ => registry.mint(),
        };
        registry.records.insert(
            identity_id,
            LoadRecord {
                token,
                phase: IdentityLoadPhase::Running,
            },
        );
        drop(registry);

        Ok(IdentityLoadGuard {
            registry: self.identity_loads.clone(),
            identity_id,
            token,
            loaded: false,
        })
    }

    /// Backstop the dispatch of the load `token` names: a task that returns
    /// without ever claiming `identity_id` still leaves a terminal record. See
    /// [`IdentityLoadDispatch`].
    pub(crate) fn backstop_identity_load(
        &self,
        identity_id: Identifier,
        token: IdentityLoadToken,
    ) -> IdentityLoadDispatch {
        IdentityLoadDispatch {
            registry: self.identity_loads.clone(),
            identity_id,
            token,
        }
    }

    /// How far the load identified by `token` got, or `None` when the record was
    /// replaced by a newer load of the same identity — in which case the load
    /// `token` names is over and its outcome is no longer observable. Cheap enough
    /// to call from the frame loop.
    pub fn identity_load_phase(
        &self,
        identity_id: &Identifier,
        token: IdentityLoadToken,
    ) -> Option<IdentityLoadPhase> {
        self.identity_loads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .records
            .get(identity_id)
            .filter(|record| record.token == token)
            .map(|record| record.phase)
    }

    /// The phase last recorded for `identity_id`, whichever load left it.
    /// Test-only: a task that mints its own token internally publishes an
    /// outcome no caller could otherwise read back.
    #[cfg(test)]
    pub(crate) fn last_identity_load_phase(
        &self,
        identity_id: &Identifier,
    ) -> Option<IdentityLoadPhase> {
        self.identity_loads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .records
            .get(identity_id)
            .map(|record| record.phase)
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

        let token = ctx.mark_identity_load_submitted(id).expect("submitted");
        assert_eq!(
            ctx.identity_load_phase(&id, token),
            Some(IdentityLoadPhase::Submitted)
        );

        let guard = ctx
            .begin_identity_load(id, Some(token))
            .expect("the submitted load's own claim");
        assert_eq!(
            ctx.identity_load_phase(&id, token),
            Some(IdentityLoadPhase::Running),
            "a load claiming its own submission keeps its token, so its screen still follows it"
        );
        assert!(
            matches!(
                ctx.begin_identity_load(id, Some(token)),
                Err(TaskError::IdentityLoadInProgress { identity_id }) if identity_id == id
            ),
            "a second load of the same identity must be rejected, not raced"
        );

        // A different identity is unrelated — it may load concurrently.
        let other_guard = ctx
            .begin_identity_load(other, None)
            .expect("unrelated claim");
        drop(other_guard);

        // Releasing the claim re-opens the identity for another load.
        drop(guard);
        assert_eq!(
            ctx.identity_load_phase(&id, token),
            Some(IdentityLoadPhase::Failed)
        );
        ctx.begin_identity_load(id, None)
            .expect("a finished identity can be loaded again");
    }

    /// Regression: a load must never adopt a record it was not dispatched under.
    /// Only the Masternodes form marks its load `Submitted`; the other entry points
    /// (Add Existing, the detail screen, an MCP tool) dispatch straight to the task.
    /// A load that adopted a stranger's `Submitted` record would publish ITS outcome
    /// under that token — the form would close reporting success while the keys and
    /// password the user actually submitted were discarded with the load that never
    /// ran.
    #[test]
    fn a_foreign_load_does_not_inherit_a_submitted_loads_token() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let id = Identifier::from([0x06; 32]);

        let token = ctx.mark_identity_load_submitted(id).expect("submitted");

        // A load of the same identity dispatched from anywhere else, while the
        // submitted one has yet to start.
        assert!(
            matches!(
                ctx.begin_identity_load(id, None),
                Err(TaskError::IdentityLoadInProgress { identity_id }) if identity_id == id
            ),
            "a foreign load must be rejected while a submitted load is outstanding, not adopt it"
        );
        assert_eq!(
            ctx.identity_load_phase(&id, token),
            Some(IdentityLoadPhase::Submitted),
            "the rejected load must leave the submitter's record untouched"
        );

        // A load carrying some *other* load's token is just as foreign.
        let elsewhere = ctx
            .mark_identity_load_submitted(Identifier::from([0x07; 32]))
            .expect("submitted");
        assert!(
            matches!(
                ctx.begin_identity_load(id, Some(elsewhere)),
                Err(TaskError::IdentityLoadInProgress { .. })
            ),
            "a token minted for another identity's load claims nothing here"
        );

        // The submitted load still owns its record, and still reports its own
        // outcome into it.
        ctx.begin_identity_load(id, Some(token))
            .expect("the submitted load's own claim")
            .loaded();
        assert_eq!(
            ctx.identity_load_phase(&id, token),
            Some(IdentityLoadPhase::Loaded)
        );
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

        let token = ctx.mark_identity_load_submitted(id).expect("submitted");
        let phase = ctx.identity_load_phase(&id, token).expect("submitted");
        assert!(
            phase.is_outstanding(),
            "a dispatched load is outstanding before its task claims it"
        );

        let guard = ctx.begin_identity_load(id, Some(token)).expect("claim");
        assert!(
            ctx.identity_load_phase(&id, token)
                .expect("running")
                .is_outstanding()
        );

        // Failure path: the guard drops without a `loaded` report.
        drop(guard);
        let phase = ctx.identity_load_phase(&id, token).expect("failed");
        assert_eq!(phase, IdentityLoadPhase::Failed);
        assert!(!phase.is_outstanding(), "a failed load is finished");

        // Success path: the task reports after its last fallible step.
        let token = ctx.mark_identity_load_submitted(id).expect("resubmitted");
        ctx.begin_identity_load(id, Some(token))
            .expect("re-claim")
            .loaded();
        let phase = ctx.identity_load_phase(&id, token).expect("loaded");
        assert_eq!(phase, IdentityLoadPhase::Loaded);
        assert!(!phase.is_outstanding(), "a loaded identity is finished");
    }

    /// Regression: submitting must never erase an active claim. Another caller —
    /// Add Existing, the detail screen, an MCP tool — may already be loading this
    /// identity; overwriting its `Running` record would let a second task claim
    /// the same identity and race the first one's storage writes.
    #[test]
    fn submitting_does_not_erase_an_active_claim() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let id = Identifier::from([0x05; 32]);

        let running = ctx
            .begin_identity_load(id, None)
            .expect("another caller's claim");
        assert!(
            ctx.mark_identity_load_submitted(id).is_none(),
            "a load of this identity is already outstanding: the submission gets no token"
        );
        assert!(
            matches!(
                ctx.begin_identity_load(id, None),
                Err(TaskError::IdentityLoadInProgress { .. })
            ),
            "a submission must not erase a running claim: a second guard would race the first"
        );

        // The running load still owns its record and reports its own outcome.
        running.loaded();
        let token = ctx.mark_identity_load_submitted(id).expect("now free");
        assert_eq!(
            ctx.identity_load_phase(&id, token),
            Some(IdentityLoadPhase::Submitted)
        );
    }

    /// A guard whose record was superseded by a newer load must not publish its
    /// outcome over that newer load's phase, and the superseded load's own token
    /// must stop resolving — its outcome is no longer observable.
    #[test]
    fn a_superseded_guard_does_not_report_over_the_load_that_replaced_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let id = Identifier::from([0x04; 32]);

        let stale_token = ctx.mark_identity_load_submitted(id).expect("submitted");
        let stale = ctx
            .begin_identity_load(id, Some(stale_token))
            .expect("claim");
        // The stale load finishes, and a newer load of the same identity starts.
        drop(stale);
        let fresh_token = ctx.mark_identity_load_submitted(id).expect("resubmitted");
        let fresh = ctx
            .begin_identity_load(id, Some(fresh_token))
            .expect("re-claim");

        assert_eq!(
            ctx.identity_load_phase(&id, stale_token),
            None,
            "a superseded load's outcome is no longer observable"
        );
        assert_eq!(
            ctx.identity_load_phase(&id, fresh_token),
            Some(IdentityLoadPhase::Running),
            "the newer load owns the record"
        );
        drop(fresh);
    }

    /// The backstop closes out a load whose task ended before it ever claimed the
    /// identity, and keeps its hands off one that did claim: that record belongs to
    /// the load's own guard, which reports the outcome it actually reached.
    #[test]
    fn the_backstop_only_closes_out_a_load_that_never_claimed_its_identity() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(tmp.path());
        let id = Identifier::from([0x08; 32]);

        let token = ctx.mark_identity_load_submitted(id).expect("submitted");
        drop(ctx.backstop_identity_load(id, token));
        assert_eq!(
            ctx.identity_load_phase(&id, token),
            Some(IdentityLoadPhase::Failed),
            "a load that ended before its claim must not stay outstanding forever"
        );

        let token = ctx.mark_identity_load_submitted(id).expect("resubmitted");
        let dispatch = ctx.backstop_identity_load(id, token);
        ctx.begin_identity_load(id, Some(token))
            .expect("claim")
            .loaded();
        drop(dispatch);
        assert_eq!(
            ctx.identity_load_phase(&id, token),
            Some(IdentityLoadPhase::Loaded),
            "the backstop must not overwrite the outcome the load itself reported"
        );
    }
}
