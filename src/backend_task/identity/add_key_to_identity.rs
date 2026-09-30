use super::BackendTaskSuccessResult;
use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::derived_identity_key::{
    derivation_index_limit, derivation_wallet, is_derivable_key_type, occupied_indices,
};
use crate::model::qualified_identity::PrivateKeyTarget::{self, PrivateKeyOnMainIdentity};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::wallet_backend::secret_prompt::SecretScope;
use crate::wallet_backend::{SecretAccess, VerifiedIdentityPassword};
use dash_sdk::Error as SdkError;
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::KeyID;
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::identity::accessors::{IdentityGettersV0, IdentitySettersV0};
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::{
    IdentityPublicKeyGettersV0, IdentityPublicKeySettersV0,
};
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, KeyDerivationType};
use dash_sdk::dpp::prelude::UserFeeIncrease;
use dash_sdk::dpp::state_transition::identity_update_transition::IdentityUpdateTransition;
use dash_sdk::dpp::state_transition::identity_update_transition::methods::IdentityUpdateTransitionMethodsV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::{Fetch, Identifier, Identity};

impl AppContext {
    pub(super) async fn add_key_to_identity(
        &self,
        sdk: &Sdk,
        qualified_identity: QualifiedIdentity,
        public_key_to_add: QualifiedIdentityPublicKey,
        private_key: [u8; 32],
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.add_identity_key(
            sdk,
            qualified_identity,
            public_key_to_add,
            NewKeyMaterial::Private(private_key),
        )
        .await
    }

    /// Add a key derived from the identity's wallet at derivation `index`.
    ///
    /// The UI snapshot in `identity` is used only for its id: the identity is
    /// reloaded from local storage and every guard (type, wallet, recovery
    /// window, occupancy) is re-evaluated here, the authoritative layer. The
    /// public key is derived from the seed through the secret chokepoint and
    /// must match the cached key the chooser worked from; a mismatch repairs
    /// the cache entry and fails with [`TaskError::DerivedKeySeedMismatch`]
    /// before anything is broadcast.
    ///
    /// `expected_key_id` is the key id the screen showed the slot choice
    /// against. If the network record assigns a different one, the add fails
    /// with [`TaskError::DerivedKeyIdChanged`] before broadcast, so a slot
    /// picked to match the key id never silently lands at another id.
    pub(super) async fn add_derived_key_to_identity(
        &self,
        sdk: &Sdk,
        identity: QualifiedIdentity,
        key: QualifiedIdentityPublicKey,
        index: u32,
        expected_key_id: KeyID,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let (identity, key) = self
            .prepare_derived_identity_key(&identity.identity.id(), key, index)
            .await?;
        self.add_identity_key(
            sdk,
            identity,
            key,
            NewKeyMaterial::Derived { expected_key_id },
        )
        .await
    }

    /// Everything [`Self::add_derived_key_to_identity`] checks before it
    /// touches the network: returns the reloaded identity and the key with its
    /// seed-verified public data and wallet path filled in.
    async fn prepare_derived_identity_key(
        &self,
        identity_id: &Identifier,
        mut key: QualifiedIdentityPublicKey,
        index: u32,
    ) -> Result<(QualifiedIdentity, QualifiedIdentityPublicKey), TaskError> {
        if !is_derivable_key_type(key.identity_public_key.key_type()) {
            return Err(TaskError::DerivedKeyTypeUnsupported);
        }
        let identity = self
            .get_local_qualified_identity(identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        let (seed_hash, identity_index) = derivation_wallet(&identity, self.network)
            .ok_or(TaskError::DerivedKeyWalletRequired)?;
        if index >= derivation_index_limit(identity.identity.get_public_key_max_id()) {
            return Err(TaskError::DerivedKeyIndexUnavailable);
        }
        let wallet = self.wallet_arc(&seed_hash)?;
        let public_key = self
            .derive_identity_auth_pubkey_from_seed(&wallet, identity_index, index)
            .await?;

        // The chooser computed occupancy from the cache; authenticate the entry
        // for this index against the seed before trusting it (SEC-001).
        let backend = self.wallet_backend()?;
        let cache_view = backend.auth_pubkey_cache();
        let network = self.network;
        // Check and repair in one serialised update, so a concurrent writer's
        // stale snapshot cannot put the bad entry back (SEC-106).
        let (cached, cache) = cache_view.update(network, &seed_hash, |cache| {
            let cached = cache.get(network, identity_index, index);
            cache.insert(network, identity_index, index, &public_key);
            (cached, cache.clone())
        })?;
        // A cold entry was just filled; a different cached key was repaired
        // and the add is refused.
        if cached.is_some_and(|cached| cached != public_key) {
            tracing::warn!(
                target = "backend_task::identity",
                identity_index,
                key_index = index,
                "Cached identity-auth public key disagrees with the seed; repaired the entry and refused the add",
            );
            return Err(TaskError::DerivedKeySeedMismatch);
        }
        // Register the key's address on the wallet, as the load flows do. The
        // entry was verified above, so this is a cache hit with no seed access.
        self.resolve_identity_auth_pubkeys_data_map(
            &wallet,
            true, // register_addresses
            true, // allow_prompt
            identity_index,
            index..index + 1,
        )
        .await?;
        if occupied_indices(&identity, self.network, seed_hash, identity_index, &cache)
            .contains(&index)
        {
            return Err(TaskError::DerivedKeyIndexUnavailable);
        }
        let data = match key.identity_public_key.key_type() {
            KeyType::ECDSA_SECP256K1 => public_key.to_bytes(),
            KeyType::ECDSA_HASH160 => {
                let hash: [u8; 20] = public_key.pubkey_hash().into();
                hash.to_vec()
            }
            _ => return Err(TaskError::DerivedKeyTypeUnsupported),
        };
        key.identity_public_key.set_data(data.into());
        key.in_wallet_at_derivation_path = Some(WalletDerivationPath {
            wallet_seed_hash: seed_hash,
            derivation_path: DerivationPath::identity_authentication_path(
                self.network,
                KeyDerivationType::ECDSA,
                identity_index,
                index,
            ),
        });
        Ok((identity, key))
    }

    async fn add_identity_key(
        &self,
        sdk: &Sdk,
        mut qualified_identity: QualifiedIdentity,
        mut public_key_to_add: QualifiedIdentityPublicKey,
        material: NewKeyMaterial,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // O-2: enforce the protected-identity precondition BEFORE any
        // on-chain side effect. If this identity is password-protected, prompt
        // for and VERIFY its object password up front; a headless host or a
        // wrong password fails closed here, so the AddKeys state transition
        // below is never built or broadcast for a protected identity we cannot
        // seal — no on-chain/local divergence. A keyless identity yields `None`
        // and the existing broadcast-then-keyless-persist path is unchanged.
        //
        // A wallet-derived key has no private bytes to seal (it stays
        // protected by its wallet), so it needs no identity password (SEC-105).
        let verify_scope = match material {
            NewKeyMaterial::Private(_) => {
                self.protected_identity_verify_scope(&qualified_identity)?
            }
            NewKeyMaterial::Derived { .. } => None,
        };
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
        let assigned_key_id = qualified_identity.identity.get_public_key_max_id() + 1;
        if let NewKeyMaterial::Derived { expected_key_id } = material {
            check_derived_key_id(assigned_key_id, expected_key_id)?;
        }
        public_key_to_add
            .identity_public_key
            .set_id(assigned_key_id);
        // `max_id` comes from the freshly published record, but the slot is
        // checked against the LOCAL store: an entry saved here but never
        // broadcast (e.g. restored from an old blob) can hold `max_id + 1`,
        // and it may be a misfiled key's only private half — so refuse rather
        // than overwrite.
        let placement = (
            PrivateKeyOnMainIdentity,
            public_key_to_add.identity_public_key.id(),
        );
        let private_key = match material {
            NewKeyMaterial::Private(private_key) => {
                qualified_identity
                    .private_keys
                    .insert_non_encrypted(placement, (public_key_to_add.clone(), private_key))?;
                Some(private_key)
            }
            NewKeyMaterial::Derived { .. } => {
                insert_derived_key(&mut qualified_identity, &public_key_to_add)?;
                None
            }
        };
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

        // Past this point the key is on the network: persist it, reporting any
        // failure as "added but not saved" (see `persist_added_identity_key`).
        let new_key = (
            PrivateKeyOnMainIdentity,
            public_key_to_add.identity_public_key.id(),
        );
        self.persist_added_identity_key(
            &qualified_identity,
            new_key,
            &public_key_to_add,
            private_key.as_ref(),
            verified_password,
        )?;
        Ok(BackendTaskSuccessResult::AddedKeyToIdentity(fee_result))
    }

    /// Seal the new key and store it on the identity's record, both under
    /// this identity's record guard.
    ///
    /// The record is re-read under the guard and only this add is applied to
    /// it: the post-broadcast on-chain identity (new public key, balance,
    /// revision) and the one new private-key entry. `snapshot` — taken before
    /// the network round trips — is never written back, so a key, alias or
    /// protection change another writer saved during the broadcast survives
    /// (SEC-101).
    ///
    /// The guard spans the seal because the seal writes private key material
    /// and the write is what makes anything point at it. Without it, an unload
    /// completing during the broadcast leaves the removal deleting only the
    /// placements the stored blob named — not this one, which is not in it yet
    /// — and clearing its cleanup manifest; the seal then lands afterwards and
    /// the record write is declined. The key would sit in the vault for an
    /// identity with no record, no roster entry and no manifest: unreachable
    /// through the identity model, so no sweep can ever collect it, on a device
    /// that told the user it had destroyed that identity's keys. So the roster
    /// is rechecked under the guard first, and a delisted identity ends the
    /// task before anything is sealed.
    ///
    /// `private_key` is `None` for a wallet-derived key, stored as
    /// `AtWalletDerivationPath`: there is no private material to seal, even
    /// for a password-protected identity, so the seal is skipped. That key
    /// stays protected by its wallet and recoverable from the wallet's
    /// recovery phrase.
    ///
    /// Everything here runs after the key was accepted on the network, so
    /// every failure is reported as
    /// [`TaskError::IdentityKeyAddedButNotSaved`] (or
    /// [`TaskError::IdentityKeyAddedButIdentityUnloaded`]) — never as a raw
    /// storage error that reads like the add failed (SEC-104). The add-key
    /// screen keeps a user-entered private key on hand in both cases; a
    /// wallet-derived key needs no copy, the wallet can derive it again.
    fn persist_added_identity_key(
        &self,
        snapshot: &QualifiedIdentity,
        new_key: (PrivateKeyTarget, KeyID),
        public_key: &QualifiedIdentityPublicKey,
        private_key: Option<&[u8; 32]>,
        verified_password: Option<VerifiedIdentityPassword>,
    ) -> Result<(), TaskError> {
        let identity_id = snapshot.identity.id();
        let lock = self.identity_record_lock(identity_id);
        let _record_guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let origin = AddedKeyOrigin::of(private_key);
        let not_saved = |source| origin.not_saved(source);
        if !self.is_identity_listed(&identity_id).map_err(not_saved)? {
            tracing::warn!(
                target = "backend_task::identity",
                identity_id = %identity_id,
                "Identity was removed from this device while its new key was being added; the key is on the network and was not sealed here",
            );
            return Err(origin.identity_unloaded());
        }

        self.store_added_identity_key_locked(
            snapshot,
            new_key,
            public_key,
            private_key,
            verified_password,
        )
        .map_err(not_saved)
    }

    /// The body of [`Self::persist_added_identity_key`]; the caller holds the
    /// identity's record guard and maps every error.
    fn store_added_identity_key_locked(
        &self,
        snapshot: &QualifiedIdentity,
        new_key: (PrivateKeyTarget, KeyID),
        public_key: &QualifiedIdentityPublicKey,
        private_key: Option<&[u8; 32]>,
        verified_password: Option<VerifiedIdentityPassword>,
    ) -> Result<(), TaskError> {
        let identity_id = snapshot.identity.id();
        // Listed but without a blob is a torn record; the snapshot is the best
        // base there is, as before.
        let mut record = self
            .get_local_qualified_identity(&identity_id)?
            .unwrap_or_else(|| snapshot.clone());

        // The on-chain identity as the broadcast left it. Public keys only
        // accumulate on-chain, so any key the stored copy knows and the
        // snapshot does not is kept rather than dropped.
        let mut identity = snapshot.identity.clone();
        for (key_id, stored_key) in record.identity.public_keys() {
            if !identity.public_keys().contains_key(key_id) {
                identity.add_public_key(stored_key.clone());
            }
        }
        record.identity = identity;

        // Both inserts refuse a slot another writer filled with a different
        // key meanwhile, before anything is sealed.
        let Some(private_key) = private_key else {
            // A wallet-derived key: file its derivation path only. Nothing to
            // seal — the key stays protected by its wallet.
            let path = public_key
                .in_wallet_at_derivation_path
                .clone()
                .ok_or(TaskError::DerivedKeyWalletRequired)?;
            record
                .private_keys
                .insert_wallet_derived(new_key, public_key.clone(), path)?;
            return self.write_local_qualified_identity_locked(&record);
        };
        record
            .private_keys
            .insert_non_encrypted(new_key.clone(), (public_key.clone(), *private_key))?;

        // A password-protected identity must never acquire a keyless key. The
        // object password was verified before the broadcast, so the new key is
        // sealed Tier-2 under that SAME password and marked `InVault` BEFORE
        // saving, so the at-rest encode writes no plaintext for it. The
        // encode-path guard (`encode_identity_blob_vault_first` →
        // `IdentityKeyProtectionDowngrade`) still fails closed if this seal is
        // skipped — including when the identity was protected during the
        // broadcast of a keyless add. NEVER fall back to a keyless write.
        if let Some(password) = verified_password {
            self.wallet_backend()?
                .secret_access()
                .seal_new_identity_key_with_password(
                    identity_id.to_buffer(),
                    &new_key.0,
                    new_key.1,
                    private_key,
                    &password,
                )?;
            // The entry was inserted just above under the record guard, so a
            // `false` is an invariant break — warn. Persistence stays safe
            // regardless: the at-rest encode guard fails closed on any
            // unmarked resident plaintext key of a protected identity.
            if !record.private_keys.mark_in_vault(&new_key) {
                tracing::warn!(
                    target = "backend_task::identity",
                    "Sealed identity key was unexpectedly absent when marking it in-vault",
                );
            }
        }

        self.write_local_qualified_identity_locked(&record)
    }
}

/// Where the private half of a key being added comes from.
#[derive(Clone, Copy)]
enum NewKeyMaterial {
    /// Entered by the user; sealed under the identity password when the
    /// identity is password-protected.
    Private([u8; 32]),
    /// Derived from the identity's wallet: no private bytes are stored.
    /// `expected_key_id` is the key id the user chose the slot against.
    Derived { expected_key_id: KeyID },
}

/// Refuse a wallet-derived key whose on-chain key id differs from the one the
/// user chose its slot against: the identity gained a key elsewhere since the
/// screen loaded, so a slot picked to match the key id would silently land at
/// another id and be restorable only in DET (SEC-103). A slot deliberately
/// picked off the key id is unaffected — only the id is compared.
fn check_derived_key_id(assigned: KeyID, expected: KeyID) -> Result<(), TaskError> {
    if assigned == expected {
        Ok(())
    } else {
        Err(TaskError::DerivedKeyIdChanged)
    }
}

fn insert_derived_key(
    identity: &mut QualifiedIdentity,
    key: &QualifiedIdentityPublicKey,
) -> Result<(), TaskError> {
    let placement = (PrivateKeyOnMainIdentity, key.identity_public_key.id());
    // Recheck the fetched identity: another device may have used this key.
    let hash = key
        .identity_public_key
        .public_key_hash()
        .map_err(SdkError::Protocol)?;
    if identity
        .identity
        .public_keys()
        .values()
        .any(|key| key.public_key_hash().ok() == Some(hash))
    {
        return Err(TaskError::DerivedKeyIndexUnavailable);
    }
    if identity
        .private_keys
        .get_cloned_private_key_data_and_wallet_info(&placement)
        .is_some()
    {
        return Err(TaskError::IdentityKeySlotOccupied);
    }
    let path = key
        .in_wallet_at_derivation_path
        .clone()
        .ok_or(TaskError::DerivedKeyWalletRequired)?;
    identity.private_keys.insert_if_absent(
        placement,
        (key.clone(), PrivateKeyData::AtWalletDerivationPath(path)),
    );
    Ok(())
}

/// O-2 add-key precondition (no SDK, no network): when the target
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

/// Where the private half of a just-broadcast key lives, which decides the
/// recovery advice when saving it on this device fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddedKeyOrigin {
    /// Pasted or generated: the private key in the form may be the only copy.
    UserEntered,
    /// Wallet-derived: the wallet derives it again, so there is nothing to keep.
    WalletDerived,
}

impl AddedKeyOrigin {
    /// Classify by key material: a key added without a private half is one the
    /// wallet derives.
    fn of(private_key: Option<&[u8; 32]>) -> Self {
        match private_key {
            Some(_) => Self::UserEntered,
            None => Self::WalletDerived,
        }
    }

    /// Map a POST-broadcast failure (roster read, record read, occupied slot,
    /// seal, vault or record write, protection-downgrade refusal) to the typed
    /// "added but not saved" error. The new key is already accepted on-chain,
    /// so the failure cannot be undone — surface a loud, actionable error that
    /// preserves the upstream failure in its `#[source]` chain, rather than a
    /// raw storage message that reads like the add failed. Never falls back to
    /// a keyless write.
    ///
    /// A user-entered key maps to [`TaskError::IdentityKeyAddedButNotSaved`]
    /// (keep the private key); a wallet-derived one to
    /// [`TaskError::DerivedIdentityKeyAddedButNotSaved`] (nothing to keep).
    ///
    /// Two user-entered causes get their own variant, because the generic
    /// remedy — refresh, then enter the key — would be refused again for the
    /// same reason: a protection-downgrade refusal and an occupied slot.
    fn not_saved(self, source: TaskError) -> TaskError {
        match (self, source) {
            (Self::UserEntered, TaskError::IdentityKeyProtectionDowngrade) => {
                TaskError::IdentityKeyAddedButNotSavedWhileProtected
            }
            (Self::UserEntered, TaskError::IdentityKeySlotOccupied) => {
                TaskError::IdentityKeyAddedButSlotOccupied
            }
            (Self::UserEntered, source) => TaskError::IdentityKeyAddedButNotSaved {
                source: Box::new(source),
            },
            (Self::WalletDerived, source) => TaskError::DerivedIdentityKeyAddedButNotSaved {
                source: Box::new(source),
            },
        }
    }

    /// The identity was removed from this device while its key was added.
    fn identity_unloaded(self) -> TaskError {
        match self {
            Self::UserEntered => TaskError::IdentityKeyAddedButIdentityUnloaded,
            Self::WalletDerived => TaskError::DerivedIdentityKeyAddedButIdentityUnloaded,
        }
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
    use dash_sdk::platform::Identifier;
    use platform_wallet_storage::secrets::{
        SecretBytes, SecretStore, SecretString, WalletId as SecretWalletId,
    };
    use std::sync::Arc;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_key_persists_as_path_and_signs_after_reload() {
        use crate::context::test_staging::stage_identity_with_vaulted_keys;
        use crate::model::derived_identity_key::test_support::fixture;
        use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1, SecretKey};

        let staged = stage_identity_with_vaulted_keys([0xAA; 32], [0xBB; 32]).await;
        let (mut identity, cache, seed_hash, seed) = fixture();
        identity.identity.set_id(staged.id);
        for (seed, wallet) in &identity.associated_wallets {
            staged
                .ctx
                .wallet_context()
                .insert_test_wallet(*seed, Arc::clone(wallet));
        }
        let mut key = identity.private_keys.identity_public_keys()[0].1.clone();
        key.identity_public_key.set_id(5);
        key.identity_public_key
            .set_security_level(dash_sdk::dpp::identity::SecurityLevel::HIGH);
        key.identity_public_key
            .set_data(cache.get(Network::Testnet, 0, 3).unwrap().to_bytes().into());
        key.in_wallet_at_derivation_path = Some(WalletDerivationPath {
            wallet_seed_hash: seed_hash,
            derivation_path: DerivationPath::identity_authentication_path(
                Network::Testnet,
                KeyDerivationType::ECDSA,
                0,
                3,
            ),
        });
        insert_derived_key(&mut identity, &key).unwrap();
        identity
            .identity
            .add_public_key(key.identity_public_key.clone());
        let placement = (PrivateKeyOnMainIdentity, 5);
        staged
            .ctx
            .persist_added_identity_key(&identity, placement.clone(), &key, None, None)
            .unwrap();
        let restored = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        let (data, path) = restored
            .private_keys
            .get_cloned_private_key_data_and_wallet_info(&placement)
            .unwrap();
        assert!(matches!(data, PrivateKeyData::AtWalletDerivationPath(_)));
        assert_eq!(path, key.in_wallet_at_derivation_path);
        assert!(
            crate::wallet_backend::IdentityKeyView::new(&staged.store, staged.id.to_buffer())
                .get(&placement.0, placement.1)
                .unwrap()
                .is_none()
        );
        let wallets = restored
            .associated_wallets
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let (_, bytes) = restored
            .private_keys
            .get_resolve_with_seed(&placement, &wallets, &seed, Network::Testnet)
            .unwrap()
            .unwrap();
        let secp = Secp256k1::new();
        let secret = SecretKey::from_slice(bytes.as_ref()).unwrap();
        let message = Message::from_digest([1; 32]);
        let signature = secp.sign_ecdsa(&message, &secret);
        secp.verify_ecdsa(
            &message,
            &signature,
            &cache.get(Network::Testnet, 0, 3).unwrap().inner,
        )
        .unwrap();
    }

    #[test]
    fn derived_key_rechecks_network_hash_aliases_and_local_slots() {
        use crate::model::derived_identity_key::test_support::fixture;
        let (mut identity, _, _, _) = fixture();
        let mut key = identity.private_keys.identity_public_keys()[0].1.clone();
        key.identity_public_key.set_id(1);
        let hash = key.identity_public_key.public_key_hash().unwrap();
        key.identity_public_key.set_key_type(KeyType::ECDSA_HASH160);
        key.identity_public_key.set_data(hash.to_vec().into());
        assert!(matches!(
            insert_derived_key(&mut identity, &key),
            Err(TaskError::DerivedKeyIndexUnavailable)
        ));
        identity.identity.set_public_keys(Default::default());
        key.identity_public_key.set_id(0);
        assert!(matches!(
            insert_derived_key(&mut identity, &key),
            Err(TaskError::IdentityKeySlotOccupied)
        ));
    }

    /// A staged context holding the derivation fixture's identity (derived key
    /// at index 0, wallet identity index 0), its wallet, the wallet's raw HD
    /// seed in the vault and a warm public-key cache for indices 0..8.
    struct DerivedStage {
        staged: crate::context::test_staging::StagedIdentity,
        identity: QualifiedIdentity,
        cache: crate::model::wallet::auth_pubkey_cache::AuthPubkeyCache,
        seed_hash: crate::model::wallet::WalletSeedHash,
    }

    async fn stage_derivation(register_wallet: bool) -> DerivedStage {
        use crate::context::test_staging::stage_identity_with_vaulted_keys;
        use crate::model::derived_identity_key::test_support::fixture;

        let staged = stage_identity_with_vaulted_keys([0xAA; 32], [0xBB; 32]).await;
        let (mut identity, cache, seed_hash, seed) = fixture();
        identity.identity.set_id(staged.id);
        if register_wallet {
            for (seed, wallet) in &identity.associated_wallets {
                staged
                    .ctx
                    .wallet_context()
                    .insert_test_wallet(*seed, Arc::clone(wallet));
            }
        }
        let backend = staged.ctx.wallet_backend().unwrap();
        backend.wallet_seeds().set_raw(&seed_hash, &seed).unwrap();
        backend
            .auth_pubkey_cache()
            .put(Network::Testnet, &seed_hash, &cache)
            .unwrap();
        staged
            .ctx
            .update_local_qualified_identity(&identity)
            .unwrap();
        DerivedStage {
            staged,
            identity,
            cache,
            seed_hash,
        }
    }

    fn derived_request(key_type: KeyType) -> QualifiedIdentityPublicKey {
        use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
        use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
        QualifiedIdentityPublicKey::from(dash_sdk::platform::IdentityPublicKey::from(
            IdentityPublicKeyV0 {
                id: 0,
                key_type,
                purpose: Purpose::AUTHENTICATION,
                security_level: SecurityLevel::HIGH,
                contract_bounds: None,
                read_only: false,
                disabled_at: None,
                data: Vec::new().into(),
            },
        ))
    }

    /// Run the real entry point against an offline mock SDK. Every guard under
    /// test fires before the first network call, so reaching the SDK at all
    /// would surface as a different (network) error.
    async fn add_derived(
        stage: &DerivedStage,
        identity: QualifiedIdentity,
        key_type: KeyType,
        index: u32,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let sdk = Sdk::new_mock();
        stage
            .staged
            .ctx
            .add_derived_key_to_identity(&sdk, identity, derived_request(key_type), index, 1)
            .await
    }

    /// An offline SDK that answers the add flow's nonce and identity fetches,
    /// returning `network` as the identity's published record.
    async fn mock_network(network: Identity) -> Sdk {
        use dash_sdk::query_types::IdentityNonceFetcher;
        let mut sdk = Sdk::new_mock();
        let id = network.id();
        sdk.mock()
            .expect_fetch::<IdentityNonceFetcher, _>(id, Some(IdentityNonceFetcher(1)))
            .await
            .expect("mock the nonce fetch");
        sdk.mock()
            .expect_fetch::<Identity, _>(id, Some(network))
            .await
            .expect("mock the identity fetch");
        sdk
    }

    /// The stage's identity as published after another device added a key
    /// (not from this wallet) at key id 1.
    fn network_record_with_foreign_key(stage: &DerivedStage) -> Identity {
        use dash_sdk::dpp::dashcore::secp256k1::{PublicKey, Secp256k1, SecretKey};
        let mut record = stage.identity.identity.clone();
        let mut foreign = record.public_keys()[&0].clone();
        foreign.set_id(1);
        foreign.set_security_level(dash_sdk::dpp::identity::SecurityLevel::HIGH);
        let secret = SecretKey::from_slice(&[9; 32]).unwrap();
        foreign.set_data(
            PublicKey::from_secret_key(&Secp256k1::new(), &secret)
                .serialize()
                .to_vec()
                .into(),
        );
        record.add_public_key(foreign);
        record
    }

    async fn add_derived_against(
        stage: &DerivedStage,
        network: Identity,
        index: u32,
        expected_key_id: KeyID,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let sdk = mock_network(network).await;
        stage
            .staged
            .ctx
            .add_derived_key_to_identity(
                &sdk,
                stage.identity.clone(),
                derived_request(KeyType::ECDSA_SECP256K1),
                index,
                expected_key_id,
            )
            .await
    }

    /// SEC-103: the local record says the next key id is 1 (the id the slot
    /// was chosen against), but the network already holds key 1, so the key
    /// would land at id 2. Refused before anything is signed or broadcast.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_add_refuses_when_the_network_assigns_another_key_id() {
        let stage = stage_derivation(true).await;
        let network = network_record_with_foreign_key(&stage);
        let result = add_derived_against(&stage, network, 1, 1).await;
        assert!(
            matches!(result, Err(TaskError::DerivedKeyIdChanged)),
            "expected DerivedKeyIdChanged, got {result:?}",
        );
        let stored = stage
            .staged
            .ctx
            .get_local_qualified_identity(&stage.staged.id)
            .unwrap()
            .unwrap();
        assert!(
            stored
                .private_keys
                .get_cloned_private_key_data_and_wallet_info(&(PrivateKeyOnMainIdentity, 2))
                .is_none(),
            "nothing was saved for the refused key",
        );
    }

    /// An off-id slot reaches the unmocked broadcast when the key id is unchanged.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_add_allows_an_off_id_slot_when_the_key_id_is_unchanged() {
        let stage = stage_derivation(true).await;
        let network = stage.identity.identity.clone();
        let result = add_derived_against(&stage, network, 3, 1).await;
        // Nonce and identity reads are mocked; only the broadcast has no expectation.
        let Err(TaskError::SdkError { source_error }) = result else {
            panic!("expected the unmocked broadcast to fail, got {result:?}");
        };
        assert!(
            matches!(
                *source_error,
                SdkError::DapiClientError(dash_sdk::dapi_client::DapiClientError::Mock(
                    dash_sdk::dapi_client::mock::MockError::MockExpectationNotFound(_)
                ))
            ),
            "expected a missing broadcast expectation, got {source_error:?}"
        );
    }

    #[test]
    fn derived_key_id_check_compares_ids_only() {
        assert!(check_derived_key_id(4, 4).is_ok());
        assert!(matches!(
            check_derived_key_id(5, 4),
            Err(TaskError::DerivedKeyIdChanged)
        ));
    }

    /// Make the stage's identity password-protected: seal a Tier-2 vault
    /// entry for its master key placement.
    fn protect_stage_identity(stage: &DerivedStage) {
        store_protected_identity_key(
            &stage.staged.store,
            stage.staged.id.to_buffer(),
            &PrivateKeyTarget::PrivateKeyOnMainIdentity,
            0,
            &[0x17; 32],
            "identity-object-passwordpw",
        );
        let scope = stage
            .staged
            .ctx
            .protected_identity_verify_scope(&stage.identity)
            .unwrap();
        assert!(scope.is_some(), "the stage identity is now protected");
    }

    /// SEC-105: a derived key has no private bytes to seal, so a protected
    /// identity is not asked for its password. On this headless stage a
    /// prompt would fail with `SecretPromptUnavailable` before the network
    /// fetch; reaching the key-id check proves no prompt was attempted.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_add_to_a_protected_identity_does_not_ask_for_its_password() {
        let stage = stage_derivation(true).await;
        protect_stage_identity(&stage);
        let network = network_record_with_foreign_key(&stage);
        let result = add_derived_against(&stage, network, 1, 1).await;
        assert!(
            matches!(result, Err(TaskError::DerivedKeyIdChanged)),
            "expected the flow to reach the key-id check without a prompt, got {result:?}",
        );
    }

    /// SEC-105 scope: a manually entered key for a protected identity still
    /// requires the identity password (it is sealed under it); headless, that
    /// fails closed before any network call.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn manual_add_to_a_protected_identity_still_requires_its_password() {
        let stage = stage_derivation(true).await;
        protect_stage_identity(&stage);
        let mut key = derived_request(KeyType::ECDSA_SECP256K1);
        key.identity_public_key.set_data(
            stage
                .cache
                .get(Network::Testnet, 0, 4)
                .unwrap()
                .to_bytes()
                .into(),
        );
        let result = stage
            .staged
            .ctx
            .add_key_to_identity(&Sdk::new_mock(), stage.identity.clone(), key, [0x42; 32])
            .await;
        assert!(
            matches!(result, Err(TaskError::SecretPromptUnavailable)),
            "expected SecretPromptUnavailable, got {result:?}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_add_rejects_a_key_type_the_wallet_cannot_derive() {
        let stage = stage_derivation(true).await;
        for key_type in [KeyType::BLS12_381, KeyType::EDDSA_25519_HASH160] {
            let result = add_derived(&stage, stage.identity.clone(), key_type, 1).await;
            assert!(
                matches!(result, Err(TaskError::DerivedKeyTypeUnsupported)),
                "{key_type:?}: expected DerivedKeyTypeUnsupported, got {result:?}",
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_add_rejects_an_identity_not_saved_on_this_device() {
        let stage = stage_derivation(true).await;
        let mut stranger = stage.identity.clone();
        stranger.identity.set_id(Identifier::from([0x5E; 32]));
        let result = add_derived(&stage, stranger, KeyType::ECDSA_SECP256K1, 1).await;
        assert!(
            matches!(result, Err(TaskError::IdentityNotFoundLocally)),
            "expected IdentityNotFoundLocally, got {result:?}",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_add_rejects_an_index_at_or_over_the_recovery_limit() {
        let stage = stage_derivation(true).await;
        // Highest key id 0 → indices 0..6 are selectable.
        for index in [6, 7, 4096, u32::MAX - 1, u32::MAX] {
            let result = add_derived(
                &stage,
                stage.identity.clone(),
                KeyType::ECDSA_SECP256K1,
                index,
            )
            .await;
            assert!(
                matches!(result, Err(TaskError::DerivedKeyIndexUnavailable)),
                "index {index}: expected DerivedKeyIndexUnavailable, got {result:?}",
            );
        }
        let (_, key) = stage
            .staged
            .ctx
            .prepare_derived_identity_key(
                &stage.staged.id,
                derived_request(KeyType::ECDSA_SECP256K1),
                5,
            )
            .await
            .expect("the highest selectable index passes every guard");
        assert_eq!(
            key.identity_public_key.data().as_slice(),
            stage.cache.get(Network::Testnet, 0, 5).unwrap().to_bytes()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_add_rejects_occupied_indices_including_a_disabled_hash160_alias() {
        use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
        use dash_sdk::dpp::identity::{Purpose, SecurityLevel};

        let stage = stage_derivation(true).await;
        // Index 0 holds the fixture's derived master key (stored wallet path).
        let result = add_derived(&stage, stage.identity.clone(), KeyType::ECDSA_HASH160, 0).await;
        assert!(
            matches!(result, Err(TaskError::DerivedKeyIndexUnavailable)),
            "expected DerivedKeyIndexUnavailable for the path-occupied index, got {result:?}",
        );

        // A disabled HASH160 key with no wallet metadata, equal to index 2.
        let mut identity = stage.identity.clone();
        let hash: [u8; 20] = stage
            .cache
            .get(Network::Testnet, 0, 2)
            .unwrap()
            .pubkey_hash()
            .into();
        identity.identity.add_public_key(
            IdentityPublicKeyV0 {
                id: 4,
                key_type: KeyType::ECDSA_HASH160,
                purpose: Purpose::AUTHENTICATION,
                security_level: SecurityLevel::HIGH,
                contract_bounds: None,
                read_only: false,
                disabled_at: Some(1),
                data: hash.to_vec().into(),
            }
            .into(),
        );
        stage
            .staged
            .ctx
            .update_local_qualified_identity(&identity)
            .unwrap();
        for key_type in [KeyType::ECDSA_SECP256K1, KeyType::ECDSA_HASH160] {
            let result = add_derived(&stage, identity.clone(), key_type, 2).await;
            assert!(
                matches!(result, Err(TaskError::DerivedKeyIndexUnavailable)),
                "{key_type:?}: expected DerivedKeyIndexUnavailable for the alias, got {result:?}",
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_add_requires_the_identitys_own_wallet() {
        // Stored paths disagree on the wallet identity index: ambiguous, so no
        // wallet is trusted for derivation.
        let stage = stage_derivation(true).await;
        let mut identity = stage.identity.clone();
        let mut stray = identity.private_keys.identity_public_keys()[0].1.clone();
        stray.identity_public_key.set_id(1);
        let stray_path = WalletDerivationPath {
            wallet_seed_hash: stage.seed_hash,
            derivation_path: DerivationPath::identity_authentication_path(
                Network::Testnet,
                KeyDerivationType::ECDSA,
                1,
                1,
            ),
        };
        stray.in_wallet_at_derivation_path = Some(stray_path.clone());
        identity
            .identity
            .add_public_key(stray.identity_public_key.clone());
        identity.private_keys.insert_if_absent(
            (PrivateKeyOnMainIdentity, 1),
            (stray, PrivateKeyData::AtWalletDerivationPath(stray_path)),
        );
        stage
            .staged
            .ctx
            .update_local_qualified_identity(&identity)
            .unwrap();
        let result = add_derived(&stage, identity, KeyType::ECDSA_SECP256K1, 2).await;
        assert!(
            matches!(result, Err(TaskError::DerivedKeyWalletRequired)),
            "expected DerivedKeyWalletRequired for an ambiguous wallet index, got {result:?}",
        );

        // The path names a wallet that is not associated with the identity here.
        let stage = stage_derivation(false).await;
        let result = add_derived(&stage, stage.identity.clone(), KeyType::ECDSA_SECP256K1, 1).await;
        assert!(
            matches!(result, Err(TaskError::DerivedKeyWalletRequired)),
            "expected DerivedKeyWalletRequired without the associated wallet, got {result:?}",
        );
    }

    /// SEC-001: a poisoned cache entry must never reach the chain. HASH160 has
    /// no proof of possession, so the backend re-derives from the seed and
    /// refuses the add on any disagreement — then repairs the entry so the next
    /// attempt uses the verified key.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_add_refuses_a_cached_key_the_seed_does_not_derive() {
        let stage = stage_derivation(true).await;
        let genuine = stage.cache.get(Network::Testnet, 0, 1).unwrap();
        let planted = stage.cache.get(Network::Testnet, 0, 7).unwrap();
        let mut poisoned = stage.cache.clone();
        poisoned.insert(Network::Testnet, 0, 1, &planted);
        let backend = stage.staged.ctx.wallet_backend().unwrap();
        let view = backend.auth_pubkey_cache();
        view.put(Network::Testnet, &stage.seed_hash, &poisoned)
            .unwrap();

        let result = add_derived(&stage, stage.identity.clone(), KeyType::ECDSA_HASH160, 1).await;
        assert!(
            matches!(result, Err(TaskError::DerivedKeySeedMismatch)),
            "expected DerivedKeySeedMismatch, got {result:?}",
        );
        assert_eq!(
            view.get(Network::Testnet, &stage.seed_hash)
                .get(Network::Testnet, 0, 1),
            Some(genuine),
            "the cache entry is repaired to the seed-derived key",
        );

        let (_, key) = stage
            .staged
            .ctx
            .prepare_derived_identity_key(
                &stage.staged.id,
                derived_request(KeyType::ECDSA_HASH160),
                1,
            )
            .await
            .expect("the repaired entry passes");
        let expected: [u8; 20] = genuine.pubkey_hash().into();
        assert_eq!(key.identity_public_key.data().as_slice(), expected);
    }

    /// A cold cache entry is filled from the seed rather than rejected, and the
    /// key carries the canonical wallet path for later signing and recovery.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn derived_add_fills_a_cold_cache_entry_from_the_seed() {
        use crate::model::wallet::auth_pubkey_cache::AuthPubkeyCache;

        let stage = stage_derivation(true).await;
        let backend = stage.staged.ctx.wallet_backend().unwrap();
        let view = backend.auth_pubkey_cache();
        let mut cold = AuthPubkeyCache::default();
        cold.insert(
            Network::Testnet,
            0,
            0,
            &stage.cache.get(Network::Testnet, 0, 0).unwrap(),
        );
        view.put(Network::Testnet, &stage.seed_hash, &cold).unwrap();

        let (_, key) = stage
            .staged
            .ctx
            .prepare_derived_identity_key(
                &stage.staged.id,
                derived_request(KeyType::ECDSA_SECP256K1),
                3,
            )
            .await
            .expect("a cold entry is derived from the seed");
        let genuine = stage.cache.get(Network::Testnet, 0, 3).unwrap();
        assert_eq!(
            key.identity_public_key.data().as_slice(),
            genuine.to_bytes()
        );
        assert_eq!(
            view.get(Network::Testnet, &stage.seed_hash)
                .get(Network::Testnet, 0, 3),
            Some(genuine)
        );
        assert_eq!(
            key.in_wallet_at_derivation_path,
            Some(WalletDerivationPath {
                wallet_seed_hash: stage.seed_hash,
                derivation_path: DerivationPath::identity_authentication_path(
                    Network::Testnet,
                    KeyDerivationType::ECDSA,
                    0,
                    3,
                ),
            })
        );
    }

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

    /// R1: an unload that completes during the broadcast must not leave the new
    /// key's private half in the vault.
    ///
    /// The removal deletes the placements the *stored blob* names. The key
    /// being added is not in that blob yet, so it is not in the removal's
    /// delete set and not in the cleanup manifest it clears. A seal that lands
    /// afterwards writes private key material for an identity with no record,
    /// no roster entry and no manifest — unreachable through the identity
    /// model, so no sweep can ever collect it, on a device that has just told
    /// the user it destroyed that identity's keys.
    ///
    /// The record guard now spans the recheck, the seal and the write, so a
    /// delisted identity ends the task before anything is sealed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unload_during_the_broadcast_leaves_no_key_in_the_vault() {
        use crate::context::test_staging::stage_identity_with_vaulted_keys_using_prompt;
        use crate::model::secret::Secret;

        const PW: &str = "identity-object-passwordpw";
        const NEW_KEY_ID: u32 = 9;
        let prompt = Arc::new(TestPrompt::new([ScriptedAnswer::once(PW)]));
        let staged = stage_identity_with_vaulted_keys_using_prompt(
            Network::Testnet,
            Some(prompt),
            [0xAA; 32],
            [0xBB; 32],
        )
        .await;
        let ctx = &staged.ctx;

        // A password-protected identity, which is the shape that seals.
        ctx.protect_identity_keys(staged.id, Secret::new(PW), None)
            .expect("seal the identity Tier-2");
        let mut qualified_identity = ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read the protected identity")
            .expect("identity present");

        // The precondition the real flow satisfies before broadcasting.
        let verify_scope = ctx
            .protected_identity_verify_scope(&qualified_identity)
            .expect("read the verify scope")
            .expect("a protected identity has one");
        let password = ctx
            .wallet_backend()
            .expect("backend wired")
            .secret_access()
            .verify_identity_object_password(&verify_scope)
            .await
            .expect("the scripted password verifies");

        // The user unloads the identity while the broadcast is in flight.
        ctx.delete_local_qualified_identity(&staged.id)
            .expect("unload the identity");

        let new_key = (PrivateKeyTarget::PrivateKeyOnMainIdentity, NEW_KEY_ID);
        let public_key = add_new_key_to_snapshot(&mut qualified_identity, NEW_KEY_ID, [0xCD; 32]);
        let error = ctx
            .persist_added_identity_key(
                &qualified_identity,
                (new_key.0.clone(), new_key.1),
                &public_key,
                Some(&[0xCD; 32]),
                Some(password),
            )
            .expect_err("an identity that is gone cannot take a new key");
        assert!(
            matches!(error, TaskError::IdentityKeyAddedButIdentityUnloaded),
            "expected IdentityKeyAddedButIdentityUnloaded, got {error:?}",
        );

        assert!(
            crate::wallet_backend::IdentityKeyView::new(&staged.store, staged.id.to_buffer())
                .get(&new_key.0, new_key.1)
                .expect("read the vault")
                .is_none(),
            "the new key must not be sealed for an identity whose keys the removal \
             just destroyed — nothing would ever point at it or collect it",
        );
        assert!(
            !ctx.is_identity_listed(&staged.id).expect("read the roster"),
            "and the identity must not be put back on the roster",
        );
    }

    /// What `add_key_to_identity` does to its snapshot before the broadcast:
    /// the new public key joins the identity and its private half is filed
    /// at `(main, key_id)`. Returns the new public key.
    fn add_new_key_to_snapshot(
        snapshot: &mut QualifiedIdentity,
        key_id: KeyID,
        private_key: [u8; 32],
    ) -> QualifiedIdentityPublicKey {
        use dash_sdk::dpp::version::PlatformVersion;
        use dash_sdk::platform::IdentityPublicKey;

        let public_key: QualifiedIdentityPublicKey = IdentityPublicKey::random_key(
            key_id,
            Some(u64::from(key_id)),
            PlatformVersion::latest(),
        )
        .into();
        snapshot
            .identity
            .add_public_key(public_key.identity_public_key.clone());
        snapshot
            .private_keys
            .insert_non_encrypted(
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id),
                (public_key.clone(), private_key),
            )
            .expect("file the new key on the snapshot");
        public_key
    }

    /// Another writer lands a manual key (and an alias) while the add-key
    /// broadcast is in flight, as a load-merge, discovery or key paste would.
    fn save_concurrent_key_and_alias(ctx: &AppContext, identity_id: &Identifier, key_id: KeyID) {
        use dash_sdk::dpp::version::PlatformVersion;
        use dash_sdk::platform::IdentityPublicKey;

        let concurrent: QualifiedIdentityPublicKey =
            IdentityPublicKey::random_key(key_id, Some(99), PlatformVersion::latest()).into();
        ctx.edit_local_qualified_identity(identity_id, |fresh| {
            fresh
                .identity
                .add_public_key(concurrent.identity_public_key.clone());
            fresh.private_keys.insert_non_encrypted(
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id),
                (concurrent.clone(), [0x11; 32]),
            )
        })
        .expect("save the concurrent key");
        ctx.set_identity_alias(identity_id, Some("renamed meanwhile"))
            .expect("rename the identity");
    }

    /// SEC-101: the post-broadcast persist applies only this add to the record
    /// as it is on disk now; a key and an alias another writer saved during
    /// the broadcast survive.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn persisting_an_added_key_keeps_what_others_saved_during_the_broadcast() {
        use crate::context::test_staging::stage_identity_with_vaulted_keys;

        const NEW_KEY_ID: KeyID = 10;
        const CONCURRENT_KEY_ID: KeyID = 9;
        let staged = stage_identity_with_vaulted_keys([0xAA; 32], [0xBB; 32]).await;
        let ctx = &staged.ctx;
        let mut snapshot = ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read the identity")
            .expect("identity present");
        let public_key = add_new_key_to_snapshot(&mut snapshot, NEW_KEY_ID, [0xCD; 32]);

        save_concurrent_key_and_alias(ctx, &staged.id, CONCURRENT_KEY_ID);

        ctx.persist_added_identity_key(
            &snapshot,
            (PrivateKeyTarget::PrivateKeyOnMainIdentity, NEW_KEY_ID),
            &public_key,
            Some(&[0xCD; 32]),
            None,
        )
        .expect("persist the added key");

        let stored = ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read back")
            .expect("identity present");
        let keys = stored.private_keys.keys_set();
        for key_id in [1, 2, 3, CONCURRENT_KEY_ID, NEW_KEY_ID] {
            assert!(
                keys.contains(&(PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id)),
                "key {key_id} must be on the stored record, found {keys:?}",
            );
        }
        assert_eq!(
            stored.alias.as_deref(),
            Some("renamed meanwhile"),
            "an alias saved during the broadcast must survive",
        );
        assert!(
            stored.identity.public_keys().contains_key(&NEW_KEY_ID),
            "the refreshed on-chain identity is stored with the new key",
        );
    }

    /// SEC-104: the identity got password-protected while a keyless add was
    /// in flight. The key is on the network but cannot be stored without a
    /// protection downgrade, so the task reports the typed "added but not
    /// saved" outcome — not a raw storage error — and writes nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_protect_during_the_broadcast_reports_the_key_as_not_saved() {
        use crate::context::test_staging::stage_identity_with_vaulted_keys;
        use crate::model::secret::Secret;

        const NEW_KEY_ID: KeyID = 10;
        let staged = stage_identity_with_vaulted_keys([0xAA; 32], [0xBB; 32]).await;
        let ctx = &staged.ctx;
        let mut snapshot = ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read the identity")
            .expect("identity present");
        let public_key = add_new_key_to_snapshot(&mut snapshot, NEW_KEY_ID, [0xCD; 32]);

        ctx.protect_identity_keys(staged.id, Secret::new("identity-object-passwordpw"), None)
            .expect("protect the identity meanwhile");

        let error = ctx
            .persist_added_identity_key(
                &snapshot,
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, NEW_KEY_ID),
                &public_key,
                Some(&[0xCD; 32]),
                None,
            )
            .expect_err("a keyless key cannot join a protected identity");
        assert!(
            matches!(&error, TaskError::IdentityKeyAddedButNotSavedWhileProtected),
            "expected IdentityKeyAddedButNotSavedWhileProtected, got {error:?}",
        );
        let stored = ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read back")
            .expect("identity present");
        assert!(
            !stored
                .private_keys
                .keys_set()
                .contains(&(PrivateKeyTarget::PrivateKeyOnMainIdentity, NEW_KEY_ID)),
            "nothing is written for the refused key",
        );
    }

    /// SEC-104: a slot taken on disk during the broadcast by a different key
    /// is a post-broadcast failure too, reported as "added but not saved".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_occupied_slot_after_the_broadcast_reports_the_key_as_not_saved() {
        use crate::context::test_staging::stage_identity_with_vaulted_keys;

        const NEW_KEY_ID: KeyID = 10;
        let staged = stage_identity_with_vaulted_keys([0xAA; 32], [0xBB; 32]).await;
        let ctx = &staged.ctx;
        let mut snapshot = ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read the identity")
            .expect("identity present");
        let public_key = add_new_key_to_snapshot(&mut snapshot, NEW_KEY_ID, [0xCD; 32]);
        // A different key lands in the same slot on disk.
        save_concurrent_key_and_alias(ctx, &staged.id, NEW_KEY_ID);

        let error = ctx
            .persist_added_identity_key(
                &snapshot,
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, NEW_KEY_ID),
                &public_key,
                Some(&[0xCD; 32]),
                None,
            )
            .expect_err("the slot belongs to another key");
        assert!(
            matches!(error, TaskError::IdentityKeyAddedButSlotOccupied),
            "expected IdentityKeyAddedButSlotOccupied, got {error:?}",
        );
    }

    /// Thread 1d955045: the two causes whose generic remedy (refresh, then
    /// enter the key) would be refused again get their own advice; every
    /// other cause keeps the generic, source-preserving variant.
    #[test]
    fn user_entered_not_saved_advice_follows_the_cause() {
        let entered = AddedKeyOrigin::of(Some(&[0x11; 32]));
        let protected = entered.not_saved(TaskError::IdentityKeyProtectionDowngrade);
        assert!(matches!(
            protected,
            TaskError::IdentityKeyAddedButNotSavedWhileProtected
        ));
        assert!(
            protected
                .to_string()
                .contains("remove the password protection from this identity"),
            "the protected cause names the extra step, got {protected}"
        );
        let occupied = entered.not_saved(TaskError::IdentityKeySlotOccupied);
        assert!(matches!(
            occupied,
            TaskError::IdentityKeyAddedButSlotOccupied
        ));
        assert!(
            occupied
                .to_string()
                .contains("remove its saved private key"),
            "the occupied cause names the conflict to resolve, got {occupied}"
        );
        for error in [&protected, &occupied] {
            let shown = error.to_string();
            assert!(shown.contains("added to your identity on the network"));
            assert!(shown.contains("Copy the new private key now"));
        }
        // A derived key keeps its single variant: reloading from the wallet
        // resolves an occupied slot, and reports partial protection itself.
        assert!(matches!(
            AddedKeyOrigin::of(None).not_saved(TaskError::IdentityKeySlotOccupied),
            TaskError::DerivedIdentityKeyAddedButNotSaved { .. }
        ));
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
        let mapped = AddedKeyOrigin::of(Some(&[0x11; 32])).not_saved(TaskError::IdentityKeyMissing);
        assert!(
            matches!(mapped, TaskError::IdentityKeyAddedButNotSaved { .. }),
            "a post-broadcast seal failure must map to the typed orphan error, got {mapped:?}"
        );
        assert!(
            matches!(
                AddedKeyOrigin::of(Some(&[0x11; 32])).identity_unloaded(),
                TaskError::IdentityKeyAddedButIdentityUnloaded
            ),
            "a user-entered key keeps the copy-your-key advice when its identity was unloaded"
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

    /// A wallet-derived key has no private half to copy, so its "added but
    /// not saved" outcomes are dedicated variants that never ask for one.
    #[test]
    fn derived_post_broadcast_failure_never_asks_to_copy_a_private_key() {
        use std::error::Error as _;
        let mapped = AddedKeyOrigin::of(None).not_saved(TaskError::IdentityKeyMissing);
        assert!(
            matches!(mapped, TaskError::DerivedIdentityKeyAddedButNotSaved { .. }),
            "a derived key maps to the derived variant, got {mapped:?}"
        );
        assert!(mapped.source().is_some(), "the upstream cause is preserved");
        for error in [mapped, AddedKeyOrigin::of(None).identity_unloaded()] {
            let shown = error.to_string();
            assert!(
                shown.contains("added to your identity on the network"),
                "message must tell the user the key is on-chain, got {shown}"
            );
            assert!(
                !shown.contains("Copy") && !shown.contains("private key"),
                "a derived key has no private key to copy, got {shown}"
            );
        }
    }

    /// A staged identity's snapshot with a wallet-derived key filed at
    /// `(main, key_id)` the way `add_identity_key` files it before the
    /// broadcast. Returns the snapshot and the new key.
    async fn derived_snapshot(
        key_id: KeyID,
    ) -> (
        crate::context::test_staging::StagedIdentity,
        QualifiedIdentity,
        QualifiedIdentityPublicKey,
    ) {
        use crate::context::test_staging::stage_identity_with_vaulted_keys;
        use crate::model::derived_identity_key::test_support::fixture;

        let staged = stage_identity_with_vaulted_keys([0xAA; 32], [0xBB; 32]).await;
        let (fixture_identity, cache, seed_hash, _) = fixture();
        let mut snapshot = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read the identity")
            .expect("identity present");
        let mut key = fixture_identity.private_keys.identity_public_keys()[0]
            .1
            .clone();
        key.identity_public_key.set_id(key_id);
        key.identity_public_key
            .set_security_level(dash_sdk::dpp::identity::SecurityLevel::HIGH);
        key.identity_public_key
            .set_data(cache.get(Network::Testnet, 0, 3).unwrap().to_bytes().into());
        key.in_wallet_at_derivation_path = Some(WalletDerivationPath {
            wallet_seed_hash: seed_hash,
            derivation_path: DerivationPath::identity_authentication_path(
                Network::Testnet,
                KeyDerivationType::ECDSA,
                0,
                3,
            ),
        });
        insert_derived_key(&mut snapshot, &key).expect("file the derived key");
        snapshot
            .identity
            .add_public_key(key.identity_public_key.clone());
        (staged, snapshot, key)
    }

    /// SEC-101 for a derived key: the persist files only the derivation path
    /// onto the record as it is on disk now, so a key and an alias saved
    /// during the broadcast survive.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn persisting_a_derived_key_keeps_what_others_saved_during_the_broadcast() {
        const NEW_KEY_ID: KeyID = 10;
        const CONCURRENT_KEY_ID: KeyID = 9;
        let (staged, snapshot, key) = derived_snapshot(NEW_KEY_ID).await;
        let ctx = &staged.ctx;
        save_concurrent_key_and_alias(ctx, &staged.id, CONCURRENT_KEY_ID);

        let placement = (PrivateKeyTarget::PrivateKeyOnMainIdentity, NEW_KEY_ID);
        ctx.persist_added_identity_key(&snapshot, placement.clone(), &key, None, None)
            .expect("persist the derived key");

        let stored = ctx
            .get_local_qualified_identity(&staged.id)
            .expect("read back")
            .expect("identity present");
        let (data, path) = stored
            .private_keys
            .get_cloned_private_key_data_and_wallet_info(&placement)
            .expect("the derived key is filed");
        assert!(matches!(data, PrivateKeyData::AtWalletDerivationPath(_)));
        assert_eq!(path, key.in_wallet_at_derivation_path);
        assert!(
            stored.private_keys.keys_set().contains(&(
                PrivateKeyTarget::PrivateKeyOnMainIdentity,
                CONCURRENT_KEY_ID
            )),
            "a key saved during the broadcast survives"
        );
        assert_eq!(stored.alias.as_deref(), Some("renamed meanwhile"));
    }

    /// A derived key whose slot another key took during the broadcast is
    /// refused and reported with the derived "added but not saved" variant.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_occupied_slot_reports_the_derived_key_as_not_saved() {
        const NEW_KEY_ID: KeyID = 10;
        let (staged, snapshot, key) = derived_snapshot(NEW_KEY_ID).await;
        let ctx = &staged.ctx;
        save_concurrent_key_and_alias(ctx, &staged.id, NEW_KEY_ID);

        let error = ctx
            .persist_added_identity_key(
                &snapshot,
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, NEW_KEY_ID),
                &key,
                None,
                None,
            )
            .expect_err("the slot belongs to another key");
        assert!(
            matches!(
                &error,
                TaskError::DerivedIdentityKeyAddedButNotSaved { source }
                    if matches!(**source, TaskError::IdentityKeySlotOccupied)
            ),
            "expected DerivedIdentityKeyAddedButNotSaved over the occupied slot, got {error:?}",
        );
    }

    /// A derived key for an identity unloaded during the broadcast reports
    /// the derived "unloaded" variant and writes nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unload_during_the_broadcast_reports_the_derived_key_as_unloaded() {
        const NEW_KEY_ID: KeyID = 10;
        let (staged, snapshot, key) = derived_snapshot(NEW_KEY_ID).await;
        let ctx = &staged.ctx;
        ctx.delete_local_qualified_identity(&staged.id)
            .expect("unload the identity");

        let error = ctx
            .persist_added_identity_key(
                &snapshot,
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, NEW_KEY_ID),
                &key,
                None,
                None,
            )
            .expect_err("an identity that is gone cannot take a new key");
        assert!(
            matches!(error, TaskError::DerivedIdentityKeyAddedButIdentityUnloaded),
            "expected DerivedIdentityKeyAddedButIdentityUnloaded, got {error:?}",
        );
        assert!(
            ctx.get_local_qualified_identity(&staged.id)
                .expect("read back")
                .is_none(),
            "nothing is written for an unloaded identity",
        );
    }
}
