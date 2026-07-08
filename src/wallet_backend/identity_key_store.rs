//! Raw identity-private-key storage over the secret seam.
//!
//! Each identity private key is stored as raw 32 bytes in the upstream vault
//! through [`SecretSeam`], scoped to the identity id
//! (`Identifier::to_buffer()`) under the label
//! `identity_key_priv.<target_tag>.<key_id>`. There is NO DET-side envelope —
//! the key bytes ride raw (the no-serialization invariant), and the `InVault`
//! placeholder in the `QualifiedIdentity` blob is the only on-disk marker that
//! the key exists.
//!
//! The keys are fetched per-use through
//! [`SecretAccess`](crate::wallet_backend::SecretAccess) at sign time and never
//! resident in memory as plaintext.

use std::sync::Arc;

use dash_sdk::dpp::identity::KeyID;
use platform_wallet_storage::secrets::{
    SecretBytes, SecretStore, SecretStoreError, SecretString, WalletId as SecretWalletId,
};
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::qualified_identity::encrypted_key_storage::VaultBoundKey;
use crate::wallet_backend::secret_access::identity_key_from_bytes;
use crate::wallet_backend::secret_prompt::SecretScope;
use crate::wallet_backend::secret_seam::{SecretScheme, SecretSeam};

/// Borrowed view over the secret seam for one identity's private keys. Cheap
/// to construct — callers build one per operation.
pub struct IdentityKeyView<'a> {
    secret_store: &'a Arc<SecretStore>,
    /// The identity id (`Identifier::to_buffer()`) used as the vault scope.
    identity_id: [u8; 32],
}

impl<'a> IdentityKeyView<'a> {
    /// Borrow the seam for the identity scoped by `identity_id`.
    pub fn new(secret_store: &'a Arc<SecretStore>, identity_id: [u8; 32]) -> Self {
        Self {
            secret_store,
            identity_id,
        }
    }

    fn scope(&self) -> SecretWalletId {
        SecretWalletId::from(self.identity_id)
    }

    fn seam(&self) -> SecretSeam<'_> {
        SecretSeam::new(self.secret_store)
    }

    /// Store one identity key's raw 32 bytes Tier-1 (keyless), overwriting any
    /// prior **unprotected** value.
    ///
    /// R2 downgrade guard: refuses to overwrite a Tier-2
    /// (`Protected`) label with a keyless write, returning
    /// [`TaskError::IdentityKeyProtectionDowngrade`]. This is the keyless
    /// migration write path ([`Self::store_all`]); it must never silently strip
    /// an opted-in identity's protection (e.g. a later `AddKeyToIdentity` that
    /// re-saves an existing protected key). The deliberate opt-out downgrade
    /// uses [`Self::store_unprotected`] instead, which bypasses the guard.
    pub fn store(
        &self,
        target: &PrivateKeyTarget,
        key_id: KeyID,
        key: &[u8; 32],
    ) -> Result<(), TaskError> {
        let label = SecretScope::identity_key_label(target, key_id);
        // Known, low-risk: this scheme-probe-then-write is a theoretical
        // check-then-act TOCTOU, bounded in practice by the upstream secret
        // store's single-writer lock and the UI in-flight gate that serialises
        // protect/unprotect/add-key on one identity. The identity-level
        // fail-closed guard in the save path is the primary defense; this
        // per-label check is defense in depth.
        if self
            .seam()
            .scheme(&self.scope(), &label)
            .map_err(identity_flavored)?
            == SecretScheme::Protected
        {
            tracing::warn!(
                target = "wallet_backend::identity_key_store",
                identity = %hex::encode(self.identity_id),
                "Refused a keyless write over a password-protected identity key",
            );
            return Err(TaskError::IdentityKeyProtectionDowngrade);
        }
        self.seam()
            .put_secret(&self.scope(), &label, &SecretBytes::from_slice(key))
            .map_err(identity_flavored)
    }

    /// Store one identity key's raw 32 bytes Tier-2 (sealed under the
    /// identity's object `password`), overwriting any prior value at the SAME
    /// label — the in-place Tier-1→Tier-2 opt-in upsert. After this the label's
    /// scheme flips to `Protected` with no second key and no delete.
    pub fn store_protected(
        &self,
        target: &PrivateKeyTarget,
        key_id: KeyID,
        key: &[u8; 32],
        password: &SecretString,
    ) -> Result<(), TaskError> {
        let label = SecretScope::identity_key_label(target, key_id);
        self.seam()
            .put_secret_protected(
                &self.scope(),
                &label,
                &SecretBytes::from_slice(key),
                password,
            )
            .map_err(identity_flavored)
    }

    /// Intentional Tier-1 (raw) write that REPLACES any Tier-2 value at the
    /// label — the deliberate opt-out downgrade (Tier-2→Tier-1 in place).
    /// Unlike [`Self::store`] it does NOT refuse a `Protected` label: removing
    /// protection is its whole job. Only the `UnprotectIdentityKeys` migration
    /// calls this.
    pub fn store_unprotected(
        &self,
        target: &PrivateKeyTarget,
        key_id: KeyID,
        key: &[u8; 32],
    ) -> Result<(), TaskError> {
        let label = SecretScope::identity_key_label(target, key_id);
        self.seam()
            .put_secret(&self.scope(), &label, &SecretBytes::from_slice(key))
            .map_err(identity_flavored)
    }

    /// The at-rest [`SecretScheme`] of one identity key — `Protected` (Tier-2),
    /// `Unprotected` (Tier-1 raw), or `Absent`. Used by the migration tasks to
    /// skip already-converted keys (idempotent re-run) and by the UI to detect a
    /// partially-protected identity ("Finish protecting").
    pub fn scheme(
        &self,
        target: &PrivateKeyTarget,
        key_id: KeyID,
    ) -> Result<SecretScheme, TaskError> {
        let label = SecretScope::identity_key_label(target, key_id);
        self.seam()
            .scheme(&self.scope(), &label)
            .map_err(identity_flavored)
    }

    /// Read one identity key's raw 32 bytes from its Tier-2 envelope, unsealing
    /// with the identity's object `password`. `None` if absent. A wrong
    /// password surfaces as [`TaskError::IdentityKeyPassphraseIncorrect`] (no
    /// oracle); the bytes wipe on drop ([`Zeroizing`]).
    pub fn get_protected(
        &self,
        target: &PrivateKeyTarget,
        key_id: KeyID,
        password: &SecretString,
    ) -> Result<Option<Zeroizing<[u8; 32]>>, TaskError> {
        let label = SecretScope::identity_key_label(target, key_id);
        let Some(bytes) = self
            .seam()
            .get_secret_protected(&self.scope(), &label, password)
            .map_err(protected_flavored)?
        else {
            return Ok(None);
        };
        let key = identity_key_from_bytes(bytes.expose_secret())?;
        Ok(Some(Zeroizing::new(key)))
    }

    /// Store every `(target, key_id) → raw 32 bytes` pair. Used by the
    /// migration after `KeyStorage::take_plaintext_for_vault` — call this
    /// BEFORE rewriting the QI blob (vault-first ordering).
    pub fn store_all(&self, keys: &[VaultBoundKey]) -> Result<(), TaskError> {
        for ((target, key_id), bytes) in keys {
            self.store(target, *key_id, bytes)?;
        }
        Ok(())
    }

    /// Read one identity key's raw 32 bytes, or `None` if absent. Wrapped in
    /// [`Zeroizing`] so it wipes on drop.
    pub fn get(
        &self,
        target: &PrivateKeyTarget,
        key_id: KeyID,
    ) -> Result<Option<Zeroizing<[u8; 32]>>, TaskError> {
        let label = SecretScope::identity_key_label(target, key_id);
        let Some(bytes) = self
            .seam()
            .get_secret(&self.scope(), &label)
            .map_err(identity_flavored)?
        else {
            return Ok(None);
        };
        let key = identity_key_from_bytes(bytes.expose_secret())?;
        Ok(Some(Zeroizing::new(key)))
    }

    /// Idempotent delete of one identity key.
    pub fn delete(&self, target: &PrivateKeyTarget, key_id: KeyID) -> Result<(), TaskError> {
        let label = SecretScope::identity_key_label(target, key_id);
        self.seam()
            .delete_secret(&self.scope(), &label)
            .map_err(identity_flavored)
    }

    /// Delete every `(target, key_id)` listed. Idempotent. Used on identity
    /// removal (`purge_identity_scope`) to leave no orphaned raw secret.
    pub fn delete_all(
        &self,
        keys: impl IntoIterator<Item = (PrivateKeyTarget, KeyID)>,
    ) -> Result<(), TaskError> {
        for (target, key_id) in keys {
            self.delete(&target, key_id)?;
        }
        Ok(())
    }
}

/// Re-flavor a generic seam error as the identity-key-domain variant so a vault
/// failure on an identity key surfaces with identity-specific banner copy. Any
/// non-`SecretSeam` error passes through unchanged.
pub(crate) fn identity_flavored(e: TaskError) -> TaskError {
    match e {
        TaskError::SecretSeam { source } => TaskError::IdentityKeyVault { source },
        other => other,
    }
}

/// Like [`identity_flavored`], but maps a wrong-password unseal to the typed
/// [`TaskError::IdentityKeyPassphraseIncorrect`] (no oracle) so the opt-out
/// migration can surface a clean "that password is not correct" rather than a
/// generic storage error. Any other seam error keeps the identity-vault flavor.
fn protected_flavored(e: TaskError) -> TaskError {
    match e {
        TaskError::SecretSeam { source } if matches!(*source, SecretStoreError::WrongPassword) => {
            TaskError::IdentityKeyPassphraseIncorrect
        }
        other => identity_flavored(other),
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

    /// Store/get/delete round-trip for one identity key through the seam.
    #[test]
    fn store_get_delete_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0x11u8; 32]);
        let key = [0xAB; 32];

        view.store(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 3, &key)
            .expect("store");
        let got = view
            .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 3)
            .expect("get")
            .expect("present");
        assert_eq!(*got, key);

        view.delete(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 3)
            .expect("delete");
        assert!(
            view.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 3)
                .expect("get after delete")
                .is_none()
        );
        // Idempotent delete.
        view.delete(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 3)
            .expect("delete twice");
    }

    /// Distinct targets and identities do not collide.
    #[test]
    fn scopes_and_targets_do_not_collide() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let a = IdentityKeyView::new(&store, [0xA1u8; 32]);
        let b = IdentityKeyView::new(&store, [0xB2u8; 32]);

        a.store(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0, &[0x01; 32])
            .unwrap();
        a.store(&PrivateKeyTarget::PrivateKeyOnVoterIdentity, 0, &[0x02; 32])
            .unwrap();
        b.store(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0, &[0x03; 32])
            .unwrap();

        assert_eq!(
            *a.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0)
                .unwrap()
                .unwrap(),
            [0x01; 32]
        );
        assert_eq!(
            *a.get(&PrivateKeyTarget::PrivateKeyOnVoterIdentity, 0)
                .unwrap()
                .unwrap(),
            [0x02; 32],
            "distinct targets under one identity do not collide"
        );
        assert_eq!(
            *b.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0)
                .unwrap()
                .unwrap(),
            [0x03; 32],
            "distinct identity scopes do not collide"
        );
    }

    /// `store_all` / `delete_all` operate over the migration's bound-key list.
    #[test]
    fn store_all_then_delete_all() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0xCC; 32]);
        let bound: Vec<VaultBoundKey> = vec![
            (
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, 1),
                Zeroizing::new([0x10; 32]),
            ),
            (
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, 2),
                Zeroizing::new([0x20; 32]),
            ),
        ];
        view.store_all(&bound).expect("store_all");
        assert!(
            view.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 1)
                .unwrap()
                .is_some()
        );

        view.delete_all([
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, 1),
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, 2),
        ])
        .expect("delete_all");
        assert!(
            view.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 2)
                .unwrap()
                .is_none()
        );
    }

    /// Opt-in store/read: a key sealed Tier-2 reads back with the
    /// password (`get_protected`), the label reports `Protected`, and a
    /// keyless `get` (Tier-1 read of a protected value) fails rather than
    /// leaking — the seam refuses the implicit downgrade.
    #[test]
    fn protected_store_get_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0xD1u8; 32]);
        let key = [0xAB; 32];
        let pw = SecretString::new("identity-object-passwd");

        view.store_protected(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 5, &key, &pw)
            .expect("store_protected");
        assert_eq!(
            view.scheme(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 5)
                .unwrap(),
            SecretScheme::Protected,
        );
        let got = view
            .get_protected(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 5, &pw)
            .expect("get_protected")
            .expect("present");
        assert_eq!(*got, key);
        // Keyless read of a protected value fails (no silent downgrade).
        assert!(
            view.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 5)
                .is_err(),
            "a keyless read of a protected identity key must fail"
        );
    }

    /// A wrong password yields the typed `IdentityKeyPassphraseIncorrect` (no
    /// oracle), not a storage error.
    #[test]
    fn protected_get_wrong_password_is_typed() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0xD2u8; 32]);
        view.store_protected(
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &[0x01; 32],
            &SecretString::new("right-password-here"),
        )
        .expect("store_protected");
        let err = view
            .get_protected(
                &PrivateKeyTarget::PrivateKeyOnMainIdentity,
                0,
                &SecretString::new("wrong-password-here"),
            )
            .expect_err("wrong password");
        assert!(
            matches!(err, TaskError::IdentityKeyPassphraseIncorrect),
            "expected IdentityKeyPassphraseIncorrect, got {err:?}"
        );
    }

    /// Opt-in is an in-place upsert at the SAME label: a Tier-1 raw key
    /// overwritten by `store_protected` flips the label's scheme to `Protected`
    /// with no second key — the design's no-blob-rewrite, no-orphan property.
    #[test]
    fn opt_in_upsert_replaces_tier1_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0xD3u8; 32]);
        let key = [0x42; 32];

        view.store(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 3, &key)
            .expect("tier-1 store");
        assert_eq!(
            view.scheme(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 3)
                .unwrap(),
            SecretScheme::Unprotected,
        );

        let pw = SecretString::new("seal-this-identity-pw");
        view.store_protected(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 3, &key, &pw)
            .expect("opt-in upsert");
        assert_eq!(
            view.scheme(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 3)
                .unwrap(),
            SecretScheme::Protected,
            "in-place Tier-1→Tier-2 upsert flips the scheme",
        );
        assert_eq!(
            *view
                .get_protected(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 3, &pw)
                .unwrap()
                .unwrap(),
            key,
        );
    }

    /// R2 downgrade guard: the keyless `store` refuses to overwrite a
    /// `Protected` label (it would silently strip protection), while the
    /// deliberate `store_unprotected` opt-out IS allowed to downgrade.
    #[test]
    fn keyless_store_refuses_to_downgrade_protected_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0xD4u8; 32]);
        let pw = SecretString::new("protect-then-attack-pw");
        view.store_protected(
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &[0x11; 32],
            &pw,
        )
        .expect("seal tier-2");

        // The guarded keyless path is refused — protection is NOT stripped.
        let err = view
            .store(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0, &[0x22; 32])
            .expect_err("keyless write over Protected must be refused");
        assert!(
            matches!(err, TaskError::IdentityKeyProtectionDowngrade),
            "expected IdentityKeyProtectionDowngrade, got {err:?}"
        );
        assert_eq!(
            view.scheme(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0)
                .unwrap(),
            SecretScheme::Protected,
            "the refused write left the key protected",
        );

        // The deliberate opt-out downgrade IS allowed.
        view.store_unprotected(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0, &[0x33; 32])
            .expect("intentional downgrade");
        assert_eq!(
            view.scheme(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0)
                .unwrap(),
            SecretScheme::Unprotected,
            "store_unprotected performs the intended Tier-2→Tier-1 downgrade",
        );
        assert_eq!(
            *view
                .get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 0)
                .unwrap()
                .unwrap(),
            [0x33; 32],
        );
    }

    /// `store_all` is unchanged for fresh/unprotected identities (the guard
    /// only fires over a `Protected` label) — the steady-state migration write
    /// keeps working byte-for-byte.
    #[test]
    fn store_all_unaffected_for_unprotected_identity() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0xD5u8; 32]);
        let bound: Vec<VaultBoundKey> = vec![(
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, 9),
            Zeroizing::new([0x55; 32]),
        )];
        view.store_all(&bound).expect("store_all on fresh identity");
        assert_eq!(
            view.scheme(&PrivateKeyTarget::PrivateKeyOnMainIdentity, 9)
                .unwrap(),
            SecretScheme::Unprotected,
        );
    }
}
