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
use platform_wallet_storage::secrets::{SecretBytes, SecretStore, WalletId as SecretWalletId};
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::qualified_identity::encrypted_key_storage::VaultBoundKey;
use crate::wallet_backend::secret_prompt::SecretScope;
use crate::wallet_backend::secret_seam::SecretSeam;

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

    /// Store one identity key's raw 32 bytes, overwriting any prior value.
    pub fn store(
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
        let key: [u8; 32] = bytes.expose_secret().try_into().map_err(|_| {
            tracing::warn!(
                target = "wallet_backend::identity_key_store",
                blob_len = bytes.expose_secret().len(),
                "Stored identity key has wrong length",
            );
            TaskError::SecretDecryptFailed
        })?;
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
fn identity_flavored(e: TaskError) -> TaskError {
    match e {
        TaskError::SecretSeam { source } => TaskError::IdentityKeyVault { source },
        other => other,
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
}
