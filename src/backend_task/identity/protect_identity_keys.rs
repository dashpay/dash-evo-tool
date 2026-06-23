//! SEC-001 opt-in / opt-out migrations: seal an identity's keys under one
//! per-identity password (Tier-2) or revert them to keyless (Tier-1).
//!
//! Both operate over the identity's existing per-key vault labels, in place
//! (same-label upsert), so there is no second label to orphan and the classic
//! "vault-write BEFORE sidecar" crash-safety ordering collapses to "vault first,
//! then the cosmetic hint sidecar." Crash mid-iteration leaves a recoverable
//! mix (some keys Tier-2, some Tier-1) — every label always holds a complete,
//! readable secret, and re-running with the same password finishes the job
//! (idempotent: an already-converted key is skipped).

use std::collections::BTreeSet;

use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use platform_wallet_storage::secrets::SecretString;

use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::qualified_identity::identity_meta::IdentityMeta;
use crate::model::secret::Secret;
use crate::wallet_backend::IdentityKeyView;
use crate::wallet_backend::secret_seam::SecretScheme;

/// Every `(target, key_id)` of an identity, the iteration unit for both
/// migrations.
type IdentityKeySet = BTreeSet<(PrivateKeyTarget, KeyID)>;

impl AppContext {
    /// SEC-001 opt-in: seal this identity's keyless vault keys Tier-2 under one
    /// per-identity `password`, then record `hint` for the prompt copy.
    pub(super) fn protect_identity_keys(
        &self,
        identity_id: Identifier,
        password: Secret,
        hint: Option<String>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let qi = self
            .get_identity_by_id(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        let backend = self.wallet_backend()?;
        let id = qi.identity.id().to_buffer();
        let keys = qi.private_keys.keys_set();
        let view = IdentityKeyView::new(backend.secret_store(), id);
        let pw = SecretString::new(password.expose_secret());

        // Vault first (the funds-/protection-safe part).
        let count = seal_identity_keys(&view, &keys, &pw)?;

        // Then the cosmetic hint sidecar — best-effort: the keys are already
        // protected, so a sidecar write failure must not report the opt-in as
        // failed (it would only cost the prompt hint).
        let hint = hint.filter(|h| !h.trim().is_empty());
        if let Err(e) = backend.identity_meta().set(
            self.network,
            &id,
            &IdentityMeta {
                password_hint: hint,
            },
        ) {
            tracing::warn!(
                target = "backend_task::identity::protect_identity_keys",
                identity = %identity_id,
                error = ?e,
                "Identity keys sealed, but recording the password hint failed",
            );
        }

        tracing::info!(
            target = "backend_task::identity::protect_identity_keys",
            identity = %identity_id,
            count,
            "Sealed identity keys under a per-identity password",
        );
        Ok(BackendTaskSuccessResult::IdentityKeysProtected { identity_id, count })
    }

    /// SEC-001 opt-out: revert this identity's password-protected vault keys to
    /// keyless (Tier-1) after verifying `password`, then drop the hint sidecar.
    pub(super) fn unprotect_identity_keys(
        &self,
        identity_id: Identifier,
        password: Secret,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let qi = self
            .get_identity_by_id(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        let backend = self.wallet_backend()?;
        let id = qi.identity.id().to_buffer();
        let keys = qi.private_keys.keys_set();
        let view = IdentityKeyView::new(backend.secret_store(), id);
        let pw = SecretString::new(password.expose_secret());

        // Vault downgrade first (a wrong password aborts before any key is
        // touched, since all keys share the one per-identity password).
        let reverted = unseal_identity_keys(&view, &keys, &pw)?;

        // Then drop the now-irrelevant hint sidecar — best-effort: the keys are
        // already keyless, so a stale hint is harmless and must not fail opt-out.
        if let Err(e) = backend.identity_meta().delete(self.network, &id) {
            tracing::warn!(
                target = "backend_task::identity::protect_identity_keys",
                identity = %identity_id,
                error = ?e,
                "Identity protection removed, but deleting the password hint failed",
            );
        }

        tracing::info!(
            target = "backend_task::identity::protect_identity_keys",
            identity = %identity_id,
            reverted,
            "Removed per-identity password protection",
        );
        Ok(BackendTaskSuccessResult::IdentityKeysUnprotected { identity_id })
    }
}

/// Seal every keyless (`Unprotected`) vault key in `keys` Tier-2 under
/// `password`, returning how many were newly sealed. Idempotent: an
/// already-`Protected` key is skipped, and an `Absent` key (not vault-stored —
/// a wallet-derived or resident-plaintext key, protected by other means) is
/// skipped. Crash-safe: the same-label upsert never loses a key, so a re-run
/// finishes a partial migration.
fn seal_identity_keys(
    view: &IdentityKeyView<'_>,
    keys: &IdentityKeySet,
    password: &SecretString,
) -> Result<usize, TaskError> {
    let mut sealed = 0usize;
    for (target, key_id) in keys {
        match view.scheme(target, *key_id)? {
            SecretScheme::Unprotected => {
                let raw = view
                    .get(target, *key_id)?
                    .ok_or(TaskError::IdentityKeyMissing)?;
                view.store_protected(target, *key_id, &raw, password)?;
                sealed += 1;
            }
            SecretScheme::Protected | SecretScheme::Absent => {}
        }
    }
    Ok(sealed)
}

/// Revert every `Protected` vault key in `keys` to keyless (Tier-1), verifying
/// `password`, returning how many were reverted. Idempotent: an already-keyless
/// (`Unprotected`) or `Absent` key is skipped. Crash-safe: the in-place
/// downgrade never loses a key, so a re-run finishes a partial opt-out.
fn unseal_identity_keys(
    view: &IdentityKeyView<'_>,
    keys: &IdentityKeySet,
    password: &SecretString,
) -> Result<usize, TaskError> {
    let mut reverted = 0usize;
    for (target, key_id) in keys {
        if view.scheme(target, *key_id)? == SecretScheme::Protected {
            let raw = view
                .get_protected(target, *key_id, password)?
                .ok_or(TaskError::IdentityKeyMissing)?;
            view.store_unprotected(target, *key_id, &raw)?;
            reverted += 1;
        }
    }
    Ok(reverted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use platform_wallet_storage::secrets::SecretStore;
    use zeroize::Zeroizing;

    use crate::wallet_backend::single_key::open_secret_store;

    fn fresh_store(dir: &std::path::Path) -> Arc<SecretStore> {
        Arc::new(open_secret_store(&dir.join("secrets.pwsvault")).expect("open vault"))
    }

    fn key_set(pairs: &[(PrivateKeyTarget, KeyID)]) -> IdentityKeySet {
        pairs.iter().cloned().collect()
    }

    const M: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
    const V: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnVoterIdentity;

    /// Opt-in seals every keyless key Tier-2, the round-trips back under the
    /// password, and a re-run seals nothing (idempotent).
    #[test]
    fn seal_then_idempotent_rerun() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0x01u8; 32]);
        view.store(&M, 0, &[0xA0; 32]).unwrap();
        view.store(&M, 1, &[0xA1; 32]).unwrap();
        view.store(&V, 0, &[0xB0; 32]).unwrap();
        let keys = key_set(&[(M, 0), (M, 1), (V, 0)]);
        let pw = SecretString::new("one-identity-password");

        let sealed = seal_identity_keys(&view, &keys, &pw).unwrap();
        assert_eq!(sealed, 3, "all three keyless keys sealed");
        for (t, k) in &keys {
            assert_eq!(view.scheme(t, *k).unwrap(), SecretScheme::Protected);
        }
        assert_eq!(
            *view.get_protected(&M, 1, &pw).unwrap().unwrap(),
            [0xA1; 32],
            "sealed key round-trips under the password",
        );

        // Re-run seals nothing — already protected.
        assert_eq!(seal_identity_keys(&view, &keys, &pw).unwrap(), 0);
    }

    /// Opt-out reverts every protected key to keyless and a re-run reverts
    /// nothing (idempotent); the exact bytes survive the round trip.
    #[test]
    fn unseal_then_idempotent_rerun() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0x02u8; 32]);
        let pw = SecretString::new("one-identity-password");
        view.store_protected(&M, 0, &[0xC0; 32], &pw).unwrap();
        view.store_protected(&M, 1, &[0xC1; 32], &pw).unwrap();
        let keys = key_set(&[(M, 0), (M, 1)]);

        let reverted = unseal_identity_keys(&view, &keys, &pw).unwrap();
        assert_eq!(reverted, 2);
        assert_eq!(view.scheme(&M, 0).unwrap(), SecretScheme::Unprotected);
        assert_eq!(*view.get(&M, 1).unwrap().unwrap(), [0xC1; 32]);

        assert_eq!(unseal_identity_keys(&view, &keys, &pw).unwrap(), 0);
    }

    /// A wrong opt-out password aborts on the FIRST protected key, before any
    /// key is downgraded — no partial, silent strip.
    #[test]
    fn unseal_wrong_password_aborts_without_downgrade() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0x03u8; 32]);
        let pw = SecretString::new("the-right-password-aa");
        view.store_protected(&M, 0, &[0xD0; 32], &pw).unwrap();
        view.store_protected(&M, 1, &[0xD1; 32], &pw).unwrap();
        let keys = key_set(&[(M, 0), (M, 1)]);

        let err = unseal_identity_keys(&view, &keys, &SecretString::new("wrong-password-bbbb"))
            .expect_err("wrong password");
        assert!(
            matches!(err, TaskError::IdentityKeyPassphraseIncorrect),
            "expected IdentityKeyPassphraseIncorrect, got {err:?}"
        );
        // Both keys remain protected — nothing was downgraded.
        assert_eq!(view.scheme(&M, 0).unwrap(), SecretScheme::Protected);
        assert_eq!(view.scheme(&M, 1).unwrap(), SecretScheme::Protected);
    }

    /// A partial-crash mix (some keys Tier-2, some Tier-1) re-runs to a clean,
    /// fully-protected state — the same-label upsert never loses a key.
    #[test]
    fn seal_finishes_a_partial_mix() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0x04u8; 32]);
        let pw = SecretString::new("one-identity-password");
        // Simulate a crash mid opt-in: key 0 sealed, key 1 still keyless.
        view.store_protected(&M, 0, &[0xE0; 32], &pw).unwrap();
        view.store(&M, 1, &[0xE1; 32]).unwrap();
        let keys = key_set(&[(M, 0), (M, 1)]);

        let sealed = seal_identity_keys(&view, &keys, &pw).unwrap();
        assert_eq!(sealed, 1, "only the still-keyless key is sealed");
        assert_eq!(view.scheme(&M, 0).unwrap(), SecretScheme::Protected);
        assert_eq!(view.scheme(&M, 1).unwrap(), SecretScheme::Protected);
        assert_eq!(
            *view.get_protected(&M, 0, &pw).unwrap().unwrap(),
            [0xE0; 32]
        );
        assert_eq!(
            *view.get_protected(&M, 1, &pw).unwrap().unwrap(),
            [0xE1; 32]
        );
    }

    /// `Absent` keys (not vault-stored — wallet-derived/resident) are skipped by
    /// both directions without error.
    #[test]
    fn absent_keys_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0x05u8; 32]);
        let pw = SecretString::new("one-identity-password");
        let keys = key_set(&[(M, 7), (V, 9)]); // nothing stored under these
        assert_eq!(seal_identity_keys(&view, &keys, &pw).unwrap(), 0);
        assert_eq!(unseal_identity_keys(&view, &keys, &pw).unwrap(), 0);
    }

    /// A full round trip with a Zeroizing-backed raw key proves the bytes are
    /// preserved through seal → unseal.
    #[test]
    fn seal_unseal_round_trip_preserves_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0x06u8; 32]);
        let raw = Zeroizing::new([0x5Au8; 32]);
        view.store(&M, 0, &raw).unwrap();
        let keys = key_set(&[(M, 0)]);
        let pw = SecretString::new("round-trip-password-x");

        seal_identity_keys(&view, &keys, &pw).unwrap();
        unseal_identity_keys(&view, &keys, &pw).unwrap();
        assert_eq!(*view.get(&M, 0).unwrap().unwrap(), *raw);
    }
}
