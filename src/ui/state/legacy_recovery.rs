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

    /// Record that a restore finished. Re-arms detection so the offer
    /// recomputes from the store and disappears once nothing is left stranded.
    pub fn completed(&mut self) {
        self.state = FetchState::NotRequested;
    }

    /// Record that an in-flight operation failed.
    ///
    /// A failed restore returns to its offer, so the user can fix what went
    /// wrong — a mistyped identity password is the common case — and press
    /// Restore again. A failed detection is terminal for this screen: retrying
    /// it automatically would re-read the same unreadable row every frame.
    ///
    /// Attribution is coarse: an error from an unrelated task that arrives
    /// while a check is outstanding also lands here. That only hides an offer
    /// until the identity is opened again, and never re-dispatches anything.
    pub fn failed(&mut self) {
        self.state = match std::mem::replace(&mut self.state, FetchState::Failed) {
            FetchState::Restoring(plan) => FetchState::Offered(plan),
            FetchState::Checking => FetchState::Failed,
            other => other,
        };
    }
}
