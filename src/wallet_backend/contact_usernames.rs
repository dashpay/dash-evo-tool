//! DET-side store of DashPay counterparties' DPNS usernames.
//!
//! platform-wallet keeps a contact's DashPay profile but not their DPNS
//! username, so DET looks the username up once per session and keeps the
//! answer here, letting the contact lists show it offline after a restart.
//!
//! Key shape (under [`DetScope::Global`]):
//!
//! ```text
//! det:contact_profile:<contact_id_base58>
//! ```
//!
//! The key and the leading `username` field are shared with the contact
//! profile blobs older versions stored, so those still read as usernames.

// TODO(upstream): carry the DPNS username on `ContactProfileEntry`; this
// store is a stopgap until platform-wallet holds it.

use std::sync::Arc;

use dash_sdk::dpp::dashcore::base58;
use dash_sdk::platform::Identifier;

use crate::wallet_backend::kv::{KvAdapterError, kv_get_logged};
use crate::wallet_backend::{DetKv, DetScope};

/// Key prefix for every stored username.
const KEY_PREFIX: &str = "det:contact_profile:";

/// The stored value. Encoded positionally, so `username` must stay the first
/// field for older blobs to keep decoding.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct StoredUsername {
    username: Option<String>,
}

fn key_for(contact: &Identifier) -> String {
    format!("{KEY_PREFIX}{}", base58::encode_slice(&contact.to_buffer()))
}

/// View borrowing a shared [`DetKv`] handle. Cheap to construct.
pub struct ContactUsernamesView<'a> {
    kv: &'a Arc<DetKv>,
}

impl<'a> ContactUsernamesView<'a> {
    pub fn new(kv: &'a Arc<DetKv>) -> Self {
        Self { kv }
    }

    /// The stored username of `contact`; `None` when none is stored or the
    /// stored value cannot be read (logged).
    pub fn get(&self, contact: &Identifier) -> Option<String> {
        kv_get_logged::<StoredUsername>(
            self.kv,
            DetScope::Global,
            &key_for(contact),
            "contact_usernames",
        )
        .and_then(|stored| stored.username)
    }

    /// Store `username` for `contact`, or forget it when `None`.
    pub fn put(&self, contact: &Identifier, username: Option<&str>) -> Result<(), KvAdapterError> {
        let key = key_for(contact);
        match username {
            Some(username) => self.kv.put(
                DetScope::Global,
                &key,
                &StoredUsername {
                    username: Some(username.to_owned()),
                },
            ),
            None => self.kv.delete(DetScope::Global, &key),
        }
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

    #[test]
    fn a_stored_username_reads_back_per_contact() {
        let kv = kv();
        let view = ContactUsernamesView::new(&kv);
        view.put(&contact(1), Some("alice")).expect("put alice");
        view.put(&contact(2), Some("bob")).expect("put bob");
        assert_eq!(view.get(&contact(1)).as_deref(), Some("alice"));
        assert_eq!(view.get(&contact(2)).as_deref(), Some("bob"));
        assert_eq!(view.get(&contact(3)), None);
    }

    #[test]
    fn storing_no_username_forgets_the_old_one() {
        let kv = kv();
        let view = ContactUsernamesView::new(&kv);
        view.put(&contact(1), Some("alice")).expect("put");
        view.put(&contact(1), None).expect("forget");
        assert_eq!(view.get(&contact(1)), None);
    }

    /// Older versions stored a whole profile under the same key; its username
    /// still reads.
    #[test]
    fn a_profile_stored_by_an_older_version_reads_as_its_username() {
        #[derive(serde::Serialize)]
        struct OlderProfile {
            username: Option<String>,
            display_name: Option<String>,
            avatar_url: Option<String>,
            bio: Option<String>,
            account_reference: Option<u32>,
        }
        let kv = kv();
        kv.put(
            DetScope::Global,
            &key_for(&contact(7)),
            &OlderProfile {
                username: Some("carol".to_owned()),
                display_name: Some("Carol".to_owned()),
                avatar_url: None,
                bio: Some("hi".to_owned()),
                account_reference: Some(2),
            },
        )
        .expect("put older profile");
        assert_eq!(
            ContactUsernamesView::new(&kv).get(&contact(7)).as_deref(),
            Some("carol")
        );
    }
}
