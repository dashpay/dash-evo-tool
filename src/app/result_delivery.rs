//! Screen ownership, finite domain observers and non-destructive cache delivery.
use super::request_routing::{ResultRoute, ScreenOwner};
use super::{AppAction, AppState, BackendTasksExecutionMode};
#[cfg(feature = "testing")]
use crate::backend_task::BackendTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, TaskResult};
use crate::ui::{RootScreenType, Screen, ScreenLike};
use std::collections::BTreeMap;

/// Cache notifications coalesced within one frame, without resetting forms.
#[derive(Default)]
pub(super) struct PendingRefresh(
    pub(super) std::collections::BTreeSet<crate::backend_task::CacheUpdate>,
);

/// Settle recovery operations on retained screens, including hosts hidden by navigation.
pub(crate) fn deliver_legacy_recovery_result<'a>(
    roots: &'a mut BTreeMap<RootScreenType, Screen>,
    stack: impl IntoIterator<Item = &'a mut Screen>,
    task_result: &TaskResult,
) -> bool {
    let context = match task_result {
        TaskResult::Success { context, result }
            if matches!(
                result.as_ref(),
                BackendTaskSuccessResult::LegacyRecoveryCandidates { .. }
                    | BackendTaskSuccessResult::LegacyRecoveryCompleted { .. }
            ) =>
        {
            context
        }
        TaskResult::Error { context, .. } if context.legacy_recovery_identity().is_some() => {
            context
        }
        _ => return false,
    };
    let completed = matches!(task_result, TaskResult::Success { result, .. }
        if matches!(result.as_ref(), BackendTaskSuccessResult::LegacyRecoveryCompleted { .. }));
    for screen in roots.values_mut().chain(stack) {
        if !screen.accepts_legacy_recovery_result(context, completed) {
            continue;
        }
        match task_result {
            TaskResult::Success { result, .. } => {
                screen.display_backend_task_result(context, *result.clone());
            }
            TaskResult::Error { error, .. } => {
                screen.display_backend_task_error(context, error);
            }
            _ => unreachable!(),
        }
    }
    true
}

/// The persistent roots that observe other screens' outcomes
/// ([`ScreenLike::observe_backend_result`]): the Identity Hub, which holds the
/// app-wide identity selection, and Masternodes, which lists identities as
/// nodes and hosts the voting workspace.
const OBSERVER_ROOTS: [RootScreenType; 2] = [
    RootScreenType::RootScreenIdentityHub,
    RootScreenType::RootScreenMasternodes,
];

impl AppState {
    /// The screen in view, as an owner of the tasks it dispatches.
    pub(super) fn visible_owner(&mut self) -> ScreenOwner {
        match self.screen_stack.top_id() {
            Some(id) => ScreenOwner::Stacked(id),
            None => ScreenOwner::Root(self.active_root()),
        }
    }

    /// Whether `owner` is still open: a root screen, or a pushed screen not yet
    /// closed.
    pub(super) fn has_screen(&self, owner: ScreenOwner) -> bool {
        match owner {
            ScreenOwner::Root(root) => self.main_screens.contains_key(&root),
            ScreenOwner::Stacked(id) => self.screen_stack.contains_id(id),
        }
    }

    /// The screen a task result is for under `route`, or `None` when no
    /// screen takes it.
    pub(super) fn result_screen_mut(&mut self, route: ResultRoute) -> Option<&mut Screen> {
        match route {
            ResultRoute::Owner(owner) if self.has_screen(owner) => match owner {
                ScreenOwner::Root(root) => self.main_screens.get_mut(&root),
                ScreenOwner::Stacked(id) => self.screen_stack.by_id_mut(id),
            },
            ResultRoute::Owner(_) | ResultRoute::ClosedOwner | ResultRoute::PreviousNetwork(_) => {
                None
            }
            ResultRoute::Visible => Some(self.visible_screen_mut()),
        }
    }

    /// Hand a successful result to the screen `route` names, if any.
    pub(super) fn deliver_result(
        &mut self,
        route: ResultRoute,
        context: &BackendTaskContext,
        result: BackendTaskSuccessResult,
    ) {
        if let Some(screen) = self.result_screen_mut(route) {
            screen.display_backend_task_result(context, result);
        }
    }

    /// Tell the open screen that dispatched a task that the task failed, for an
    /// error the general error path does not deliver. A screen waiting on its
    /// own task would otherwise wait for good.
    pub(super) fn tell_owner_its_task_failed(
        &mut self,
        route: ResultRoute,
        context: &BackendTaskContext,
        error: &TaskError,
    ) {
        if let ResultRoute::Owner(_) = route
            && let Some(screen) = self.result_screen_mut(route)
        {
            screen.display_backend_task_error(context, error);
        }
    }

    /// Where the result behind `task_result` goes. The sender of a dispatch is
    /// taken off the record: each dispatch reports once.
    pub(super) fn take_result_route(&mut self, task_result: &TaskResult) -> ResultRoute {
        let context = match task_result {
            TaskResult::Success { context, .. } | TaskResult::Error { context, .. } => context,
            TaskResult::CacheUpdated(_) | TaskResult::Repaint => {
                return ResultRoute::Visible;
            }
        };
        let active_network = self.current_app_context().network;
        match self.request_owners.take(context) {
            Some((owner, network)) if network != active_network => {
                ResultRoute::PreviousNetwork(Some(owner))
            }
            Some((owner, _)) if self.has_screen(owner) => ResultRoute::Owner(owner),
            Some(_) => ResultRoute::ClosedOwner,
            // The app's own sweep names its network: a late one is the
            // previous network's, like a late dispatch.
            None => match context.scheduled_vote_sweep_network() {
                Some(network) if network != active_network => ResultRoute::PreviousNetwork(None),
                _ => ResultRoute::Visible,
            },
        }
    }

    /// End the wait of a screen whose task was dispatched on a network that is
    /// no longer active, without handing it that network's result. A root
    /// screen outlives the switch and would otherwise wait for good; stacked
    /// screens are closed by the switch.
    pub(super) fn end_previous_network_request(
        &mut self,
        owner: ScreenOwner,
        context: &BackendTaskContext,
    ) {
        let error = TaskError::NetworkSwitchedBeforeResult;
        let ended = TaskResult::Error {
            context: context.clone(),
            error: TaskError::NetworkSwitchedBeforeResult,
        };
        if deliver_legacy_recovery_result(
            &mut self.main_screens,
            self.screen_stack.iter_mut(),
            &ended,
        ) {
            return;
        }
        if let Some(screen) = self.result_screen_mut(ResultRoute::Owner(owner)) {
            screen.display_backend_task_error(context, &error);
        }
    }

    /// Test seam: record the screen in view as the sender of `context`, as the
    /// frame loop does for a task that screen returns. Lets a kittest stand in
    /// for a dispatch whose task must not run.
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn test_record_visible_dispatch(&mut self, context: &BackendTaskContext) {
        let owner = self.visible_owner();
        let network = self.current_app_context().network;
        self.request_owners.record(context, owner, network);
    }

    /// Test seam: from now on, record each task dispatched with a context
    /// instead of running it. The frame loop still does everything else a
    /// dispatch involves, such as recording the screen that sent it.
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn test_capture_dispatches(&mut self) {
        self.captured_dispatches.get_or_insert_with(Vec::new);
    }

    /// Test seam: the tasks captured since [`Self::test_capture_dispatches`].
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn test_captured_dispatches(&self) -> &[(BackendTask, BackendTaskContext)] {
        self.captured_dispatches.as_deref().unwrap_or_default()
    }

    /// Test seam: how many dispatched tasks still have a recorded sender.
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn test_dispatch_owner_count(&self) -> usize {
        self.request_owners.len()
    }

    /// Dispatch a screen action without running captured backend work.
    #[cfg(feature = "testing")]
    pub fn test_dispatch_screen_action(&mut self, action: AppAction) {
        let owner = self.visible_owner();
        self.dispatch_screen_task(Some(owner), action);
    }

    pub(super) fn dispatch_screen_task(&mut self, owner: Option<ScreenOwner>, action: AppAction) {
        let network = self.current_app_context().network;
        let (tasks, mode) = match action {
            AppAction::BackendTask(task) => {
                let context = BackendTaskContext::for_dispatch_on(&task, network);
                (vec![(task, context)], BackendTasksExecutionMode::Sequential)
            }
            AppAction::BackendTaskWithContext { task, context } => {
                (vec![(task, context)], BackendTasksExecutionMode::Sequential)
            }
            AppAction::BackendTasks(tasks, mode) => (
                tasks
                    .into_iter()
                    .map(|task| {
                        let context = BackendTaskContext::for_dispatch_on(&task, network);
                        (task, context)
                    })
                    .collect(),
                mode,
            ),
            // The screen chose each task's context, so it can tell one
            // dispatch of a task from another.
            AppAction::BackendTasksWithContexts(tasks, mode) => (tasks, mode),
            _ => return,
        };
        if let Some(owner) = owner {
            for (_, context) in &tasks {
                self.request_owners.record(context, owner, network);
            }
        }
        self.execute_requests(tasks, mode);
    }

    pub fn visible_screen_mut(&mut self) -> &mut Screen {
        if self.screen_stack.is_empty() {
            self.active_root_screen_mut()
        } else {
            self.screen_stack
                .last_mut()
                .expect("invariant: screen_stack is non-empty in this branch")
        }
    }

    /// Deliver merged cache changes to interested screens without resetting inputs.
    pub(super) fn run_pending_refresh(&mut self, pending: &mut PendingRefresh) {
        for update in std::mem::take(&mut pending.0) {
            for screen in self
                .main_screens
                .values_mut()
                .chain(self.screen_stack.iter_mut())
            {
                screen.cached_data_changed(update);
            }
        }
    }

    pub(super) fn route_result_to_observers(
        &mut self,
        route: ResultRoute,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) {
        if let ResultRoute::PreviousNetwork(_) = route {
            return;
        }
        let visible = self.visible_owner();
        for root in OBSERVER_ROOTS {
            if !route.delivers_to(ScreenOwner::Root(root), visible)
                && let Some(screen) = self.main_screens.get_mut(&root)
            {
                screen.observe_backend_result(context, result);
            }
        }
    }

    pub(super) fn route_error_to_observers(
        &mut self,
        route: ResultRoute,
        context: &BackendTaskContext,
        error: &TaskError,
    ) {
        if let ResultRoute::PreviousNetwork(_) = route {
            return;
        }
        let visible = self.visible_owner();
        for root in OBSERVER_ROOTS {
            if !route.delivers_to(ScreenOwner::Root(root), visible)
                && let Some(screen) = self.main_screens.get_mut(&root)
            {
                screen.observe_backend_error(context, error);
            }
        }
    }
}

#[cfg(test)]
mod pending_refresh_tests {
    use super::PendingRefresh;
    use crate::backend_task::{CacheDomain, CacheUpdate};
    use dash_sdk::dpp::dashcore::Network;

    /// Screens hear of a cache domain's change once per frame however many
    /// tasks reported it; changes on another network stay apart.
    #[test]
    fn identical_cache_notifications_in_one_frame_collapse_into_one() {
        let mut pending = PendingRefresh::default();
        let identities = CacheUpdate {
            network: Network::Testnet,
            domain: CacheDomain::Identities,
        };
        pending.0.insert(identities);
        pending.0.insert(identities);
        pending.0.insert(CacheUpdate {
            network: Network::Mainnet,
            ..identities
        });

        assert_eq!(pending.0.len(), 2);
        assert_eq!(pending.0.iter().filter(|u| **u == identities).count(), 1);
    }
}
