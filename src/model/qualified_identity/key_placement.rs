//! Which key store an identity key's private half belongs to.
//!
//! Two different questions, deliberately answered by two different functions:
//!
//! * *Where is this key's private half filed?* — [`KeyStorage::candidates`],
//!   which matches on the stored public-key material instead of trusting a
//!   derived target. Every read and delete asks this one.
//! * *Where should a private half we are about to store go?* —
//!   [`QualifiedIdentity::placement_of`], which reads the identity's own
//!   on-chain key lists. Only the write path asks this one.
//!
//! [`KeyStorage::candidates`]: super::encrypted_key_storage::KeyStorage::candidates
//! [`QualifiedIdentity::placement_of`]: super::QualifiedIdentity::placement_of
//!
//! Deriving the store from a key's [`Purpose`](dash_sdk::dpp::identity::Purpose)
//! answers neither: a voting-purpose key filed on the main identity is a
//! supported shape, and no purpose distinguishes it from one on a voter
//! identity.

use crate::model::qualified_identity::PrivateKeyTarget;

/// Every store a private half can be filed under, in resolution order.
///
/// The order fixes which candidate wins when a key legitimately matches more
/// than one, so resolution never depends on map iteration order.
/// [`PrivateKeyOnOperatorIdentity`](PrivateKeyTarget::PrivateKeyOnOperatorIdentity)
/// is included even though nothing in this app writes it any more: a v0.9.3-era
/// blob can carry one, and omitting the probe would stand those keys up as
/// unfindable.
pub const PROBE_ORDER: [PrivateKeyTarget; 3] = [
    PrivateKeyTarget::PrivateKeyOnMainIdentity,
    PrivateKeyTarget::PrivateKeyOnVoterIdentity,
    PrivateKeyTarget::PrivateKeyOnOperatorIdentity,
];

/// Which of an identity's key lists publishes a given key.
///
/// This is the identity's *own on-chain record*, so it answers where a private
/// half should be filed — not where one already is. A key can be published and
/// still have its private half filed elsewhere by an older build; only
/// [`KeyStorage::candidates`](super::encrypted_key_storage::KeyStorage::candidates)
/// answers that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyPlacement {
    /// Exactly one list publishes this key.
    Resolved(PrivateKeyTarget),
    /// More than one list publishes the same key under the same id. Nothing may
    /// guess between them.
    Ambiguous(Vec<PrivateKeyTarget>),
    /// No list publishes it — normal for a key added locally whose state
    /// transition has not been broadcast yet.
    Unknown,
}

impl KeyPlacement {
    /// The single target, or `None` for `Ambiguous`/`Unknown`.
    pub fn resolved(&self) -> Option<PrivateKeyTarget> {
        match self {
            KeyPlacement::Resolved(target) => Some(target.clone()),
            _ => None,
        }
    }
}
