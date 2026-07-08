//! DET-side contact-profile cache.
//!
//! Upstream rehydrates a *local* identity's own DashPay profile into its
//! `ManagedIdentity`, but a *contact's* profile (display name, avatar URL,
//! bio, DPNS username) belongs to an out-of-wallet identity and is therefore
//! never rehydrated — [`WalletBackend::dashpay_set_profile`] is a no-op for a
//! non-managed identity. Without a DET-side cache, the contact list must
//! re-fetch every contact's profile and DPNS name from the network on every
//! view.
//!
//! [`ContactProfileCacheView`] stores the last fetched contact profile keyed
//! by the contact's identity in the cross-network app-level k/v store, so the
//! offline contact read can paint names and avatars without a round-trip. The
//! cache is refreshed whenever the network read fetches fresh profile data.
//!
//! Key shape (under [`DetScope::Global`]):
//!
//! ```text
//! det:contact_profile:<contact_id_base58>
//! ```

use std::sync::Arc;

use dash_sdk::dpp::dashcore::base58;
use dash_sdk::platform::Identifier;

use crate::backend_task::error::TaskError;
use crate::wallet_backend::kv::KvAdapterError;
use crate::wallet_backend::{DetKv, DetScope};

/// Key prefix for every cached contact profile.
const KEY_PREFIX: &str = "det:contact_profile:";

/// A cached contact profile — the network-fetched display fields DET serves
/// offline. Pure data; serialised whole into the k/v store.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachedContactProfile {
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    /// The contact's account reference (the "Account #N" badge), decoded from
    /// the network contact's private data. Cached so the offline read can show
    /// the badge and sort consistently with the post-refresh view. `None` when
    /// the network read had no account reference to cache.
    #[serde(default)]
    pub account_reference: Option<u32>,
}

impl CachedContactProfile {
    /// Whether the cached profile carries any displayable field. An all-`None`
    /// profile is not worth writing or serving. The account reference is
    /// metadata, not a displayable field, so it alone does not make a profile
    /// worth caching.
    pub fn is_empty(&self) -> bool {
        self.username.is_none()
            && self.display_name.is_none()
            && self.avatar_url.is_none()
            && self.bio.is_none()
    }
}

/// Build the canonical k/v key for a contact's cached profile.
fn key_for(contact: &Identifier) -> String {
    format!("{KEY_PREFIX}{}", base58::encode_slice(&contact.to_buffer()))
}

/// View borrowing a shared [`DetKv`] handle. Cheap to construct.
pub struct ContactProfileCacheView<'a> {
    kv: &'a Arc<DetKv>,
}

impl<'a> ContactProfileCacheView<'a> {
    /// Borrow a [`DetKv`] handle as a typed contact-profile cache view.
    pub fn new(kv: &'a Arc<DetKv>) -> Self {
        Self { kv }
    }

    /// Fetch the cached profile for `contact`. Returns `None` when the contact
    /// has no cached profile or its blob fails to decode (logged, treated as
    /// absent so the caller falls back to a network fetch).
    pub fn get(&self, contact: &Identifier) -> Option<CachedContactProfile> {
        let key = key_for(contact);
        match self.kv.get::<CachedContactProfile>(DetScope::Global, &key) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target = "wallet_backend::contact_profile_cache",
                    error = ?e,
                    "Failed to read cached contact profile; treating as absent",
                );
                None
            }
        }
    }

    /// Upsert the cached profile for `contact`. An all-`None` profile is
    /// dropped instead of stored, so an empty fetch does not overwrite a
    /// previously cached profile with nothing.
    pub fn put(
        &self,
        contact: &Identifier,
        profile: &CachedContactProfile,
    ) -> Result<(), TaskError> {
        if profile.is_empty() {
            return self.invalidate(contact);
        }
        let key = key_for(contact);
        self.kv
            .put(DetScope::Global, &key, profile)
            .map_err(map_kv_error_to_task_error)
    }

    /// Drop the cached profile for `contact`. Idempotent.
    pub fn invalidate(&self, contact: &Identifier) -> Result<(), TaskError> {
        let key = key_for(contact);
        self.kv
            .delete(DetScope::Global, &key)
            .map_err(map_kv_error_to_task_error)
    }
}

/// Contact-profile-cache adapter errors funnel into the dedicated
/// [`TaskError::AvatarCacheStorage`] envelope — the same offline-cache surface
/// the contact list shows.
fn map_kv_error_to_task_error(e: KvAdapterError) -> TaskError {
    TaskError::AvatarCacheStorage {
        source: Box::new(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::wallet_backend::kv_test_support::InMemoryKv;

    fn kv() -> Arc<DetKv> {
        Arc::new(DetKv::from_store(Arc::new(InMemoryKv::default())))
    }

    fn contact(b: u8) -> Identifier {
        Identifier::from([b; 32])
    }

    fn profile(name: &str) -> CachedContactProfile {
        CachedContactProfile {
            username: Some(format!("{name}.dash")),
            display_name: Some(name.to_string()),
            avatar_url: Some(format!("https://example.com/{name}.png")),
            bio: None,
            account_reference: None,
        }
    }

    #[test]
    fn put_then_get_round_trips() {
        let kv = kv();
        let view = ContactProfileCacheView::new(&kv);
        let c = contact(1);
        let p = profile("alice");
        view.put(&c, &p).expect("put");
        assert_eq!(view.get(&c), Some(p));
    }

    #[test]
    fn get_missing_returns_none() {
        let kv = kv();
        let view = ContactProfileCacheView::new(&kv);
        assert_eq!(view.get(&contact(9)), None);
    }

    #[test]
    fn empty_profile_is_not_stored_and_clears_existing() {
        let kv = kv();
        let view = ContactProfileCacheView::new(&kv);
        let c = contact(2);
        view.put(&c, &profile("bob")).expect("seed");
        // An all-None fetch must not overwrite the cached profile with nothing
        // — it clears the entry rather than persisting an empty record.
        view.put(&c, &CachedContactProfile::default())
            .expect("empty put");
        assert_eq!(view.get(&c), None);
    }

    #[test]
    fn invalidate_is_idempotent() {
        let kv = kv();
        let view = ContactProfileCacheView::new(&kv);
        let c = contact(3);
        view.invalidate(&c).expect("invalidate absent");
        view.put(&c, &profile("carol")).expect("put");
        view.invalidate(&c).expect("first invalidate");
        view.invalidate(&c).expect("second invalidate");
        assert_eq!(view.get(&c), None);
    }

    #[test]
    fn distinct_contacts_do_not_collide() {
        let kv = kv();
        let view = ContactProfileCacheView::new(&kv);
        view.put(&contact(4), &profile("dave")).expect("put dave");
        view.put(&contact(5), &profile("erin")).expect("put erin");
        assert_eq!(view.get(&contact(4)), Some(profile("dave")));
        assert_eq!(view.get(&contact(5)), Some(profile("erin")));
    }
}
