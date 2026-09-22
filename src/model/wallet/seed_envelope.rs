//! Encrypted seed envelope persisted in the upstream `SecretStore`.
//!
//! Each HD wallet's BIP-39 seed is wrapped twice at rest: DET's own
//! AES-GCM envelope (the legacy `(encrypted_seed, salt, nonce,
//! uses_password)` quartet derived from the user's per-wallet
//! password) is itself stored inside the upstream vault's Argon2id +
//! XChaCha20-Poly1305 file. Double encryption is wasteful CPU but
//! correctness-preserving — the per-wallet password UX stays
//! identical to the legacy behaviour while the storage location moves
//! to the upstream vault.
//!
//! `xpub_encoded` carries the BIP44 ECDSA account-0 extended public key
//! so the wallet picker can render at cold boot without unlocking the
//! vault — the `WalletMeta` sidecar in `det-app.sqlite` keeps the same
//! bytes for the same reason; the two copies are an intentional
//! redundancy.
//!
//! **Status since the Tier-2 adoption: this is a DECODE-ONLY legacy reader.**
//! New protected seeds are sealed directly by the upstream Tier-2 object-password
//! envelope (no DET-side AES-GCM re-wrap on write); a `StoredSeedEnvelope` is only
//! decoded, to migrate not-yet-migrated wallets on first unlock, and the path
//! shrinks as wallets re-wrap to Tier-2.
//!
//! RUSTSEC-2025-0141: bincode 1.x is flagged unmaintained
//! (informational, not an exploitable CVE). This envelope encodes/decodes with
//! bincode **2.x** (`bincode::serde` + `config::standard`); `bincode 1.3.3` in
//! `Cargo.lock` is only a transitive dependency. Exposure is minimal regardless —
//! the payload is read from the LOCAL, owner-only vault, never from untrusted
//! network input.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Current on-disk version tag for the bincode-encoded
/// [`StoredSeedEnvelope`] payload. The version byte is framed by the
/// storage layer (see [`crate::wallet_backend::wallet_seed_store`]) so
/// future shape changes can bump the tag without touching the struct
/// shape that round-trips here.
pub const STORED_SEED_ENVELOPE_VERSION: u8 = 1;

/// Full encrypted seed envelope stored under one upstream `SecretStore`
/// entry per HD wallet. Mirrors the legacy `wallet` table columns DET
/// reads to reconstruct a `Wallet`: the AES-GCM ciphertext, the
/// Argon2-derived salt, the GCM nonce, the optional user hint, and the
/// `uses_password` flag the password-prompt UI keys off.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSeedEnvelope {
    /// AES-GCM ciphertext of the BIP-39 seed bytes. When
    /// `uses_password` is `false`, contains the raw 64-byte seed
    /// verbatim — DET's encryption layer is bypassed for unprotected
    /// wallets (the upstream vault still encrypts at rest).
    pub encrypted_seed: Zeroizing<Vec<u8>>,
    /// Argon2 salt used to derive the AES-GCM key from the user's
    /// password. Empty when `uses_password` is `false`.
    pub salt: Vec<u8>,
    /// AES-GCM nonce. Empty when `uses_password` is `false`.
    pub nonce: Vec<u8>,
    /// Optional user-supplied hint shown next to the password prompt.
    pub password_hint: Option<String>,
    /// `true` when the envelope is encrypted with a user-supplied
    /// password. Drives whether the unlock UI prompts for one.
    pub uses_password: bool,
    /// `ExtendedPubKey::encode()` bytes for the wallet's BIP44 ECDSA
    /// account-0 master xpub. Lets the cold-boot wallet picker render
    /// addresses without unlocking the vault.
    pub xpub_encoded: Vec<u8>,
}

impl std::fmt::Debug for StoredSeedEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredSeedEnvelope")
            .field("uses_password", &self.uses_password)
            .field("encrypted_seed", &"[redacted]")
            .field("salt_len", &self.salt.len())
            .field("nonce_len", &self.nonce.len())
            .field("password_hint", &self.password_hint)
            .field("xpub_encoded_len", &self.xpub_encoded.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_does_not_expose_seed_bytes() {
        let seed_bytes = [0x42u8; 64];
        let envelope = StoredSeedEnvelope {
            encrypted_seed: Zeroizing::new(seed_bytes.to_vec()),
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
            uses_password: false,
            xpub_encoded: vec![0xCD; 78],
        };
        let dbg = format!("{envelope:?}");
        assert!(
            !dbg.contains("42, 42, 42"),
            "debug output leaked seed bytes: {dbg}"
        );
        assert!(
            dbg.contains("[redacted]"),
            "expected [redacted] in debug output: {dbg}"
        );
    }

    /// Round-trip through bincode — the persisted shape must survive
    /// encode/decode without losing any field. Adding a non-defaulted
    /// field that breaks decoding surfaces here.
    #[test]
    fn stored_seed_envelope_round_trips_through_bincode() {
        let original = StoredSeedEnvelope {
            encrypted_seed: Zeroizing::new(vec![0xAB; 80]),
            salt: vec![0x01; 16],
            nonce: vec![0x02; 12],
            password_hint: Some("granny's birthday".into()),
            uses_password: true,
            xpub_encoded: vec![0xCD; 78],
        };
        let bytes =
            bincode::serde::encode_to_vec(&original, bincode::config::standard()).expect("encode");
        let (decoded, _): (StoredSeedEnvelope, _) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).expect("decode");
        assert_eq!(decoded, original);
    }

    /// Non-password envelopes round-trip with empty salt/nonce and
    /// `uses_password = false`. The picker still has the xpub bytes
    /// so cold-boot rendering works without the vault unlock.
    #[test]
    fn non_password_envelope_round_trips() {
        let original = StoredSeedEnvelope {
            encrypted_seed: Zeroizing::new(vec![0x11; 64]),
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
            uses_password: false,
            xpub_encoded: vec![0x22; 78],
        };
        let bytes =
            bincode::serde::encode_to_vec(&original, bincode::config::standard()).expect("encode");
        let (decoded, _): (StoredSeedEnvelope, _) =
            bincode::serde::decode_from_slice(&bytes, bincode::config::standard()).expect("decode");
        assert_eq!(decoded, original);

        fn assert_zeroize_on_drop<T: zeroize::ZeroizeOnDrop>(_: &T) {}
        assert_zeroize_on_drop(&decoded.encrypted_seed);
    }
}
