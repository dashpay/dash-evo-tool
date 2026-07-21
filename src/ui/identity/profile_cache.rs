//! Best-effort, async-populated DashPay profile cache for the Identities hub.
//!
//! The local SQLite DashPay-profile cache was removed in the platform-wallet
//! migration; profiles now live in the upstream `DashpayView` and are only
//! reachable through the async [`DashPayTask::LoadProfile`] task. Hub tabs
//! render synchronously, so they read this cache (empty until the first load
//! completes) and queue a load on a miss. The hub dispatches the queued load
//! after rendering and feeds the result back in via [`ProfileCache::record_result`].

use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use std::collections::{HashMap, HashSet};

/// Loaded DashPay profile fields for one identity. Mirrors the
/// `BackendTaskSuccessResult::DashPayProfile` tuple; empty strings mean unset.
#[derive(Debug, Clone, Default)]
pub struct ProfileFields {
    pub display_name: String,
    pub bio: String,
    pub avatar_url: String,
}

impl ProfileFields {
    /// Display name when set to a non-blank value.
    pub fn display_name_opt(&self) -> Option<&str> {
        let trimmed = self.display_name.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    }
}

/// Per-identity profile cache with a single in-flight async load.
#[derive(Debug, Default)]
pub struct ProfileCache {
    /// Loaded state per identity: `Some(fields)` = a profile exists,
    /// `None` = loaded but no published profile. Absent key = not loaded yet.
    loaded: HashMap<Identifier, Option<ProfileFields>>,
    /// Identities a load has already been dispatched for (debounce).
    requested: HashSet<Identifier>,
    /// Identity of the in-flight load. The result variant carries no owner id,
    /// so it is associated with this id on arrival.
    in_flight: Option<Identifier>,
    /// Identities a tab asked for this frame that still need a load dispatched.
    wanted: Vec<QualifiedIdentity>,
}

impl ProfileCache {
    /// Loaded profile state for `identity`, queuing a load on a miss.
    ///
    /// `Some(Some(_))` = profile present, `Some(None)` = loaded with none
    /// published, `None` = not loaded yet (a load is queued for dispatch).
    pub fn get_or_request(
        &mut self,
        identity: &QualifiedIdentity,
    ) -> Option<&Option<ProfileFields>> {
        let id = identity.identity.id();
        let known = self.loaded.contains_key(&id)
            || self.requested.contains(&id)
            || self.wanted.iter().any(|q| q.identity.id() == id);
        if !known {
            self.wanted.push(identity.clone());
        }
        self.loaded.get(&id)
    }

    /// Dispatch one queued profile load, if any and none is in flight. Call
    /// after rendering the tabs; fold the returned action into the frame's.
    pub fn dispatch_pending(&mut self) -> AppAction {
        if self.in_flight.is_some() {
            return AppAction::None;
        }
        let Some(identity) = self.wanted.pop() else {
            return AppAction::None;
        };
        let id = identity.identity.id();
        self.requested.insert(id);
        self.in_flight = Some(id);
        AppAction::BackendTask(BackendTask::DashPayTask(Box::new(
            DashPayTask::LoadProfile { identity },
        )))
    }

    /// Record a `LoadProfile` result against the in-flight identity. Returns
    /// `true` when the result was consumed (a load was in flight).
    pub fn record_result(&mut self, result: &BackendTaskSuccessResult) -> bool {
        let BackendTaskSuccessResult::DashPayProfile(data) = result else {
            return false;
        };
        let Some(id) = self.in_flight.take() else {
            return false;
        };
        let fields = data
            .clone()
            .map(|(display_name, bio, avatar_url)| ProfileFields {
                display_name,
                bio,
                avatar_url,
            });
        self.loaded.insert(id, fields);
        true
    }

    /// Optimistically record a just-saved profile so every tab reflects it
    /// immediately, without waiting for a re-fetch.
    ///
    /// `record_result` only consumes `LoadProfile` results; a save arrives as
    /// `DashPayProfileUpdated(id)`, which carries no fields, so without this the
    /// cache keeps the pre-save profile and the save appears lost across the
    /// app. Clears the debounce bookkeeping for `id` so a later explicit refresh
    /// can still re-resolve the authoritative state from the network.
    pub fn record_saved(&mut self, id: Identifier, fields: ProfileFields) {
        self.loaded.insert(id, Some(fields));
        self.requested.remove(&id);
        if self.in_flight == Some(id) {
            self.in_flight = None;
        }
        self.wanted.retain(|q| q.identity.id() != id);
    }

    /// Drop cached state and pending loads so a refresh re-resolves profiles.
    pub fn reset(&mut self) {
        self.loaded.clear();
        self.requested.clear();
        self.in_flight = None;
        self.wanted.clear();
    }
}
