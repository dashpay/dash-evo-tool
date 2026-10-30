//! A screen's single outstanding backend request.
//!
//! A screen that waits on one task of its own keeps the task's dispatch
//! context here. The context is unique per dispatch, so the screen can tell
//! its own result or error from another task's, even one that runs the same
//! operation.

use crate::app::AppAction;
use crate::backend_task::{BackendTask, BackendTaskContext};
use dash_sdk::dpp::dashcore::Network;

/// One outstanding request and the `subject` it is about, such as the identity
/// a load is for. Screens with nothing to remember use the default `()`.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingRequest<S = ()> {
    slot: Option<(BackendTaskContext, S)>,
}

impl<S> Default for PendingRequest<S> {
    fn default() -> Self {
        Self { slot: None }
    }
}

impl<S> PendingRequest<S> {
    /// Dispatch `task` under a context of its own and wait on it, replacing
    /// any earlier request.
    pub fn dispatch_with(&mut self, task: BackendTask, subject: S) -> AppAction {
        let context = BackendTaskContext::for_dispatch(&task);
        self.track_with(context.clone(), subject);
        AppAction::BackendTaskWithContext { task, context }
    }

    /// As [`dispatch_with`](Self::dispatch_with), binding the request to the
    /// network it is dispatched on.
    pub fn dispatch_on_with(
        &mut self,
        task: BackendTask,
        network: Network,
        subject: S,
    ) -> AppAction {
        let context = BackendTaskContext::for_dispatch_on(&task, network);
        self.track_with(context.clone(), subject);
        AppAction::BackendTaskWithContext { task, context }
    }

    /// Wait on a dispatch whose context the caller built itself.
    pub fn track_with(&mut self, context: BackendTaskContext, subject: S) {
        self.slot = Some((context, subject));
    }

    /// Whether `context` is the request this slot waits on.
    pub fn owns(&self, context: &BackendTaskContext) -> bool {
        self.slot.as_ref().is_some_and(|(own, _)| own == context)
    }

    /// The subject of the request, when `context` is that request.
    pub fn subject_for(&self, context: &BackendTaskContext) -> Option<&S> {
        self.slot
            .as_ref()
            .filter(|(own, _)| own == context)
            .map(|(_, subject)| subject)
    }

    /// End the wait when `context` is this request, returning its subject.
    /// Any other context leaves the request outstanding.
    pub fn settle(&mut self, context: &BackendTaskContext) -> Option<S> {
        if !self.owns(context) {
            return None;
        }
        self.slot.take().map(|(_, subject)| subject)
    }

    /// Stop waiting, whatever the request was. A late result then belongs to
    /// nobody.
    pub fn clear(&mut self) -> Option<S> {
        self.slot.take().map(|(_, subject)| subject)
    }

    /// Whether a request is outstanding.
    pub fn is_pending(&self) -> bool {
        self.slot.is_some()
    }

    /// The context of the outstanding request.
    pub fn context(&self) -> Option<&BackendTaskContext> {
        self.slot.as_ref().map(|(context, _)| context)
    }

    /// The subject of the outstanding request.
    pub fn subject(&self) -> Option<&S> {
        self.slot.as_ref().map(|(_, subject)| subject)
    }
}

/// Several outstanding requests of one kind, each with the `subject` it is
/// about — for actions a screen may fire on more than one row before the
/// first answers. Unlike [`PendingRequest`], a new dispatch never replaces an
/// earlier one, so no request is orphaned.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingRequests<S> {
    slots: Vec<(BackendTaskContext, S)>,
}

impl<S> Default for PendingRequests<S> {
    fn default() -> Self {
        Self { slots: Vec::new() }
    }
}

impl<S> PendingRequests<S> {
    /// Dispatch `task` under a context of its own and wait on it alongside
    /// the requests already outstanding.
    pub fn dispatch_with(&mut self, task: BackendTask, subject: S) -> AppAction {
        let context = BackendTaskContext::for_dispatch(&task);
        self.slots.push((context.clone(), subject));
        AppAction::BackendTaskWithContext { task, context }
    }

    /// End the wait on the request `context` is, returning its subject. Any
    /// other context leaves every request outstanding.
    pub fn settle(&mut self, context: &BackendTaskContext) -> Option<S> {
        let index = self.slots.iter().position(|(own, _)| own == context)?;
        Some(self.slots.remove(index).1)
    }

    /// Whether any request is outstanding.
    pub fn is_pending(&self) -> bool {
        !self.slots.is_empty()
    }

    /// How many requests are outstanding.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether no request is outstanding.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

impl PendingRequest {
    /// Dispatch `task` under a context of its own and wait on it.
    pub fn dispatch(&mut self, task: BackendTask) -> AppAction {
        self.dispatch_with(task, ())
    }

    /// Dispatch `task` bound to `network` and wait on it.
    pub fn dispatch_on(&mut self, task: BackendTask, network: Network) -> AppAction {
        self.dispatch_on_with(task, network, ())
    }

    /// Wait on a dispatch whose context the caller built itself.
    pub fn track(&mut self, context: BackendTaskContext) {
        self.track_with(context, ());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::identity::IdentityTask;

    fn refresh_task() -> BackendTask {
        BackendTask::IdentityTask(IdentityTask::RefreshMyUsernameRequests)
    }

    fn dispatched(action: AppAction) -> (BackendTask, BackendTaskContext) {
        match action {
            AppAction::BackendTaskWithContext { task, context } => (task, context),
            other => panic!("expected a dispatch, got {other:?}"),
        }
    }

    #[test]
    fn a_dispatch_waits_on_the_context_it_sends() {
        let mut pending = PendingRequest::default();
        let (task, context) = dispatched(pending.dispatch(refresh_task()));

        assert_eq!(task, refresh_task());
        assert!(pending.is_pending());
        assert!(pending.owns(&context));
        assert_eq!(pending.context(), Some(&context));
    }

    #[test]
    fn the_same_operation_dispatched_again_is_not_owned() {
        let mut pending = PendingRequest::default();
        let (_, own) = dispatched(pending.dispatch(refresh_task()));
        let foreign = BackendTaskContext::for_dispatch(&refresh_task());

        assert!(!pending.owns(&foreign));
        assert!(!pending.owns(&BackendTaskContext::Other));
        assert_eq!(pending.settle(&foreign), None);
        assert!(
            pending.owns(&own),
            "a foreign result leaves the wait in place"
        );
    }

    #[test]
    fn settling_its_own_result_ends_the_wait_once() {
        let mut pending = PendingRequest::default();
        let (_, own) = dispatched(pending.dispatch(refresh_task()));

        assert_eq!(pending.settle(&own), Some(()));
        assert!(!pending.is_pending());
        assert_eq!(pending.settle(&own), None);
    }

    #[test]
    fn a_new_dispatch_replaces_the_earlier_request() {
        let mut pending = PendingRequest::default();
        let (_, older) = dispatched(pending.dispatch(refresh_task()));
        let (_, newer) = dispatched(pending.dispatch(refresh_task()));

        assert!(!pending.owns(&older));
        assert!(pending.owns(&newer));
    }

    #[test]
    fn clearing_forgets_the_request() {
        let mut pending = PendingRequest::default();
        let (_, own) = dispatched(pending.dispatch(refresh_task()));

        assert_eq!(pending.clear(), Some(()));
        assert!(!pending.owns(&own));
        assert_eq!(pending.clear(), None);
    }

    #[test]
    fn a_network_bound_dispatch_keeps_its_network() {
        let mut pending = PendingRequest::default();
        let task = BackendTask::IdentityTask(IdentityTask::CheckLegacyRecovery {
            identity_id: Default::default(),
        });
        let (_, context) = dispatched(pending.dispatch_on(task, Network::Testnet));

        assert_eq!(context.legacy_recovery_network(), Some(Network::Testnet));
        assert!(pending.owns(&context));
    }

    #[test]
    fn the_subject_is_returned_only_for_its_own_request() {
        let mut pending = PendingRequest::<u8>::default();
        let (_, own) = dispatched(pending.dispatch_with(refresh_task(), 7));
        let foreign = BackendTaskContext::for_dispatch(&refresh_task());

        assert_eq!(pending.subject(), Some(&7));
        assert_eq!(pending.subject_for(&own), Some(&7));
        assert_eq!(pending.subject_for(&foreign), None);
        assert_eq!(pending.settle(&own), Some(7));
    }

    #[test]
    fn several_requests_settle_independently() {
        let mut pending = PendingRequests::<u8>::default();
        let (_, first) = dispatched(pending.dispatch_with(refresh_task(), 1));
        let (_, second) = dispatched(pending.dispatch_with(refresh_task(), 2));
        assert_eq!(pending.len(), 2);
        assert_eq!(
            pending.settle(&BackendTaskContext::for_dispatch(&refresh_task())),
            None
        );
        assert_eq!(
            pending.settle(&first),
            Some(1),
            "the earlier request is kept"
        );
        assert!(pending.is_pending());
        assert_eq!(pending.settle(&first), None);
        assert_eq!(pending.settle(&second), Some(2));
        assert!(pending.is_empty());
    }

    #[test]
    fn a_tracked_context_is_owned_as_built() {
        let mut pending = PendingRequest::default();
        let context = BackendTaskContext::LegacyRecoveryOnNetwork {
            network: Network::Testnet,
            operation: Box::new(BackendTaskContext::for_dispatch(&refresh_task())),
        };
        pending.track(context.clone());

        assert!(pending.owns(&context));
    }
}
