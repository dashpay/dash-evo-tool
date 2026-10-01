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
        let task = BackendTask::DashPayTask(Box::new(DashPayTask::LoadProfile { identity }));
        let context = BackendTaskContext::for_dispatch(&task);
        self.in_flight = Some((id, context.clone()));
        AppAction::BackendTaskWithContext { task, context }
    }

    /// Record a `LoadProfile` result against the in-flight identity. Returns
    /// `true` when the result was consumed (a load was in flight).
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
            return true;
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
        if self
            .in_flight
            .as_ref()
            .is_some_and(|(_, expected)| expected == context)
        {
            self.in_flight = None;
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
    }

    /// Drop cached state and pending loads so a refresh re-resolves profiles.
    pub fn reset(&mut self) {
        self.loaded.clear();
        self.requested.clear();
        self.in_flight = None;
        self.wanted.clear();
    }
}

#[cfg(test)]
mod race_tests {
    use super::*;

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
