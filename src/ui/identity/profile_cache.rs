//! Cached profile fields with correlated background refreshes for the Identities hub.

use crate::app::AppAction;
use crate::backend_task::dashpay::DashPayTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
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
    /// The owner and exact dispatch whose completion this cache accepts.
    in_flight: Option<(Identifier, BackendTaskContext)>,
    /// Identities a tab asked for this frame that still need a load dispatched.
    wanted: Vec<QualifiedIdentity>,
    /// Loaded identities due a reload; their profile is shown until it lands.
    stale: HashSet<Identifier>,
    /// Identities whose load failed only because the network was still
    /// syncing; [`Self::retry_after_sync`] asks for them again.
    awaiting_sync: HashSet<Identifier>,
}

impl ProfileCache {
    /// Seed a missing entry from wallet memory while queuing an authoritative refresh.
    pub(crate) fn seed_cached(
        &mut self,
        app_context: &crate::context::AppContext,
        identity: &QualifiedIdentity,
    ) {
        let id = identity.identity.id();
        if !self.loaded.contains_key(&id)
            && let Some(profile) = app_context.cached_identity_profile(id)
        {
            self.get_or_request(identity);
            self.loaded.insert(
                id,
                Some(ProfileFields {
                    display_name: profile.display_name.unwrap_or_default(),
                    bio: profile.bio.unwrap_or_default(),
                    avatar_url: profile.avatar_url.unwrap_or_default(),
                }),
            );
        }
    }

    /// Loaded profile state for `identity`, queuing a load on a miss or for a
    /// kept profile due a reload.
    ///
    /// `Some(Some(_))` = profile present, `Some(None)` = loaded with none
    /// published, `None` = not loaded yet (a load is queued for dispatch).
    pub fn get_or_request(
        &mut self,
        identity: &QualifiedIdentity,
    ) -> Option<&Option<ProfileFields>> {
        let id = identity.identity.id();
        let stale = self.stale.remove(&id);
        let queued = self.wanted.iter().any(|q| q.identity.id() == id);
        let known = self.loaded.contains_key(&id) || self.requested.contains(&id) || queued;
        if !known || (stale && !queued) {
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
        let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadProfile { identity }));
        let context = BackendTaskContext::for_dispatch(&task);
        self.in_flight = Some((id, context.clone()));
        AppAction::BackendTaskWithContext { task, context }
    }

    /// Record a `LoadProfile` result against the in-flight identity. Returns
    /// `true` only when a correlated, current profile replaced the cache.
    pub fn record_result(
        &mut self,
        app_context: &crate::context::AppContext,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) -> bool {
        let BackendTaskSuccessResult::DashPayProfile(data) = result else {
            return false;
        };
        let Some((id, expected)) = self.in_flight.as_ref() else {
            return false;
        };
        if expected != context {
            return false;
        }
        let id = *id;
        self.in_flight = None;
        if data.owner != id || !app_context.profile_snapshot_is_current(data) {
            self.requested.remove(&id);
            self.loaded.remove(&id);
            return false;
        }
        let fields = data
            .profile
            .clone()
            .map(|(display_name, bio, avatar_url)| ProfileFields {
                display_name,
                bio,
                avatar_url,
            });
        self.loaded.insert(id, fields);
        true
    }

    /// Release a failed load so other identities can load; retry it on refresh.
    pub fn record_error(&mut self, context: &BackendTaskContext) {
        self.release_failed(context);
    }

    /// Release a load that failed only because the network was still syncing;
    /// [`Self::retry_after_sync`] asks for it again once the network is ready.
    pub fn record_error_while_syncing(&mut self, context: &BackendTaskContext) {
        if let Some(id) = self.release_failed(context) {
            self.awaiting_sync.insert(id);
        }
    }

    /// Clear the in-flight load if `context` is its dispatch, returning its owner.
    fn release_failed(&mut self, context: &BackendTaskContext) -> Option<Identifier> {
        let (id, expected) = self.in_flight.as_ref()?;
        if expected != context {
            return None;
        }
        let id = *id;
        self.in_flight = None;
        Some(id)
    }

    /// Ask again for every profile whose load failed while the network was
    /// still syncing, keeping any cached profile on screen until it reloads.
    /// Call once the network is ready; cheap when none failed.
    pub fn retry_after_sync(&mut self) {
        for id in self.awaiting_sync.drain() {
            self.requested.remove(&id);
            self.stale.insert(id);
        }
    }

    /// Record the normalized fields confirmed by the backend.
    pub fn record_saved(&mut self, id: Identifier, fields: ProfileFields) {
        self.invalidate(id);
        self.loaded.insert(id, Some(fields));
    }

    /// Make this owner's next read refresh authoritative fields.
    pub(crate) fn invalidate(&mut self, id: Identifier) {
        self.loaded.remove(&id);
        self.requested.remove(&id);
        if self
            .in_flight
            .as_ref()
            .is_some_and(|(loading, _)| *loading == id)
        {
            self.in_flight = None;
        }
        self.wanted.retain(|q| q.identity.id() != id);
        self.stale.remove(&id);
        self.awaiting_sync.remove(&id);
    }

    /// Keep every loaded profile on screen but reload each on its next read.
    /// A profile whose load is still running is left to that load; one whose
    /// load failed before anything was loaded is asked for again.
    pub fn reload_kept(&mut self) {
        let loading = self.in_flight.as_ref().map(|(id, _)| *id);
        let loaded = &self.loaded;
        self.requested
            .retain(|id| loaded.contains_key(id) || Some(*id) == loading);
        // A failed load is no longer `requested`, so its next read asks again
        // without waiting for the network to sync.
        self.awaiting_sync.clear();
        self.stale = self
            .loaded
            .keys()
            .copied()
            .filter(|id| Some(*id) != loading)
            .collect();
    }

    /// Drop cached state and pending loads so a refresh re-resolves profiles.
    pub fn reset(&mut self) {
        self.loaded.clear();
        self.requested.clear();
        self.in_flight = None;
        self.wanted.clear();
        self.stale.clear();
        self.awaiting_sync.clear();
    }
}

#[cfg(test)]
mod race_tests {
    use super::*;

    /// Only a load that failed because the network was still syncing is asked
    /// for again once it is ready; other failures wait for a refresh.
    #[test]
    fn only_a_load_failed_while_syncing_is_retried_after_sync() {
        let dir = tempfile::tempdir().unwrap();
        let app = crate::context::test_support::test_app_context(dir.path());
        let identity = crate::context::test_support::user_identity_with_signing_key(
            Identifier::from([72; 32]),
            app.network,
            false,
        );
        for network_syncing in [true, false] {
            let mut cache = ProfileCache::default();
            assert!(cache.get_or_request(&identity).is_none());
            let AppAction::BackendTaskWithContext { context, .. } = cache.dispatch_pending() else {
                panic!("profile load")
            };
            let record = |cache: &mut ProfileCache, context: &BackendTaskContext| {
                if network_syncing {
                    cache.record_error_while_syncing(context);
                } else {
                    cache.record_error(context);
                }
            };
            record(&mut cache, &BackendTaskContext::Other);
            assert!(
                matches!(cache.dispatch_pending(), AppAction::None),
                "another dispatch's failure leaves this load in flight"
            );
            record(&mut cache, &context);
            cache.retry_after_sync();
            cache.get_or_request(&identity);
            assert_eq!(
                matches!(
                    cache.dispatch_pending(),
                    AppAction::BackendTaskWithContext { .. }
                ),
                network_syncing,
                "network_syncing = {network_syncing}"
            );
        }
    }

    #[test]
    fn kept_profile_whose_refresh_failed_while_syncing_is_retried_after_sync() {
        let dir = tempfile::tempdir().unwrap();
        let app = crate::context::test_support::test_app_context(dir.path());
        let identity = crate::context::test_support::user_identity_with_signing_key(
            Identifier::from([73; 32]),
            app.network,
            false,
        );
        let id = identity.identity.id();
        let mut cache = ProfileCache::default();
        cache.get_or_request(&identity);
        let kept = ProfileFields {
            display_name: "Kept".into(),
            ..Default::default()
        };
        cache.loaded.insert(id, Some(kept));
        let AppAction::BackendTaskWithContext { context, .. } = cache.dispatch_pending() else {
            panic!("profile refresh")
        };
        cache.record_error_while_syncing(&context);
        cache.retry_after_sync();
        assert_eq!(
            cache
                .get_or_request(&identity)
                .and_then(|profile| profile.as_ref())
                .map(|profile| profile.display_name.as_str()),
            Some("Kept"),
            "the kept profile stays on screen while it reloads"
        );
        assert!(
            matches!(
                cache.dispatch_pending(),
                AppAction::BackendTaskWithContext { .. }
            ),
            "the kept profile is refreshed once the network is ready"
        );
    }

    #[test]
    fn completed_save_rejects_queued_present_and_absent_profile_loads() {
        let dir = tempfile::tempdir().unwrap();
        let app = crate::context::test_support::test_app_context(dir.path());
        let id = Identifier::from([71; 32]);
        for accepted_before_save in [true, false] {
            for data in [Some(("Old".into(), String::new(), String::new())), None] {
                let revision = app.begin_identity_profile_load(id);
                if accepted_before_save {
                    assert!(app.record_identity_profile_name(id, revision, Some("Old")));
                }
                let context = BackendTaskContext::Other;
                let mut cache = ProfileCache {
                    in_flight: Some((id, context.clone())),
                    ..Default::default()
                };
                app.save_identity_profile_name(id, Some("Saved"));
                if !accepted_before_save {
                    assert!(!app.record_identity_profile_name(id, revision, Some("Old")));
                }
                cache.record_result(
                    &app,
                    &context,
                    &BackendTaskSuccessResult::DashPayProfile(
                        crate::model::dashpay::ProfileSnapshot {
                            network: app.network,
                            owner: id,
                            revision,
                            profile: data,
                        },
                    ),
                );
                assert!(
                    !cache.loaded.contains_key(&id),
                    "a queued load must not undo a completed save"
                );
                assert!(
                    cache.in_flight.is_none(),
                    "stale results must terminate the load"
                );
            }
        }
    }
}
