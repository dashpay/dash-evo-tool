//! DET-owned identity-metadata sidecar (SEC-001).
//!
//! Carries the cosmetic prompt copy for an identity whose keys are
//! password-protected — currently just the user-set password hint. It is
//! **display-only**: it NEVER decides whether a password is required. The
//! authoritative flag is the at-rest vault scheme (see
//! [`SecretAccess::scope_has_passphrase`](crate::wallet_backend::SecretAccess)),
//! so a sidecar that drifts from the vault can only mis-render a hint, never
//! mis-route a prompt. A missing or corrupt sidecar degrades to "no hint",
//! never an error.
//!
//! Persisted as a single positional-`bincode` blob per `(network, identity_id)`
//! in the cross-network `det-app.sqlite` k/v store behind the `DetKv`
//! schema-version envelope — a deliberate twin of
//! [`WalletMeta`](crate::model::wallet::meta::WalletMeta). The view that reads
//! and writes it is
//! [`IdentityMetaView`](crate::wallet_backend::IdentityMetaView).

use serde::{Deserialize, Serialize};

/// DET-owned per-identity metadata for the password-protection feature.
///
/// `IdentityMeta` is stored as a positional `bincode::config::standard()` blob
/// behind the `DetKv` schema envelope, so adding, removing, or reordering any
/// field here is a format-breaking change for already-stored blobs. Evolve the
/// shape the same way [`WalletMeta`](crate::model::wallet::meta::WalletMeta)
/// does — a decode-only legacy shape plus a dual-format reader — never by
/// relying on `#[serde(default)]` alone (positional bincode reads past the end
/// of a shorter blob and errors rather than defaulting a trailing field).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentityMeta {
    /// Optional user-set password hint for this identity's protected keys.
    /// Shown next to the sign-time prompt. Plain text by design — the user
    /// chose it as a memory aid, and it is never the password itself.
    #[serde(default)]
    pub password_hint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ID-META-001 — round-trip through bincode so the persisted shape is
    /// covered the same way `WalletMeta` is. A field that breaks decoding
    /// surfaces here.
    #[test]
    fn identity_meta_round_trips_through_bincode() {
        let original = IdentityMeta {
            password_hint: Some("granny's birthday".into()),
        };
        let bytes =
            bincode::serde::encode_to_vec(&original, bincode::config::standard()).expect("encode");
        let (decoded, _): (IdentityMeta, _) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).expect("decode");
        assert_eq!(decoded, original);
    }

    /// ID-META-002 — `Default` is the "no hint" shape.
    #[test]
    fn default_has_no_hint() {
        assert!(IdentityMeta::default().password_hint.is_none());
    }

    /// ID-META-003 — the sidecar structurally cannot carry a secret (no key
    /// field); canary coverage that a future field never smuggles one in.
    #[test]
    fn id_meta_003_blob_has_no_secret() {
        use crate::wallet_backend::leak_test_support::{
            assert_no_leak_bytes, distinctive_secret_64,
        };
        let secret = distinctive_secret_64();
        let meta = IdentityMeta {
            password_hint: Some("a hint, not the password".into()),
        };
        let blob =
            bincode::serde::encode_to_vec(&meta, bincode::config::standard()).expect("encode");
        let rendered = format!("{blob:?}");
        assert_no_leak_bytes(&rendered, &secret, "IdentityMeta sidecar blob");
    }
}
