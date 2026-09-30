use super::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

impl AppContext {
    /// Read a profile name without network requests, disk reads, or waiting on wallet state.
    pub fn identity_display_name(&self, id: Identifier) -> Option<String> {
        let accepted = self
            .identity_profile_names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id);
        accepted.unwrap_or_else(|| {
            self.wallet_backend()
                .ok()
                .and_then(|backend| backend.dashpay().cached_display_name(&id))
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
