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
    in_flight: Option<(Identifier, u64, u64)>,
    /// Identities a tab asked for this frame that still need a load dispatched.
    wanted: Vec<QualifiedIdentity>,
    /// Invalidates every request dispatched before a full reset.
    cache_generation: u64,
    /// Invalidates requests dispatched before one identity was unloaded.
    identity_generations: HashMap<Identifier, u64>,
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
        let identity_generation = self.identity_generations.get(&id).copied().unwrap_or(0);
        self.in_flight = Some((id, self.cache_generation, identity_generation));
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
        let Some((id, cache_generation, identity_generation)) = self.in_flight.take() else {
            return false;
        };
        self.requested.remove(&id);
        let is_current = cache_generation == self.cache_generation
            && identity_generation == self.identity_generations.get(&id).copied().unwrap_or(0);
        if !is_current {
            return true;
        }
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

    /// Remove one identity's cached and queued profile state.
    ///
    /// An already-dispatched request remains the single in-flight operation,
    /// but its generation is invalidated so a late response is discarded.
    pub fn remove_identity(&mut self, identity_id: &Identifier) {
        self.loaded.remove(identity_id);
        self.requested.remove(identity_id);
        self.wanted
            .retain(|identity| identity.identity.id() != *identity_id);
        let generation = self.identity_generations.entry(*identity_id).or_default();
        *generation = generation.wrapping_add(1);
    }

    /// Drop cached state and pending loads so a refresh re-resolves profiles.
    pub fn reset(&mut self) {
        self.loaded.clear();
        self.requested.clear();
        self.wanted.clear();
        self.in_flight = None;
        self.cache_generation = self.cache_generation.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType};
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identity;
    use std::collections::BTreeMap;

    fn qualified_identity(id: Identifier) -> QualifiedIdentity {
        QualifiedIdentity {
            identity: Identity::create_basic_identity(id, PlatformVersion::latest())
                .expect("create identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: Vec::new(),
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    #[test]
    fn late_profile_result_after_unload_does_not_repopulate_cache() {
        let id = Identifier::from([0x41; 32]);
        let identity = qualified_identity(id);
        let mut cache = ProfileCache::default();

        assert!(cache.get_or_request(&identity).is_none());
        assert!(matches!(
            cache.dispatch_pending(),
            AppAction::BackendTask(BackendTask::DashPayTask(_))
        ));
        cache.loaded.insert(
            id,
            Some(ProfileFields {
                display_name: "Previously cached".to_string(),
                ..Default::default()
            }),
        );

        cache.remove_identity(&id);
        assert!(
            cache.record_result(&BackendTaskSuccessResult::DashPayProfile(Some((
                "Stale name".to_string(),
                "Stale bio".to_string(),
                "https://example.invalid/stale.png".to_string(),
            ))))
        );

        assert!(
            !cache.loaded.contains_key(&id),
            "a response started before unload must stay discarded"
        );

        assert!(cache.get_or_request(&identity).is_none());
        assert!(matches!(
            cache.dispatch_pending(),
            AppAction::BackendTask(BackendTask::DashPayTask(_))
        ));
        assert!(
            cache.record_result(&BackendTaskSuccessResult::DashPayProfile(Some((
                "Fresh name".to_string(),
                "Fresh bio".to_string(),
                "https://example.invalid/fresh.png".to_string(),
            ))))
        );
        assert_eq!(
            cache
                .loaded
                .get(&id)
                .and_then(Option::as_ref)
                .map(|fields| fields.display_name.as_str()),
            Some("Fresh name"),
            "the next request after reload must populate normally"
        );
    }

    #[test]
    fn reset_unblocks_dispatch_after_profile_load_error() {
        let failed_identity = qualified_identity(Identifier::from([0x42; 32]));
        let next_identity = qualified_identity(Identifier::from([0x43; 32]));
        let mut cache = ProfileCache::default();

        assert!(cache.get_or_request(&failed_identity).is_none());
        assert!(matches!(
            cache.dispatch_pending(),
            AppAction::BackendTask(BackendTask::DashPayTask(_))
        ));

        cache.reset();

        assert!(cache.get_or_request(&next_identity).is_none());
        assert!(
            matches!(
                cache.dispatch_pending(),
                AppAction::BackendTask(BackendTask::DashPayTask(_))
            ),
            "reset must abandon an unresolved request so another identity can load"
        );
    }
}
