use super::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

/// Accepted names and load revisions for this network, including explicit clears.
#[derive(Debug, Default)]
pub(super) struct ProfileName {
    revision: u64,
    accepted: Option<Option<String>>,
}

impl ProfileName {
    fn accept(&mut self, name: Option<&str>) {
        self.accepted = Some(
            name.map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        );
    }
}

impl AppContext {
    /// Read a profile name without network requests, disk reads, or waiting on wallet state.
    pub fn identity_display_name(&self, id: Identifier) -> Option<String> {
        self.identity_display_name_or(id, None)
    }

    /// Include an already-loaded screen profile before falling back to an unnamed identity.
    pub(crate) fn identity_display_name_or(
        &self,
        id: Identifier,
        profile_name: Option<&str>,
    ) -> Option<String> {
        let accepted = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&id)
            .and_then(|entry| entry.accepted.clone());
        accepted.unwrap_or_else(|| {
            self.wallet_backend()
                .ok()
                .and_then(|backend| backend.dashpay_view().cached_display_name(&id))
                .or_else(|| {
                    profile_name
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                })
        })
    }

    /// Name an identity consistently with its profile, username, or shortened identifier.
    pub fn identity_display_label(&self, identity: &QualifiedIdentity) -> String {
        identity.display_name_label(
            self.identity_display_name(identity.identity.id())
                .as_deref(),
        )
    }

    pub(crate) fn begin_identity_profile_load(&self, id: Identifier) -> u64 {
        let mut names = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = names.entry(id).or_default();
        entry.revision += 1;
        entry.revision
    }

    pub(crate) fn record_identity_profile_name(
        &self,
        id: Identifier,
        revision: u64,
        name: Option<&str>,
    ) -> bool {
        let mut names = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(entry) = names
            .get_mut(&id)
            .filter(|entry| entry.revision == revision)
        else {
            return false;
        };
        entry.accept(name);
        true
    }

    pub(crate) fn save_identity_profile_name(&self, id: Identifier, name: Option<&str>) {
        let mut names = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = names.entry(id).or_default();
        entry.revision += 1;
        entry.accept(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;

    #[test]
    fn identity_display_name_updates_reject_stale_loads_and_isolate_networks() {
        let dir = tempfile::tempdir().unwrap();
        let context = test_app_context(dir.path());
        let id = Identifier::from([42; 32]);
        assert_eq!(context.identity_display_name(id), None);
        let stale = context.begin_identity_profile_load(id);
        context.save_identity_profile_name(id, Some("Alex Profile"));
        assert!(!context.record_identity_profile_name(id, stale, Some("Old Profile")));
        assert_eq!(
            context.identity_display_name(id).as_deref(),
            Some("Alex Profile")
        );
        let pill = crate::ui::identity::identity_pill::IdentityPill::new(
            context.identity_display_name(id).as_deref(),
            Some("alex.dash"),
            "abcdefghijk",
        );
        assert_eq!(pill.resolved_label(), "Alex Profile");
        let stale = context.begin_identity_profile_load(id);
        context.save_identity_profile_name(id, None);
        assert!(!context.record_identity_profile_name(id, stale, Some("Old Profile")));
        assert_eq!(
            context.identity_display_name_or(id, Some("Old profile")),
            None
        );
        let old = context.begin_identity_profile_load(id);
        let new = context.begin_identity_profile_load(id);
        assert!(!context.record_identity_profile_name(id, old, Some("Old")));
        assert!(context.record_identity_profile_name(id, new, Some("Latest")));
        assert_eq!(context.identity_display_name(id).as_deref(), Some("Latest"));
        assert_eq!(
            context.identity_display_name(Identifier::from([7; 32])),
            None
        );
        let other_dir = tempfile::tempdir().unwrap();
        let other_network = crate::context::test_support::test_app_context_for_network(
            other_dir.path(),
            dash_sdk::dpp::dashcore::Network::Mainnet,
        );
        assert_eq!(other_network.identity_display_name(id), None);
    }
}
