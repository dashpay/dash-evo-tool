use super::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

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
            .get(id);
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
        self.identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .begin_load(id)
    }

    pub(crate) fn record_identity_profile_name(
        &self,
        id: Identifier,
        revision: u64,
        name: Option<&str>,
    ) -> bool {
        self.identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .loaded(id, revision, name)
    }

    pub(crate) fn save_identity_profile_name(&self, id: Identifier, name: Option<&str>) {
        self.identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .saved(id, name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::model::qualified_identity::{DPNSNameInfo, IdentityStatus, IdentityType};
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::version::PlatformVersion;
    use std::collections::BTreeMap;

    #[test]
    fn identity_display_name_updates_reject_stale_loads_and_legacy_names() {
        let dir = tempfile::tempdir().unwrap();
        let context = test_app_context(dir.path());
        let id = Identifier::from([42; 32]);
        let identity = QualifiedIdentity {
            identity: Identity::create_basic_identity(id, PlatformVersion::latest()).unwrap(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: Some("Private nickname".into()),
            private_keys: KeyStorage::default(),
            dpns_names: vec![DPNSNameInfo {
                name: "alex.dash".into(),
                acquired_at: 0,
            }],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: context.network(),
        };
        assert_eq!(context.identity_display_label(&identity), "alex.dash");
        let stale = context.begin_identity_profile_load(id);
        context.save_identity_profile_name(id, Some("Alex Profile"));
        assert!(!context.record_identity_profile_name(id, stale, Some("Old Profile")));
        assert_eq!(context.identity_display_label(&identity), "Alex Profile");
        let pill = crate::ui::identity::identity_pill::IdentityPill::new(
            context.identity_display_name(id).as_deref(),
            Some("alex.dash"),
            "abcdefghijk",
        );
        assert_eq!(pill.resolved_label(), "Alex Profile");
        context.save_identity_profile_name(id, None);
        assert_eq!(
            context.identity_display_name_or(id, Some("Old profile")),
            None
        );
        assert_eq!(context.identity_display_label(&identity), "alex.dash");
    }
}
