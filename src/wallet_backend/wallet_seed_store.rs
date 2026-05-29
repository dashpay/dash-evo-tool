//! Encrypted-envelope view over the upstream [`SecretStore`].
//!
//! Each HD wallet's seed envelope (the full
//! [`StoredSeedEnvelope`] struct — ciphertext, salt, nonce, optional
//! hint, `uses_password` flag, master xpub) is bincode-encoded and
//! stored in the upstream Argon2id + XChaCha20-Poly1305 vault at one
//! label per wallet:
//!
//! ```text
//! service: WalletId(seed_hash)
//! label:   "envelope.v1"
//! value:   SecretBytes(bincode-encoded StoredSeedEnvelope)
//! ```
//!
//! The `WalletSeedHash` is reused directly as the upstream `WalletId`
//! (both are `[u8; 32]`). The envelope itself is already AES-GCM
//! encrypted when `uses_password` is set, and the vault adds its own
//! at-rest layer on top — the double encryption is a deliberate
//! trade-off so the per-wallet password UX stays identical to the
//! legacy behaviour.
//!
//! All accessors funnel storage errors into the dedicated
//! [`TaskError::WalletSeedStorage`] envelope so banner copy can speak
//! about "your wallet" rather than the more generic "imported keys"
//! wording on [`TaskError::SecretStore`].

use std::sync::Arc;

use platform_wallet_storage::secrets::{
    FileStoreError, SecretBytes, SecretStore, WalletId as SecretWalletId,
};

use crate::backend_task::error::TaskError;
use crate::model::wallet::WalletSeedHash;
use crate::model::wallet::seed_envelope::{STORED_SEED_ENVELOPE_VERSION, StoredSeedEnvelope};

/// Label under which the bincode-encoded envelope is stored. Versioned
/// so a future shape change (e.g. an additional field that breaks
/// decoding) bumps the label rather than reinterpreting existing rows.
pub(crate) const ENVELOPE_LABEL: &str = "envelope.v1";

/// Leading payload byte that tags the on-disk shape. Pre-v1 entries
/// (written by the same `set` path before this tag was introduced) start
/// with bincode's length prefix for `encrypted_seed` (a varint) and
/// almost never collide with [`STORED_SEED_ENVELOPE_VERSION`] for any
/// realistic envelope shape — the version-byte test below covers the
/// boundary case and the reader falls back to legacy decoding when the
/// tag prefix doesn't match.
fn encode_with_version(envelope: &StoredSeedEnvelope) -> Result<Vec<u8>, TaskError> {
    let body =
        bincode::serde::encode_to_vec(envelope, bincode::config::standard()).map_err(|_| {
            TaskError::WalletSeedStorage {
                source: Box::new(FileStoreError::MalformedVault),
            }
        })?;
    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(STORED_SEED_ENVELOPE_VERSION);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode the on-disk shape, accepting both the leading-version-byte
/// form (current) and a legacy bare-bincode form. A future schema bump
/// matches on the leading byte to dispatch to the right decoder.
fn decode_with_version(bytes: &[u8]) -> Result<StoredSeedEnvelope, TaskError> {
    let malformed = || TaskError::WalletSeedStorage {
        source: Box::new(FileStoreError::MalformedVault),
    };
    if let Some((&tag, rest)) = bytes.split_first()
        && tag == STORED_SEED_ENVELOPE_VERSION
        && let Ok((decoded, _)) = bincode::serde::decode_from_slice::<StoredSeedEnvelope, _>(
            rest,
            bincode::config::standard(),
        )
    {
        return Ok(decoded);
    }
    // Legacy entry: bare bincode of the envelope with no leading
    // version byte. Treated as v1.
    let (decoded, _) = bincode::serde::decode_from_slice::<StoredSeedEnvelope, _>(
        bytes,
        bincode::config::standard(),
    )
    .map_err(|_| malformed())?;
    Ok(decoded)
}

/// View borrowing the shared upstream [`SecretStore`] handle. Cheap to
/// construct — callers may build one per operation.
pub struct WalletSeedView<'a> {
    pub(crate) secret_store: &'a Arc<SecretStore>,
}

impl<'a> WalletSeedView<'a> {
    /// Borrow a [`SecretStore`] as a typed seed-envelope view. Kept
    /// `pub` so benches and downstream tooling can build the view
    /// without going through [`WalletBackend::wallet_seeds`].
    pub fn new(secret_store: &'a Arc<SecretStore>) -> Self {
        Self { secret_store }
    }

    /// Fetch the stored envelope for `seed_hash`, or `None` if nothing
    /// is stored under that hash. A row that fails to decode as a
    /// `StoredSeedEnvelope` is surfaced as
    /// [`TaskError::WalletSeedStorage`] — the vault returned bytes,
    /// but DET cannot make sense of them.
    pub fn get(&self, seed_hash: &WalletSeedHash) -> Result<Option<StoredSeedEnvelope>, TaskError> {
        let Some(bytes) = self
            .secret_store
            .get(&scope_for(seed_hash), ENVELOPE_LABEL)
            .map_err(map_err)?
        else {
            return Ok(None);
        };
        Ok(Some(decode_with_version(bytes.expose_secret())?))
    }

    /// Store `envelope` under `seed_hash`, overwriting any prior value.
    /// Idempotent — upstream `SecretStore::set` upserts.
    pub fn set(
        &self,
        seed_hash: &WalletSeedHash,
        envelope: &StoredSeedEnvelope,
    ) -> Result<(), TaskError> {
        let bytes = encode_with_version(envelope)?;
        let value = SecretBytes::from_slice(&bytes);
        self.secret_store
            .set(&scope_for(seed_hash), ENVELOPE_LABEL, &value)
            .map_err(map_err)
    }

    /// Forget the envelope at `seed_hash`. Idempotent — a missing
    /// entry returns `Ok(())`.
    pub fn delete(&self, seed_hash: &WalletSeedHash) -> Result<(), TaskError> {
        self.secret_store
            .delete(&scope_for(seed_hash), ENVELOPE_LABEL)
            .map_err(map_err)
    }
}

/// Reuse the 32-byte `WalletSeedHash` as the upstream `WalletId`
/// scope. Both types are `[u8; 32]` newtypes — see
/// `packages/rs-platform-wallet-storage/src/secrets/validate.rs`.
fn scope_for(seed_hash: &WalletSeedHash) -> SecretWalletId {
    SecretWalletId::from(*seed_hash)
}

fn map_err(source: FileStoreError) -> TaskError {
    TaskError::WalletSeedStorage {
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wallet_backend::single_key::open_secret_store;

    fn fresh_store(dir: &std::path::Path) -> Arc<SecretStore> {
        let path = dir.join("secrets.pwsvault");
        Arc::new(open_secret_store(&path).expect("open vault"))
    }

    fn sample_password_envelope() -> StoredSeedEnvelope {
        StoredSeedEnvelope {
            encrypted_seed: vec![0xAB; 80],
            salt: vec![0x01; 16],
            nonce: vec![0x02; 12],
            password_hint: Some("granny's birthday".into()),
            uses_password: true,
            xpub_encoded: vec![0xCD; 78],
        }
    }

    fn sample_non_password_envelope() -> StoredSeedEnvelope {
        StoredSeedEnvelope {
            encrypted_seed: vec![0x11; 64],
            salt: Vec::new(),
            nonce: Vec::new(),
            password_hint: None,
            uses_password: false,
            xpub_encoded: vec![0x22; 78],
        }
    }

    /// TC-W-001 (storage half) — a written envelope round-trips
    /// through `get` and the decoded struct equals the input field by
    /// field. Guards the bincode codec and the label binding.
    #[test]
    fn tc_w_001_envelope_round_trips_through_view() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let seed_hash: WalletSeedHash = [0x11; 32];
        let envelope = sample_password_envelope();
        view.set(&seed_hash, &envelope).expect("set");
        let got = view.get(&seed_hash).expect("get").expect("entry present");
        assert_eq!(got, envelope);
    }

    /// TC-W-002 — re-running `set` with the same envelope is
    /// idempotent: the second pass overwrites with the same bytes
    /// and `get` still returns the same struct.
    #[test]
    fn tc_w_002_seed_migration_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let seed_hash: WalletSeedHash = [0x22; 32];
        let envelope = sample_password_envelope();
        view.set(&seed_hash, &envelope).expect("first");
        view.set(&seed_hash, &envelope).expect("second");
        let got = view.get(&seed_hash).expect("get").expect("present");
        assert_eq!(got, envelope);
    }

    /// A `uses_password = true` envelope round-trips with all
    /// password-related fields preserved (salt / nonce / hint). The
    /// migration step uses this path; if a future field is dropped
    /// here the unlock UI would lose information.
    #[test]
    fn password_protected_envelope_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let seed_hash: WalletSeedHash = [0x33; 32];
        let envelope = sample_password_envelope();
        view.set(&seed_hash, &envelope).expect("set");
        let got = view.get(&seed_hash).expect("get").expect("present");
        assert!(got.uses_password);
        assert_eq!(got.salt, envelope.salt);
        assert_eq!(got.nonce, envelope.nonce);
        assert_eq!(got.password_hint.as_deref(), Some("granny's birthday"));
        assert_eq!(got.xpub_encoded, envelope.xpub_encoded);
    }

    /// A `uses_password = false` envelope round-trips with empty
    /// salt/nonce and the `false` flag preserved. The xpub still has
    /// to come back so the picker keeps working.
    #[test]
    fn non_password_envelope_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let seed_hash: WalletSeedHash = [0x44; 32];
        let envelope = sample_non_password_envelope();
        view.set(&seed_hash, &envelope).expect("set");
        let got = view.get(&seed_hash).expect("get").expect("present");
        assert!(!got.uses_password);
        assert!(got.salt.is_empty());
        assert!(got.nonce.is_empty());
        assert_eq!(got.xpub_encoded, envelope.xpub_encoded);
    }

    /// `get` on a missing hash returns `None`, not an error.
    #[test]
    fn get_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let seed_hash: WalletSeedHash = [0x55; 32];
        assert!(view.get(&seed_hash).unwrap().is_none());
    }

    /// `delete` is idempotent — removing an absent entry succeeds, and
    /// a subsequent delete after `set` also succeeds.
    #[test]
    fn delete_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let seed_hash: WalletSeedHash = [0x66; 32];
        view.delete(&seed_hash).expect("delete absent");
        view.set(&seed_hash, &sample_non_password_envelope())
            .unwrap();
        view.delete(&seed_hash).expect("first delete");
        view.delete(&seed_hash).expect("second delete");
        assert!(view.get(&seed_hash).unwrap().is_none());
    }

    /// SEC-005 — a freshly-written envelope's on-disk payload starts
    /// with [`STORED_SEED_ENVELOPE_VERSION`], and a legacy bare-bincode
    /// payload (written without the leading version byte) still decodes
    /// cleanly. Locks the framing the reader needs to keep accepting
    /// pre-tag entries.
    #[test]
    fn sec_005_version_byte_round_trip_and_legacy_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let seed_hash: WalletSeedHash = [0x99; 32];
        let envelope = sample_password_envelope();
        view.set(&seed_hash, &envelope).expect("set");

        // The raw on-disk bytes start with the version tag — the
        // storage layer prepends a byte that bincode did not produce.
        let raw = store
            .get(&scope_for(&seed_hash), ENVELOPE_LABEL)
            .expect("get")
            .expect("present");
        assert_eq!(
            raw.expose_secret()[0],
            STORED_SEED_ENVELOPE_VERSION,
            "stored payload must start with the current version tag",
        );
        // The trailing bytes round-trip through bincode unchanged.
        let (re_decoded, _): (StoredSeedEnvelope, _) = bincode::serde::decode_from_slice(
            &raw.expose_secret()[1..],
            bincode::config::standard(),
        )
        .expect("trailing bytes decode as bincode envelope");
        assert_eq!(re_decoded, envelope);

        // Legacy fallback — a value written without the version byte
        // (bare bincode) still decodes.
        let legacy_bytes =
            bincode::serde::encode_to_vec(&envelope, bincode::config::standard()).unwrap();
        let legacy = SecretBytes::from_slice(&legacy_bytes);
        let other_hash: WalletSeedHash = [0xAA; 32];
        store
            .set(&scope_for(&other_hash), ENVELOPE_LABEL, &legacy)
            .unwrap();
        let got = view.get(&other_hash).expect("get").expect("present");
        assert_eq!(got, envelope, "legacy untagged payload still decodes");
    }

    /// Distinct seed hashes live in distinct scopes. The `WalletId`
    /// partition is the only thing keeping wallets apart, so a
    /// cross-wallet read must surface `None` rather than another
    /// wallet's envelope.
    #[test]
    fn scope_partitions_by_wallet_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let a: WalletSeedHash = [0x77; 32];
        let b: WalletSeedHash = [0x88; 32];
        let envelope_a = sample_password_envelope();
        let mut envelope_b = sample_non_password_envelope();
        envelope_b.xpub_encoded = vec![0xEE; 78];

        view.set(&a, &envelope_a).unwrap();
        view.set(&b, &envelope_b).unwrap();

        assert_eq!(view.get(&a).unwrap().unwrap(), envelope_a);
        assert_eq!(view.get(&b).unwrap().unwrap(), envelope_b);
    }
}
