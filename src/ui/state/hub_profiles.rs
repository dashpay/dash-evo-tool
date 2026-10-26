//! The DashPay profiles the Identities hub shows, read from the DashPay
//! snapshots platform-wallet's sync keeps current.

use crate::context::AppContext;
use crate::model::dashpay::{ProfileState, StoredProfile};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use std::collections::HashMap;

/// One identity's DashPay profile fields; empty strings mean unset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

impl From<&StoredProfile> for ProfileFields {
    fn from(profile: &StoredProfile) -> Self {
        Self {
            display_name: profile.display_name.clone().unwrap_or_default(),
            bio: profile.bio.clone().unwrap_or_default(),
            avatar_url: profile.avatar_url.clone().unwrap_or_default(),
        }
    }
}

/// The profiles of the identities on screen, re-read every frame.
#[derive(Debug, Default)]
pub struct HubProfiles {
    /// `Some(fields)` = a profile is published, `None` = none is. Identities
    /// whose profile no sync has read yet are absent.
    shown: HashMap<Identifier, Option<ProfileFields>>,
}

impl HubProfiles {
    /// Re-read the profiles of `ids` from their DashPay snapshots.
    pub fn read(&mut self, app_context: &AppContext, ids: impl IntoIterator<Item = Identifier>) {
        self.shown.clear();
        for id in ids {
            let Some(snapshot) = app_context.dashpay_snapshot(&id) else {
                continue;
            };
            match &snapshot.profile {
                ProfileState::Present(profile) => {
                    self.shown.insert(id, Some((&**profile).into()));
                }
                ProfileState::Absent => {
                    self.shown.insert(id, None);
                }
                ProfileState::Unknown => {}
            }
        }
    }

    /// `identity`'s profile: `Some(Some(_))` = published, `Some(None)` = none
    /// published, `None` = not read yet.
    pub fn get(&self, identity: &QualifiedIdentity) -> Option<&Option<ProfileFields>> {
        self.shown.get(&identity.identity.id())
    }

    /// Show `profile` for `id`, as a sync would.
    #[cfg(any(test, feature = "testing"))]
    pub fn set_for_test(&mut self, id: Identifier, profile: Option<ProfileFields>) {
        self.shown.insert(id, profile);
    }
}
