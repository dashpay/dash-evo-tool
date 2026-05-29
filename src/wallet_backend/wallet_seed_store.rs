//! Plaintext-seed view over the upstream encrypted [`SecretStore`].
//!
//! Each HD wallet's BIP-39 seed bytes are stored in the upstream
//! Argon2id + XChaCha20-Poly1305 vault at one label per wallet:
//!
//! ```text
//! service: WalletId(seed_hash)
//! label:   "seed.v1"
//! value:   SecretBytes(raw seed bytes)
//! ```
//!
//! The `WalletSeedHash` is reused directly as the upstream `WalletId`
//! (both are `[u8; 32]`). Seeds are written plaintext to the vault —
//! the vault provides the at-rest crypto, so DET stops shipping its own
//! AES-GCM envelope.
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

/// Label under which the plaintext seed bytes are stored. Versioned so
/// a future shape change (e.g. wrapping in a header struct) bumps the
/// label rather than reinterpreting existing rows.
pub(crate) const SEED_LABEL: &str = "seed.v1";

/// View borrowing the shared upstream [`SecretStore`] handle. Cheap to
/// construct — callers may build one per operation.
pub struct WalletSeedView<'a> {
    pub(crate) secret_store: &'a Arc<SecretStore>,
}

impl<'a> WalletSeedView<'a> {
    pub(crate) fn new(secret_store: &'a Arc<SecretStore>) -> Self {
        Self { secret_store }
    }

    /// Fetch the plaintext seed bytes for `seed_hash`, or `None` if
    /// nothing is stored under that hash. The returned [`SecretBytes`]
    /// zeroizes on drop — callers must not copy the contents into a
    /// non-zeroizing buffer.
    pub fn get(&self, seed_hash: &WalletSeedHash) -> Result<Option<SecretBytes>, TaskError> {
        self.secret_store
            .get(&scope_for(seed_hash), SEED_LABEL)
            .map_err(map_err)
    }

    /// Store `seed` under `seed_hash`, overwriting any prior value.
    /// Idempotent — upstream `SecretStore::set` upserts.
    pub fn set(&self, seed_hash: &WalletSeedHash, seed: &SecretBytes) -> Result<(), TaskError> {
        self.secret_store
            .set(&scope_for(seed_hash), SEED_LABEL, seed)
            .map_err(map_err)
    }

    /// Forget the seed at `seed_hash`. Idempotent — a missing entry
    /// returns `Ok(())`.
    pub fn delete(&self, seed_hash: &WalletSeedHash) -> Result<(), TaskError> {
        self.secret_store
            .delete(&scope_for(seed_hash), SEED_LABEL)
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

    /// W-SEED-001 (TC-W-001 storage half) — a written seed round-trips
    /// through `get` and survives a fresh view over the same store.
    #[test]
    fn set_then_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let seed_hash: WalletSeedHash = [0x11; 32];
        let seed_bytes = SecretBytes::from_slice(&[0xAB; 64]);
        view.set(&seed_hash, &seed_bytes).expect("set");
        let got = view.get(&seed_hash).expect("get").expect("entry present");
        assert_eq!(got.expose_secret(), &[0xAB; 64]);
    }

    /// W-SEED-002 — `set` overwrites silently (upstream contract).
    /// Mirrors the W-META-VIEW-002 contract from the metadata sidecar.
    #[test]
    fn set_overwrites_existing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let seed_hash: WalletSeedHash = [0x22; 32];
        view.set(&seed_hash, &SecretBytes::from_slice(&[0x01; 64]))
            .unwrap();
        view.set(&seed_hash, &SecretBytes::from_slice(&[0x02; 64]))
            .unwrap();
        let got = view.get(&seed_hash).unwrap().expect("present");
        assert_eq!(got.expose_secret(), &[0x02; 64]);
    }

    /// W-SEED-003 — `get` on a missing hash returns `None`, not an
    /// error. Matches the absence contract used by every other DET
    /// sidecar accessor.
    #[test]
    fn get_missing_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let seed_hash: WalletSeedHash = [0x33; 32];
        assert!(view.get(&seed_hash).unwrap().is_none());
    }

    /// W-SEED-004 — `delete` is idempotent and removes the entry.
    #[test]
    fn delete_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let seed_hash: WalletSeedHash = [0x44; 32];
        view.delete(&seed_hash).expect("delete absent");
        view.set(&seed_hash, &SecretBytes::from_slice(&[0x55; 64]))
            .unwrap();
        view.delete(&seed_hash).expect("first delete");
        view.delete(&seed_hash).expect("second delete");
        assert!(view.get(&seed_hash).unwrap().is_none());
    }

    /// W-SEED-005 — distinct seed hashes live in distinct scopes. The
    /// `WalletId` partition is the only thing keeping wallets apart, so
    /// a cross-wallet read must surface `None` rather than another
    /// wallet's seed.
    #[test]
    fn distinct_seed_hashes_partition_storage() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = WalletSeedView::new(&store);
        let a: WalletSeedHash = [0x66; 32];
        let b: WalletSeedHash = [0x77; 32];
        view.set(&a, &SecretBytes::from_slice(&[0xA0; 64])).unwrap();
        view.set(&b, &SecretBytes::from_slice(&[0xB0; 64])).unwrap();
        assert_eq!(view.get(&a).unwrap().unwrap().expose_secret(), &[0xA0; 64]);
        assert_eq!(view.get(&b).unwrap().unwrap().expose_secret(), &[0xB0; 64]);
    }
}
