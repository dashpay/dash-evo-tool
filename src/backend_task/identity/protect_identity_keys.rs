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
use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
use crate::model::qualified_identity::identity_meta::IdentityMeta;
use crate::model::secret::Secret;
use crate::model::wallet::passphrase::validate_single_key_passphrase;
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
        // Backend = authoritative validation (SEC-002): re-enforce the password
        // policy here, not only in the UI, so a future MCP/CLI caller cannot
        // seal under a too-short password.
        validate_protection_password(&password)?;

        let qi = self
            .get_identity_by_id(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;

        // SEC-001 fail-closed: any resident plaintext key left by an incomplete
        // get-path migration has an `Absent` label `seal_identity_keys` would
        // skip, so refuse here rather than emit a false-protected result.
        reject_resident_identity_plaintext(&qi.private_keys)?;

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

/// Backend-authoritative password policy for identity-key protection (SEC-002).
/// Re-uses the single-key passphrase validator (the same minimum length the UI
/// shows) so the rule lives in one place and a non-UI caller cannot bypass it.
/// The confirmation match is a UI concern, so the password is passed as its own
/// confirmation here — only the length check is meaningful at this layer.
fn validate_protection_password(password: &Secret) -> Result<(), TaskError> {
    let pw = password.expose_secret();
    validate_single_key_passphrase(pw, pw)
}

/// SEC-001 fail-closed guard for the protect boundary: reject an identity that
/// still carries resident plaintext (`Clear`/`AlwaysClear`) keys on disk. Such a
/// key means the eager load-path vault migration did not complete — its vault
/// write failed, or it was skipped on an already-protected identity — so the key
/// has no vault label and [`seal_identity_keys`] would silently skip its
/// `Absent` scheme and report a false success. Wallet-derived
/// (`AtWalletDerivationPath`) and already-vaulted (`InVault`) keys carry no
/// resident plaintext, so a legitimately keyless / wallet-derived identity is
/// never rejected.
fn reject_resident_identity_plaintext(private_keys: &KeyStorage) -> Result<(), TaskError> {
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
/// At-rest residual (SEC-004, known): the in-place upsert replaces the value at
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
    // SEC-001 one-password invariant: a Mixed-state "Finish protecting" re-run
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
/// sealing mutates the vault. Enforces SEC-001's one-password-per-identity
/// invariant on a Mixed-state opt-in re-run: if a prior partial run sealed some
/// keys under password A and the user now supplies password B, the mismatch
/// surfaces from `get_protected` as [`TaskError::IdentityKeyPassphraseIncorrect`]
/// (no oracle) with zero state changes. Keyless (`Unprotected`) and `Absent`
/// keys impose no password constraint and are skipped.
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

    /// SEC-001 one-password invariant: a Mixed-state "Finish protecting" re-run
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

    /// SEC-002: the backend enforces the password policy — a too-short password
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

    /// SEC-001 fail-closed: an identity still carrying a resident-plaintext key
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
}
