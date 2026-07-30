//! Identity key password protection opt-in / opt-out migrations: seal an identity's keys under one
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
use crate::model::identity_key_protection::validate_protection_password;
use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
use crate::model::qualified_identity::identity_meta::IdentityMeta;
use crate::model::qualified_identity::{PrivateKeyTarget, QualifiedIdentity};
use crate::model::secret::Secret;
use crate::wallet_backend::IdentityKeyView;
use crate::wallet_backend::secret_seam::SecretScheme;

/// Every `(target, key_id)` of an identity, the iteration unit for both
/// migrations.
type IdentityKeySet = BTreeSet<(PrivateKeyTarget, KeyID)>;

impl AppContext {
    /// Opt-in: seal this identity's keyless vault keys Tier-2 under one
    /// per-identity `password`, then record `hint` for the prompt copy.
    ///
    /// Holds this identity's
    /// [`identity_record_lock`](AppContext::identity_record_lock) for the whole
    /// migration: a tier change decides which password every one of the
    /// identity's keys opens under, so it must not interleave with another
    /// writer that seals keys of its own — see
    /// [`recover_legacy_identity_data`](AppContext::recover_legacy_identity_data).
    pub(super) fn protect_identity_keys(
        &self,
        identity_id: Identifier,
        password: Secret,
        hint: Option<String>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Backend = authoritative validation: re-enforce the password
        // policy here, not only in the UI, so a future MCP/CLI caller cannot
        // seal under a too-short password.
        validate_protection_password(&password)?;

        let lock = self.identity_record_lock(identity_id);
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let qi = self
            .get_identity_by_id(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;

        self.protect_loaded_identity_keys(&qi, &password, hint)
    }

    /// Seal an ALREADY-LOADED identity's keyless vault keys Tier-2 under one
    /// per-identity `password`, then record `hint`. Split from
    /// [`Self::protect_identity_keys`] so the fail-closed guard, the seal, and
    /// the success result are exercised on a real `qi` as the task runs them —
    /// proving the guard is wired into the protect path, not merely callable.
    fn protect_loaded_identity_keys(
        &self,
        qi: &QualifiedIdentity,
        password: &Secret,
        hint: Option<String>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let identity_id = qi.identity.id();

        // Fail-closed: any resident plaintext key left by an incomplete
        // get-path migration has an `Absent` label `seal_identity_keys` would
        // skip, so refuse here rather than emit a false-protected result.
        reject_resident_identity_plaintext(&qi.private_keys)?;

        let backend = self.wallet_backend()?;
        let id = identity_id.to_buffer();
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

    /// Opt-out: revert this identity's password-protected vault keys to
    /// keyless (Tier-1) after verifying `password`, then drop the hint sidecar.
    ///
    /// Holds this identity's
    /// [`identity_record_lock`](AppContext::identity_record_lock) for the same
    /// reason [`Self::protect_identity_keys`] does.
    pub(super) fn unprotect_identity_keys(
        &self,
        identity_id: Identifier,
        password: Secret,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let lock = self.identity_record_lock(identity_id);
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
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

/// Fail-closed guard for two of the three boundaries that seal an identity's
/// keys — the protect opt-in and the legacy-recovery merge, but *not* the
/// merge-load path (see the TODO at its seal in `load_identity.rs`): reject an
/// identity that still carries resident plaintext (`Clear`/`AlwaysClear`) keys
/// on disk. Such a
/// key means the eager load-path vault migration did not complete — its vault
/// write failed, or it was skipped on an already-protected identity — so the key
/// has no vault label and [`seal_identity_keys`] would silently skip its
/// `Absent` scheme and report a false success. Wallet-derived
/// (`AtWalletDerivationPath`) and already-vaulted (`InVault`) keys carry no
/// resident plaintext, so a legitimately keyless / wallet-derived identity is
/// never rejected.
///
/// Also rejects legacy `Encrypted` keys (decode-only, no current producer):
/// their vault scheme is also `Absent`, so the seal step would silently skip
/// them and issue a false-protected result. See [`KeyStorage::has_encrypted_legacy_keys`].
///
/// The two rejections carry DIFFERENT recovery actions, so they map to distinct
/// errors: resident plaintext is finished by the load-path migration on the next
/// launch ([`TaskError::IdentityKeyProtectionIncomplete`] → "close and reopen"),
/// whereas a legacy `Encrypted` key has no migration path
/// ([`TaskError::IdentityKeyProtectionLegacyFormat`] → "load the identity again").
/// Legacy keys are checked first: re-loading the identity also clears any
/// resident plaintext, so it is the single action that resolves both.
pub(super) fn reject_resident_identity_plaintext(
    private_keys: &KeyStorage,
) -> Result<(), TaskError> {
    if private_keys.has_encrypted_legacy_keys() {
        return Err(TaskError::IdentityKeyProtectionLegacyFormat);
    }
    if private_keys.has_plaintext_for_vault() {
        return Err(TaskError::IdentityKeyProtectionIncomplete);
    }
    Ok(())
}

/// Seal every keyless (`Unprotected`) vault key in `keys` Tier-2 under
/// `password`, returning how many were newly sealed. Idempotent: an
/// already-`Protected` key is skipped, and an `Absent` key (not vault-stored —
/// a wallet-derived or resident-plaintext key, protected by other means) is
/// skipped. Crash-safe: the same-label upsert never loses a key, so a re-run
/// finishes a partial migration.
///
/// At-rest residual (known): the in-place upsert replaces the value at
/// the label, but the PRE-opt-in keyless plaintext may persist in freed
/// filesystem blocks (atomic-rename/copy-on-write residue, filesystem-owned)
/// until those blocks are reused. This is a strict improvement over the keyless
/// default and matches the residual already accepted for the seed/single-key
/// Tier-2 re-wrap; secure-erase of freed blocks is out of this layer's control.
fn seal_identity_keys(
    view: &IdentityKeyView<'_>,
    keys: &IdentityKeySet,
    password: &SecretString,
) -> Result<usize, TaskError> {
    // One-password invariant: a Mixed-state "Finish protecting" re-run
    // (some keys already Tier-2 from a prior partial opt-in, some still keyless)
    // must not seal the remaining keys under a DIFFERENT password than the
    // existing ones. Verify the supplied password opens every already-`Protected`
    // key BEFORE mutating any label, so a mismatch returns up front with zero
    // state changes — the identity can never be split across two passwords.
    verify_existing_protection_password(view, keys, password)?;

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

/// Verify `password` opens EVERY already-`Protected` key in `keys`, before any
/// vault mutation. Both migrations call this up front so they are atomic
/// by construction: if `password` fails to open any protected key, the mismatch
/// surfaces from `get_protected` as [`TaskError::IdentityKeyPassphraseIncorrect`]
/// (no oracle) with zero state changes — opt-in can't seal the rest under a
/// second password, and opt-out can't strip a prefix before aborting. Keyless
/// (`Unprotected`) and `Absent` keys impose no password constraint and are skipped.
fn verify_existing_protection_password(
    view: &IdentityKeyView<'_>,
    keys: &IdentityKeySet,
    password: &SecretString,
) -> Result<(), TaskError> {
    for (target, key_id) in keys {
        if view.scheme(target, *key_id)? == SecretScheme::Protected {
            view.get_protected(target, *key_id, password)?
                .ok_or(TaskError::IdentityKeyMissing)?;
        }
    }
    Ok(())
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
    // Atomic opt-out: prove `password` opens EVERY `Protected` key
    // BEFORE downgrading any label (mirrors the opt-in preflight), so a password
    // that opens only a prefix can't leave that prefix stripped. Mismatch → no-op.
    verify_existing_protection_password(view, keys, password)?;

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

    use std::collections::BTreeMap;

    use platform_wallet_storage::secrets::SecretStore;
    use zeroize::Zeroizing;

    use crate::model::qualified_identity::encrypted_key_storage::{
        PrivateKeyData, WalletDerivationPath,
    };
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
    use crate::wallet_backend::single_key::open_secret_store;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
    use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Identifier, IdentityPublicKey};

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

    /// Atomic opt-out (CWE-460): on a Mixed-password identity — key 0
    /// sealed under password A, key 1 under password B — an opt-out with
    /// password A must NOT downgrade the key it CAN open before aborting on the
    /// one it cannot. The one-password invariant forbids this state, but a
    /// tampered or legacy vault could still present it, so opt-out must be
    /// all-or-nothing by construction. The all-keys preflight rejects up front
    /// with `IdentityKeyPassphraseIncorrect`, leaving BOTH keys protected — no
    /// silent partial protection downgrade. Without the preflight, key 0 (which
    /// password A opens, and which sorts first) would be stripped to keyless
    /// plaintext while key 1 stayed sealed.
    #[test]
    fn unseal_mixed_password_aborts_without_partial_downgrade() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0x08u8; 32]);
        let pw_a = SecretString::new("password-for-key-zero");
        let pw_b = SecretString::new("password-for-key-one-");
        // (M, 0) sorts before (M, 1): a downgrade-as-you-go loop would reach
        // key 0 first and strip it before failing the password check on key 1.
        view.store_protected(&M, 0, &[0x80; 32], &pw_a).unwrap();
        view.store_protected(&M, 1, &[0x81; 32], &pw_b).unwrap();
        let keys = key_set(&[(M, 0), (M, 1)]);

        let err = unseal_identity_keys(&view, &keys, &pw_a)
            .expect_err("password A does not open key 1 — opt-out must abort");
        assert!(
            matches!(err, TaskError::IdentityKeyPassphraseIncorrect),
            "expected IdentityKeyPassphraseIncorrect, got {err:?}"
        );
        // Neither key was downgraded: key 0 — which password A COULD open — is
        // still Protected because the preflight ran before any mutation.
        assert_eq!(view.scheme(&M, 0).unwrap(), SecretScheme::Protected);
        assert_eq!(view.scheme(&M, 1).unwrap(), SecretScheme::Protected);
        // The sealed bytes are intact under each key's original password.
        assert_eq!(
            *view.get_protected(&M, 0, &pw_a).unwrap().unwrap(),
            [0x80; 32]
        );
        assert_eq!(
            *view.get_protected(&M, 1, &pw_b).unwrap().unwrap(),
            [0x81; 32]
        );
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

    /// One-password invariant: a Mixed-state "Finish protecting" re-run
    /// supplied with a DIFFERENT password than the already-sealed key is
    /// rejected up front with `IdentityKeyPassphraseIncorrect`, leaving every
    /// key untouched — the identity can never be split across two passwords.
    /// Re-running with the ORIGINAL password finishes the job, sealing all keys
    /// under that one password.
    #[test]
    fn seal_rejects_mismatched_password_on_mixed_identity() {
        let dir = tempfile::tempdir().unwrap();
        let store = fresh_store(dir.path());
        let view = IdentityKeyView::new(&store, [0x07u8; 32]);
        let original = SecretString::new("the-original-password");
        // Crash mid opt-in: key 0 sealed under the original password, key 1
        // still keyless.
        view.store_protected(&M, 0, &[0xF0; 32], &original).unwrap();
        view.store(&M, 1, &[0xF1; 32]).unwrap();
        let keys = key_set(&[(M, 0), (M, 1)]);

        // A re-run with a DIFFERENT password is rejected before any sealing.
        let err = seal_identity_keys(&view, &keys, &SecretString::new("a-different-password"))
            .expect_err("mismatched password must be rejected");
        assert!(
            matches!(err, TaskError::IdentityKeyPassphraseIncorrect),
            "expected IdentityKeyPassphraseIncorrect, got {err:?}"
        );
        // Nothing changed: key 0 still Protected (under the original password),
        // key 1 still keyless — no split, no partial seal.
        assert_eq!(view.scheme(&M, 0).unwrap(), SecretScheme::Protected);
        assert_eq!(view.scheme(&M, 1).unwrap(), SecretScheme::Unprotected);

        // Re-running with the ORIGINAL password finishes the job: both keys end
        // sealed under the one per-identity password.
        let sealed = seal_identity_keys(&view, &keys, &original).unwrap();
        assert_eq!(sealed, 1, "only the still-keyless key is sealed");
        assert_eq!(view.scheme(&M, 1).unwrap(), SecretScheme::Protected);
        assert_eq!(
            *view.get_protected(&M, 0, &original).unwrap().unwrap(),
            [0xF0; 32]
        );
        assert_eq!(
            *view.get_protected(&M, 1, &original).unwrap().unwrap(),
            [0xF1; 32]
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

    /// The backend enforces the password policy — a too-short password
    /// is rejected with the typed error before any sealing, regardless of the UI.
    #[test]
    fn weak_password_is_rejected_by_backend_policy() {
        let err = validate_protection_password(&Secret::new("short")).expect_err("too short");
        assert!(
            matches!(err, TaskError::SingleKeyPassphraseTooShort { .. }),
            "expected SingleKeyPassphraseTooShort, got {err:?}"
        );
        // A policy-compliant password passes.
        validate_protection_password(&Secret::new("long-enough-password"))
            .expect("compliant password accepted");
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

    /// A `KeyStorage` holding a single resident-plaintext `Clear` key — the state
    /// the load-path vault migration leaves behind when its vault write failed or
    /// was skipped, so the key's vault label is `Absent`.
    fn ks_with_resident_clear() -> KeyStorage {
        let pv = PlatformVersion::latest();
        let mut ks = KeyStorage::default();
        let k = IdentityPublicKey::random_key(1, Some(1), pv);
        ks.private_keys.insert(
            (M, k.id()),
            (
                QualifiedIdentityPublicKey::from(k),
                PrivateKeyData::Clear([0xCC; 32]),
            ),
        );
        ks
    }

    /// A `KeyStorage` holding a single legacy `Encrypted` key — the decode-only
    /// variant an old DET version left behind. Its vault scheme is `Absent` (no
    /// migration path), so the seal step would silently skip it.
    fn ks_with_encrypted_legacy() -> KeyStorage {
        let pv = PlatformVersion::latest();
        let mut ks = KeyStorage::default();
        let k = IdentityPublicKey::random_key(1, Some(1), pv);
        ks.private_keys.insert(
            (M, k.id()),
            (
                QualifiedIdentityPublicKey::from(k),
                PrivateKeyData::Encrypted(vec![0x33; 48]),
            ),
        );
        ks
    }

    /// A `KeyStorage` whose keys are all legitimately not-resident: one already
    /// vault-backed (`InVault`) and one wallet-derived (`AtWalletDerivationPath`,
    /// whose vault scheme is `Absent` by design, not by a failed migration).
    fn ks_invault_plus_wallet_derived() -> KeyStorage {
        let pv = PlatformVersion::latest();
        let mut ks = KeyStorage::default();
        let vaulted = IdentityPublicKey::random_key(1, Some(1), pv);
        ks.private_keys.insert(
            (M, vaulted.id()),
            (
                QualifiedIdentityPublicKey::from(vaulted),
                PrivateKeyData::InVault,
            ),
        );
        let derived = IdentityPublicKey::random_key(2, Some(2), pv);
        ks.private_keys.insert(
            (M, derived.id()),
            (
                QualifiedIdentityPublicKey::from(derived),
                PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                    wallet_seed_hash: [0x07; 32],
                    derivation_path: DerivationPath::from(vec![]),
                }),
            ),
        );
        ks
    }

    /// A keyless `QualifiedIdentity` with two resident-plaintext keys (`Clear`
    /// and `AlwaysClear`) plus one wallet-derived key — the normal opt-in shape
    /// after a fresh import.
    fn qi_clear_pair_plus_wallet_derived() -> QualifiedIdentity {
        let pv = PlatformVersion::latest();
        let mut ks = KeyStorage::default();
        let a = IdentityPublicKey::random_key(1, Some(1), pv);
        ks.private_keys.insert(
            (M, a.id()),
            (
                QualifiedIdentityPublicKey::from(a),
                PrivateKeyData::Clear([0xA0; 32]),
            ),
        );
        let b = IdentityPublicKey::random_key(2, Some(2), pv);
        ks.private_keys.insert(
            (M, b.id()),
            (
                QualifiedIdentityPublicKey::from(b),
                PrivateKeyData::AlwaysClear([0xB0; 32]),
            ),
        );
        let derived = IdentityPublicKey::random_key(3, Some(3), pv);
        ks.private_keys.insert(
            (M, derived.id()),
            (
                QualifiedIdentityPublicKey::from(derived),
                PrivateKeyData::AtWalletDerivationPath(WalletDerivationPath {
                    wallet_seed_hash: [0x07; 32],
                    derivation_path: DerivationPath::from(vec![]),
                }),
            ),
        );
        let identity =
            Identity::create_basic_identity(Identifier::default(), pv).expect("basic identity");
        QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: ks,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        }
    }

    /// Fail-closed: an identity still carrying a resident-plaintext key
    /// (the load-path vault migration did not move it, so its vault label is
    /// `Absent`) is rejected at the protect boundary rather than reported as
    /// protected — the false-`IdentityKeysProtected{count:0}` regression.
    #[test]
    fn protect_rejects_resident_plaintext_key() {
        let ks = ks_with_resident_clear();
        let err = reject_resident_identity_plaintext(&ks)
            .expect_err("resident plaintext must fail closed");
        assert!(
            matches!(err, TaskError::IdentityKeyProtectionIncomplete),
            "expected IdentityKeyProtectionIncomplete, got {err:?}"
        );
    }

    /// Fail-closed: an identity carrying a legacy `Encrypted` key (no
    /// migration path) is rejected with the dedicated
    /// [`TaskError::IdentityKeyProtectionLegacyFormat`] — NOT the resident-
    /// plaintext `IdentityKeyProtectionIncomplete` — so the user is told to load
    /// the identity again rather than uselessly close and reopen.
    #[test]
    fn protect_rejects_legacy_encrypted_key_with_distinct_error() {
        let ks = ks_with_encrypted_legacy();
        let err = reject_resident_identity_plaintext(&ks)
            .expect_err("legacy Encrypted key must fail closed");
        assert!(
            matches!(err, TaskError::IdentityKeyProtectionLegacyFormat),
            "expected IdentityKeyProtectionLegacyFormat, got {err:?}"
        );
    }

    /// No false positive: an identity whose keys are wallet-derived
    /// (`AtWalletDerivationPath`, legitimately `Absent`) or already vault-backed
    /// (`InVault`) carries no resident plaintext and is accepted — opt-in must
    /// not regress for normal identities.
    #[test]
    fn protect_accepts_wallet_derived_and_vaulted_keys() {
        let ks = ks_invault_plus_wallet_derived();
        reject_resident_identity_plaintext(&ks)
            .expect("wallet-derived / already-vaulted keys must not be rejected");
    }

    /// End-to-end no-false-positive: a normal keyless opt-in still succeeds. The
    /// insert migrates the two resident-plaintext keys into the keyless vault,
    /// `protect_identity_keys` passes the fail-closed guard, seals exactly those
    /// two keys Tier-2, and skips the wallet-derived (`Absent`) key.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protect_normal_opt_in_seals_vault_keys_and_skips_wallet_derived() {
        use crate::app::TaskResult;
        use crate::app_dir::ensure_env_file;
        use crate::context::connection_status::ConnectionStatus;
        use crate::database::test_helpers::create_database_at_path;
        use crate::utils::egui_mpsc::SenderAsync;
        use crate::utils::tasks::TaskManager;

        // Offline wired AppContext (no network I/O) so the secret store is a real,
        // writable vault the insert/opt-in paths can migrate into.
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("offline testnet AppContext::new");
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");

        let qi = qi_clear_pair_plus_wallet_derived();
        let identity_id = qi.identity.id();
        ctx.insert_local_qualified_identity(&qi, &None)
            .expect("insert identity (migrates resident plaintext into the keyless vault)");

        let result = ctx
            .protect_identity_keys(identity_id, Secret::new("one-identity-password"), None)
            .expect("normal opt-in must succeed, not fail closed");
        match result {
            BackendTaskSuccessResult::IdentityKeysProtected {
                identity_id: got,
                count,
            } => {
                assert_eq!(got, identity_id, "result reports the same identity");
                assert_eq!(
                    count, 2,
                    "both keyless vault keys sealed; the wallet-derived key skipped, not rejected",
                );
            }
            other => panic!("expected IdentityKeysProtected, got {other:?}"),
        }

        if let Ok(backend) = ctx.wallet_backend() {
            backend.shutdown().await;
        }
    }

    /// Wiring guard: the fail-closed check must be PLUGGED INTO the
    /// protect path, not merely callable in isolation. Drive the real post-load
    /// protect logic (`protect_loaded_identity_keys`, which `protect_identity_keys`
    /// runs after `get_identity_by_id`) on a `qi` carrying resident plaintext and
    /// assert it returns `IdentityKeyProtectionIncomplete` — NOT
    /// `Ok(IdentityKeysProtected{count:0})`. Deleting the guard line makes this
    /// test fail: the seal then skips the vault-`Absent` keys and reports a false
    /// success.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protect_loaded_identity_with_resident_plaintext_fails_closed() {
        use crate::app::TaskResult;
        use crate::app_dir::ensure_env_file;
        use crate::context::connection_status::ConnectionStatus;
        use crate::database::test_helpers::create_database_at_path;
        use crate::utils::egui_mpsc::SenderAsync;
        use crate::utils::tasks::TaskManager;

        // Offline wired AppContext (backend wired so the post-guard seal path is
        // real — with the guard deleted it reaches the seal and returns the false
        // `count:0`, which is exactly what this test must catch).
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("offline testnet AppContext::new");
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");

        // A loaded identity still carrying resident plaintext (the state an
        // incomplete get-path migration leaves: `Clear`/`AlwaysClear` with an
        // `Absent` vault label). It is NOT stored in the vault, so the seal would
        // see only `Absent` and report a false success without the guard.
        let qi = qi_clear_pair_plus_wallet_derived();
        let err = ctx
            .protect_loaded_identity_keys(&qi, &Secret::new("one-identity-password"), None)
            .expect_err("resident plaintext must fail closed, not report count:0");
        assert!(
            matches!(err, TaskError::IdentityKeyProtectionIncomplete),
            "expected IdentityKeyProtectionIncomplete, got {err:?}"
        );

        if let Ok(backend) = ctx.wallet_backend() {
            backend.shutdown().await;
        }
    }
}
