//! Explicit conversion of legacy resident identity keys during storage preparation.

use super::*;

impl AppContext {
    /// Migrate stored identity keys under each identity's record lock.
    /// Failed secret writes leave the legacy blob intact for the next preparation.
    pub(crate) fn migrate_local_identity_keys_to_vault(&self) -> Result<(), TaskError> {
        let kv = self.det_kv()?;
        for id in load_identity_index(&kv)? {
            let lock = self.identity_record_lock(Identifier::from(id));
            let _guard = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(mut stored): Option<StoredQualifiedIdentity> = kv
                .get(DetScope::Identity(&id), IDENTITY_KEY)
                .map_err(identity_err)?
            else {
                continue;
            };
            let mut qi = decode_stored_identity(&stored.qi_bytes, self.network)?;
            migrate_keystore_to_vault(&self.secret_store, &id, &mut qi, |migrated| {
                stored.qi_bytes = migrated.to_bytes();
                kv.put(DetScope::Identity(&id), IDENTITY_KEY, &stored)
                    .map_err(identity_err)
            });
        }
        Ok(())
    }
}

/// Outcome of [`migrate_keystore_to_vault`], so callers/tests can assert what
/// happened without re-inspecting the blob.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum KeystoreMigration {
    /// No plaintext keys to migrate — `qi` was untouched.
    Nothing,
    /// The vault write failed; `qi` was restored to its resident plaintext and
    /// the blob was NOT persisted (next startup retries — no key loss).
    VaultWriteFailed,
    /// `n` keys moved to the vault and `qi` rewritten to `InVault` placeholders.
    Migrated(usize),
    /// The identity is password-protected, so a resident plaintext key
    /// was NOT migrated to a keyless vault entry. `qi` keeps its resident key (it
    /// still signs this session) and nothing is persisted; the add-key path seals
    /// new keys Tier-2 explicitly.
    ProtectedSkipped,
}

/// Move legacy plaintext keys to the vault before rewriting the blob.
/// Keep the original blob on failure so a later startup can retry safely.
pub(super) fn migrate_keystore_to_vault(
    secret_store: &Arc<platform_wallet_storage::secrets::SecretStore>,
    id: &[u8; 32],
    qi: &mut QualifiedIdentity,
    persist: impl FnOnce(&QualifiedIdentity) -> std::result::Result<(), TaskError>,
) -> KeystoreMigration {
    // Probe before cloning: the steady-state (already all-`InVault`) case must
    // not pay for a full `KeyStorage` clone — that clone exists only to restore
    // the resident plaintext on a vault-write failure.
    if !qi.private_keys.has_plaintext_for_vault() {
        return KeystoreMigration::Nothing;
    }
    // Fail-closed: never migrate a protected identity's resident
    // plaintext to a KEYLESS vault entry — that would silently strip protection
    // off a new key. Leave it resident (it still signs this session) and persist
    // nothing; the add-key path seals new keys Tier-2 under the identity password.
    if find_protected_identity_key_scope(secret_store, id, qi).is_some() {
        tracing::warn!(
            target = "context::identity_db",
            identity = %hex::encode(id),
            "Skipped keyless migration of a resident key on a password-protected identity",
        );
        return KeystoreMigration::ProtectedSkipped;
    }
    let mut before = qi.private_keys.clone();
    let taken = qi.private_keys.take_plaintext_for_vault();
    let view = crate::wallet_backend::IdentityKeyView::new(secret_store, *id);
    if let Err(e) = view.store_all(&taken) {
        qi.private_keys = before;
        tracing::warn!(
            target = "context::identity_db",
            identity = %hex::encode(id),
            error = ?e,
            "Identity-key vault migration deferred (vault write failed)",
        );
        return KeystoreMigration::VaultWriteFailed;
    }
    let migrated = taken.len();
    // The migrated plaintext now lives only in the vault; drop the `taken` copy
    // (it zeroizes on drop) so its key bytes do not linger across the DB write.
    drop(taken);
    // The vault write succeeded — the rollback clone is no longer
    // needed. Zeroize its plaintext bytes (Clear/AlwaysClear) before it drops
    // so no identity private key lingers in freed heap.
    let _ = before.take_plaintext_for_vault();
    if let Err(e) = persist(qi) {
        tracing::warn!(
            target = "context::identity_db",
            identity = %hex::encode(id),
            error = ?e,
            "Identity-key blob rewrite deferred after vault migration",
        );
    } else {
        tracing::info!(
            target = "context::identity_db",
            identity = %hex::encode(id),
            migrated,
            "Migrated identity keys to the secret vault",
        );
    }
    KeystoreMigration::Migrated(migrated)
}
