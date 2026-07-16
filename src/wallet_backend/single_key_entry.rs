//! Per-key passphrase envelope for imported single-key WIFs.
//!
//! The upstream `SecretStore` row at `single_key_priv.<addr>` used to
//! hold the raw 32-byte secret. With per-key passphrases, the row
//! instead holds a [`SingleKeyEntry`] payload framed as
//! `[version_byte || bincode-encoded SingleKeyEntry]`. The version byte
//! discriminates from the legacy 32-byte raw form: a raw legacy entry
//! is exactly 32 bytes; the framed form is always longer, so the
//! reader falls back to "treat as legacy raw key" when the input is
//! 32 bytes and otherwise decodes the version-tagged shape.
//!
//! When `has_passphrase` is `false`, `ciphertext` carries the raw 32
//! private-key bytes verbatim (matches the legacy in-vault layout,
//! sandwiched in a struct so future per-entry metadata has a place to
//! land). Salt and nonce are empty. When `has_passphrase` is `true`,
//! `ciphertext` is the AES-GCM ciphertext produced by
//! [`crate::model::wallet::encryption::encrypt_message`] over the raw
//! 32 private-key bytes, with `salt` and `nonce` carrying the Argon2id
//! salt and the GCM nonce respectively.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::wallet_backend::versioned_bincode::{decode_tagged_or, encode_tagged};

/// Current on-disk version tag for [`SingleKeyEntry`].
pub const SINGLE_KEY_ENTRY_VERSION: u8 = 1;

/// Length of a raw (un-versioned) legacy entry — the bare 32 private
/// key bytes that pre-per-key-passphrase code wrote.
pub const LEGACY_RAW_KEY_LEN: usize = 32;

/// On-disk shape of a single imported private key. See module docs.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SingleKeyEntry {
    /// `true` iff `ciphertext` is the AES-GCM encryption of the raw 32
    /// private-key bytes under a user-supplied passphrase. `false`
    /// means `ciphertext` holds the raw 32 bytes verbatim.
    pub has_passphrase: bool,
    /// Argon2id salt used by the AES-GCM key derivation. Empty when
    /// `has_passphrase = false`.
    pub salt: Vec<u8>,
    /// GCM nonce used during encryption. Empty when
    /// `has_passphrase = false`.
    pub nonce: Vec<u8>,
    /// Either the raw 32 private-key bytes (no passphrase) or the
    /// AES-GCM ciphertext over the same bytes (passphrase).
    pub ciphertext: Vec<u8>,
    /// Optional user-supplied hint shown next to the passphrase
    /// prompt. Stored next to the row so a re-launched app can still
    /// surface it without keeping any in-memory state.
    pub passphrase_hint: Option<String>,
    /// Compressed SEC1-encoded public key derived from the underlying
    /// private key. Stored so the cold-boot picker can rebuild the
    /// passphrase-locked entry's `SingleKeyWallet` without unlocking
    /// it. Empty for legacy entries that pre-date this field; the
    /// caller falls back to deriving from plaintext when unlocked.
    #[serde(default)]
    pub public_key_bytes: Vec<u8>,
}

impl std::fmt::Debug for SingleKeyEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SingleKeyEntry")
            .field("has_passphrase", &self.has_passphrase)
            .field("ciphertext", &"[redacted]")
            .field("salt_len", &self.salt.len())
            .field("nonce_len", &self.nonce.len())
            .field("passphrase_hint", &self.passphrase_hint)
            .field("public_key_bytes_len", &self.public_key_bytes.len())
            .finish()
    }
}

impl SingleKeyEntry {
    /// Build a passphrase-less entry that just holds the raw 32 key
    /// bytes. Mirrors the legacy in-vault shape but inside the
    /// versioned struct so all entries (with or without passphrase)
    /// share the same code path.
    pub fn unprotected(raw_key: [u8; 32]) -> Self {
        Self {
            has_passphrase: false,
            salt: Vec::new(),
            nonce: Vec::new(),
            ciphertext: raw_key.to_vec(),
            passphrase_hint: None,
            public_key_bytes: Vec::new(),
        }
    }

    /// Build a passphrase-protected entry over `raw_key` with
    /// `passphrase`. Uses the same AES-GCM + Argon2id helper the HD
    /// wallet seed-password path uses, so a single dependency floor
    /// covers both code paths. `public_key_bytes` carries the
    /// compressed SEC1 pubkey so the cold-boot picker can rebuild the
    /// locked entry's wallet without unlocking it.
    pub fn protected(
        raw_key: &[u8; 32],
        passphrase: &str,
        hint: Option<String>,
        public_key_bytes: Vec<u8>,
    ) -> Result<Self, TaskError> {
        let envelope = crate::model::wallet::encryption::encrypt_message(raw_key, passphrase)
            .map_err(|detail| {
                tracing::warn!(
                    target = "wallet_backend::single_key_entry",
                    ?detail,
                    "Failed to encrypt single-key entry with user passphrase",
                );
                TaskError::SingleKeyCryptoFailure
            })?;
        Ok(Self {
            has_passphrase: true,
            salt: envelope.salt,
            nonce: envelope.nonce,
            ciphertext: envelope.ciphertext,
            passphrase_hint: hint,
            public_key_bytes,
        })
    }

    /// Recover the raw 32 private-key bytes. For passphrase-protected
    /// entries the caller must supply the passphrase; for unprotected
    /// entries it is ignored.
    ///
    /// Returned wrapped in [`Zeroizing`]: the key bytes zeroize when
    /// the caller drops the binding, so a copy never lingers on the stack after
    /// crossing this boundary.
    pub fn decrypt(&self, passphrase: Option<&str>) -> Result<Zeroizing<[u8; 32]>, TaskError> {
        if !self.has_passphrase {
            let raw: [u8; 32] = self.ciphertext.as_slice().try_into().map_err(|_| {
                tracing::warn!(
                    target = "wallet_backend::single_key_entry",
                    blob_len = self.ciphertext.len(),
                    "Unprotected single-key entry has wrong raw-key length",
                );
                TaskError::SingleKeyCryptoFailure
            })?;
            return Ok(Zeroizing::new(raw));
        }
        let passphrase = match passphrase {
            Some(p) if !p.is_empty() => p,
            _ => {
                // Caller should have routed via SingleKeyPassphraseRequired;
                // surfacing an Incorrect here is the safer default since
                // we cannot proceed.
                return Err(TaskError::SingleKeyPassphraseIncorrect);
            }
        };
        let plaintext = crate::model::wallet::encryption::decrypt_message(
            &self.ciphertext,
            &self.salt,
            &self.nonce,
            32,
            passphrase,
            "single_key_entry::decrypt",
        )
        .map_err(|e| match e {
            crate::model::wallet::encryption::DecryptError::WrongPassword => {
                TaskError::SingleKeyPassphraseIncorrect
            }
            crate::model::wallet::encryption::DecryptError::Malformed => {
                TaskError::SingleKeyCryptoFailure
            }
        })?;
        let raw: [u8; 32] = plaintext.as_slice().try_into().map_err(|_| {
            tracing::warn!(
                target = "wallet_backend::single_key_entry",
                blob_len = plaintext.len(),
                "Decrypted single-key entry is not 32 bytes",
            );
            TaskError::SingleKeyCryptoFailure
        })?;
        Ok(Zeroizing::new(raw))
    }

    /// Encode for the upstream vault: `[version || bincode(self)]`.
    pub fn encode(&self) -> Result<Zeroizing<Vec<u8>>, TaskError> {
        encode_tagged(SINGLE_KEY_ENTRY_VERSION, self).map_err(|detail| {
            tracing::warn!(
                target = "wallet_backend::single_key_entry",
                error = %detail,
                "bincode encode failed for single-key entry",
            );
            TaskError::SingleKeyCryptoFailure
        })
    }

    /// Decode from the upstream vault. Accepts:
    ///   * exactly 32 bytes — a legacy raw-private-key entry
    ///     (treated as `has_passphrase = false`),
    ///   * leading `SINGLE_KEY_ENTRY_VERSION` byte + bincode body —
    ///     the current shape.
    pub fn decode(bytes: &[u8]) -> Result<Self, TaskError> {
        if bytes.len() == LEGACY_RAW_KEY_LEN {
            let mut raw = [0u8; LEGACY_RAW_KEY_LEN];
            raw.copy_from_slice(bytes);
            return Ok(Self::unprotected(raw));
        }
        // Not a 32-byte legacy raw key: it must be the version-tagged shape.
        // Anything else (empty, unknown tag, or corrupt body) is a decode
        // failure — the fallback here has no further legacy form to try.
        decode_tagged_or(bytes, SINGLE_KEY_ENTRY_VERSION, |bytes| {
            if let Some((&tag, _)) = bytes.split_first()
                && tag != SINGLE_KEY_ENTRY_VERSION
            {
                tracing::warn!(
                    target = "wallet_backend::single_key_entry",
                    ?tag,
                    "Unknown single-key entry version tag",
                );
            }
            Err(TaskError::SingleKeyCryptoFailure)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unprotected_round_trip_preserves_key_bytes() {
        let raw = [0x77u8; 32];
        let entry = SingleKeyEntry::unprotected(raw);
        let bytes = entry.encode().expect("encode");
        let decoded = SingleKeyEntry::decode(&bytes).expect("decode");
        assert!(!decoded.has_passphrase);
        assert_eq!(*decoded.decrypt(None).expect("plaintext"), raw);
    }

    #[test]
    fn protected_round_trip_requires_correct_passphrase() {
        let raw = [0x11u8; 32];
        let entry = SingleKeyEntry::protected(&raw, "correct horse battery", None, Vec::new())
            .expect("encrypt");
        let bytes = entry.encode().expect("encode");
        let decoded = SingleKeyEntry::decode(&bytes).expect("decode");
        assert!(decoded.has_passphrase);
        assert_eq!(
            *decoded
                .decrypt(Some("correct horse battery"))
                .expect("plaintext"),
            raw
        );
        // Wrong passphrase surfaces the typed Incorrect variant.
        match decoded.decrypt(Some("wrong")) {
            Err(TaskError::SingleKeyPassphraseIncorrect) => {}
            other => panic!("expected SingleKeyPassphraseIncorrect, got {other:?}"),
        }
        // Missing passphrase also surfaces Incorrect (caller is
        // expected to route via SingleKeyPassphraseRequired before this).
        match decoded.decrypt(None) {
            Err(TaskError::SingleKeyPassphraseIncorrect) => {}
            other => panic!("expected SingleKeyPassphraseIncorrect, got {other:?}"),
        }
    }

    #[test]
    fn legacy_32_byte_blob_decodes_as_unprotected() {
        let raw = [0xABu8; 32];
        let decoded = SingleKeyEntry::decode(&raw).expect("decode legacy");
        assert!(!decoded.has_passphrase);
        assert!(decoded.salt.is_empty());
        assert!(decoded.nonce.is_empty());
        assert_eq!(decoded.ciphertext, raw.to_vec());
    }

    #[test]
    fn debug_output_does_not_expose_private_bytes() {
        let raw = [0x42u8; 32];
        let entry = SingleKeyEntry::unprotected(raw);
        let dbg = format!("{entry:?}");
        // The raw key byte 0x42 must not appear verbatim in the debug string.
        assert!(
            !dbg.contains("42, 42, 42"),
            "debug output leaked private bytes: {dbg}"
        );
        assert!(
            dbg.contains("[redacted]"),
            "expected [redacted] in debug output: {dbg}"
        );
    }

    #[test]
    fn wrong_length_nonce_returns_typed_error_not_panic() {
        // A corrupt at-rest entry whose nonce is not 12 bytes must surface a
        // typed error rather than panic inside `Nonce::from_slice` (which
        // would poison the long-lived secret-store mutex).
        let entry = SingleKeyEntry {
            has_passphrase: true,
            salt: vec![0u8; 16],
            nonce: vec![0u8; 7], // wrong length on purpose
            ciphertext: vec![0u8; 48],
            passphrase_hint: None,
            public_key_bytes: Vec::new(),
        };
        match entry.decrypt(Some("any-passphrase")) {
            Err(TaskError::SingleKeyCryptoFailure) => {}
            other => panic!("expected SingleKeyCryptoFailure, got {other:?}"),
        }
    }

    #[test]
    fn wrong_length_unprotected_blob_returns_typed_error_not_panic() {
        // An unprotected entry whose raw blob is not 32 bytes must also be a
        // typed error, not a panic.
        let entry = SingleKeyEntry {
            has_passphrase: false,
            salt: Vec::new(),
            nonce: Vec::new(),
            ciphertext: vec![0u8; 31], // one byte short
            passphrase_hint: None,
            public_key_bytes: Vec::new(),
        };
        match entry.decrypt(None) {
            Err(TaskError::SingleKeyCryptoFailure) => {}
            other => panic!("expected SingleKeyCryptoFailure, got {other:?}"),
        }
    }

    #[test]
    fn passphrase_hint_round_trips() {
        let raw = [0x22u8; 32];
        let entry = SingleKeyEntry::protected(
            &raw,
            "pp",
            Some("nickname of childhood pet".into()),
            Vec::new(),
        )
        .expect("encrypt");
        let bytes = entry.encode().expect("encode");
        let decoded = SingleKeyEntry::decode(&bytes).expect("decode");
        assert_eq!(
            decoded.passphrase_hint.as_deref(),
            Some("nickname of childhood pet")
        );
    }
}
