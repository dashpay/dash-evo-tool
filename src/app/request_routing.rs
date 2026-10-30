//! Delivery of backend-task results to the screen that dispatched them.
//!
//! A screen that waits on a task of its own sends it under a
//! [`BackendTaskContext::Dispatched`] context. [`AppState`](super::AppState)
//! records which screen sent each such dispatch in [`RequestOwners`] and hands
//! its result or error to that screen wherever it is: a hidden root screen, or
//! a screen covered on the stack. When that screen has been closed, no screen
//! receives the result ([`ResultRoute::ClosedOwner`]): it is not the visible
//! screen's to act on, so banners report it instead. An undispatched task goes
//! to the visible screen ([`ResultRoute::Visible`]).
//!
//! Each dispatch is also recorded with the network it was sent on. Its result
//! reaches screens only while that network is active: one arriving after a
//! switch to another network ([`ResultRoute::PreviousNetwork`]) reaches no
//! screen and no observer, and banners report it as for a closed sender. A
//! switch back before it arrives delivers it as usual. Results without a
//! dispatch id that name their network are held to the same rule: the app's
//! own scheduled-vote sweeps and background refreshes, and the wallet
//! backend's chain-sync events (a received payment, a balance push, Platform
//! becoming reachable), in their context
//! ([`BackendTaskContext::undispatched_network`]); a contest refresh's mid-task
//! `RefreshedDpnsContests`, in the result itself. A wallet event's data still
//! applies to the wallets of the network it came from; only screens and
//! banners are kept to the active network.
//!
//! A report a request sends while still running — progress, a partial result,
//! one step's failure — arrives under a [`BackendTaskContext::Interim`]
//! context naming the request and its network. It goes where the request's
//! result will go, but ends nothing: the request's sender stays on record
//! ([`RequestOwners::peek`]), no screen's wait for the request ends on it, and
//! a previous network's report reaches no screen and raises no banner.
//!
//! Known limitation: undispatched app tasks whose context names no network
//! still go to the screen in view after a switch.
//!
//! Stacked screens have no identity of their own (`ScreenType` equality
//! ignores its payload), so [`ScreenStack`] gives each pushed screen an id
//! that is never reused. Root screens are identified by their
//! [`RootScreenType`]: there is one per type and it lives for the process.

use std::collections::HashMap;
use std::ops::Deref;

use crate::backend_task::BackendTaskContext;
use crate::ui::{RootScreenType, Screen};
use dash_sdk::dpp::dashcore::Network;

/// The identity of one screen pushed onto the [`ScreenStack`]. Never reused,
/// so a closed screen's id cannot name a screen opened later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StackedScreenId(u64);

/// The screen that dispatched a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScreenOwner {
    Root(RootScreenType),
    Stacked(StackedScreenId),
}

/// Where a task result goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultRoute {
    /// To the open screen that dispatched the task, in view or not.
    Owner(ScreenOwner),
    /// The screen that dispatched the task has been closed. The result is
    /// not another screen's to act on, so no screen takes it; banners report
    /// it instead.
    ClosedOwner,
    /// The task ran on a network that is no longer active. No screen of the
    /// current network takes it, and banners report it as for a closed
    /// sender. Holds the screen that dispatched it, or `None` for a task the
    /// app started itself.
    PreviousNetwork(Option<ScreenOwner>),
    /// No sender is on record — an undispatched task — so the screen in view
    /// takes it.
    Visible,
}

impl ResultRoute {
    /// Whether primary delivery reaches this screen. A closed owner never does.
    pub fn delivers_to(self, screen: ScreenOwner, visible: ScreenOwner) -> bool {
        match self {
            Self::Owner(owner) => owner == screen,
            Self::Visible => screen == visible,
            Self::ClosedOwner | Self::PreviousNetwork(_) => false,
        }
    }
    /// The open screen that dispatched the task.
    pub fn owner(self) -> Option<ScreenOwner> {
        match self {
            Self::Owner(owner) => Some(owner),
            Self::ClosedOwner | Self::PreviousNetwork(_) | Self::Visible => None,
        }
    }
    /// Whether the screen that sent the task can no longer take its result:
    /// it was closed, or the network it ran on was switched away from.
    pub fn sender_gone(self) -> bool {
        matches!(self, Self::ClosedOwner | Self::PreviousNetwork(_))
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    #[test]
    fn a_closed_owner_never_suppresses_the_visible_domain_observer() {
        let root = ScreenOwner::Root(RootScreenType::RootScreenIdentityHub);
        assert!(ResultRoute::Visible.delivers_to(root, root));
        assert!(ResultRoute::Owner(root).delivers_to(root, root));
        assert!(!ResultRoute::ClosedOwner.delivers_to(root, root));
        assert!(!ResultRoute::PreviousNetwork(Some(root)).delivers_to(root, root));
        assert!(ResultRoute::PreviousNetwork(Some(root)).sender_gone());
        assert!(!ResultRoute::PreviousNetwork(None).delivers_to(root, root));
        assert_eq!(ResultRoute::PreviousNetwork(None).owner(), None);
    }
}

/// The pushed screens, topmost last, each with its [`StackedScreenId`].
///
/// Reads go through the slice it dereferences to. Changes go through its own
/// methods only, which keep every screen paired with its id.
#[derive(Debug)]
pub struct ScreenStack<S = Screen> {
    screens: Vec<S>,
    ids: Vec<StackedScreenId>,
    next_id: u64,
}

impl<S> Default for ScreenStack<S> {
    fn default() -> Self {
        Self {
            screens: Vec::new(),
            ids: Vec::new(),
            next_id: 0,
        }
    }
}

impl<S> ScreenStack<S> {
    /// Push `screen` on top and return its id.
    pub fn push(&mut self, screen: S) -> StackedScreenId {
        let id = StackedScreenId(self.next_id);
        self.next_id += 1;
        self.screens.push(screen);
        self.ids.push(id);
        id
    }

    /// Remove the topmost screen.
    pub fn pop(&mut self) -> Option<S> {
        self.ids.pop();
        self.screens.pop()
    }

    /// Remove every screen.
    pub fn clear(&mut self) {
        self.ids.clear();
        self.screens.clear();
    }

    /// Replace every screen with `screen` alone.
    pub fn replace_with(&mut self, screen: S) -> StackedScreenId {
        self.clear();
        self.push(screen)
    }

    /// The id of the topmost screen.
    pub fn top_id(&self) -> Option<StackedScreenId> {
        self.ids.last().copied()
    }

    /// The topmost screen.
    pub fn last_mut(&mut self) -> Option<&mut S> {
        self.screens.last_mut()
    }

    /// Every screen, bottom first, for changes in place.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, S> {
        self.screens.iter_mut()
    }

    /// Whether the screen with `id` is still on the stack.
    pub fn contains_id(&self, id: StackedScreenId) -> bool {
        self.ids.contains(&id)
    }

    /// The screen with `id`, if it is still on the stack.
    pub fn by_id_mut(&mut self, id: StackedScreenId) -> Option<&mut S> {
        let position = self.ids.iter().position(|candidate| *candidate == id)?;
        self.screens.get_mut(position)
    }
}

impl<S> Deref for ScreenStack<S> {
    type Target = [S];

    fn deref(&self) -> &[S] {
        &self.screens
    }
}

/// Which screen sent each dispatch whose result has not arrived yet.
///
/// Each dispatch reports exactly once — its result, its error, or the panic
/// forwarded under its context — and that arrival takes the entry, so the map
/// holds only requests still running.
#[derive(Debug, Default)]
pub struct RequestOwners {
    owners: HashMap<u64, (ScreenOwner, Network)>,
}

impl RequestOwners {
    /// Record `owner` as the sender of `context`, dispatched on `network`.
    /// Contexts without a dispatch id belong to no screen and are not
    /// recorded.
    pub fn record(&mut self, context: &BackendTaskContext, owner: ScreenOwner, network: Network) {
        if let Some(dispatch_id) = context.dispatch_id() {
            self.owners.insert(dispatch_id, (owner, network));
        }
    }

    /// The sender of `context` and the network it was dispatched on,
    /// forgotten as they are returned: the result of a dispatch arrives only
    /// once. Entries outlive a network switch, so a late result is still told
    /// apart from an undispatched one.
    pub fn take(&mut self, context: &BackendTaskContext) -> Option<(ScreenOwner, Network)> {
        self.owners.remove(&context.dispatch_id()?)
    }

    /// The sender of `context` and the network it was dispatched on, left on
    /// record: a report the request sends while still running is not its
    /// arrival.
    pub fn peek(&self, context: &BackendTaskContext) -> Option<(ScreenOwner, Network)> {
        self.owners.get(&context.dispatch_id()?).copied()
    }

    /// How many dispatches still wait for their result.
    pub fn len(&self) -> usize {
        self.owners.len()
    }

    /// Whether no dispatch waits for its result.
    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::BackendTask;
    use crate::backend_task::identity::IdentityTask;

    fn dispatch() -> BackendTaskContext {
        BackendTaskContext::for_dispatch(&BackendTask::IdentityTask(
            IdentityTask::RefreshMyUsernameRequests,
        ))
    }

    #[test]
    fn pushed_screens_get_distinct_ids() {
        let mut stack = ScreenStack::default();
        let first = stack.push("first");
        let second = stack.push("second");

        assert_ne!(first, second);
        assert_eq!(stack.top_id(), Some(second));
        assert_eq!(stack.by_id_mut(first), Some(&mut "first"));
        assert_eq!(stack.by_id_mut(second), Some(&mut "second"));
        assert_eq!(&*stack, &["first", "second"]);
    }

    #[test]
    fn a_closed_screen_id_is_never_reused() {
        let mut stack = ScreenStack::default();
        let closed = stack.push("closed");
        assert_eq!(stack.pop(), Some("closed"));
        let reopened = stack.push("closed");

        assert_ne!(closed, reopened);
        assert_eq!(stack.by_id_mut(closed), None);
        assert!(!stack.contains_id(closed));
        assert!(stack.contains_id(reopened));

        let replaced = stack.replace_with("replacement");
        assert_eq!(stack.by_id_mut(reopened), None);
        assert_eq!(stack.by_id_mut(replaced), Some(&mut "replacement"));

        stack.clear();
        assert!(stack.is_empty());
        assert_eq!(stack.top_id(), None);
        assert_eq!(stack.by_id_mut(replaced), None);
    }

    #[test]
    fn a_covered_screen_keeps_its_id() {
        let mut stack = ScreenStack::default();
        let covered = stack.push("covered");
        stack.push("cover");
        stack.pop();
        stack.push("another cover");

        assert_eq!(stack.by_id_mut(covered), Some(&mut "covered"));
        if let Some(screen) = stack.last_mut() {
            *screen = "edited in place";
        }
        assert_eq!(stack.last(), Some(&"edited in place"));
    }

    #[test]
    fn the_owner_of_a_dispatch_is_taken_once() {
        let mut owners = RequestOwners::default();
        let context = dispatch();
        let owner = ScreenOwner::Root(RootScreenType::RootScreenIdentityHub);
        owners.record(&context, owner, Network::Testnet);

        assert_eq!(
            owners.take(&dispatch()),
            None,
            "another dispatch has no owner"
        );
        assert_eq!(owners.take(&context), Some((owner, Network::Testnet)));
        assert_eq!(owners.take(&context), None);
        assert!(owners.is_empty());
    }

    /// A report a request sends while running finds the request's sender
    /// and leaves it on record for the request's own result.
    #[test]
    fn a_report_of_a_running_request_leaves_its_owner_on_record() {
        let mut owners = RequestOwners::default();
        let request = dispatch();
        let owner = ScreenOwner::Root(RootScreenType::RootScreenIdentityHub);
        owners.record(&request, owner, Network::Testnet);
        let report = BackendTaskContext::interim(Network::Testnet, request.clone());

        assert_eq!(owners.take(&report), None, "a report takes no sender");
        assert_eq!(owners.peek(&request), Some((owner, Network::Testnet)));
        assert_eq!(owners.len(), 1);
        assert_eq!(owners.take(&request), Some((owner, Network::Testnet)));
        assert_eq!(owners.peek(&request), None);
    }

    #[test]
    fn undispatched_contexts_have_no_owner() {
        let mut owners = RequestOwners::default();
        let owner = ScreenOwner::Root(RootScreenType::RootScreenIdentityHub);
        owners.record(&BackendTaskContext::Other, owner, Network::Testnet);
        owners.record(
            &BackendTaskContext::TokenBalanceRefresh,
            owner,
            Network::Testnet,
        );

        assert!(owners.is_empty());
        assert_eq!(owners.take(&BackendTaskContext::Other), None);
    }

    #[test]
    fn a_network_bound_recovery_dispatch_keeps_its_owner() {
        let mut owners = RequestOwners::default();
        let mut stack = ScreenStack::default();
        let owner = ScreenOwner::Stacked(stack.push("recovery host"));
        let context = BackendTaskContext::LegacyRecoveryOnNetwork {
            network: Network::Testnet,
            operation: Box::new(dispatch()),
        };
        owners.record(&context, owner, Network::Testnet);

        assert_eq!(owners.len(), 1);
        assert_eq!(owners.take(&context), Some((owner, Network::Testnet)));
    }

    #[test]
    fn an_owner_keeps_the_network_its_dispatch_was_sent_on() {
        let mut owners = RequestOwners::default();
        let owner = ScreenOwner::Root(RootScreenType::RootScreenDocumentQuery);
        let on_testnet = dispatch();
        let on_mainnet = dispatch();
        owners.record(&on_testnet, owner, Network::Testnet);
        owners.record(&on_mainnet, owner, Network::Mainnet);

        assert_eq!(owners.take(&on_mainnet), Some((owner, Network::Mainnet)));
        assert_eq!(owners.take(&on_testnet), Some((owner, Network::Testnet)));
        assert!(owners.is_empty());
    }
}
