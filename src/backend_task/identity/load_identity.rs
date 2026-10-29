use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityInputToLoad, IdentityLoadMode};
use crate::backend_task::{NETWORK_REQUEST_TIMEOUT, await_network_request_with_timeout};
use crate::context::AppContext;
use crate::model::derived_identity_key::recovery_scan_bound;
use crate::model::identity_key_protection::validate_protection_password;
use crate::model::key_input::verify_key_input;
use crate::model::masternode_input::decode_identity_id;
use crate::model::masternode_keys::MasternodeKeyHashes;
use crate::model::qualified_identity::PrivateKeyTarget::{
    self, PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::identity::add_new_identity_screen::MAX_IDENTITY_INDEX;
use crate::wallet_backend::MasternodeWalletKeys;
use dash_sdk::Sdk;
use dash_sdk::dashcore_rpc::dashcore::PrivateKey;
use dash_sdk::dpp::dashcore::ProTxHash;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::identity::SecurityLevel;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyID, KeyType, Purpose};
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, KeyDerivationType};
use dash_sdk::platform::transition::masternode_vote_keys::{select_voting_key, voter_identity_id};
use dash_sdk::platform::{Fetch, Identity};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::convert::TryInto;
use std::sync::{Arc, RwLock};
use zeroize::Zeroizing;

type WalletKeyMap = BTreeMap<(PrivateKeyTarget, u32), (QualifiedIdentityPublicKey, PrivateKeyData)>;
type WalletMatchResult = Option<(WalletSeedHash, u32, WalletKeyMap)>;

/// Merge an already-stored identity's keys and associations into a freshly
/// built one, preserving anything the new (partial) load did not resupply
/// (§10.8, the "Add voting key" in-place update). Keys the new load provides
/// win on collision; keys it omits (e.g. Owner/Payout on a voting-key-only
/// update) are carried over from `existing` rather than lost. The existing
/// alias and identity associations are kept only when the new build lacks them.
///
/// Load-path only: this fills gaps from field *absence*, which is safe when a
/// user is actively re-loading (the missing field is genuinely being resupplied)
/// but is NOT valid for the background legacy migration, where an absent field
/// can be a deliberate removal (a cleared alias, a "Remove private key from DET").
///
/// A carried-over main-identity key takes the state the freshly fetched
/// identity gives it, so a key the network disabled since it was stored (an
/// owner or payout key replaced by a rotation) is no longer offered for
/// signing; its private material is kept.
fn merge_existing_keys_into(new: &mut QualifiedIdentity, existing: QualifiedIdentity) {
    for (placement, (mut public_key, data)) in existing.private_keys.into_entries() {
        if placement.0 == PrivateKeyOnMainIdentity
            && let Some(current) = new.identity.get_public_key_by_id(placement.1)
            && current.data() == public_key.identity_public_key.data()
        {
            public_key.identity_public_key = current.clone();
        }
        new.private_keys
            .insert_if_absent(placement, (public_key, data));
    }
    if new.alias.is_none() {
        new.alias = existing.alias;
    }
    if new.associated_voter_identity.is_none() {
        new.associated_voter_identity = existing.associated_voter_identity;
    }
    if new.associated_operator_identity.is_none() {
        new.associated_operator_identity = existing.associated_operator_identity;
    }
    if new.associated_owner_key_id.is_none() {
        new.associated_owner_key_id = existing.associated_owner_key_id;
    }
}

/// Drop `existing`'s voter identity and voting keys when the masternode list
/// names a different voting key (`current_voting_hash`), so a merge cannot
/// carry a retired voting key over a load that found no replacement. With no
/// current list to check against (`None`: unread, still catching up, or
/// offline), everything is kept, since a stale list may name a key the node
/// has since replaced.
fn drop_retired_voting_key(
    existing: &mut QualifiedIdentity,
    current_voting_hash: Option<[u8; 20]>,
) {
    use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
    let Some(current) = current_voting_hash else {
        return;
    };
    let retired =
        |key: &dash_sdk::platform::IdentityPublicKey| key.public_key_hash().ok() != Some(current);
    if existing
        .associated_voter_identity
        .as_ref()
        .is_some_and(|(_, key)| retired(key))
    {
        existing.associated_voter_identity = None;
    }
    let retired_keys: Vec<_> = existing
        .private_keys
        .identity_public_keys()
        .into_iter()
        .filter(|(target, key)| {
            **target == PrivateKeyOnVoterIdentity && retired(&key.identity_public_key)
        })
        .map(|(target, key)| (target.clone(), key.identity_public_key.id()))
        .collect();
    for placement in retired_keys {
        // TODO: a retired voting key's vault secret, if it had one, stays in
        // the vault, as an `Overwrite` load leaves a replaced key's today.
        existing.private_keys.remove_at(&placement);
    }
}

/// How a load writes the identity record once its keys are settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IdentityRecordWrite {
    /// Deliberate import: lists the identity and retires an unload marker.
    Import,
    /// Scoped update of a listed identity; never restores a removed one.
    UpdateListed,
}

impl AppContext {
    pub(super) async fn load_identity(
        &self,
        sdk: &Sdk,
        input: IdentityInputToLoad,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let IdentityInputToLoad {
            identity_id_input,
            identity_type,
            voting_private_key_input,
            alias_input,
            owner_private_key_input,
            payout_address_private_key_input,
            keys_input,
            derive_keys_from_wallets,
            selected_wallet_seed_hash,
            encryption_password,
            load_mode,
            load_token,
        } = input;

        // Parse the identity ID. It names the load, so it has to be resolved before
        // anything that can fail — a load that failed has to be reportable, and a
        // load is only reportable once it is claimed under its identity.
        let identity_id = match decode_identity_id(&identity_id_input) {
            Ok(id) => id,
            Err(_e) => {
                // For masternodes/evonodes the identity id field IS a ProTxHash
                // (hex) or Base58 identity id — surface the ProTxHash-specific
                // message so the user is told the exact accepted formats.
                if identity_type != IdentityType::User {
                    return Err(TaskError::MalformedProTxHash {
                        input: identity_id_input,
                    });
                }
                return Err(TaskError::IdentifierParsingError {
                    input: identity_id_input,
                });
            }
        };

        // Claim the identity before the first fallible step. Two jobs:
        //
        // The duplicate check below, the network fetch, the insert and the key seal
        // are not one atomic step: two overlapping loads of this identity could both
        // pass the check and both insert, the last one clobbering the first's alias,
        // keys and protection tier. The claim excludes a second load of it, from any
        // screen, tool or CLI, for that whole span.
        //
        // The guard is also how this load reports its outcome to a screen that
        // navigated away (task results reach only the visible screen): dropped on
        // any error path — including the validation below — it records `Failed`, and
        // only the explicit `loaded()` after the last fallible step records success.
        // Validating before the claim would leave a rejected load unreportable, and
        // the screen waiting on it stuck on "Loading…" for the rest of the session.
        //
        // The claim is made under this load's own token, so it reports into the
        // record its dispatcher is waiting on — never one another load owns.
        let load_guard = self.begin_identity_load(identity_id, load_token)?;

        // Validate before the network fetch and again at the protected storage boundary.
        if let Some(password) = &encryption_password {
            validate_protection_password(password)?;
        }

        // Verify the owner private key
        let owner_private_key_bytes = verify_key_input(owner_private_key_input, "Owner")?;

        // Verify the voting private key
        let voting_private_key_bytes = verify_key_input(voting_private_key_input, "Voting")?;

        let payout_address_private_key_bytes =
            verify_key_input(payout_address_private_key_input, "Payout Address")?;

        // §10.9 / TC-EDGE-07: a fresh load rejects a ProTxHash already stored,
        // before any network fetch — so the existing node's alias/keys/protection
        // tier are never silently overwritten. Checked here, at the storage
        // layer, so every `RejectIfExists` caller is guarded uniformly.
        let existing_stored = self.get_local_qualified_identity(&identity_id)?;
        match load_mode {
            IdentityLoadMode::RejectIfExists if existing_stored.is_some() => {
                return Err(TaskError::DuplicateProTxHash { identity_id });
            }
            IdentityLoadMode::MergeIntoExisting if existing_stored.is_none() => {
                return Err(TaskError::IdentityNotFoundLocally);
            }
            _ => {}
        }

        // Prompt before network work; revalidate against current protection under the record lock.
        let merge_seal_password = match (&load_mode, existing_stored.as_ref()) {
            (IdentityLoadMode::MergeIntoExisting, Some(existing))
                if encryption_password.is_none() =>
            {
                match self.protected_identity_verify_scope(existing)? {
                    Some(verify_scope) => Some(
                        self.wallet_backend()?
                            .secret_access()
                            .verify_identity_object_password(&verify_scope)
                            .await?,
                    ),
                    None => None,
                }
            }
            _ => None,
        };

        // Fetch the identity using the SDK
        let identity = match await_network_request_with_timeout(
            NETWORK_REQUEST_TIMEOUT,
            Identity::fetch_by_identifier(sdk, identity_id),
            |source| TaskError::IdentityLoadTimeout { source },
        )
        .await?
        {
            Ok(Some(identity)) => identity,
            // For masternode/evonode loads the input is a ProTxHash, so surface a
            // node-specific message instead of the generic identity-not-found copy
            // (which talks about an "ID or name" the user never entered here).
            Ok(None) if identity_type != IdentityType::User => {
                return Err(TaskError::MasternodeNotFound { identity_id });
            }
            Ok(None) => return Err(TaskError::IdentityNotFound),
            Err(e) => return Err(TaskError::from(e)),
        };

        let mut encrypted_private_keys = BTreeMap::new();

        let wallets = self.wallet_context().wallets();

        if identity_type == IdentityType::User
            && derive_keys_from_wallets
            && let Some((_, _, wallet_private_keys)) = self
                .match_user_identity_keys_with_wallet(
                    &identity,
                    &wallets,
                    selected_wallet_seed_hash,
                )
                .await?
        {
            encrypted_private_keys.extend(wallet_private_keys);
        }

        // The voting key is not on the node's identity: the masternode list
        // says which one is current. `None` unless that list is current, so a
        // list still catching up never retires the stored key.
        let current_voting_hash = match self.wallet_backend() {
            Ok(backend)
                if identity_type != IdentityType::User && voting_private_key_bytes.is_none() =>
            {
                backend.masternode_voting_key_hash(identity.id()).await
            }
            _ => None,
        };
        // A node key left blank may be held by one of this device's wallets:
        // reference it at its derivation path instead of asking for it. A
        // merge never swaps a key the node already stores for a reference, but
        // a stored node key the network has since disabled or replaced does not
        // count: after a rotation the new key is looked up.
        let (node_key_hashes, wallet_keys) = if identity_type != IdentityType::User {
            let already_stored = |target: PrivateKeyTarget, purpose: Purpose| {
                load_mode == IdentityLoadMode::MergeIntoExisting
                    && existing_stored.as_ref().is_some_and(|existing| {
                        stores_current_node_key(
                            existing,
                            &identity,
                            target,
                            purpose,
                            current_voting_hash,
                        )
                    })
            };
            self.find_node_keys_in_wallets(
                &identity,
                owner_private_key_bytes.is_none()
                    && !already_stored(PrivateKeyOnMainIdentity, Purpose::OWNER),
                payout_address_private_key_bytes.is_none()
                    && !already_stored(PrivateKeyOnMainIdentity, Purpose::TRANSFER),
                current_voting_hash
                    .filter(|_| !already_stored(PrivateKeyOnVoterIdentity, Purpose::VOTING)),
            )
            .await
        } else {
            Default::default()
        };
        let mut insert_wallet_held_key = |purpose, hash: Option<[u8; 20]>, path| {
            if let Some((key_id, entry)) = wallet_held_key_entry(&identity, purpose, hash, path) {
                encrypted_private_keys.insert((PrivateKeyOnMainIdentity, key_id), entry);
            }
        };
        if owner_private_key_bytes.is_none() {
            insert_wallet_held_key(Purpose::OWNER, node_key_hashes.owner, wallet_keys.owner);
        }
        if payout_address_private_key_bytes.is_none() {
            insert_wallet_held_key(
                Purpose::TRANSFER,
                node_key_hashes.payout,
                wallet_keys.payout,
            );
        }

        if identity_type != IdentityType::User
            && let Some(owner_private_key_bytes) = owner_private_key_bytes
        {
            let key =
                self.verify_owner_key_exists_on_identity(&identity, &owner_private_key_bytes)?;
            let key_id = key.id();
            let qualified_key =
                QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
                    key,
                    self.network,
                    &wallets.values().collect::<Vec<_>>(),
                );
            encrypted_private_keys.insert(
                (PrivateKeyOnMainIdentity, key_id),
                (
                    qualified_key,
                    PrivateKeyData::Clear(*owner_private_key_bytes),
                ),
            );
        }

        if identity_type != IdentityType::User
            && let Some(payout_address_private_key_bytes) = payout_address_private_key_bytes
        {
            let key = self.verify_payout_address_key_exists_on_identity(
                &identity,
                &payout_address_private_key_bytes,
            )?;
            let key_id = key.id();
            let qualified_key =
                QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
                    key,
                    self.network,
                    &wallets.values().collect::<Vec<_>>(),
                );
            encrypted_private_keys.insert(
                (PrivateKeyOnMainIdentity, key_id),
                (
                    qualified_key,
                    PrivateKeyData::Clear(*payout_address_private_key_bytes),
                ),
            );
        }

        // The voting key comes from the typed input, or else from a wallet that
        // holds the key the masternode list records for this node.
        let voting_key = match voting_private_key_bytes {
            _ if identity_type == IdentityType::User => None,
            Some(bytes) => {
                let private_key = PrivateKey::from_byte_array(&bytes, self.network)
                    .map_err(|_| TaskError::InvalidPrivateKey)?;
                Some((
                    private_key.public_key().pubkey_hash().to_byte_array(),
                    PrivateKeyData::Clear(*bytes),
                ))
            }
            None => node_key_hashes
                .voting
                .zip(wallet_keys.voting)
                .map(|(hash, path)| (hash, PrivateKeyData::AtWalletDerivationPath(path))),
        };

        let mut associated_voter_identity = None;
        if let Some((voting_address, voting_key_data)) = voting_key {
            // A wallet-found voting key is a bonus the user did not ask for:
            // without a usable voter identity the node loads without it
            // instead of failing.
            let from_wallet = matches!(voting_key_data, PrivateKeyData::AtWalletDerivationPath(_));
            let voter_identifier = voter_identity_id(
                ProTxHash::from_byte_array(identity_id.to_buffer()),
                &voting_address,
            );
            let fetched = await_network_request_with_timeout(
                NETWORK_REQUEST_TIMEOUT,
                Identity::fetch_by_identifier(sdk, voter_identifier),
                |source| TaskError::IdentityLoadTimeout { source },
            )
            .await
            .and_then(|result| result.map_err(TaskError::from));
            let voter_identity = match fetched {
                Ok(Some(identity)) => Some(identity),
                Ok(None) if from_wallet => None,
                Ok(None) => return Err(TaskError::MasternodeVotingKeyNotFound),
                Err(error) if from_wallet => {
                    tracing::warn!(
                        ?error,
                        "Could not fetch the voter identity for the voting key found in a wallet; loading the node without it"
                    );
                    None
                }
                Err(error) => return Err(error),
            };
            let usable = voter_identity.and_then(|voter| {
                let key = select_voting_key(&voter, &voting_address)?.clone();
                Some((voter, key))
            });
            match usable {
                Some((voter_identity, key)) => {
                    // A wallet reference stays in the private-key data only;
                    // see `wallet_held_key_entry`.
                    let qualified_key = match &voting_key_data {
                        PrivateKeyData::AtWalletDerivationPath(_) => {
                            QualifiedIdentityPublicKey::from(key.clone())
                        }
                        _ => {
                            QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
                                key.clone(),
                                self.network,
                                &wallets.values().collect::<Vec<_>>(),
                            )
                        }
                    };
                    encrypted_private_keys.insert(
                        (PrivateKeyOnVoterIdentity, key.id()),
                        (qualified_key, voting_key_data),
                    );
                    associated_voter_identity = Some((voter_identity, key));
                }
                None if from_wallet => tracing::debug!(
                    "The voting key found in a wallet has no usable voter identity; loading the node without it"
                ),
                None => return Err(TaskError::MasternodeVotingKeyNotUsable),
            }
        }

        // let mut wallet_seed_hash: Option<(WalletSeedHash, u32)> = None;

        if identity_type == IdentityType::User {
            // Reserved up front: a growing `Vec` moves its keys to a new
            // buffer and frees the old one without wiping it.
            let mut input_private_keys: Vec<PrivateKey> = Vec::with_capacity(keys_input.len());
            for key_string in keys_input {
                if let Some(sk) = verify_key_input(key_string, "User Key")? {
                    input_private_keys.push(
                        PrivateKey::from_byte_array(&sk, self.network)
                            .map_err(|_| TaskError::InvalidPrivateKey)?,
                    );
                }
            }

            #[allow(clippy::type_complexity)]
            let (public_key_lookup, public_key_hash_lookup): (
                HashMap<Vec<u8>, Zeroizing<[u8; 32]>>,
                HashMap<[u8; 20], Zeroizing<[u8; 32]>>,
            ) = input_private_keys
                .into_iter()
                .map(|private_key| {
                    let public_key = private_key.public_key();
                    let public_key_bytes = public_key.to_bytes();
                    let pub_key_hash = public_key.pubkey_hash().to_byte_array();
                    (
                        (public_key_bytes, private_key.inner.to_bytes()),
                        (pub_key_hash, private_key.inner.to_bytes()),
                    )
                })
                .unzip();

            for (&key_id, public_key) in identity.public_keys().iter() {
                let key_map_key = (PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id);
                let qualified_key =
                    QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
                        public_key.clone(),
                        self.network,
                        &wallets.values().collect::<Vec<_>>(),
                    );
                if let Some(private_key_bytes) =
                    public_key_lookup.get(public_key.data().0.as_slice())
                {
                    let private_data = match public_key.security_level() {
                        SecurityLevel::MEDIUM => PrivateKeyData::AlwaysClear(**private_key_bytes),
                        _ => PrivateKeyData::Clear(**private_key_bytes),
                    };
                    encrypted_private_keys
                        .insert(key_map_key, (qualified_key.clone(), private_data));
                    continue;
                }

                if let Some(private_key_bytes) =
                    public_key_hash_lookup.get(public_key.data().0.as_slice())
                {
                    let private_data = match public_key.security_level() {
                        SecurityLevel::MEDIUM => PrivateKeyData::AlwaysClear(**private_key_bytes),
                        _ => PrivateKeyData::Clear(**private_key_bytes),
                    };
                    encrypted_private_keys
                        .insert(key_map_key, (qualified_key.clone(), private_data));
                    continue;
                }

                if encrypted_private_keys.contains_key(&key_map_key) {
                    continue;
                }

                if let Some(wallet_derivation_path) =
                    qualified_key.in_wallet_at_derivation_path.clone()
                {
                    encrypted_private_keys.insert(
                        key_map_key,
                        (
                            qualified_key,
                            PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path),
                        ),
                    );
                }
            }
        }

        let maybe_owned_dpns_names = await_network_request_with_timeout(
            NETWORK_REQUEST_TIMEOUT,
            self.fetch_owned_dpns_names(sdk, identity_id),
            |source| TaskError::IdentityLoadTimeout { source },
        )
        .await??;

        let mut qualified_identity = QualifiedIdentity {
            identity,
            associated_voter_identity,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type,
            alias: None,
            private_keys: encrypted_private_keys.into(),
            dpns_names: maybe_owned_dpns_names,
            associated_wallets: wallets
                .values()
                .map(|wallet| {
                    let w = wallet.read()?;
                    Ok::<_, TaskError>((w.seed_hash(), wallet.clone()))
                })
                .collect::<Result<_, _>>()?,
            secret_access: self.wallet_backend().ok().map(|b| b.secret_access()),
            wallet_index: None, //todo
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: self.network,
        };
        // Name an unnamed node after its main username, never an arbitrary one.
        let main_username = self.main_username(&qualified_identity);
        qualified_identity.initialize_node_alias(Some(&alias_input), main_username.as_deref());
        // Recheck the scoped load and merge the current stored keys under the
        // record guard before any seal or write: removal and protection changes
        // may have happened while the network requests were in flight.
        let wallet_info = self.finish_identity_load_storage(
            &mut qualified_identity,
            load_mode,
            merge_seal_password.as_ref(),
            &load_guard,
            encryption_password.as_ref(),
            current_voting_hash,
        )?;

        if let Some((wallet_seed_hash, identity_index)) = wallet_info
            && let Some(wallet_arc) = wallets.get(&wallet_seed_hash)
        {
            let mut wallet = wallet_arc.write().map_err(TaskError::from)?;
            wallet
                .identities
                .insert(identity_index, qualified_identity.identity.clone());
        }

        // Past the last fallible step: the node is stored with its keys as
        // requested. Anything that failed before this — including a key seal that
        // left the insert behind — reported `Failed` when the guard dropped.
        load_guard.loaded();

        Ok(BackendTaskSuccessResult::LoadedIdentity(qualified_identity))
    }

    /// Store a fetched identity under its record lock, after confirming that
    /// this load is still current: removal, a fresh import and protection
    /// changes may have happened while the network requests were in flight.
    ///
    /// `current_voting_hash` is the voting key the masternode list names for
    /// a node, if read: a merge drops a stored voting key that differs.
    fn finish_identity_load_storage(
        &self,
        qualified_identity: &mut QualifiedIdentity,
        load_mode: IdentityLoadMode,
        merge_password: Option<&crate::wallet_backend::VerifiedIdentityPassword>,
        load_guard: &crate::context::identity_load_registry::IdentityLoadGuard,
        encryption_password: Option<&crate::model::secret::Secret>,
        current_voting_hash: Option<[u8; 20]>,
    ) -> Result<Option<(WalletSeedHash, u32)>, TaskError> {
        if let Some(password) = encryption_password {
            validate_protection_password(password)?;
        }
        let identity_id = qualified_identity.identity.id();
        let lock = self.identity_record_lock(identity_id);
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        load_guard.ensure_current()?;
        if load_mode != IdentityLoadMode::MergeIntoExisting {
            let existing = self.get_local_qualified_identity(&identity_id)?;
            return self.persist_loaded_identity_locked(
                qualified_identity,
                existing,
                encryption_password,
                load_mode,
                IdentityRecordWrite::Import,
            );
        }
        // A scoped merge is a key fix-up of a listed identity; it must never
        // restore an identity removed while the network requests were in flight.
        if !self.is_identity_listed(&identity_id)? {
            return Err(TaskError::IdentityNotFoundLocally);
        }
        let mut existing = self
            .get_identity_by_id(&identity_id)?
            .ok_or(TaskError::IdentityNotFoundLocally)?;
        if encryption_password.is_none() {
            // A verified password belongs to the protection state observed before
            // the fetch. Never seal a new key using it after that state changes.
            match (
                self.protected_identity_verify_scope(&existing)?,
                merge_password,
            ) {
                (None, None) => {}
                (Some(scope), Some(password))
                    if self
                        .wallet_backend()?
                        .secret_access()
                        .identity_object_password_still_opens(&scope, password)? => {}
                _ => return Err(TaskError::IdentityLoadSuperseded { identity_id }),
            }
        }
        // The form may carry metadata captured before the fetch; preserve edits
        // made to the current local record.
        qualified_identity.alias = existing.alias.clone();
        qualified_identity.associated_wallets = existing.associated_wallets.clone();
        qualified_identity.wallet_index = existing.wallet_index;
        qualified_identity.top_ups = existing.top_ups.clone();
        qualified_identity.status = existing.status;
        drop_retired_voting_key(&mut existing, current_voting_hash);
        match encryption_password {
            None => self.persist_merged_identity_locked(
                qualified_identity,
                Some(existing),
                merge_password,
                IdentityRecordWrite::UpdateListed,
            ),
            Some(password) => self.persist_loaded_identity_locked(
                qualified_identity,
                Some(existing),
                Some(password),
                load_mode,
                IdentityRecordWrite::UpdateListed,
            ),
        }
    }

    /// Merge `qi` into its stored record, taking the record lock itself.
    #[cfg(test)]
    fn persist_merged_identity(
        &self,
        qi: &mut QualifiedIdentity,
        password: Option<&crate::wallet_backend::VerifiedIdentityPassword>,
    ) -> Result<Option<(WalletSeedHash, u32)>, TaskError> {
        let identity_id = qi.identity.id();
        let lock = self.identity_record_lock(identity_id);
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let existing = self.get_local_qualified_identity(&identity_id)?;
        self.persist_merged_identity_locked(qi, existing, password, IdentityRecordWrite::Import)
    }

    /// Merge the newly supplied keys of `qi` into `existing` and seal them under
    /// the revalidated identity password. The caller holds the record lock.
    fn persist_merged_identity_locked(
        &self,
        qi: &mut QualifiedIdentity,
        existing: Option<QualifiedIdentity>,
        password: Option<&crate::wallet_backend::VerifiedIdentityPassword>,
        write: IdentityRecordWrite,
    ) -> Result<Option<(WalletSeedHash, u32)>, TaskError> {
        let identity_id = qi.identity.id();
        let published = existing.is_some();
        if let Some(existing) = existing {
            merge_existing_keys_into(qi, existing);
        }
        if self.protected_identity_verify_scope(qi)?.is_some() {
            // A surviving legacy `Encrypted` key has no vault entry, so sealing would
            // skip it and leave the protected record half-protected. Checked after the
            // merge so a resupplied key (the documented recovery) replaces it instead.
            if qi.private_keys.has_encrypted_legacy_keys() {
                return Err(TaskError::IdentityKeyProtectionLegacyFormat);
            }
            let password = password.ok_or(if published {
                TaskError::IdentityKeyProtectionDowngrade
            } else {
                TaskError::IdentityImportPasswordRequired
            })?;
            let backend = self.wallet_backend()?;
            let access = backend.secret_access();
            let view = crate::wallet_backend::IdentityKeyView::new(
                backend.secret_store(),
                identity_id.to_buffer(),
            );
            let mut placements = qi.private_keys.keys_set();
            placements.extend(self.retained_identity_import_keys(&identity_id)?);
            for (target, key_id) in &placements {
                if view.scheme(target, *key_id)?
                    == crate::wallet_backend::secret_seam::SecretScheme::Protected
                {
                    let scope = crate::wallet_backend::secret_prompt::SecretScope::IdentityKey {
                        identity_id: identity_id.to_buffer(),
                        target: target.clone(),
                        key_id: *key_id,
                    };
                    if !access.identity_object_password_still_opens(&scope, password)? {
                        return Err(TaskError::IdentityKeyPassphraseIncorrect);
                    }
                }
            }
            self.seal_merged_plaintext_keys(qi, password)?;
        }
        let wallet_info = qi
            .determine_wallet_info()
            .map_err(|detail| TaskError::WalletInfoDeterminationFailed { detail })?;
        self.write_loaded_identity_locked(qi, &wallet_info, write)?;
        Ok(wallet_info)
    }

    /// Store a loaded identity, taking the record lock itself.
    #[cfg(test)]
    fn persist_loaded_identity(
        &self,
        qi: &mut QualifiedIdentity,
        password: Option<&crate::model::secret::Secret>,
        load_mode: IdentityLoadMode,
    ) -> Result<Option<(WalletSeedHash, u32)>, TaskError> {
        if let Some(password) = password {
            validate_protection_password(password)?;
        }
        let identity_id = qi.identity.id();
        let lock = self.identity_record_lock(identity_id);
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let existing = self.get_local_qualified_identity(&identity_id)?;
        self.persist_loaded_identity_locked(
            qi,
            existing,
            password,
            load_mode,
            IdentityRecordWrite::Import,
        )
    }

    /// Store a loaded identity, sealing every key under `password` before the
    /// record is written. The caller holds the record lock and has validated
    /// `password`.
    fn persist_loaded_identity_locked(
        &self,
        qi: &mut QualifiedIdentity,
        existing: Option<QualifiedIdentity>,
        password: Option<&crate::model::secret::Secret>,
        load_mode: IdentityLoadMode,
        write: IdentityRecordWrite,
    ) -> Result<Option<(WalletSeedHash, u32)>, TaskError> {
        let identity_id = qi.identity.id();
        if load_mode == IdentityLoadMode::RejectIfExists && existing.is_some() {
            return Err(TaskError::DuplicateProTxHash { identity_id });
        }
        let Some(password) = password else {
            if existing.is_none()
                && qi.private_keys.has_plaintext_for_vault()
                && self.protected_identity_verify_scope(qi)?.is_some()
            {
                return Err(TaskError::IdentityImportPasswordRequired);
            }
            let wallet_info = qi
                .determine_wallet_info()
                .map_err(|detail| TaskError::WalletInfoDeterminationFailed { detail })?;
            self.write_loaded_identity_locked(qi, &wallet_info, write)?;
            return Ok(wallet_info);
        };
        let mut relevant_keys = qi.private_keys.keys_set();
        relevant_keys.extend(self.retained_identity_import_keys(&identity_id)?);
        if let Some(existing) = existing {
            // Resident plaintext is sealed by the startup migration on relaunch.
            if existing.private_keys.has_plaintext_for_vault() {
                return Err(TaskError::IdentityKeyProtectionIncomplete);
            }
            relevant_keys.extend(existing.private_keys.keys_set());
            if load_mode == IdentityLoadMode::MergeIntoExisting {
                merge_existing_keys_into(qi, existing);
            }
        }
        // A legacy `Encrypted` key has no vault entry, so it cannot be sealed. Checked
        // on the effective key set, after the merge, so a resupplied key (the documented
        // recovery) or an `Overwrite` replaces it, while a surviving one fails before
        // any secret write.
        if qi.private_keys.has_encrypted_legacy_keys() {
            return Err(TaskError::IdentityKeyProtectionLegacyFormat);
        }
        let backend = self.wallet_backend()?;
        let view = crate::wallet_backend::IdentityKeyView::new(
            backend.secret_store(),
            identity_id.to_buffer(),
        );
        let password =
            platform_wallet_storage::secrets::SecretString::new(password.expose_secret());
        use crate::wallet_backend::secret_seam::SecretScheme;
        // Verify every existing password before changing any label, including keys omitted on reload.
        let mut stored_keys = BTreeMap::new();
        for (target, key_id) in relevant_keys.iter().cloned() {
            let scheme = view.scheme(&target, key_id)?;
            let raw = match scheme {
                SecretScheme::Protected => view.get_protected(&target, key_id, &password)?,
                SecretScheme::Unprotected => view.get(&target, key_id)?,
                SecretScheme::Absent => None,
            };
            stored_keys.insert((target, key_id), (scheme, raw));
        }
        let mut pending = Vec::new();
        for (placement, (_, data)) in qi.private_keys.iter() {
            let (scheme, stored) = stored_keys
                .get(placement)
                .ok_or(TaskError::IdentityKeyMissing)?;
            match data {
                PrivateKeyData::Clear(raw) | PrivateKeyData::AlwaysClear(raw) => {
                    if stored.as_ref().is_some_and(|stored| **stored != *raw) {
                        return Err(TaskError::IdentityImportKeyConflict);
                    }
                    if *scheme != SecretScheme::Protected {
                        pending.push((placement.clone(), zeroize::Zeroizing::new(*raw)));
                    }
                }
                PrivateKeyData::InVault => {
                    let raw = stored.as_ref().ok_or(TaskError::IdentityKeyMissing)?;
                    if *scheme != SecretScheme::Protected {
                        pending.push((placement.clone(), raw.clone()));
                    }
                }
                PrivateKeyData::Encrypted(_) => {
                    return Err(TaskError::IdentityKeyProtectionLegacyFormat);
                }
                PrivateKeyData::AtWalletDerivationPath(_) => {
                    if *scheme == SecretScheme::Unprotected {
                        let raw = stored.as_ref().ok_or(TaskError::IdentityKeyMissing)?;
                        pending.push((placement.clone(), raw.clone()));
                    }
                }
            }
        }
        self.record_identity_import_keys(&identity_id, &relevant_keys)?;
        // Retained placements remain discoverable even if the identity record never lands.
        for ((target, key_id), raw) in pending {
            view.store_protected(&target, key_id, &raw, &password)?;
        }
        drop(qi.private_keys.take_plaintext_for_vault());
        let wallet_info = qi
            .determine_wallet_info()
            .map_err(|detail| TaskError::WalletInfoDeterminationFailed { detail })?;
        self.write_loaded_identity_locked(qi, &wallet_info, write)?;
        Ok(wallet_info)
    }

    fn write_loaded_identity_locked(
        &self,
        qi: &QualifiedIdentity,
        wallet_info: &Option<(WalletSeedHash, u32)>,
        write: IdentityRecordWrite,
    ) -> Result<(), TaskError> {
        match write {
            IdentityRecordWrite::Import => {
                self.insert_local_qualified_identity_under_lock(qi, wallet_info)
            }
            IdentityRecordWrite::UpdateListed => self.write_local_qualified_identity_locked(qi),
        }
    }

    /// Record placements and seal plaintext under the caller-held record lock and revalidated password.
    pub(super) fn seal_merged_plaintext_keys(
        &self,
        qi: &mut QualifiedIdentity,
        password: &crate::wallet_backend::VerifiedIdentityPassword,
    ) -> Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        let secret_access = backend.secret_access();
        let id = qi.identity.id().to_buffer();
        self.record_identity_import_keys(&qi.identity.id(), &qi.private_keys.keys_set())?;
        // `take_plaintext_for_vault` flips each Clear/AlwaysClear key to `InVault`
        // and hands back its raw bytes; sealing each Tier-2 leaves the identity
        // fully protected with no keyless residue.
        for ((target, key_id), raw) in qi.private_keys.take_plaintext_for_vault() {
            secret_access
                .seal_new_identity_key_with_password(id, &target, key_id, &raw, password)?;
        }
        Ok(())
    }

    pub(super) async fn match_user_identity_keys_with_wallet(
        &self,
        identity: &Identity,
        wallets: &BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>,
        wallet_filter: Option<WalletSeedHash>,
    ) -> Result<WalletMatchResult, TaskError> {
        let highest_identity_key_id = identity.public_keys().keys().copied().max().unwrap_or(0);
        let top_bound = identity_key_scan_bound(highest_identity_key_id);

        for (&wallet_seed_hash, wallet_arc) in wallets.iter() {
            if wallet_filter.is_some_and(|filter| filter != wallet_seed_hash) {
                continue;
            }
            // Skip poisoned or closed wallets rather than failing the
            // whole operation; the read guard is released before any await.
            match wallet_arc.read() {
                Ok(guard) if guard.is_open() => {}
                _ => continue,
            }

            if let Some((identity_index, wallet_private_keys)) = self
                .attempt_match_identity_with_wallet(
                    identity,
                    wallet_arc,
                    wallet_seed_hash,
                    top_bound,
                )
                .await?
            {
                return Ok(Some((
                    wallet_seed_hash,
                    identity_index,
                    wallet_private_keys,
                )));
            }
        }

        Ok(None)
    }

    async fn attempt_match_identity_with_wallet(
        &self,
        identity: &Identity,
        wallet: &Arc<RwLock<Wallet>>,
        wallet_seed_hash: WalletSeedHash,
        top_bound: u32,
    ) -> Result<Option<(u32, WalletKeyMap)>, TaskError> {
        let identity_id = identity.id();

        let existing_index = wallet
            .read()?
            .identities
            .iter()
            .find(|(_, existing)| existing.id() == identity_id)
            .map(|(&index, _)| index);

        if let Some(identity_index) = existing_index {
            let (public_key_map, public_key_hash_map) = self
                .resolve_identity_auth_pubkeys_data_map(
                    wallet,
                    true,
                    true,
                    identity_index,
                    0..top_bound,
                )
                .await?;
            let wallet_private_keys = self.build_wallet_private_key_map(
                identity,
                wallet_seed_hash,
                identity_index,
                &public_key_map,
                &public_key_hash_map,
            );

            if !wallet_private_keys.is_empty() {
                return Ok(Some((identity_index, wallet_private_keys)));
            }
        }

        for candidate_index in 0..MAX_IDENTITY_INDEX {
            let (public_key_map, public_key_hash_map) = self
                .resolve_identity_auth_pubkeys_data_map(
                    wallet,
                    false,
                    true,
                    candidate_index,
                    0..top_bound,
                )
                .await?;

            if !Self::identity_matches_wallet_key_material(
                identity,
                &public_key_map,
                &public_key_hash_map,
            ) {
                continue;
            }

            let (public_key_map, public_key_hash_map) = self
                .resolve_identity_auth_pubkeys_data_map(
                    wallet,
                    true,
                    true,
                    candidate_index,
                    0..top_bound,
                )
                .await?;

            let wallet_private_keys = self.build_wallet_private_key_map(
                identity,
                wallet_seed_hash,
                candidate_index,
                &public_key_map,
                &public_key_hash_map,
            );

            if wallet_private_keys.is_empty() {
                continue;
            }

            return Ok(Some((candidate_index, wallet_private_keys)));
        }

        Ok(None)
    }

    fn identity_matches_wallet_key_material(
        identity: &Identity,
        public_key_map: &BTreeMap<Vec<u8>, u32>,
        public_key_hash_map: &BTreeMap<[u8; 20], u32>,
    ) -> bool {
        identity
            .public_keys()
            .values()
            .any(|public_key| match public_key.key_type() {
                KeyType::ECDSA_SECP256K1 => {
                    if public_key_map.contains_key(public_key.data().as_slice()) {
                        true
                    } else if let Ok(hash) = <[u8; 20]>::try_from(public_key.data().as_slice()) {
                        public_key_hash_map.contains_key(&hash)
                    } else {
                        false
                    }
                }
                KeyType::ECDSA_HASH160 => {
                    if let Ok(hash) = <[u8; 20]>::try_from(public_key.data().as_slice()) {
                        public_key_hash_map.contains_key(&hash)
                    } else {
                        false
                    }
                }
                _ => false,
            })
    }

    fn build_wallet_private_key_map(
        &self,
        identity: &Identity,
        wallet_seed_hash: WalletSeedHash,
        identity_index: u32,
        public_key_map: &BTreeMap<Vec<u8>, u32>,
        public_key_hash_map: &BTreeMap<[u8; 20], u32>,
    ) -> WalletKeyMap {
        identity
            .public_keys()
            .values()
            .filter_map(|public_key| {
                let index =
                    match public_key.key_type() {
                        KeyType::ECDSA_SECP256K1 => public_key_map
                            .get(public_key.data().as_slice())
                            .copied()
                            .or_else(|| {
                                public_key.data().as_slice().try_into().ok().and_then(
                                    |hash: [u8; 20]| public_key_hash_map.get(&hash).copied(),
                                )
                            }),
                        KeyType::ECDSA_HASH160 => public_key
                            .data()
                            .as_slice()
                            .try_into()
                            .ok()
                            .and_then(|hash: [u8; 20]| public_key_hash_map.get(&hash).copied()),
                        _ => None,
                    }?;

                let derivation_path = DerivationPath::identity_authentication_path(
                    self.network,
                    KeyDerivationType::ECDSA,
                    identity_index,
                    index,
                );

                let wallet_derivation_path = WalletDerivationPath {
                    wallet_seed_hash,
                    derivation_path,
                };

                Some((
                    (PrivateKeyTarget::PrivateKeyOnMainIdentity, public_key.id()),
                    (
                        QualifiedIdentityPublicKey::from_identity_public_key_in_wallet(
                            public_key.clone(),
                            Some(wallet_derivation_path.clone()),
                        ),
                        PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path),
                    ),
                ))
            })
            .collect()
    }
}

impl AppContext {
    /// The node's key hashes and where this device's wallets hold them, for
    /// the roles the user left blank. Best effort: a node loads with what was
    /// typed when no wallet holds a key or the wallets cannot be checked.
    async fn find_node_keys_in_wallets(
        &self,
        identity: &Identity,
        want_owner: bool,
        want_payout: bool,
        voting: Option<[u8; 20]>,
    ) -> (MasternodeKeyHashes, MasternodeWalletKeys) {
        let Ok(backend) = self.wallet_backend() else {
            return Default::default();
        };
        let on_identity = MasternodeKeyHashes::from_identity(identity);
        let hashes = MasternodeKeyHashes {
            owner: on_identity.owner.filter(|_| want_owner),
            payout: on_identity.payout.filter(|_| want_payout),
            voting,
        };
        match backend.find_masternode_wallet_keys(hashes).await {
            Ok(found) => (hashes, found),
            Err(error) => {
                tracing::warn!(
                    ?error,
                    "Could not check this device's wallets for the node's keys"
                );
                (hashes, MasternodeWalletKeys::default())
            }
        }
    }
}

/// Whether `existing` already stores the current `purpose` node key at
/// `target`, so a merge leaves it in place rather than looking the key up in
/// a wallet. A stored key that a rotation replaced does not count: a main
/// identity key must still be active on the freshly fetched `identity`, and a
/// voting key must hash to `current_voting_hash`, the voting key the
/// masternode list records. When that list cannot be read or is not current
/// (`None`) there is nothing to look up, so a stored voting key counts.
fn stores_current_node_key(
    existing: &QualifiedIdentity,
    identity: &Identity,
    target: PrivateKeyTarget,
    purpose: Purpose,
    current_voting_hash: Option<[u8; 20]>,
) -> bool {
    use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
    existing
        .private_keys
        .identity_public_keys()
        .iter()
        .any(|(t, key)| {
            let stored = &key.identity_public_key;
            if **t != target || stored.purpose() != purpose {
                return false;
            }
            match &target {
                PrivateKeyOnMainIdentity => identity
                    .get_public_key_by_id(stored.id())
                    .is_some_and(|current| current.purpose() == purpose && !current.is_disabled()),
                PrivateKeyOnVoterIdentity => current_voting_hash.is_none_or(|current| {
                    stored.public_key_hash().is_ok_and(|hash| hash == current)
                }),
                PrivateKeyTarget::PrivateKeyOnOperatorIdentity => true,
            }
        })
}

/// The key-storage entry for the active `purpose` key of `identity` whose
/// hash160 is `hash`, held by a wallet at `path`. Only the derivation
/// reference is stored — never key bytes.
///
/// The reference lives in the private-key data only. The public key's own
/// wallet path is what [`QualifiedIdentity::determine_wallet_info`] reads to
/// file an identity as one of a wallet's identities, and a provider or funding
/// path carries no identity index, so a node must not be filed there.
fn wallet_held_key_entry(
    identity: &Identity,
    purpose: Purpose,
    hash: Option<[u8; 20]>,
    path: Option<WalletDerivationPath>,
) -> Option<(KeyID, (QualifiedIdentityPublicKey, PrivateKeyData))> {
    let (hash, path) = hash.zip(path)?;
    let key = identity.public_keys().values().find(|key| {
        key.purpose() == purpose
            && key.key_type() == KeyType::ECDSA_HASH160
            && !key.is_disabled()
            && key.data().as_slice() == hash.as_slice()
    })?;
    Some((
        key.id(),
        (
            QualifiedIdentityPublicKey::from(key.clone()),
            PrivateKeyData::AtWalletDerivationPath(path),
        ),
    ))
}

/// Exclusive key-index bound of the wallet-match scan for an identity whose
/// highest key id is `highest_identity_key_id`. Built on the shared
/// seed-recovery window so a wallet-derived key stays rediscoverable.
pub(super) fn identity_key_scan_bound(highest_identity_key_id: u32) -> u32 {
    recovery_scan_bound(highest_identity_key_id).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_dir::ensure_env_file;
    use crate::backend_task::TaskResult;
    use crate::context::connection_status::ConnectionStatus;
    use crate::context::test_support::identity_public_key;
    use crate::database::test_helpers::create_database_at_path;
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::secret::Secret;
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::utils::tasks::TaskManager;
    use crate::wallet_backend::IdentityKeyView;
    use crate::wallet_backend::secret_seam::SecretScheme;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identifier::MasternodeIdentifiers;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::KeyID;
    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
    use dash_sdk::dpp::platform_value::string_encoding::Encoding;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::{Identifier, IdentityPublicKey};
    use platform_wallet_storage::secrets::SecretString;

    const M: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
    const V: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnVoterIdentity;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn additional_scenarios_wrong_node_voting_key_is_rejected_without_merge() {
        let staged = crate::context::test_staging::stage_identity_with_vaulted_keys(
            rand::random(),
            rand::random(),
        )
        .await;
        let before = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        let private_key = PrivateKey::from_byte_array(&rand::random(), Network::Testnet).unwrap();
        let address = private_key.public_key().pubkey_hash();
        let other_node = Identifier::from([0x75; 32]);
        let other_voter =
            Identifier::create_voter_identifier(other_node.as_bytes(), address.as_ref());
        let selected_voter =
            Identifier::create_voter_identifier(staged.id.as_bytes(), address.as_ref());
        assert_ne!(selected_voter, other_voter);
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch(staged.id, Some(before.identity.clone()))
            .await
            .unwrap();
        let mut other_identity =
            Identity::create_basic_identity(other_voter, PlatformVersion::latest()).unwrap();
        other_identity.add_public_keys([identity_public_key(
            0,
            dash_sdk::dpp::identity::Purpose::VOTING,
            SecurityLevel::HIGH,
            KeyType::ECDSA_HASH160,
            address.to_byte_array().to_vec(),
        )]);
        select_voting_key(&other_identity, &address.to_byte_array())
            .expect("the key is a usable voting key of the other node");
        sdk.mock()
            .expect_fetch(other_voter, Some(other_identity))
            .await
            .unwrap();
        sdk.mock()
            .expect_fetch(selected_voter, None::<Identity>)
            .await
            .unwrap();
        let input = IdentityInputToLoad {
            identity_id_input: staged.id.to_string(Encoding::Hex),
            identity_type: IdentityType::Masternode,
            alias_input: String::new(),
            voting_private_key_input: Secret::new(private_key.to_wif()),
            owner_private_key_input: Secret::default(),
            payout_address_private_key_input: Secret::default(),
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password: None,
            load_mode: IdentityLoadMode::MergeIntoExisting,
            load_token: None,
        };
        let error = staged.ctx.load_identity(&sdk, input).await.unwrap_err();
        let after = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            after.associated_voter_identity,
            before.associated_voter_identity
        );
        assert!(
            after.private_keys == before.private_keys,
            "a rejected key must preserve all stored key placements and values"
        );
        assert_eq!(
            error.to_string(),
            "This voting key could not be matched to the selected node. Check that node's voting private key and try again."
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn node_voting_key_disabled_on_the_network_is_refused() {
        let staged = crate::context::test_staging::stage_identity_with_vaulted_keys(
            rand::random(),
            rand::random(),
        )
        .await;
        let before = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        let private_key = PrivateKey::from_byte_array(&rand::random(), Network::Testnet).unwrap();
        let address = private_key.public_key().pubkey_hash();
        let voter = Identifier::create_voter_identifier(staged.id.as_bytes(), address.as_ref());
        let mut voter_identity =
            Identity::create_basic_identity(voter, PlatformVersion::latest()).unwrap();
        let mut voting_key = identity_public_key(
            0,
            dash_sdk::dpp::identity::Purpose::VOTING,
            SecurityLevel::HIGH,
            KeyType::ECDSA_HASH160,
            address.to_byte_array().to_vec(),
        );
        voting_key.set_disabled_at(1);
        voter_identity.add_public_keys([voting_key]);
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch(staged.id, Some(before.identity.clone()))
            .await
            .unwrap();
        sdk.mock()
            .expect_fetch(voter, Some(voter_identity))
            .await
            .unwrap();
        let input = IdentityInputToLoad {
            identity_id_input: staged.id.to_string(Encoding::Hex),
            identity_type: IdentityType::Masternode,
            alias_input: String::new(),
            voting_private_key_input: Secret::new(private_key.to_wif()),
            owner_private_key_input: Secret::default(),
            payout_address_private_key_input: Secret::default(),
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password: None,
            load_mode: IdentityLoadMode::MergeIntoExisting,
            load_token: None,
        };

        let error = staged.ctx.load_identity(&sdk, input).await.unwrap_err();

        assert!(
            matches!(error, TaskError::MasternodeVotingKeyNotUsable),
            "a voting key the network disabled must be refused as such, got {error:?}"
        );
        let after = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            after.associated_voter_identity,
            before.associated_voter_identity
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scoped_merge_cannot_restore_an_identity_removed_during_loading() {
        let staged =
            crate::context::test_staging::stage_identity_with_vaulted_keys([0x77; 32], [0x88; 32])
                .await;
        let mut loaded = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        let claim = staged.ctx.begin_identity_load(staged.id, None).unwrap();
        staged
            .ctx
            .delete_local_qualified_identity(&staged.id)
            .unwrap();
        let result = staged.ctx.finish_identity_load_storage(
            &mut loaded,
            IdentityLoadMode::MergeIntoExisting,
            None,
            &claim,
            None,
            None,
        );
        assert!(result.is_err(), "a scoped merge must not undo a removal");
        assert!(!staged.ctx.is_identity_listed(&staged.id).unwrap());
        assert!(
            staged
                .ctx
                .get_local_qualified_identity(&staged.id)
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scoped_merge_cannot_modify_a_reimported_identity() {
        let staged =
            crate::context::test_staging::stage_identity_with_vaulted_keys([0x79; 32], [0x89; 32])
                .await;
        let mut loaded = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        let claim = staged.ctx.begin_identity_load(staged.id, None).unwrap();
        staged
            .ctx
            .delete_local_qualified_identity(&staged.id)
            .unwrap();
        let mut reimported = loaded.clone();
        reimported.alias = Some("explicit reimport".into());
        staged
            .ctx
            .insert_local_qualified_identity(&reimported, &None)
            .unwrap();
        assert!(matches!(
            staged.ctx.finish_identity_load_storage(
                &mut loaded,
                IdentityLoadMode::MergeIntoExisting,
                None,
                &claim,
                None,
                None
            ),
            Err(TaskError::IdentityLoadSuperseded { .. })
        ));
        claim.loaded();
        assert_eq!(
            staged
                .ctx
                .get_local_qualified_identity(&staged.id)
                .unwrap()
                .unwrap()
                .alias,
            reimported.alias
        );
        assert_eq!(
            staged.ctx.last_identity_load_phase(&staged.id),
            Some(crate::context::identity_load_registry::IdentityLoadPhase::Failed)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn removal_cancels_older_imports_but_allows_a_fresh_explicit_import() {
        let staged =
            crate::context::test_staging::stage_identity_with_vaulted_keys([0x7c; 32], [0x8c; 32])
                .await;
        let mut loaded = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        let old_claim = staged.ctx.begin_identity_load(staged.id, None).unwrap();
        staged
            .ctx
            .delete_local_qualified_identity(&staged.id)
            .unwrap();
        for mode in [
            IdentityLoadMode::Overwrite,
            IdentityLoadMode::RejectIfExists,
        ] {
            assert!(
                matches!(
                    staged.ctx.finish_identity_load_storage(
                        &mut loaded,
                        mode,
                        None,
                        &old_claim,
                        None,
                        None
                    ),
                    Err(TaskError::IdentityLoadSuperseded { .. })
                ),
                "a removed load must not store after its claim was invalidated"
            );
        }
        let fresh_claim = staged.ctx.begin_identity_load(staged.id, None).unwrap();
        loaded.alias = Some("fresh explicit import".into());
        staged
            .ctx
            .finish_identity_load_storage(
                &mut loaded,
                IdentityLoadMode::Overwrite,
                None,
                &fresh_claim,
                None,
                None,
            )
            .unwrap();
        fresh_claim.loaded();
        drop(old_claim);
        assert!(staged.ctx.is_identity_listed(&staged.id).unwrap());
        assert_eq!(
            staged.ctx.last_identity_load_phase(&staged.id),
            Some(crate::context::identity_load_registry::IdentityLoadPhase::Loaded)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scoped_merge_preserves_changes_made_while_loading() {
        let staged =
            crate::context::test_staging::stage_identity_with_vaulted_keys([0x7a; 32], [0x8a; 32])
                .await;
        let mut loaded = staged.ctx.get_identity_by_id(&staged.id).unwrap().unwrap();
        let claim = staged.ctx.begin_identity_load(staged.id, None).unwrap();
        let mut current = loaded.clone();
        current.alias = Some("renamed during load".into());
        current.top_ups.insert(9, 77);
        staged
            .ctx
            .save_top_ups(&staged.id, &current.top_ups)
            .unwrap();
        current.status = IdentityStatus::NotFound;
        loaded.alias = Some("stale form alias".into());
        loaded.private_keys = Default::default();
        staged
            .ctx
            .insert_local_qualified_identity(&current, &None)
            .unwrap();
        staged
            .ctx
            .finish_identity_load_storage(
                &mut loaded,
                IdentityLoadMode::MergeIntoExisting,
                None,
                &claim,
                None,
                None,
            )
            .unwrap();
        let stored = staged.ctx.get_identity_by_id(&staged.id).unwrap().unwrap();
        assert_eq!(stored.alias, current.alias);
        assert_eq!(stored.top_ups, current.top_ups);
        assert_eq!(stored.status, current.status);
        assert_eq!(
            stored.private_keys.keys_set(),
            current.private_keys.keys_set()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scoped_merge_rejects_protection_changed_while_loading() {
        let staged =
            crate::context::test_staging::stage_identity_with_vaulted_keys([0x7b; 32], [0x8b; 32])
                .await;
        let mut loaded = staged
            .ctx
            .get_local_qualified_identity(&staged.id)
            .unwrap()
            .unwrap();
        let claim = staged.ctx.begin_identity_load(staged.id, None).unwrap();
        staged
            .ctx
            .protect_identity_keys(staged.id, Secret::new("new-identity-password"), None)
            .unwrap();
        assert!(matches!(
            staged.ctx.finish_identity_load_storage(
                &mut loaded,
                IdentityLoadMode::MergeIntoExisting,
                None,
                &claim,
                None,
                None
            ),
            Err(TaskError::IdentityLoadSuperseded { .. })
        ));
        let backend = staged.ctx.wallet_backend().unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), staged.id.to_buffer());
        for (target, key_id) in [(M, 1), (M, 2)] {
            assert_eq!(
                view.scheme(&target, key_id).unwrap(),
                SecretScheme::Protected
            );
            assert!(
                view.get_protected(&target, key_id, &SecretString::new("new-identity-password"))
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[tokio::test]
    async fn identity_network_timeout_is_typed_and_actionable() {
        let error = crate::backend_task::await_network_request_with_timeout(
            std::time::Duration::from_millis(1),
            std::future::pending::<()>(),
            |source| TaskError::IdentityLoadTimeout { source },
        )
        .await
        .expect_err("a pending identity request must time out");

        assert!(matches!(error, TaskError::IdentityLoadTimeout { .. }));
        assert!(error.to_string().contains("Check your connection"));
    }

    /// A keyless masternode-shaped identity: an owner key + an identity auth key
    /// on the main identity, plus a voting key on the voter identity — the shape
    /// `load_identity` builds for a Masternode. Returns the qi and its
    /// `(target, key_id)` triple.
    fn masternode_shaped_qi() -> (QualifiedIdentity, [(PrivateKeyTarget, KeyID); 3]) {
        let pv = PlatformVersion::latest();
        let mut ks = KeyStorage::default();
        let owner = IdentityPublicKey::random_key(1, Some(1), pv);
        let voter = IdentityPublicKey::random_key(2, Some(2), pv);
        let id_key = IdentityPublicKey::random_key(3, Some(3), pv);
        let triple = [(M, owner.id()), (V, voter.id()), (M, id_key.id())];
        ks.insert_at(
            (M, owner.id()),
            (
                QualifiedIdentityPublicKey::from(owner),
                PrivateKeyData::Clear([0xA0; 32]),
            ),
        );
        ks.insert_at(
            (V, voter.id()),
            (
                QualifiedIdentityPublicKey::from(voter),
                PrivateKeyData::Clear([0xB0; 32]),
            ),
        );
        ks.insert_at(
            (M, id_key.id()),
            (
                QualifiedIdentityPublicKey::from(id_key),
                PrivateKeyData::Clear([0xC0; 32]),
            ),
        );
        let identity =
            Identity::create_basic_identity(Identifier::random(), pv).expect("basic identity");
        let qi = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: None,
            private_keys: ks,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: Network::Testnet,
        };
        (qi, triple)
    }

    async fn protected_import_context() -> (Arc<AppContext>, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = open_protected_import_context(temp_dir.path()).await;
        (ctx, temp_dir)
    }

    async fn open_protected_import_context(data_dir: &std::path::Path) -> Arc<AppContext> {
        open_import_context(data_dir, None).await
    }

    async fn open_import_context(
        data_dir: &std::path::Path,
        prompt: Option<Arc<dyn crate::wallet_backend::secret_prompt::SecretPrompt>>,
    ) -> Arc<AppContext> {
        let data_dir = data_dir.to_path_buf();
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
        if let Some(prompt) = prompt {
            ctx.install_secret_prompt(prompt);
        }
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");

        ctx
    }

    fn reopen_vault_snapshot(
        dir: &std::path::Path,
    ) -> Arc<platform_wallet_storage::secrets::SecretStore> {
        // The live store holds an exclusive lock; reopen a copy of its completed on-disk writes.
        let snapshot = dir.join("snapshot");
        std::fs::create_dir_all(snapshot.join("secrets")).unwrap();
        let vault = "secrets/det-secrets.pwsvault";
        if dir.join(vault).exists() {
            std::fs::copy(dir.join(vault), snapshot.join(vault)).unwrap();
        }
        AppContext::open_secret_store(&snapshot).expect("reopen persisted vault snapshot")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_merge_retains_keys_after_failed_write_and_removal_cleans_them() {
        use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        let dir = tempfile::tempdir().unwrap();
        let ctx = open_import_context(
            dir.path(),
            Some(Arc::new(TestPrompt::new([ScriptedAnswer::once(
                "synthetic-merge-password",
            )]))),
        )
        .await;
        let (mut original, _) = masternode_shaped_qi();
        let id = original.identity.id();
        let password = Secret::new("synthetic-merge-password");
        ctx.persist_loaded_identity(
            &mut original,
            Some(&password),
            IdentityLoadMode::RejectIfExists,
        )
        .unwrap();
        let backend = ctx.wallet_backend().unwrap();
        let scope = ctx
            .protected_identity_verify_scope(&original)
            .unwrap()
            .unwrap();
        let verified = backend
            .secret_access()
            .verify_identity_object_password(&scope)
            .await
            .unwrap();
        let mut merged = original.clone();
        for id in [8, 9] {
            let key = IdentityPublicKey::random_key(id, Some(id as u64), PlatformVersion::latest());
            merged.private_keys.insert_at(
                (V, id),
                (
                    QualifiedIdentityPublicKey::from(key),
                    PrivateKeyData::Clear([id as u8; 32]),
                ),
            );
        }
        let fault = WriteFault::arm(2);
        assert!(
            ctx.persist_merged_identity(&mut merged, Some(&verified))
                .is_err()
        );
        assert_eq!(
            fault.schemes(),
            vec![SecretScheme::Protected, SecretScheme::Protected]
        );
        drop(fault);
        let retained = ctx.retained_identity_import_keys(&id).unwrap();
        assert!(
            retained.contains(&(V, 8)) && retained.contains(&(V, 9)),
            "every attempted merge placement must survive a failed seal"
        );
        let stored = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
        assert!(!stored.private_keys.has(&(V, 8)));
        ctx.delete_local_qualified_identity(&id).unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), id.to_buffer());
        assert_eq!(view.scheme(&V, 8).unwrap(), SecretScheme::Absent);
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_merge_rechecks_password_and_uses_current_protection() {
        use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        for reprotected in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let ctx = open_import_context(
                dir.path(),
                Some(Arc::new(TestPrompt::new([ScriptedAnswer::once(
                    "synthetic-merge-password",
                )]))),
            )
            .await;
            let (mut original, _) = masternode_shaped_qi();
            let id = original.identity.id();
            let password = Secret::new("synthetic-merge-password");
            ctx.persist_loaded_identity(
                &mut original,
                Some(&password),
                IdentityLoadMode::RejectIfExists,
            )
            .unwrap();
            let backend = ctx.wallet_backend().unwrap();
            let scope = ctx
                .protected_identity_verify_scope(&original)
                .unwrap()
                .unwrap();
            let verified = backend
                .secret_access()
                .verify_identity_object_password(&scope)
                .await
                .unwrap();
            ctx.unprotect_identity_keys(id, password).unwrap();
            if reprotected {
                ctx.protect_identity_keys(id, Secret::new("replacement-merge-password"), None)
                    .unwrap();
            }
            let mut merged = original;
            let key = IdentityPublicKey::random_key(9, Some(9), PlatformVersion::latest());
            merged.private_keys.insert_at(
                (V, 9),
                (
                    QualifiedIdentityPublicKey::from(key),
                    PrivateKeyData::Clear([9; 32]),
                ),
            );
            let fault = WriteFault::arm(0);
            let result = ctx.persist_merged_identity(&mut merged, Some(&verified));
            if reprotected {
                assert!(matches!(
                    result,
                    Err(TaskError::IdentityKeyPassphraseIncorrect)
                ));
                assert!(
                    fault.schemes().is_empty(),
                    "stale password must fail before any secret write"
                );
            } else {
                result.unwrap();
                assert_eq!(fault.schemes(), vec![SecretScheme::Unprotected]);
            }
            drop(fault);
            backend.shutdown().await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_never_stages_unprotected_keys() {
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        for fail_at in [1, 2, 3, 0] {
            let (ctx, dir) = protected_import_context().await;
            let (mut qi, triple) = masternode_shaped_qi();
            let identity_id = qi.identity.id();
            let fault = WriteFault::arm(fail_at);
            let result = ctx.persist_loaded_identity(
                &mut qi,
                Some(&Secret::new("synthetic-import-password")),
                IdentityLoadMode::RejectIfExists,
            );
            let schemes = fault.schemes();
            drop(fault);
            assert_eq!(result.is_err(), fail_at != 0);
            assert!(
                !schemes.is_empty(),
                "the fault must exercise a secret write"
            );
            assert!(
                schemes.iter().all(|s| *s == SecretScheme::Protected),
                "password-selected imports must protect the very first durable write"
            );
            let reopened = reopen_vault_snapshot(dir.path());
            let view = IdentityKeyView::new(&reopened, identity_id.to_buffer());
            for (target, key_id) in &triple {
                assert_ne!(
                    view.scheme(target, *key_id).unwrap(),
                    SecretScheme::Unprotected
                );
            }
            if fail_at != 0 {
                assert!(!ctx.is_identity_listed(&identity_id).unwrap());
                assert!(ctx.stored_identity_blob(&identity_id).unwrap().is_none());
            } else {
                assert!(ctx.is_identity_listed(&identity_id).unwrap());
                assert!(!qi.private_keys.has_plaintext_for_vault());
                for (target, key_id) in &triple {
                    assert!(
                        view.get_protected(
                            target,
                            *key_id,
                            &SecretString::new("synthetic-import-password")
                        )
                        .unwrap()
                        .is_some()
                    );
                }
            }
            ctx.wallet_backend().unwrap().shutdown().await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_retry_preserves_partial_keys() {
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        let (ctx, _dir) = protected_import_context().await;
        let (qi, _) = masternode_shaped_qi();
        let password = Secret::new("synthetic-import-password");
        let fault = WriteFault::arm(3);
        assert!(
            ctx.persist_loaded_identity(
                &mut qi.clone(),
                Some(&password),
                IdentityLoadMode::RejectIfExists
            )
            .is_err()
        );
        drop(fault);
        let error = ctx
            .persist_loaded_identity(&mut qi.clone(), None, IdentityLoadMode::RejectIfExists)
            .expect_err("a retained protected import needs its original password");
        assert_eq!(
            error.to_string(),
            "This import has password-protected keys saved from an earlier attempt. Retry the import with the password you chose for that attempt."
        );
        let fault = WriteFault::arm(0);
        let result = ctx.persist_loaded_identity(
            &mut qi.clone(),
            Some(&Secret::new("different-synthetic-password")),
            IdentityLoadMode::RejectIfExists,
        );
        assert!(
            result.is_err(),
            "retry must verify existing protected keys before writing"
        );
        assert!(
            fault.schemes().is_empty(),
            "wrong password must not mutate any label"
        );
        drop(fault);
        let mut retry = qi;
        ctx.persist_loaded_identity(
            &mut retry,
            Some(&password),
            IdentityLoadMode::RejectIfExists,
        )
        .expect("same-password retry");
        assert!(ctx.is_identity_listed(&retry.identity.id()).unwrap());
        ctx.wallet_backend().unwrap().shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_changed_retry_checks_retained_keys_and_removal_cleans_them() {
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        let (ctx, dir) = protected_import_context().await;
        let (qi, _) = masternode_shaped_qi();
        let id = qi.identity.id();
        let password = Secret::new("synthetic-import-password");
        let first = qi.private_keys.keys_set().first().unwrap().clone();
        let last = qi.private_keys.keys_set().last().unwrap().clone();
        let fault = WriteFault::arm(2);
        assert!(
            ctx.persist_loaded_identity(
                &mut qi.clone(),
                Some(&password),
                IdentityLoadMode::RejectIfExists,
            )
            .is_err()
        );
        drop(fault);
        assert!(ctx.stored_identity_blob(&id).unwrap().is_none());
        ctx.wallet_backend().unwrap().shutdown().await;
        // The SDK provider retains the original context; reopen durable snapshots instead.
        let snapshot = tempfile::tempdir().unwrap();
        for name in ["det-app.sqlite", "det-testnet.sqlite"] {
            let connection = rusqlite::Connection::open(dir.path().join(name)).unwrap();
            connection
                .execute(
                    "VACUUM INTO ?1",
                    [snapshot.path().join(name).to_str().unwrap()],
                )
                .unwrap();
        }
        std::fs::create_dir_all(snapshot.path().join("secrets")).unwrap();
        std::fs::copy(
            dir.path().join("secrets/det-secrets.pwsvault"),
            snapshot.path().join("secrets/det-secrets.pwsvault"),
        )
        .unwrap();
        let ctx = open_protected_import_context(snapshot.path()).await;

        let mut retry = qi.clone();
        retry.private_keys = KeyStorage::default();
        retry.private_keys.insert_at(
            last.clone(),
            qi.private_keys.entry_at(&last).unwrap().clone(),
        );
        let fault = WriteFault::arm(0);
        assert!(matches!(
            ctx.persist_loaded_identity(
                &mut retry,
                Some(&Secret::new("different-import-password")),
                IdentityLoadMode::RejectIfExists,
            ),
            Err(TaskError::IdentityKeyPassphraseIncorrect)
        ));
        assert!(fault.schemes().is_empty());
        drop(fault);

        ctx.persist_loaded_identity(
            &mut retry,
            Some(&password),
            IdentityLoadMode::RejectIfExists,
        )
        .expect("same-password subset retry");
        let backend = ctx.wallet_backend().unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), id.to_buffer());
        assert_eq!(
            view.scheme(&first.0, first.1).unwrap(),
            SecretScheme::Protected
        );
        assert!(
            !ctx.get_local_qualified_identity(&id)
                .unwrap()
                .unwrap()
                .private_keys
                .keys_set()
                .contains(&first)
        );
        ctx.unprotect_identity_keys(id, password)
            .expect("remove protection including retained keys");
        for placement in [&first, &last] {
            assert_eq!(
                view.scheme(&placement.0, placement.1).unwrap(),
                SecretScheme::Unprotected
            );
        }
        let replacement = Secret::new("replacement-import-password");
        ctx.protect_identity_keys(id, replacement.clone(), None)
            .expect("protect all retained keys again");
        for placement in [&first, &last] {
            assert!(
                view.get_protected(
                    &placement.0,
                    placement.1,
                    &SecretString::new(replacement.expose_secret())
                )
                .unwrap()
                .is_some()
            );
        }
        ctx.delete_local_qualified_identity(&id)
            .expect("remove imported identity and retained keys");
        for (target, key_id) in qi.private_keys.keys_set() {
            assert_eq!(view.scheme(&target, key_id).unwrap(), SecretScheme::Absent);
        }
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_retained_key_requires_password_for_later_keys() {
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        let (ctx, _dir) = protected_import_context().await;
        let (qi, _) = masternode_shaped_qi();
        let fault = WriteFault::arm(2);
        assert!(
            ctx.persist_loaded_identity(
                &mut qi.clone(),
                Some(&Secret::new("synthetic-password")),
                IdentityLoadMode::RejectIfExists
            )
            .is_err()
        );
        drop(fault);
        let mut watch_only = qi.clone();
        watch_only.private_keys = KeyStorage::default();
        ctx.persist_loaded_identity(&mut watch_only, None, IdentityLoadMode::RejectIfExists)
            .unwrap();
        assert!(
            ctx.protected_identity_verify_scope(&watch_only)
                .unwrap()
                .is_some(),
            "retained protected entries must require password verification"
        );
        let fault = WriteFault::arm(0);
        assert!(matches!(
            ctx.update_local_qualified_identity(&qi),
            Err(TaskError::IdentityKeyProtectionDowngrade)
        ));
        assert!(
            fault.schemes().is_empty(),
            "record writes must not downgrade retained protected keys"
        );
        drop(fault);
        ctx.wallet_backend().unwrap().shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_inventory_failure_prevents_secret_writes() {
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        let (ctx, dir) = protected_import_context().await;
        let (mut qi, _) = masternode_shaped_qi();
        let conn = rusqlite::Connection::open(dir.path().join("det-testnet.sqlite")).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_inventory BEFORE INSERT ON meta_global \
             WHEN NEW.key LIKE 'det:identity_import_keys:v1:%' \
             BEGIN SELECT RAISE(FAIL, 'injected inventory persistence failure'); END;",
        )
        .unwrap();
        let fault = WriteFault::arm(0);
        assert!(
            ctx.persist_loaded_identity(
                &mut qi,
                Some(&Secret::new("synthetic-import-password")),
                IdentityLoadMode::RejectIfExists,
            )
            .is_err()
        );
        assert!(fault.schemes().is_empty());
        drop(fault);
        assert!(
            ctx.stored_identity_blob(&qi.identity.id())
                .unwrap()
                .is_none()
        );
        assert!(
            ctx.retained_identity_import_keys(&qi.identity.id())
                .unwrap()
                .is_empty()
        );
        ctx.wallet_backend().unwrap().shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_inventory_cleanup_failure_is_resumed() {
        let (ctx, dir) = protected_import_context().await;
        let (mut qi, _) = masternode_shaped_qi();
        let id = qi.identity.id();
        ctx.persist_loaded_identity(
            &mut qi,
            Some(&Secret::new("synthetic-import-password")),
            IdentityLoadMode::RejectIfExists,
        )
        .unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("det-testnet.sqlite")).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_inventory_cleanup BEFORE DELETE ON meta_global \
             WHEN OLD.key LIKE 'det:identity_import_keys:v1:%' \
             BEGIN SELECT RAISE(FAIL, 'injected inventory cleanup failure'); END;",
        )
        .unwrap();
        assert!(ctx.delete_local_qualified_identity(&id).is_err());
        assert!(ctx.stored_identity_blob(&id).unwrap().is_none());
        assert!(!ctx.retained_identity_import_keys(&id).unwrap().is_empty());
        conn.execute_batch("DROP TRIGGER fail_inventory_cleanup")
            .unwrap();
        ctx.resume_pending_vault_cleanups();
        assert!(ctx.retained_identity_import_keys(&id).unwrap().is_empty());
        let backend = ctx.wallet_backend().unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), id.to_buffer());
        for (target, key_id) in qi.private_keys.keys_set() {
            assert_eq!(view.scheme(&target, key_id).unwrap(), SecretScheme::Absent);
        }
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_failed_reimport_keys_are_removed_by_pending_cleanup() {
        let (ctx, dir) = protected_import_context().await;
        let (qi, placements) = masternode_shaped_qi();
        let id = qi.identity.id();
        let password = Secret::new("synthetic-import-password");
        let mut original = qi.clone();
        original.private_keys = KeyStorage::default();
        let first = placements[0].clone();
        original.private_keys.insert_at(
            first.clone(),
            qi.private_keys.entry_at(&first).unwrap().clone(),
        );
        ctx.persist_loaded_identity(
            &mut original,
            Some(&password),
            IdentityLoadMode::RejectIfExists,
        )
        .unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("det-testnet.sqlite")).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_inventory_cleanup BEFORE DELETE ON meta_global \
             WHEN OLD.key LIKE 'det:identity_import_keys:v1:%' \
             BEGIN SELECT RAISE(FAIL, 'injected inventory cleanup failure'); END;",
        )
        .unwrap();
        assert!(ctx.delete_local_qualified_identity(&id).is_err());
        assert!(!ctx.is_identity_listed(&id).unwrap());
        conn.execute_batch(
            "DROP TRIGGER fail_inventory_cleanup;
             CREATE TRIGGER fail_reimport BEFORE INSERT ON meta_global
             WHEN NEW.key = 'det:identity_index:v1'
             BEGIN SELECT RAISE(FAIL, 'injected reimport failure'); END;",
        )
        .unwrap();
        assert!(
            ctx.persist_loaded_identity(
                &mut qi.clone(),
                Some(&password),
                IdentityLoadMode::RejectIfExists,
            )
            .is_err()
        );
        assert!(!ctx.is_identity_listed(&id).unwrap());
        assert!(ctx.stored_identity_blob(&id).unwrap().is_none());
        let backend = ctx.wallet_backend().unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), id.to_buffer());
        for (target, key_id) in &placements {
            assert_eq!(
                view.scheme(target, *key_id).unwrap(),
                SecretScheme::Protected
            );
        }
        assert_eq!(
            ctx.retained_identity_import_keys(&id).unwrap().len(),
            placements.len()
        );
        conn.execute_batch("DROP TRIGGER fail_reimport").unwrap();

        ctx.resume_pending_vault_cleanups();

        for (target, key_id) in &placements {
            assert_eq!(
                view.scheme(target, *key_id).unwrap(),
                SecretScheme::Absent,
                "resumed cleanup must include keys added after its manifest was saved"
            );
        }
        assert!(ctx.retained_identity_import_keys(&id).unwrap().is_empty());
        ctx.resume_pending_vault_cleanups();
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_persist_failure_keeps_recoverable_protected_keys() {
        for table in ["meta_global", "meta_identity"] {
            let (ctx, dir) = protected_import_context().await;
            let conn = rusqlite::Connection::open(dir.path().join("det-testnet.sqlite")).unwrap();
            let key = if table == "meta_global" {
                "det:identity_index:v1"
            } else {
                "det:identity:v1"
            };
            conn.execute_batch(&format!(
                "CREATE TRIGGER fail_import BEFORE INSERT ON {table} WHEN NEW.key = '{key}' \
                 BEGIN SELECT RAISE(FAIL, 'injected identity persistence failure'); END;"
            ))
            .unwrap();
            let (mut qi, triple) = masternode_shaped_qi();
            let password = Secret::new("synthetic-import-password");
            let expected = qi
                .private_keys
                .iter()
                .map(|(placement, (_, data))| {
                    let PrivateKeyData::Clear(raw) = data else {
                        panic!("synthetic clear key")
                    };
                    (placement.clone(), *raw)
                })
                .collect::<BTreeMap<_, _>>();
            assert!(
                ctx.persist_loaded_identity(
                    &mut qi,
                    Some(&password),
                    IdentityLoadMode::RejectIfExists
                )
                .is_err()
            );
            assert!(ctx.load_local_qualified_identities().unwrap().is_empty());
            assert!(
                ctx.stored_identity_blob(&qi.identity.id())
                    .unwrap()
                    .is_none()
            );
            let reopened = reopen_vault_snapshot(dir.path());
            let view = IdentityKeyView::new(&reopened, qi.identity.id().to_buffer());
            for placement in &triple {
                assert_eq!(
                    view.scheme(&placement.0, placement.1).unwrap(),
                    SecretScheme::Protected
                );
                assert_eq!(
                    *view
                        .get_protected(
                            &placement.0,
                            placement.1,
                            &SecretString::new(password.expose_secret())
                        )
                        .unwrap()
                        .unwrap(),
                    expected[placement]
                );
            }
            conn.execute_batch("DROP TRIGGER fail_import").unwrap();
            ctx.persist_loaded_identity(&mut qi, Some(&password), IdentityLoadMode::RejectIfExists)
                .expect("retry the saved protected keys");
            let stored = ctx
                .stored_identity_blob(&qi.identity.id())
                .unwrap()
                .unwrap();
            for raw in expected.values() {
                assert!(!stored.windows(raw.len()).any(|window| window == raw));
            }
            ctx.wallet_backend().unwrap().shutdown().await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_conflicts_are_preflighted_without_writes() {
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        for protected in [false, true] {
            let (ctx, _dir) = protected_import_context().await;
            let (mut qi, triple) = masternode_shaped_qi();
            let backend = ctx.wallet_backend().unwrap();
            let view = IdentityKeyView::new(backend.secret_store(), qi.identity.id().to_buffer());
            let (target, key_id) = &triple[2];
            let original = [0xD9; 32];
            let password = Secret::new("synthetic-import-password");
            if protected {
                view.store_protected(
                    target,
                    *key_id,
                    &original,
                    &SecretString::new(password.expose_secret()),
                )
                .unwrap();
            } else {
                view.store(target, *key_id, &original).unwrap();
            }
            let fault = WriteFault::arm(0);
            let result = ctx.persist_loaded_identity(
                &mut qi,
                Some(&password),
                IdentityLoadMode::RejectIfExists,
            );
            assert!(matches!(result, Err(TaskError::IdentityImportKeyConflict)));
            assert!(
                fault.schemes().is_empty(),
                "value conflicts must precede every write"
            );
            drop(fault);
            let saved = if protected {
                view.get_protected(
                    target,
                    *key_id,
                    &SecretString::new(password.expose_secret()),
                )
            } else {
                view.get(target, *key_id)
            }
            .unwrap()
            .unwrap();
            assert_eq!(*saved, original);
            assert!(!ctx.is_identity_listed(&qi.identity.id()).unwrap());
            backend.shutdown().await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_checks_every_existing_password_before_writes() {
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        let (ctx, _dir) = protected_import_context().await;
        let (mut qi, triple) = masternode_shaped_qi();
        let backend = ctx.wallet_backend().unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), qi.identity.id().to_buffer());
        let password = Secret::new("synthetic-import-password");
        for (index, (target, key_id)) in triple.iter().enumerate().take(2) {
            let (_, PrivateKeyData::Clear(raw)) = qi
                .private_keys
                .entry_at(&(target.clone(), *key_id))
                .unwrap()
            else {
                panic!("synthetic clear key")
            };
            let pw = if index == 0 {
                password.expose_secret()
            } else {
                "different-synthetic-password"
            };
            view.store_protected(target, *key_id, raw, &SecretString::new(pw))
                .unwrap();
        }
        let fault = WriteFault::arm(0);
        assert!(matches!(
            ctx.persist_loaded_identity(&mut qi, Some(&password), IdentityLoadMode::RejectIfExists),
            Err(TaskError::IdentityKeyPassphraseIncorrect)
        ));
        assert!(fault.schemes().is_empty());
        drop(fault);
        assert!(!ctx.is_identity_listed(&qi.identity.id()).unwrap());
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_merge_preserves_existing_keys_and_checks_omitted_labels() {
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        let (ctx, _dir) = protected_import_context().await;
        let (mut existing, triple) = masternode_shaped_qi();
        let password = Secret::new("synthetic-import-password");
        ctx.persist_loaded_identity(
            &mut existing,
            Some(&password),
            IdentityLoadMode::RejectIfExists,
        )
        .unwrap();
        let identity_id = existing.identity.id();
        let before = ctx.stored_identity_blob(&identity_id).unwrap();
        let mut incoming = existing.clone();
        incoming.private_keys = KeyStorage::default();
        let added = IdentityPublicKey::random_key(99, Some(99), PlatformVersion::latest());
        incoming.private_keys.insert_at(
            (M, added.id()),
            (
                QualifiedIdentityPublicKey::from(added),
                PrivateKeyData::AlwaysClear([0xE9; 32]),
            ),
        );
        let fault = WriteFault::arm(0);
        assert!(matches!(
            ctx.persist_loaded_identity(
                &mut incoming,
                Some(&Secret::new("different-synthetic-password")),
                IdentityLoadMode::Overwrite
            ),
            Err(TaskError::IdentityKeyPassphraseIncorrect)
        ));
        assert!(
            fault.schemes().is_empty(),
            "verify protected keys omitted by an overwrite"
        );
        drop(fault);
        let fault = WriteFault::arm(1);
        assert!(
            ctx.persist_loaded_identity(
                &mut incoming,
                Some(&password),
                IdentityLoadMode::MergeIntoExisting
            )
            .is_err()
        );
        drop(fault);
        assert_eq!(ctx.stored_identity_blob(&identity_id).unwrap(), before);
        ctx.persist_loaded_identity(
            &mut incoming,
            Some(&password),
            IdentityLoadMode::MergeIntoExisting,
        )
        .unwrap();
        let loaded = ctx
            .get_local_qualified_identity(&identity_id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.private_keys.len(), 4);
        let backend = ctx.wallet_backend().unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), identity_id.to_buffer());
        for (target, key_id) in triple.into_iter().chain([(M, 99)]) {
            assert!(
                view.get_protected(
                    &target,
                    key_id,
                    &SecretString::new(password.expose_secret())
                )
                .unwrap()
                .is_some()
            );
        }
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merge_preserves_wallet_link_when_only_voting_key_is_supplied_without_password() {
        assert_merge_preserves_wallet_link(false).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merge_preserves_wallet_link_when_only_voting_key_is_supplied_with_password() {
        assert_merge_preserves_wallet_link(true).await;
    }

    async fn assert_merge_preserves_wallet_link(with_password: bool) {
        let (ctx, _dir) = protected_import_context().await;
        let (mut existing, _) = masternode_shaped_qi();
        let wallet_link = Some(([0x77; 32], 7));
        let path = WalletDerivationPath {
            wallet_seed_hash: [0x77; 32],
            derivation_path: "m/9'/7'/0'".parse().unwrap(),
        };
        let owner = IdentityPublicKey::random_key(10, Some(10), PlatformVersion::latest());
        existing.private_keys.insert_at(
            (M, owner.id()),
            (
                QualifiedIdentityPublicKey::from_identity_public_key_in_wallet(
                    owner,
                    Some(path.clone()),
                ),
                PrivateKeyData::AtWalletDerivationPath(path),
            ),
        );
        ctx.insert_local_qualified_identity(&existing, &wallet_link)
            .unwrap();
        let mut merged = existing.clone();
        merged.private_keys = KeyStorage::default();
        let voter = IdentityPublicKey::random_key(11, Some(11), PlatformVersion::latest());
        merged.private_keys.insert_at(
            (V, voter.id()),
            (
                QualifiedIdentityPublicKey::from(voter),
                PrivateKeyData::Clear(rand::random()),
            ),
        );
        assert_eq!(merged.determine_wallet_info().unwrap(), None);
        let persisted_link = if with_password {
            ctx.persist_loaded_identity(
                &mut merged,
                Some(&Secret::new(hex::encode(rand::random::<[u8; 32]>()))),
                IdentityLoadMode::MergeIntoExisting,
            )
            .unwrap()
        } else {
            ctx.persist_merged_identity(&mut merged, None).unwrap()
        };
        assert_eq!(persisted_link, wallet_link);
        assert_eq!(
            ctx.stored_identity_wallet_link(&existing.identity.id())
                .unwrap(),
            wallet_link
        );
        assert_eq!(
            ctx.load_local_qualified_identities_for_wallet(&[0x77; 32])
                .unwrap()
                .len(),
            1
        );
        ctx.wallet_backend().unwrap().shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_preserves_wallet_derived_keys_without_storing_them() {
        let (ctx, _dir) = protected_import_context().await;
        let mut qi =
            crate::context::test_staging::qi_with_plaintext_and_derived([0x91; 32], [0x92; 32]);
        ctx.persist_loaded_identity(
            &mut qi,
            Some(&Secret::new("synthetic-import-password")),
            IdentityLoadMode::RejectIfExists,
        )
        .unwrap();
        assert!(!qi.private_keys.has_plaintext_for_vault());
        let backend = ctx.wallet_backend().unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), qi.identity.id().to_buffer());
        for ((target, key_id), (_, data)) in qi.private_keys.iter() {
            let expected = if matches!(data, PrivateKeyData::AtWalletDerivationPath(_)) {
                SecretScheme::Absent
            } else {
                SecretScheme::Protected
            };
            assert_eq!(view.scheme(target, *key_id).unwrap(), expected);
        }
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_import_seals_an_existing_copy_of_a_wallet_derived_key() {
        let (ctx, _dir) = protected_import_context().await;
        let mut qi =
            crate::context::test_staging::qi_with_plaintext_and_derived([0x91; 32], [0x92; 32]);
        let derived = qi
            .private_keys
            .iter()
            .find_map(|(placement, (_, data))| {
                matches!(data, PrivateKeyData::AtWalletDerivationPath(_)).then(|| placement.clone())
            })
            .unwrap();
        let backend = ctx.wallet_backend().unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), qi.identity.id().to_buffer());
        let saved = [0x93; 32];
        view.store(&derived.0, derived.1, &saved).unwrap();
        let password = Secret::new("synthetic-import-password");
        ctx.persist_loaded_identity(&mut qi, Some(&password), IdentityLoadMode::RejectIfExists)
            .unwrap();
        assert!(matches!(
            qi.private_keys.entry_at(&derived).unwrap().1,
            PrivateKeyData::AtWalletDerivationPath(_)
        ));
        assert_eq!(
            *view
                .get_protected(
                    &derived.0,
                    derived.1,
                    &SecretString::new(password.expose_secret())
                )
                .unwrap()
                .unwrap(),
            saved
        );
        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn load_without_password_keeps_keys_unprotected() {
        let (ctx, _dir) = protected_import_context().await;
        let (mut qi, triple) = masternode_shaped_qi();
        ctx.persist_loaded_identity(&mut qi, None, IdentityLoadMode::RejectIfExists)
            .unwrap();
        let backend = ctx.wallet_backend().unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), qi.identity.id().to_buffer());
        for (target, key_id) in triple {
            assert_eq!(
                view.scheme(&target, key_id).unwrap(),
                SecretScheme::Unprotected
            );
        }
        backend.shutdown().await;
    }

    /// A hash160 node key, the form a masternode identity carries its owner
    /// and payout keys in.
    fn node_key(id: KeyID, purpose: Purpose, disabled: bool) -> IdentityPublicKey {
        use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
        IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id,
            purpose,
            security_level: SecurityLevel::CRITICAL,
            contract_bounds: None,
            key_type: KeyType::ECDSA_HASH160,
            read_only: true,
            data: rand::random::<[u8; 20]>().to_vec().into(),
            disabled_at: disabled.then_some(1),
        })
    }

    /// A masternode whose stored owner key `old` a rotation has replaced with
    /// `new`, as the freshly fetched identity reports it.
    fn rotated_owner_node() -> (QualifiedIdentity, QualifiedIdentity, KeyID, KeyID) {
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
        let (mut existing, _) = masternode_shaped_qi();
        existing.private_keys = KeyStorage::default();
        let old = node_key(10, Purpose::OWNER, false);
        existing.identity.add_public_key(old.clone());
        existing.private_keys.insert_at(
            (M, old.id()),
            (
                QualifiedIdentityPublicKey::from(old.clone()),
                PrivateKeyData::Clear(rand::random()),
            ),
        );

        let mut fetched = existing.clone();
        fetched.private_keys = KeyStorage::default();
        let mut disabled_old = old.clone();
        disabled_old.set_disabled_at(1);
        let replacement = node_key(11, Purpose::OWNER, false);
        fetched.identity.add_public_key(disabled_old);
        fetched.identity.add_public_key(replacement.clone());
        fetched.private_keys.insert_at(
            (M, replacement.id()),
            (
                QualifiedIdentityPublicKey::from(replacement.clone()),
                PrivateKeyData::Clear(rand::random()),
            ),
        );
        (existing, fetched, old.id(), replacement.id())
    }

    /// A stored node whose voting key is `voting_key`, with its voter identity
    /// loaded, and a fresh load of it that found no voting key.
    fn node_with_stored_voter() -> (QualifiedIdentity, QualifiedIdentity, IdentityPublicKey) {
        let (mut existing, _) = masternode_shaped_qi();
        existing.private_keys = KeyStorage::default();
        let mut fetched = existing.clone();
        let voting_key = node_key(0, Purpose::VOTING, false);
        let voter =
            Identity::create_basic_identity(Identifier::random(), PlatformVersion::latest())
                .unwrap();
        existing.private_keys.insert_at(
            (V, voting_key.id()),
            (
                QualifiedIdentityPublicKey::from(voting_key.clone()),
                PrivateKeyData::Clear(rand::random()),
            ),
        );
        existing.associated_voter_identity = Some((voter, voting_key.clone()));
        fetched.associated_voter_identity = None;
        (existing, fetched, voting_key)
    }

    fn voting_hash(key: &IdentityPublicKey) -> [u8; 20] {
        use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
        key.public_key_hash().unwrap()
    }

    /// After the masternode list reports a new voting key, a merge that found
    /// no replacement must not bring the retired voter back: the node would
    /// still look ready to vote with a key the network no longer accepts.
    #[test]
    fn a_retired_voting_key_is_not_carried_into_the_merge() {
        let (mut existing, mut fetched, voting_key) = node_with_stored_voter();

        drop_retired_voting_key(&mut existing, Some(rand::random()));
        merge_existing_keys_into(&mut fetched, existing);

        assert!(fetched.associated_voter_identity.is_none());
        assert!(!fetched.private_keys.has(&(V, voting_key.id())));
        assert!(!fetched.can_cast_masternode_vote());
    }

    /// A voting key the masternode list still names, or one that cannot be
    /// checked because the list is unavailable, is kept.
    #[test]
    fn a_current_or_unchecked_voting_key_is_kept() {
        for current in [None, Some(())] {
            let (mut existing, mut fetched, voting_key) = node_with_stored_voter();
            let current = current.map(|()| voting_hash(&voting_key));

            drop_retired_voting_key(&mut existing, current);
            merge_existing_keys_into(&mut fetched, existing);

            assert!(fetched.associated_voter_identity.is_some(), "{current:?}");
            assert!(
                fetched.private_keys.has(&(V, voting_key.id())),
                "{current:?}"
            );
        }
    }

    /// After a key rotation, a merge keeps the replaced key's private half
    /// but takes its disabled state from the fetched identity, so the
    /// withdrawal screen offers only the replacement.
    #[test]
    fn merge_after_rotation_offers_only_the_active_key_for_withdrawal() {
        let (existing, mut fetched, old_id, replacement_id) = rotated_owner_node();

        merge_existing_keys_into(&mut fetched, existing);

        assert!(fetched.private_keys.has(&(M, old_id)), "kept, not dropped");
        let offered: Vec<KeyID> = fetched
            .available_withdrawal_keys()
            .iter()
            .map(|key| key.identity_public_key.id())
            .collect();
        assert_eq!(offered, vec![replacement_id]);
        assert_eq!(
            fetched
                .default_withdrawal_key()
                .map(|key| key.identity_public_key.id()),
            Some(replacement_id),
        );
    }

    /// A stored owner key counts only while the fetched identity still has it
    /// active; a stored voting key only while it is the one the masternode
    /// list records.
    #[test]
    fn a_rotated_node_key_is_looked_up_again() {
        let (existing, fetched, _, _) = rotated_owner_node();
        assert!(!stores_current_node_key(
            &existing,
            &fetched.identity,
            M,
            Purpose::OWNER,
            None
        ));
        assert!(stores_current_node_key(
            &existing,
            &existing.identity,
            M,
            Purpose::OWNER,
            None
        ));

        let mut voter = existing.clone();
        let voting = node_key(0, Purpose::VOTING, false);
        let stored_hash: [u8; 20] = voting.data().as_slice().try_into().unwrap();
        voter.private_keys.insert_at(
            (V, voting.id()),
            (
                QualifiedIdentityPublicKey::from(voting),
                PrivateKeyData::Clear(rand::random()),
            ),
        );
        let current =
            |hash| stores_current_node_key(&voter, &voter.identity, V, Purpose::VOTING, hash);
        assert!(
            current(Some(stored_hash)),
            "the recorded voting key is kept"
        );
        assert!(
            !current(Some(rand::random())),
            "a rotated voting key is looked up again",
        );
        assert!(
            current(None),
            "without the masternode list, the stored key stays"
        );
    }

    /// §10.8 — the testable core of the "Add voting key" in-place
    /// update. A voter-key-only rebuild (blank Owner/Payout, so `associated_*`
    /// and the Owner/Payout private keys are absent) MUST NOT erase the
    /// already-stored Owner and Payout keys: `merge_existing_keys_into` carries
    /// over every key the new partial build omitted, while the resupplied voting
    /// key wins on collision.
    #[test]
    fn merge_preserves_owner_and_payout_when_only_voting_key_resupplied() {
        // `existing`: a fully-loaded masternode (owner + voter + identity keys).
        let (existing, triple) = masternode_shaped_qi();
        let [owner_key, voter_key, idkey_key] = triple;

        // `new`: what the scoped "Add voting key" prompt rebuilds — a voter key
        // only. It carries the freshly-entered voting key but nothing else.
        let mut new = existing.clone();
        new.alias = None;
        new.associated_voter_identity = None;
        new.associated_operator_identity = None;
        new.associated_owner_key_id = None;
        new.private_keys = KeyStorage::default();
        // Resupply ONLY the voting key, with a distinct byte so we can prove the
        // new value wins on collision.
        let (voter_pk, _) = existing
            .private_keys
            .entry_at(&voter_key)
            .expect("existing voter key")
            .clone();
        new.private_keys.insert_at(
            voter_key.clone(),
            (voter_pk, PrivateKeyData::Clear([0xEE; 32])),
        );

        merge_existing_keys_into(&mut new, existing);

        // Owner and identity-auth keys survive the voter-key-only update.
        assert!(
            new.private_keys.has(&owner_key),
            "owner key must survive a voting-key-only update",
        );
        assert!(
            new.private_keys.has(&idkey_key),
            "identity-auth key must survive a voting-key-only update",
        );
        // The resupplied voting key wins on collision (0xEE, not the old 0xB0).
        let (_, merged_voter) = new
            .private_keys
            .entry_at(&voter_key)
            .expect("voter key present after merge");
        assert!(
            matches!(merged_voter, PrivateKeyData::Clear(b) if *b == [0xEE; 32]),
            "the resupplied voting key must win on collision",
        );
    }

    /// §10.9 / TC-EDGE-07 — a fresh load (`RejectIfExists`) of a
    /// ProTxHash already stored is rejected with [`TaskError::DuplicateProTxHash`]
    /// BEFORE any network fetch, and the already-stored node is left untouched.
    /// Runs fully offline: the existence check fires before the SDK is used.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reject_if_exists_rejects_duplicate_pro_tx_hash_offline() {
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

        let (qi, triple) = masternode_shaped_qi();
        let identity_id = qi.identity.id();
        ctx.insert_local_qualified_identity(&qi, &None)
            .expect("insert first masternode identity");

        let input = IdentityInputToLoad {
            identity_id_input: identity_id.to_string(Encoding::Hex),
            identity_type: IdentityType::Masternode,
            alias_input: String::new(),
            voting_private_key_input: Secret::new(""),
            owner_private_key_input: Secret::new(""),
            payout_address_private_key_input: Secret::new(""),
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password: None,
            load_mode: IdentityLoadMode::RejectIfExists,
            load_token: None,
        };

        let sdk = ctx.sdk();
        let result = ctx.load_identity(&sdk, input).await;
        match result {
            Err(TaskError::DuplicateProTxHash { identity_id: got }) => {
                assert_eq!(got, identity_id, "reject must name the duplicate id");
            }
            other => panic!("expected DuplicateProTxHash, got {other:?}"),
        }

        // The first node's stored keys are untouched by the rejected load.
        let still = ctx
            .get_local_qualified_identity(&identity_id)
            .expect("read stored identity")
            .expect("first node still stored");
        for (t, k) in &triple {
            assert!(
                still.private_keys.has(&(t.clone(), *k)),
                "key ({t:?}, {k}) of the first node must survive a rejected duplicate load",
            );
        }

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// A protected merge persists its new key without creating any unprotected vault entry.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merge_into_tier2_node_seals_new_key_and_insert_succeeds() {
        use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};

        const PW: &str = "one-identity-password";

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
        // The scoped merge prompt asks for the node's object password once.
        ctx.install_secret_prompt(Arc::new(TestPrompt::new([ScriptedAnswer::once(PW)])));
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");

        // Seed a masternode and seal all its keys Tier-2.
        let (qi, _triple) = masternode_shaped_qi();
        let identity_id = qi.identity.id();
        ctx.insert_local_qualified_identity(&qi, &None)
            .expect("insert masternode identity");
        ctx.protect_identity_keys(identity_id, Secret::new(PW), None)
            .expect("seal Tier-2");

        // Reload the sealed node: every key is now InVault.
        let mut existing = ctx
            .get_local_qualified_identity(&identity_id)
            .expect("read stored identity")
            .expect("node stored");

        // Simulate the merge product: a freshly-supplied resident-plaintext
        // voting key on a new key id (what `merge_existing_keys_into` yields).
        let pv = PlatformVersion::latest();
        let new_voter = IdentityPublicKey::random_key(9, Some(9), pv);
        let new_voter_id = new_voter.id();
        let new_key = (V, new_voter_id);
        existing.private_keys.insert_at(
            new_key.clone(),
            (
                QualifiedIdentityPublicKey::from(new_voter),
                PrivateKeyData::Clear([0xDD; 32]),
            ),
        );

        // Verify the node's object password up front (as the load path does),
        // then seal the merged plaintext key Tier-2 before insert.
        let verify_scope = ctx
            .protected_identity_verify_scope(&existing)
            .expect("verify scope lookup")
            .expect("node is Tier-2, so a verify scope exists");
        let password = ctx
            .wallet_backend()
            .expect("backend wired")
            .secret_access()
            .verify_identity_object_password(&verify_scope)
            .await
            .expect("scripted password verifies");
        let claim = ctx.begin_identity_load(identity_id, None).unwrap();
        // A password rotation during the fetch must reject the old verified
        // password before writing even one new key into the vault.
        ctx.unprotect_identity_keys(identity_id, Secret::new(PW))
            .unwrap();
        ctx.protect_identity_keys(identity_id, Secret::new("rotated-identity-password"), None)
            .unwrap();
        assert!(matches!(
            ctx.finish_identity_load_storage(
                &mut existing,
                IdentityLoadMode::MergeIntoExisting,
                Some(&password),
                &claim,
                None,
                None
            ),
            Err(TaskError::IdentityLoadSuperseded { .. })
        ));
        let backend = ctx.wallet_backend().unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), identity_id.to_buffer());
        assert_eq!(view.scheme(&V, new_voter_id).unwrap(), SecretScheme::Absent);
        ctx.unprotect_identity_keys(identity_id, Secret::new("rotated-identity-password"))
            .unwrap();
        ctx.protect_identity_keys(identity_id, Secret::new(PW), None)
            .unwrap();
        ctx.finish_identity_load_storage(
            &mut existing,
            IdentityLoadMode::MergeIntoExisting,
            Some(&password),
            &claim,
            None,
            None,
        )
        .expect("seal and persist a merged key under the current password");

        // The new key flipped to InVault in the in-memory identity...
        assert!(
            matches!(
                existing.private_keys.entry_at(&new_key),
                Some((_, PrivateKeyData::InVault)),
            ),
            "the merged voting key must be marked InVault after sealing",
        );

        // ...and the new key reads back as a Tier-2 (Protected) sealed secret.
        let backend = ctx.wallet_backend().expect("backend wired");
        let view = IdentityKeyView::new(backend.secret_store(), identity_id.to_buffer());
        assert_eq!(
            view.scheme(&V, new_voter_id).expect("scheme"),
            SecretScheme::Protected,
            "the merged voting key must be sealed Tier-2",
        );
        assert!(
            view.get_protected(&V, new_voter_id, &SecretString::new(PW))
                .expect("get_protected")
                .is_some(),
            "the sealed voting key must round-trip under the object password",
        );

        backend.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_merge_seals_resident_plaintext_alongside_new_key() {
        use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
        const PW: &str = "synthetic-resident-merge-password";
        for always_clear in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let ctx = open_import_context(
                dir.path(),
                Some(Arc::new(TestPrompt::new([ScriptedAnswer::once(PW)]))),
            )
            .await;
            let (qi, _) = masternode_shaped_qi();
            let id = qi.identity.id();
            ctx.insert_local_qualified_identity(&qi, &None).unwrap();
            ctx.protect_identity_keys(id, Secret::new(PW), None)
                .unwrap();
            let mut resident = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
            let mut incoming = resident.clone();
            let pv = PlatformVersion::latest();
            let resident_key = IdentityPublicKey::random_key(10, Some(10), pv);
            let new_key = IdentityPublicKey::random_key(11, Some(11), pv);
            let resident_bytes = zeroize::Zeroizing::new(rand::random::<[u8; 32]>());
            let new_bytes = zeroize::Zeroizing::new(rand::random::<[u8; 32]>());
            // An incomplete migration can leave a resident key absent from the vault.
            resident.private_keys.insert_at(
                (V, 10),
                (
                    QualifiedIdentityPublicKey::from(resident_key),
                    if always_clear {
                        PrivateKeyData::AlwaysClear(*resident_bytes)
                    } else {
                        PrivateKeyData::Clear(*resident_bytes)
                    },
                ),
            );
            incoming.private_keys.insert_at(
                (V, 11),
                (
                    QualifiedIdentityPublicKey::from(new_key),
                    PrivateKeyData::Clear(*new_bytes),
                ),
            );
            let backend = ctx.wallet_backend().unwrap();
            let scope = ctx
                .protected_identity_verify_scope(&incoming)
                .unwrap()
                .unwrap();
            let verified = backend
                .secret_access()
                .verify_identity_object_password(&scope)
                .await
                .unwrap();
            {
                let lock = ctx.identity_record_lock(id);
                let _guard = lock.lock().unwrap();
                ctx.persist_merged_identity_locked(
                    &mut incoming,
                    Some(resident),
                    Some(&verified),
                    IdentityRecordWrite::UpdateListed,
                )
                .unwrap();
            }
            let reread = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
            assert!(!reread.private_keys.has_plaintext_for_vault());
            let view = IdentityKeyView::new(backend.secret_store(), id.to_buffer());
            for (key_id, expected) in [(10, &resident_bytes), (11, &new_bytes)] {
                assert!(matches!(
                    reread.private_keys.entry_at(&(V, key_id)),
                    Some((_, PrivateKeyData::InVault))
                ));
                assert_eq!(view.scheme(&V, key_id).unwrap(), SecretScheme::Protected);
                let actual = view
                    .get_protected(&V, key_id, &SecretString::new(PW))
                    .unwrap()
                    .unwrap();
                assert!(actual.as_slice() == expected.as_slice());
            }
            backend.shutdown().await;
        }
    }

    /// Merge×Tier-2 with a stored legacy `Encrypted` key: sealing skips that key
    /// (no vault entry), so a merge that keeps it must fail before any secret
    /// write. Resupplying the key replaces the legacy entry and the merge seals it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protected_merge_rejects_surviving_legacy_encrypted_key() {
        use crate::wallet_backend::secret_prompt::test_support::{ScriptedAnswer, TestPrompt};
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        const PW: &str = "synthetic-legacy-merge-password";
        for resupplied in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let ctx = open_import_context(
                dir.path(),
                Some(Arc::new(TestPrompt::new([ScriptedAnswer::once(PW)]))),
            )
            .await;
            let (qi, _) = masternode_shaped_qi();
            let id = qi.identity.id();
            ctx.insert_local_qualified_identity(&qi, &None).unwrap();
            ctx.protect_identity_keys(id, Secret::new(PW), None)
                .unwrap();
            let mut stored = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
            let mut fresh = stored.clone();

            let pv = PlatformVersion::latest();
            let legacy = IdentityPublicKey::random_key(10, Some(10), pv);
            let legacy_key = (V, legacy.id());
            stored.private_keys.insert_at(
                legacy_key.clone(),
                (
                    QualifiedIdentityPublicKey::from(legacy.clone()),
                    PrivateKeyData::Encrypted(vec![0x33; 48]),
                ),
            );
            ctx.insert_local_qualified_identity(&stored, &None).unwrap();

            let new_voter = IdentityPublicKey::random_key(9, Some(9), pv);
            fresh.private_keys.insert_at(
                (V, new_voter.id()),
                (
                    QualifiedIdentityPublicKey::from(new_voter),
                    PrivateKeyData::Clear([0xDD; 32]),
                ),
            );
            if resupplied {
                fresh.private_keys.insert_at(
                    legacy_key.clone(),
                    (
                        QualifiedIdentityPublicKey::from(legacy),
                        PrivateKeyData::Clear([0xEE; 32]),
                    ),
                );
            }

            let backend = ctx.wallet_backend().unwrap();
            let scope = ctx
                .protected_identity_verify_scope(&fresh)
                .unwrap()
                .unwrap();
            let verified = backend
                .secret_access()
                .verify_identity_object_password(&scope)
                .await
                .unwrap();
            let fault = WriteFault::arm(0);
            let result = ctx.persist_merged_identity(&mut fresh, Some(&verified));
            if resupplied {
                result.unwrap();
                let view = IdentityKeyView::new(backend.secret_store(), id.to_buffer());
                assert_eq!(
                    view.scheme(&legacy_key.0, legacy_key.1).unwrap(),
                    SecretScheme::Protected,
                    "a resupplied legacy key must be sealed Tier-2",
                );
                let reread = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
                assert!(!reread.private_keys.has_encrypted_legacy_keys());
            } else {
                assert!(
                    matches!(result, Err(TaskError::IdentityKeyProtectionLegacyFormat)),
                    "expected IdentityKeyProtectionLegacyFormat, got {result:?}",
                );
                assert!(
                    fault.schemes().is_empty(),
                    "legacy rejection must precede any secret write",
                );
                let reread = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
                assert!(
                    reread.private_keys.entry_at(&(V, 9)).is_none(),
                    "a rejected merge must not persist the new key",
                );
            }
            drop(fault);
            backend.shutdown().await;
        }
    }

    /// Password-supplied import over a stored record carrying a legacy
    /// `Encrypted` key: the legacy check runs on the effective key set, so a
    /// reload that resupplies the key (or an `Overwrite` that drops it)
    /// succeeds, while a merge that keeps it fails before any secret write.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn password_import_validates_legacy_keys_after_replacement() {
        use crate::wallet_backend::secret_seam::write_fault_test_support::WriteFault;
        const PW: &str = "synthetic-legacy-import-password";
        let cases = [
            (IdentityLoadMode::MergeIntoExisting, false),
            (IdentityLoadMode::MergeIntoExisting, true),
            (IdentityLoadMode::Overwrite, false),
        ];
        for (load_mode, resupplied) in cases {
            let dir = tempfile::tempdir().unwrap();
            let ctx = open_import_context(dir.path(), None).await;
            let (qi, _) = masternode_shaped_qi();
            let id = qi.identity.id();
            ctx.insert_local_qualified_identity(&qi, &None).unwrap();
            ctx.protect_identity_keys(id, Secret::new(PW), None)
                .unwrap();
            let mut stored = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
            let mut fresh = qi.clone();

            let pv = PlatformVersion::latest();
            let legacy = IdentityPublicKey::random_key(10, Some(10), pv);
            let legacy_key = (V, legacy.id());
            stored.private_keys.insert_at(
                legacy_key.clone(),
                (
                    QualifiedIdentityPublicKey::from(legacy.clone()),
                    PrivateKeyData::Encrypted(vec![0x33; 48]),
                ),
            );
            ctx.insert_local_qualified_identity(&stored, &None).unwrap();

            let new_voter = IdentityPublicKey::random_key(9, Some(9), pv);
            fresh.private_keys.insert_at(
                (V, new_voter.id()),
                (
                    QualifiedIdentityPublicKey::from(new_voter),
                    PrivateKeyData::Clear([0xDD; 32]),
                ),
            );
            if resupplied {
                fresh.private_keys.insert_at(
                    legacy_key.clone(),
                    (
                        QualifiedIdentityPublicKey::from(legacy),
                        PrivateKeyData::Clear([0xEE; 32]),
                    ),
                );
            }

            let fault = WriteFault::arm(0);
            let result = ctx.persist_loaded_identity(&mut fresh, Some(&Secret::new(PW)), load_mode);
            let schemes = fault.schemes();
            drop(fault);
            let backend = ctx.wallet_backend().unwrap();
            let view = IdentityKeyView::new(backend.secret_store(), id.to_buffer());
            let reread = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
            if load_mode == IdentityLoadMode::MergeIntoExisting && !resupplied {
                assert!(
                    matches!(result, Err(TaskError::IdentityKeyProtectionLegacyFormat)),
                    "expected IdentityKeyProtectionLegacyFormat, got {result:?}",
                );
                assert!(
                    schemes.is_empty(),
                    "legacy rejection must precede any secret write",
                );
                assert!(
                    reread.private_keys.entry_at(&(V, 9)).is_none(),
                    "a rejected import must not persist the new key",
                );
            } else {
                result.unwrap();
                assert!(
                    schemes.iter().all(|s| *s == SecretScheme::Protected),
                    "a password-selected import must only write protected secrets",
                );
                assert!(!reread.private_keys.has_encrypted_legacy_keys());
                assert_eq!(
                    view.scheme(&V, 9).unwrap(),
                    SecretScheme::Protected,
                    "the new key must be sealed Tier-2",
                );
                if resupplied {
                    assert_eq!(
                        view.scheme(&legacy_key.0, legacy_key.1).unwrap(),
                        SecretScheme::Protected,
                        "a resupplied legacy key must be sealed Tier-2",
                    );
                }
            }
            backend.shutdown().await;
        }
    }

    /// Merge×Tier-2 (headless fail-closed) — a `MergeIntoExisting` load into a Tier-2
    /// node with no interactive prompt (the default `NullSecretPrompt`) fails
    /// closed with [`TaskError::SecretPromptUnavailable`] and — critically —
    /// BEFORE the network fetch, because the object password is verified up
    /// front. No prompt means no way to seal the merged key, so the load is
    /// rejected rather than silently dropping to a keyless downgrade.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn merge_into_tier2_node_headless_fails_closed_before_fetch() {
        const PW: &str = "one-identity-password";

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
        // No prompt installed: the default NullSecretPrompt fails closed.
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");

        // Seed a Tier-2 masternode.
        let (qi, _triple) = masternode_shaped_qi();
        let identity_id = qi.identity.id();
        ctx.insert_local_qualified_identity(&qi, &None)
            .expect("insert masternode identity");
        ctx.protect_identity_keys(identity_id, Secret::new(PW), None)
            .expect("seal Tier-2");

        let input = IdentityInputToLoad {
            identity_id_input: identity_id.to_string(Encoding::Hex),
            identity_type: IdentityType::Masternode,
            alias_input: String::new(),
            voting_private_key_input: Secret::new(""),
            owner_private_key_input: Secret::new(""),
            payout_address_private_key_input: Secret::new(""),
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password: None,
            load_mode: IdentityLoadMode::MergeIntoExisting,
            load_token: None,
        };

        // The verify happens before the SDK fetch, so this resolves offline.
        let sdk = ctx.sdk();
        let result = ctx.load_identity(&sdk, input).await;
        assert!(
            matches!(result, Err(TaskError::SecretPromptUnavailable)),
            "a headless merge into a Tier-2 node must fail closed, got {result:?}",
        );

        // The stored node is untouched — still fully Tier-2.
        let backend = ctx.wallet_backend().expect("backend wired");
        let view = IdentityKeyView::new(backend.secret_store(), identity_id.to_buffer());
        assert_eq!(
            view.scheme(&M, 1).expect("scheme"),
            SecretScheme::Protected,
            "a rejected headless merge must leave the node's keys sealed",
        );

        backend.shutdown().await;
    }

    /// A malformed identity-id input surfaces the ProTxHash-specific
    /// [`TaskError::MalformedProTxHash`] for masternode/evonode loads (where the
    /// field IS a ProTxHash), and the generic [`TaskError::IdentifierParsingError`]
    /// for User loads — both offline, at the parse arm, before any network fetch.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn malformed_id_routes_to_pro_tx_hash_error_for_nodes_only() {
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

        let make_input = |identity_type| IdentityInputToLoad {
            identity_id_input: "not-a-valid-identifier".to_string(),
            identity_type,
            alias_input: String::new(),
            voting_private_key_input: Secret::new(""),
            owner_private_key_input: Secret::new(""),
            payout_address_private_key_input: Secret::new(""),
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password: None,
            load_mode: IdentityLoadMode::Overwrite,
            load_token: None,
        };

        let sdk = ctx.sdk();
        let node_result = ctx
            .load_identity(&sdk, make_input(IdentityType::Masternode))
            .await;
        assert!(
            matches!(node_result, Err(TaskError::MalformedProTxHash { .. })),
            "a masternode load with a malformed id must report MalformedProTxHash, got {node_result:?}",
        );

        let user_result = ctx
            .load_identity(&sdk, make_input(IdentityType::User))
            .await;
        assert!(
            matches!(user_result, Err(TaskError::IdentifierParsingError { .. })),
            "a User load with a malformed id must report IdentifierParsingError, got {user_result:?}",
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// Regression: a load that fails its own input validation must still report a
    /// terminal phase. The dispatching screen marks the load `Submitted` before
    /// the task exists, and only the load can close that out — so the claim, which
    /// records the outcome when it drops, has to be established before the first
    /// fallible step. Validating first strands the load as forever-outstanding and
    /// leaves the form stuck on "Loading…" for the rest of the session.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_load_that_fails_validation_still_reports_a_terminal_phase() {
        use crate::context::identity_load_registry::IdentityLoadPhase;

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

        let identity_id = Identifier::from([0x5a; 32]);
        let token = ctx
            .mark_identity_load_submitted(identity_id)
            .expect("nothing else is loading this identity");

        // A too-short at-load password fails validation before any network use.
        let input = IdentityInputToLoad {
            identity_id_input: identity_id.to_string(Encoding::Hex),
            identity_type: IdentityType::Masternode,
            alias_input: String::new(),
            voting_private_key_input: Secret::new(""),
            owner_private_key_input: Secret::new(""),
            payout_address_private_key_input: Secret::new(""),
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password: Some(Secret::new("x")),
            load_mode: IdentityLoadMode::RejectIfExists,
            load_token: Some(token),
        };

        let sdk = ctx.sdk();
        let result = ctx.load_identity(&sdk, input).await;
        assert!(result.is_err(), "a one-character password must be rejected");
        assert_eq!(
            ctx.identity_load_phase(&identity_id, token),
            Some(IdentityLoadPhase::Failed),
            "a load that fails validation must report Failed, not stay outstanding forever"
        );

        ctx.wallet_backend().expect("backend").shutdown().await;
    }

    /// hash160 of the key at `path` under `seed`, computed straight from the
    /// seed — an independent DIP-3 vector.
    fn wallet_key_hash(seed: &[u8; 64], path: &str) -> [u8; 20] {
        use std::str::FromStr;
        DerivationPath::from_str(path)
            .unwrap()
            .derive_priv_ecdsa_for_master_seed(seed, Network::Testnet)
            .unwrap()
            .to_priv()
            .public_key()
            .pubkey_hash()
            .to_byte_array()
    }

    fn hash160_key(
        id: KeyID,
        purpose: dash_sdk::dpp::identity::Purpose,
        hash: [u8; 20],
    ) -> IdentityPublicKey {
        IdentityPublicKey::V0(
            dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0 {
                id,
                purpose,
                security_level: SecurityLevel::CRITICAL,
                contract_bounds: None,
                key_type: KeyType::ECDSA_HASH160,
                read_only: false,
                data: hash.to_vec().into(),
                disabled_at: None,
            },
        )
    }

    /// The wallet-held node's wallet seed: random per run, never committed.
    static WALLET_NODE_SEED: std::sync::LazyLock<[u8; 64]> = std::sync::LazyLock::new(rand::random);
    const WALLET_NODE_OWNER_PATH: &str = "m/9'/1'/3'/2'/3";
    const WALLET_NODE_PAYOUT_PATH: &str = "m/44'/1'/0'/0/2";

    /// A context holding one registered wallet, and a mock SDK serving a node
    /// whose owner and payout keys that wallet holds.
    async fn wallet_held_node() -> (
        Arc<AppContext>,
        tempfile::TempDir,
        WalletSeedHash,
        Identifier,
        Sdk,
    ) {
        use dash_sdk::dpp::identity::Purpose;
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let wallet =
            Wallet::new_from_seed(*WALLET_NODE_SEED, Network::Testnet, None, None).unwrap();
        let (seed_hash, _) = ctx
            .register_wallet(
                wallet,
                &WALLET_NODE_SEED,
                crate::model::wallet::birth_height::WalletOrigin::Fresh,
            )
            .unwrap();
        ctx.wallet_backend()
            .unwrap()
            .register_wallet_from_seed(&seed_hash, &WALLET_NODE_SEED, Some(0))
            .await
            .unwrap();

        let node_id = Identifier::from([0x5C; 32]);
        let mut identity =
            Identity::create_basic_identity(node_id, PlatformVersion::latest()).unwrap();
        identity.add_public_keys([
            hash160_key(
                0,
                Purpose::OWNER,
                wallet_key_hash(&WALLET_NODE_SEED, WALLET_NODE_OWNER_PATH),
            ),
            hash160_key(
                1,
                Purpose::TRANSFER,
                wallet_key_hash(&WALLET_NODE_SEED, WALLET_NODE_PAYOUT_PATH),
            ),
        ]);
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch(node_id, Some(identity))
            .await
            .unwrap();
        sdk.mock()
            .expect_fetch_many::<Identifier, dash_sdk::platform::Document, _, dash_sdk::query_types::Documents>(
                ctx.owned_dpns_names_query(node_id),
                Some(dash_sdk::query_types::Documents::default()),
            )
            .await
            .unwrap();
        (ctx, dir, seed_hash, node_id, sdk)
    }

    fn node_load_input(
        node_id: Identifier,
        owner_key: Secret,
        load_mode: IdentityLoadMode,
    ) -> IdentityInputToLoad {
        IdentityInputToLoad {
            identity_id_input: node_id.to_string(Encoding::Hex),
            identity_type: IdentityType::Masternode,
            alias_input: String::new(),
            voting_private_key_input: Secret::default(),
            owner_private_key_input: owner_key,
            payout_address_private_key_input: Secret::default(),
            keys_input: vec![],
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password: None,
            load_mode,
            load_token: None,
        }
    }

    fn wallet_reference(seed_hash: WalletSeedHash, path: &str) -> WalletDerivationPath {
        WalletDerivationPath {
            wallet_seed_hash: seed_hash,
            derivation_path: path.parse().unwrap(),
        }
    }

    /// A node registered with keys from a wallet on this device loads with
    /// its owner and payout keys referenced at their wallet derivation paths
    /// (DIP-3 owner path, non-hardened index) — nothing typed, nothing copied
    /// into the vault.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn node_keys_held_by_a_wallet_load_without_typing_them() {
        let (ctx, _dir, seed_hash, node_id, sdk) = wallet_held_node().await;
        let input = node_load_input(node_id, Secret::default(), IdentityLoadMode::RejectIfExists);
        let BackendTaskSuccessResult::LoadedIdentity(loaded) =
            ctx.load_identity(&sdk, input).await.unwrap()
        else {
            panic!("expected a loaded identity");
        };

        for (key_id, path) in [(0, WALLET_NODE_OWNER_PATH), (1, WALLET_NODE_PAYOUT_PATH)] {
            let (_, data) = loaded
                .private_keys
                .entry_at(&(M, key_id))
                .unwrap_or_else(|| panic!("key {key_id} must be held"));
            assert!(
                matches!(data, PrivateKeyData::AtWalletDerivationPath(found)
                    if *found == wallet_reference(seed_hash, path)),
                "key {key_id} must reference {path}, got {data:?}"
            );
        }
        let backend = ctx.wallet_backend().unwrap();
        let view = IdentityKeyView::new(backend.secret_store(), node_id.to_buffer());
        for key_id in [0, 1] {
            assert_eq!(view.scheme(&M, key_id).unwrap(), SecretScheme::Absent);
        }
        let stored = ctx.get_local_qualified_identity(&node_id).unwrap().unwrap();
        assert!(stored.private_keys == loaded.private_keys);
        // Key references into a wallet do not make the node one of that
        // wallet's own identities.
        assert_eq!(loaded.determine_wallet_info().unwrap(), None);
        let filed_in_wallet = ctx.wallet_context().wallets()[&seed_hash]
            .read()
            .unwrap()
            .identities
            .values()
            .any(|identity| identity.id() == node_id);
        assert!(
            !filed_in_wallet,
            "the node must not be filed as a wallet identity"
        );
        backend.shutdown().await;
    }

    /// A typed key wins over the wallet, and a later merge with blank fields
    /// keeps the stored key instead of swapping it for a wallet reference.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn typed_and_stored_node_keys_are_never_replaced_by_wallet_references() {
        let (ctx, _dir, seed_hash, node_id, sdk) = wallet_held_node().await;
        let owner_wif = WALLET_NODE_OWNER_PATH
            .parse::<DerivationPath>()
            .unwrap()
            .derive_priv_ecdsa_for_master_seed(&*WALLET_NODE_SEED, Network::Testnet)
            .unwrap()
            .to_priv()
            .to_wif();
        let typed = node_load_input(
            node_id,
            Secret::new(owner_wif),
            IdentityLoadMode::RejectIfExists,
        );
        ctx.load_identity(&sdk, typed).await.unwrap();
        let merge = node_load_input(
            node_id,
            Secret::default(),
            IdentityLoadMode::MergeIntoExisting,
        );
        ctx.load_identity(&sdk, merge).await.unwrap();

        let stored = ctx.get_local_qualified_identity(&node_id).unwrap().unwrap();
        let (_, owner) = stored.private_keys.entry_at(&(M, 0)).unwrap();
        assert!(
            !matches!(owner, PrivateKeyData::AtWalletDerivationPath(_)),
            "the typed owner key must stay, got {owner:?}"
        );
        let (_, payout) = stored.private_keys.entry_at(&(M, 1)).unwrap();
        assert!(
            matches!(payout, PrivateKeyData::AtWalletDerivationPath(found)
            if *found == wallet_reference(seed_hash, WALLET_NODE_PAYOUT_PATH))
        );
        ctx.wallet_backend().unwrap().shutdown().await;
    }

    /// A mock SDK serving `node_id` with `keys` and no DPNS names.
    async fn node_sdk(ctx: &AppContext, node_id: Identifier, keys: Vec<IdentityPublicKey>) -> Sdk {
        let mut identity =
            Identity::create_basic_identity(node_id, PlatformVersion::latest()).unwrap();
        identity.add_public_keys(keys);
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch(node_id, Some(identity))
            .await
            .unwrap();
        sdk.mock()
            .expect_fetch_many::<Identifier, dash_sdk::platform::Document, _, dash_sdk::query_types::Documents>(
                ctx.owned_dpns_names_query(node_id),
                Some(dash_sdk::query_types::Documents::default()),
            )
            .await
            .unwrap();
        sdk
    }

    /// After an owner key rotation, the old key still stored for the node must
    /// not stop a merge from finding the new key in a wallet: Platform refuses
    /// a disabled key, so keeping only the old one leaves nothing to sign with.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_rotated_node_key_is_found_in_a_wallet_despite_the_stale_stored_one() {
        use dash_sdk::dpp::identity::Purpose;
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeySettersV0;
        let old_seed: [u8; 64] = rand::random();
        let (ctx, _dir, seed_hash, node_id, _) = wallet_held_node().await;
        let old_owner = hash160_key(
            0,
            Purpose::OWNER,
            wallet_key_hash(&old_seed, WALLET_NODE_OWNER_PATH),
        );
        let payout = hash160_key(
            1,
            Purpose::TRANSFER,
            wallet_key_hash(&WALLET_NODE_SEED, WALLET_NODE_PAYOUT_PATH),
        );
        let old_owner_wif = WALLET_NODE_OWNER_PATH
            .parse::<DerivationPath>()
            .unwrap()
            .derive_priv_ecdsa_for_master_seed(&old_seed, Network::Testnet)
            .unwrap()
            .to_priv()
            .to_wif();
        let before = node_sdk(&ctx, node_id, vec![old_owner.clone(), payout.clone()]).await;
        ctx.load_identity(
            &before,
            node_load_input(
                node_id,
                Secret::new(old_owner_wif),
                IdentityLoadMode::RejectIfExists,
            ),
        )
        .await
        .unwrap();

        let mut disabled_owner = old_owner;
        disabled_owner.set_disabled_at(1);
        let new_owner = hash160_key(
            2,
            Purpose::OWNER,
            wallet_key_hash(&WALLET_NODE_SEED, WALLET_NODE_OWNER_PATH),
        );
        let after = node_sdk(&ctx, node_id, vec![disabled_owner, payout, new_owner]).await;
        ctx.load_identity(
            &after,
            node_load_input(
                node_id,
                Secret::default(),
                IdentityLoadMode::MergeIntoExisting,
            ),
        )
        .await
        .unwrap();

        let stored = ctx.get_local_qualified_identity(&node_id).unwrap().unwrap();
        let new_entry = stored.private_keys.entry_at(&(M, 2));
        assert!(
            matches!(new_entry, Some((_, PrivateKeyData::AtWalletDerivationPath(found)))
                if *found == wallet_reference(seed_hash, WALLET_NODE_OWNER_PATH)),
            "the rotated owner key must be referenced in the wallet, got {new_entry:?}"
        );
        ctx.wallet_backend().unwrap().shutdown().await;
    }
}
