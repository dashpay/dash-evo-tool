//! Delivery of backend-task results to the screen that dispatched them.
//!
//! A screen that waits on a task of its own sends it under a
//! [`BackendTaskContext::Dispatched`] context. [`AppState`](super::AppState)
//! records which screen sent each such dispatch in [`RequestOwners`] and hands
//! its result or error to that screen wherever it is: a hidden root screen, or
//! a screen covered on the stack. Anything else — an undispatched task, an
//! owner that has since been closed, or a task sent before a network switch —
//! goes to the visible screen.
//!
//! Stacked screens have no identity of their own (`ScreenType` equality
//! ignores its payload), so [`ScreenStack`] gives each pushed screen an id
//! that is never reused. Root screens are identified by their
//! [`RootScreenType`]: there is one per type and it lives for the process.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use crate::backend_task::BackendTaskContext;
use crate::ui::{RootScreenType, Screen};

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

/// The pushed screens, topmost last, each with its [`StackedScreenId`].
///
/// Reads and in-place edits go through the slice it dereferences to; the
/// length changes only through its own methods, which keep every screen paired
/// with its id. Reordering the slice would break that pairing; nothing
/// reorders the stack.
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

impl<S> DerefMut for ScreenStack<S> {
    fn deref_mut(&mut self) -> &mut [S] {
        &mut self.screens
    }
}

/// Which screen sent each dispatch whose result has not arrived yet.
///
/// Each dispatch reports exactly once — its result, its error, or the panic
/// forwarded under its context — and that arrival takes the entry, so the map
/// holds only requests still running.
#[derive(Debug, Default)]
pub struct RequestOwners {
    owners: HashMap<u64, ScreenOwner>,
}

impl RequestOwners {
    /// Record `owner` as the sender of `context`. Contexts without a dispatch
    /// id belong to no screen and are not recorded.
    pub fn record(&mut self, context: &BackendTaskContext, owner: ScreenOwner) {
        if let Some(dispatch_id) = context.dispatch_id() {
            self.owners.insert(dispatch_id, owner);
        }
    }

    /// The sender of `context`, forgotten as it is returned: the result of a
    /// dispatch arrives only once.
    pub fn take(&mut self, context: &BackendTaskContext) -> Option<ScreenOwner> {
        self.owners.remove(&context.dispatch_id()?)
    }

    /// Forget every sender.
    pub fn clear(&mut self) {
        self.owners.clear();
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
    use dash_sdk::dpp::dashcore::Network;

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
        owners.record(&context, owner);

        assert_eq!(
            owners.take(&dispatch()),
            None,
            "another dispatch has no owner"
        );
        assert_eq!(owners.take(&context), Some(owner));
        assert_eq!(owners.take(&context), None);
        assert!(owners.is_empty());
    }

    #[test]
    fn undispatched_contexts_have_no_owner() {
        let mut owners = RequestOwners::default();
        let owner = ScreenOwner::Root(RootScreenType::RootScreenIdentityHub);
        owners.record(&BackendTaskContext::Other, owner);
        owners.record(&BackendTaskContext::TokenBalanceRefresh, owner);

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
        owners.record(&context, owner);

        assert_eq!(owners.len(), 1);
        assert_eq!(owners.take(&context), Some(owner));
    }

    #[test]
    fn clearing_forgets_every_owner() {
        let mut owners = RequestOwners::default();
        owners.record(
            &dispatch(),
            ScreenOwner::Root(RootScreenType::RootScreenIdentityHub),
        );
        owners.clear();

        assert!(owners.is_empty());
    }
}
