use super::BackendTaskSuccessResult;
use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::wallet_backend::secret_prompt::SecretScope;
use crate::wallet_backend::{SecretAccess, VerifiedIdentityPassword};
use dash_sdk::Error as SdkError;
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::prelude::UserFeeIncrease;
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::identity_update_transition::methods::IdentityUpdateTransitionMethodsV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::{Fetch, Identity};

impl AppContext {
    pub(super) async fn add_key_to_identity(
        &self,
        sdk: &Sdk,
        mut qualified_identity: QualifiedIdentity,
        mut public_key_to_add: QualifiedIdentityPublicKey,
        private_key: [u8; 32],
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // SEC-001 O-2: enforce the protected-identity precondition BEFORE any
        // on-chain side effect. If this identity is password-protected, prompt
        // for and VERIFY its object password up front; a headless host or a
        // wrong password fails closed here, so the AddKeys state transition
        // below is never built or broadcast for a protected identity we cannot
        // seal — no on-chain/local divergence. A keyless identity yields `None`
        // and the existing broadcast-then-keyless-persist path is unchanged.
        let verify_scope = self.protected_identity_verify_scope(&qualified_identity)?;
        let verified_password = verify_protected_identity_precondition(
            &self.wallet_backend()?.secret_access(),
            verify_scope,
        )
        .await?;

        let new_identity_nonce = sdk
            .get_identity_nonce(qualified_identity.identity.id(), true, None)
            .await?;
        let Some(master_key) = qualified_identity.can_sign_with_master_key() else {
            return Err(TaskError::MasterKeyNotFound);
        };
        let master_key_id = master_key.identity_public_key.id();
        let identity = Identity::fetch_by_identifier(sdk, qualified_identity.identity.id())
            .await?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        qualified_identity.identity = identity;
        qualified_identity.identity.bump_revision();
        public_key_to_add
            .identity_public_key
            .set_id(qualified_identity.identity.get_public_key_max_id() + 1);
        qualified_identity.private_keys.insert_non_encrypted(
            (
                PrivateKeyOnMainIdentity,
                public_key_to_add.identity_public_key.id(),
            ),
            (public_key_to_add.clone(), private_key),
        );
        // Track balance before operation for fee calculation
        let balance_before = qualified_identity.identity.balance();
        let estimated_fee = self.fee_estimator().estimate_identity_update();

        let state_transition = IdentityUpdateTransition::try_from_identity_with_signer(
            &qualified_identity.identity,
            &master_key_id,
            vec![public_key_to_add.identity_public_key.clone()],
            vec![],
            new_identity_nonce,
            UserFeeIncrease::default(),
            &qualified_identity,
            sdk.version(),
            None,
        )
        .await
        .map_err(|e| TaskError::IdentityUpdateTransitionError {
            source_error: Box::new(SdkError::Protocol(e)),
        })?;

        let result = state_transition.broadcast_and_wait(sdk, None).await?;

        // Log and handle the proof result
        tracing::info!("AddKeyToIdentity proof result: {}", result);

        let new_balance = match result {
            StateTransitionProofResult::VerifiedPartialIdentity(identity) => {
                // Update the identity with proof-verified public keys
                let balance = identity.balance;
                for public_key in identity.loaded_public_keys.into_values() {
                    qualified_identity.identity.add_public_key(public_key);
                }
                balance
            }
            other => {
                tracing::warn!(
                    "Unexpected proof result type for add key to identity: {}",
                    other
                );
                // Still add the key we tried to add, since the broadcast succeeded
                qualified_identity
                    .identity
                    .add_public_key(public_key_to_add.identity_public_key.clone());
                None
            }
        };

        // Calculate and log actual fee paid
        let actual_fee = if let Some(balance_after) = new_balance {
            let fee = balance_before.saturating_sub(balance_after);
            tracing::info!(
                "AddKeyToIdentity complete: estimated fee {} credits, actual fee {} credits",
                estimated_fee,
                fee
            );
            if fee != estimated_fee {
                tracing::warn!(
                    "Fee mismatch: estimated {} vs actual {} (diff: {})",
                    estimated_fee,
                    fee,
                    fee as i64 - estimated_fee as i64
                );
            }
            qualified_identity.identity.set_balance(balance_after);
            fee
        } else {
            // If we couldn't determine the balance, use the estimate
            estimated_fee
        };

        let fee_result = FeeResult::new(estimated_fee, actual_fee);

        // SEC-001: a password-protected identity must never acquire a keyless
        // key. The object password was already verified up front (before the
        // broadcast above), so here we just seal the newly-added key Tier-2
        // under that SAME password and mark it `InVault` BEFORE saving, so the
        // at-rest encode writes no plaintext for it. The encode-path guard
        // (`encode_identity_blob_vault_first` → `IdentityKeyProtectionDowngrade`)
        // still fails closed if this seal is ever skipped.
        //
        // This seal is the one fallible disk write between the broadcast above
        // and the persist below, and on-chain + local cannot be made atomic. If
        // it fails (I/O error, corrupt keystore), the key is already on-chain but
        // not saved here: fail with the typed, actionable
        // `IdentityKeyAddedButNotSaved` (the key is on the network; retry after
        // freeing disk space) instead of a silent loss or a misleading storage
        // error — and NEVER fall back to a keyless write (that would strip the
        // protection this branch exists to preserve).
        let new_key = (
            PrivateKeyOnMainIdentity,
            public_key_to_add.identity_public_key.id(),
        );
        if let Some(password) = verified_password {
            self.wallet_backend()?
                .secret_access()
                .seal_new_identity_key_with_password(
                    qualified_identity.identity.id().to_buffer(),
                    &new_key.0,
                    new_key.1,
                    &private_key,
                    &password,
                )
                .map_err(key_added_but_not_saved)?;
            // O-1: `mark_in_vault` reports whether the key was present to flip.
            // In this single-threaded flow the key we just inserted is always
            // present, so a `false` is an unexpected invariant break — warn.
            // Persistence stays safe regardless: the at-rest encode guard fails
            // closed on any unmarked resident plaintext key of a protected
            // identity, so no keyless key can ever be written.
            if !qualified_identity.private_keys.mark_in_vault(&new_key) {
                tracing::warn!(
                    target = "backend_task::identity",
                    "Sealed identity key was unexpectedly absent when marking it in-vault",
                );
            }
        }

        self.update_local_qualified_identity(&qualified_identity)?;
        Ok(BackendTaskSuccessResult::AddedKeyToIdentity(fee_result))
    }
}

/// SEC-001 O-2 add-key precondition (no SDK, no network): when the target
/// identity is password-protected, prompt for and VERIFY its object password
/// before the caller performs any irreversible on-chain action. `verify_scope`
/// is [`AppContext::protected_identity_verify_scope`]'s result — `Some(existing
/// protected key)` for a protected identity, `None` for a keyless one.
///
/// A protected identity that cannot be verified — headless
/// ([`NullSecretPrompt`](crate::wallet_backend::secret_prompt::NullSecretPrompt))
/// → [`TaskError::SecretPromptUnavailable`], or a wrong/cancelled password —
/// fails closed HERE. Since [`AppContext::add_key_to_identity`] calls this with
/// `?` before it builds or broadcasts the AddKeys state transition, that error
/// returns the task before any on-chain side effect: no on-chain/local
/// divergence. A keyless identity returns `Ok(None)` and the keyless add path is
/// unchanged. On success the verified password is returned to seal the new key
/// after the broadcast — a single prompt, split across it.
async fn verify_protected_identity_precondition(
    secret_access: &SecretAccess,
    verify_scope: Option<SecretScope>,
) -> Result<Option<VerifiedIdentityPassword>, TaskError> {
    match verify_scope {
        Some(verify) => Ok(Some(
            secret_access
                .verify_identity_object_password(&verify)
                .await?,
        )),
        None => Ok(None),
    }
}

/// Map a POST-broadcast seal failure to the typed
/// [`TaskError::IdentityKeyAddedButNotSaved`]. By the time the seal runs the new
/// key is already accepted on-chain, so a vault-write failure here cannot be
/// undone — surface a loud, actionable error (the key is on the network; retry
/// after freeing disk space) that preserves the upstream seal failure in its
/// `#[source]` chain, rather than a silent loss or a misleading storage message.
/// Never falls back to a keyless write (the SEC-001 protected invariant holds).
fn key_added_but_not_saved(source: TaskError) -> TaskError {
    TaskError::IdentityKeyAddedButNotSaved {
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_identity::PrivateKeyTarget;
    use crate::wallet_backend::SecretSeam;
    use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
    use crate::wallet_backend::secret_prompt::{NullSecretPrompt, SecretPrompt};
    use crate::wallet_backend::single_key::open_secret_store;
    use dash_sdk::dpp::dashcore::Network;
    use platform_wallet_storage::secrets::{
        SecretBytes, SecretStore, SecretString, WalletId as SecretWalletId,
    };
    use std::sync::Arc;

    fn fresh_store(dir: &std::path::Path) -> Arc<SecretStore> {
        Arc::new(open_secret_store(&dir.join("secrets.pwsvault")).expect("open vault"))
    }

    fn access(store: Arc<SecretStore>, prompt: Arc<dyn SecretPrompt>) -> SecretAccess {
        SecretAccess::new(store, prompt, Network::Testnet)
    }

    /// Seal a raw identity key Tier-2 under `password`, making the identity
    /// password-protected (the precondition's verify anchor).
    fn store_protected_identity_key(
        store: &Arc<SecretStore>,
        identity_id: [u8; 32],
        target: &PrivateKeyTarget,
        key_id: u32,
        key: &[u8; 32],
        password: &str,
    ) {
        let label = SecretScope::identity_key_label(target, key_id);
        SecretSeam::new(store)
            .put_secret_protected(
                &SecretWalletId::from(identity_id),
                &label,
                &SecretBytes::from_slice(key),
                &SecretString::new(password),
            )
            .expect("seal identity key tier-2");
    }

    fn main_identity_scope(identity_id: [u8; 32], key_id: u32) -> SecretScope {
        SecretScope::IdentityKey {
            identity_id,
            target: PrivateKeyTarget::PrivateKeyOnMainIdentity,
            key_id,
        }
    }

    /// O-2 fail-closed: a HEADLESS add-key precondition for a PROTECTED identity
    /// returns `SecretPromptUnavailable`. `add_key_to_identity` propagates this
    /// with `?` BEFORE it builds or broadcasts the AddKeys state transition, so
    /// no on-chain state transition is ever produced — proving the headless add
    /// fails closed before the broadcast.
    #[tokio::test]
    async fn headless_protected_precondition_fails_closed_before_broadcast() {
        let dir = tempfile::tempdir().unwrap();
        let identity_id = [0x71u8; 32];
        let store = fresh_store(dir.path());
        // Make the identity protected via an existing Tier-2 key — the verify
        // scope `protected_identity_verify_scope` would derive.
        store_protected_identity_key(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &[0x15u8; 32],
            "identity-object-passwordpw",
        );
        let sa = access(store, Arc::new(NullSecretPrompt));

        let err =
            verify_protected_identity_precondition(&sa, Some(main_identity_scope(identity_id, 0)))
                .await
                .expect_err("headless protected precondition must fail closed");
        assert!(
            matches!(err, TaskError::SecretPromptUnavailable),
            "expected SecretPromptUnavailable, got {err:?}"
        );
    }

    /// The keyless (non-protected) add path is unchanged: a `None` verify scope
    /// returns `Ok(None)` without ever prompting, so the broadcast-then-keyless
    /// -persist flow proceeds exactly as before.
    #[tokio::test]
    async fn keyless_precondition_returns_none_without_prompting() {
        let dir = tempfile::tempdir().unwrap();
        // `TestPrompt::never()` panics if asked — proving no prompt fires.
        let sa = access(fresh_store(dir.path()), Arc::new(TestPrompt::never()));

        let result = verify_protected_identity_precondition(&sa, None)
            .await
            .expect("keyless precondition is a no-op");
        assert!(
            result.is_none(),
            "keyless identity yields no verified password",
        );
    }

    /// An interactive add-key to a protected identity verifies the correct
    /// password up front (one prompt) — the precondition the GUI satisfies
    /// before the broadcast — yielding the password used to seal afterwards.
    #[tokio::test]
    async fn interactive_protected_precondition_verifies_then_yields_password() {
        let dir = tempfile::tempdir().unwrap();
        let identity_id = [0x72u8; 32];
        const PW: &str = "identity-object-passwordpw";
        let store = fresh_store(dir.path());
        store_protected_identity_key(
            &store,
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &[0x16u8; 32],
            PW,
        );
        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(PW)]));
        let sa = access(store, prompt.clone());

        let password =
            verify_protected_identity_precondition(&sa, Some(main_identity_scope(identity_id, 0)))
                .await
                .expect("interactive verify succeeds")
                .expect("protected identity yields a verified password");
        assert_eq!(prompt.ask_count(), 1, "verified with a single prompt");

        // The yielded password seals a new key Tier-2 with no further prompt.
        sa.seal_new_identity_key_with_password(
            identity_id,
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            5,
            &[0x26u8; 32],
            &password,
        )
        .expect("seal new key with the verified password");
        assert_eq!(prompt.ask_count(), 1, "sealing did not prompt again");
    }

    /// A post-broadcast seal failure maps to the typed
    /// `IdentityKeyAddedButNotSaved` and preserves the upstream cause in the
    /// `#[source]` chain — so the banner can speak about the on-chain key while
    /// logs keep the storage diagnostic, and the key is never silently dropped.
    #[test]
    fn post_broadcast_seal_failure_maps_to_typed_orphan_error() {
        use std::error::Error as _;
        // Any upstream seal error stands in for a vault-write failure; the
        // mapping wraps it without inspecting the specific variant.
        let mapped = key_added_but_not_saved(TaskError::IdentityKeyMissing);
        assert!(
            matches!(mapped, TaskError::IdentityKeyAddedButNotSaved { .. }),
            "a post-broadcast seal failure must map to the typed orphan error, got {mapped:?}"
        );
        // The upstream cause survives in the source chain (Display/Debug split).
        let source = mapped.source().expect("upstream seal error is preserved");
        assert!(
            source
                .to_string()
                .contains("could not be found on this device"),
            "expected the upstream cause in the chain, got {source}"
        );
        // The user-facing message states the key is on the network and is
        // actionable (free disk space, retry) — no jargon, no silent loss.
        let shown = mapped.to_string();
        assert!(
            shown.contains("added to your identity on the network"),
            "message must tell the user the key is on-chain, got {shown}"
        );
    }
}
