use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{IdentityInputToLoad, IdentityLoadMode};
use crate::context::AppContext;
use crate::model::identity_key_protection::validate_protection_password;
use crate::model::key_input::verify_key_input;
use crate::model::qualified_identity::PrivateKeyTarget::{
    self, PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity,
};
use crate::model::wallet::{Wallet, WalletSeedHash};
use crate::ui::identities::add_new_identity_screen::MAX_IDENTITY_INDEX;
use dash_sdk::Sdk;
use dash_sdk::dashcore_rpc::dashcore::PrivateKey;
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identifier::MasternodeIdentifiers;
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::identity::SecurityLevel;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, KeyDerivationType};
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::drive::query::{SelectProjection, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier, Identity};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::convert::TryInto;
use std::sync::{Arc, RwLock};

type WalletKeyMap = BTreeMap<(PrivateKeyTarget, u32), (QualifiedIdentityPublicKey, PrivateKeyData)>;
type WalletMatchResult = Option<(WalletSeedHash, u32, WalletKeyMap)>;

/// Merge an already-stored identity's keys and associations into a freshly
/// built one, preserving anything the new (partial) load did not resupply
/// (§10.8, the "Add voting key" in-place update). Keys the new load provides
/// win on collision; keys it omits (e.g. Owner/Payout on a voting-key-only
/// update) are carried over from `existing` rather than lost. The existing
/// alias and identity associations are kept only when the new build lacks them.
fn merge_existing_keys_into(new: &mut QualifiedIdentity, existing: QualifiedIdentity) {
    for (key, value) in existing.private_keys.private_keys {
        new.private_keys.private_keys.entry(key).or_insert(value);
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
        } = input;

        // FR-8: validate the load-time encryption password up front, before the
        // network fetch, so a too-short password fails fast. The seal path
        // re-enforces the same rule authoritatively after insert.
        if let Some(password) = &encryption_password {
            validate_protection_password(password)?;
        }

        // Verify the owner private key
        let owner_private_key_bytes = verify_key_input(owner_private_key_input, "Owner")?;

        // Verify the voting private key
        let voting_private_key_bytes = verify_key_input(voting_private_key_input, "Voting")?;

        let payout_address_private_key_bytes =
            verify_key_input(payout_address_private_key_input, "Payout Address")?;

        // Parse the identity ID
        let identity_id = match Identifier::from_string(&identity_id_input, Encoding::Base58)
            .or_else(|_| Identifier::from_string(&identity_id_input, Encoding::Hex))
        {
            Ok(id) => id,
            Err(_e) => {
                return Err(TaskError::IdentifierParsingError {
                    input: identity_id_input,
                });
            }
        };

        // §10.9 / TC-EDGE-07: a fresh load rejects a ProTxHash already stored,
        // before any network fetch — so the existing node's alias/keys/protection
        // tier are never silently overwritten. Checked here, at the storage
        // layer, so every `RejectIfExists` caller is guarded uniformly.
        let existing_stored = self.get_local_qualified_identity(&identity_id)?;
        match load_mode {
            IdentityLoadMode::RejectIfExists if existing_stored.is_some() => {
                return Err(TaskError::DuplicateProTxHash {
                    identity_id,
                });
            }
            _ => {}
        }

        // Fetch the identity using the SDK
        let identity = match Identity::fetch_by_identifier(sdk, identity_id).await {
            Ok(Some(identity)) => identity,
            Ok(None) => return Err(TaskError::IdentityNotFound),
            Err(e) => return Err(TaskError::from(e)),
        };

        let mut encrypted_private_keys = BTreeMap::new();

        let wallets = self.wallets.read().map_err(TaskError::from)?.clone();

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
                    PrivateKeyData::Clear(owner_private_key_bytes),
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
                    PrivateKeyData::Clear(payout_address_private_key_bytes),
                ),
            );
        }

        // If the identity type is not a User, and we have a voting private key, verify it
        let associated_voter_identity = if identity_type != IdentityType::User {
            if let Some(voting_private_key_bytes) = voting_private_key_bytes {
                if let Ok(private_key) =
                    PrivateKey::from_byte_array(&voting_private_key_bytes, self.network)
                {
                    // Make the vote identifier
                    let address = private_key.public_key(&Secp256k1::new()).pubkey_hash();
                    let voter_identifier = Identifier::create_voter_identifier(
                        identity_id.as_bytes(),
                        address.as_ref(),
                    );

                    // Fetch the voter identifier
                    let voter_identity =
                        match Identity::fetch_by_identifier(sdk, voter_identifier).await {
                            Ok(Some(identity)) => identity,
                            Ok(None) => return Err(TaskError::IdentityNotFound),
                            Err(e) => return Err(TaskError::from(e)),
                        };

                    let key = self.verify_voting_key_exists_on_identity(
                        &voter_identity,
                        &voting_private_key_bytes,
                    )?;
                    let qualified_key =
                        QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
                            key.clone(),
                            self.network,
                            &wallets.values().collect::<Vec<_>>(),
                        );
                    encrypted_private_keys.insert(
                        (PrivateKeyOnVoterIdentity, key.id()),
                        (
                            qualified_key,
                            PrivateKeyData::Clear(voting_private_key_bytes),
                        ),
                    );
                    Some((voter_identity, key))
                } else {
                    return Err(TaskError::InvalidPrivateKey);
                }
            } else {
                None
            }
        } else {
            None
        };

        // let mut wallet_seed_hash: Option<(WalletSeedHash, u32)> = None;

        if identity_type == IdentityType::User {
            let input_private_keys = keys_input
                .into_iter()
                .filter_map(|key_string| {
                    Some(
                        verify_key_input(key_string, "User Key")
                            .map_err(TaskError::from)
                            .transpose()?
                            .and_then(|sk| {
                                PrivateKey::from_byte_array(&sk, self.network)
                                    .map_err(|_| TaskError::InvalidPrivateKey)
                            }),
                    )
                })
                .collect::<Result<Vec<PrivateKey>, TaskError>>()?;

            let secp = Secp256k1::new();
            #[allow(clippy::type_complexity)]
            let (public_key_lookup, public_key_hash_lookup): (
                HashMap<Vec<u8>, [u8; 32]>,
                HashMap<[u8; 20], [u8; 32]>,
            ) = input_private_keys
                .into_iter()
                .map(|private_key| {
                    let public_key = private_key.public_key(&secp);
                    let public_key_bytes = public_key.to_bytes();
                    let pub_key_hash = public_key.pubkey_hash().to_byte_array();
                    (
                        (public_key_bytes, private_key.inner.secret_bytes()),
                        (pub_key_hash, private_key.inner.secret_bytes()),
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
                        SecurityLevel::MEDIUM => PrivateKeyData::AlwaysClear(*private_key_bytes),
                        _ => PrivateKeyData::Clear(*private_key_bytes),
                    };
                    encrypted_private_keys
                        .insert(key_map_key, (qualified_key.clone(), private_data));
                    continue;
                }

                if let Some(private_key_bytes) =
                    public_key_hash_lookup.get(public_key.data().0.as_slice())
                {
                    let private_data = match public_key.security_level() {
                        SecurityLevel::MEDIUM => PrivateKeyData::AlwaysClear(*private_key_bytes),
                        _ => PrivateKeyData::Clear(*private_key_bytes),
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

        // Fetch DPNS names using SDK
        let dpns_names_document_query = DocumentQuery {
            select: SelectProjection::documents(),
            data_contract: self.dpns_contract.clone(),
            document_type_name: "domain".to_string(),
            where_clauses: vec![WhereClause {
                field: "records.identity".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(identity_id.into()),
            }],
            group_by: Vec::new(),
            having: Vec::new(),
            order_by_clauses: vec![],
            limit: 100,
            start: None,
        };

        let maybe_owned_dpns_names = Document::fetch_many(sdk, dpns_names_document_query)
            .await
            .map(|document_map| {
                document_map
                    .values()
                    .filter_map(|maybe_doc| {
                        maybe_doc.as_ref().and_then(|doc| {
                            let name = doc
                                .get("label")
                                .map(|label| label.to_str().unwrap_or_default());
                            let acquired_at = doc
                                .created_at()
                                .into_iter()
                                .chain(doc.transferred_at())
                                .max();

                            match (name, acquired_at) {
                                (Some(name), Some(acquired_at)) => Some(DPNSNameInfo {
                                    name: name.to_string(),
                                    acquired_at,
                                }),
                                _ => None,
                            }
                        })
                    })
                    .collect::<Vec<DPNSNameInfo>>()
            })
            .map_err(|e| TaskError::DpnsFetchError {
                source: Box::new(e),
            })?;

        // Determine alias: use user input, or fall back to first DPNS name if available
        let alias = if !alias_input.is_empty() {
            Some(alias_input)
        } else if !maybe_owned_dpns_names.is_empty() {
            Some(format!("{}.dash", maybe_owned_dpns_names[0].name))
        } else {
            None
        };

        let mut qualified_identity = QualifiedIdentity {
            identity,
            associated_voter_identity,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type,
            alias,
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
        // §10.8: an in-place update (the "Add voting key" fix-up) merges the
        // newly-supplied keys into the already-stored identity's keys instead of
        // clobbering them — the new voting key is added while the existing
        // Owner/Payout keys (which the update leaves blank) survive.
        if load_mode == IdentityLoadMode::MergeIntoExisting
            && let Some(existing) = existing_stored
        {
            merge_existing_keys_into(&mut qualified_identity, existing);
        }

        let wallet_info = qualified_identity
            .determine_wallet_info()
            .map_err(|e| TaskError::WalletInfoDeterminationFailed { detail: e })?;

        // Insert qualified identity into the database
        self.insert_local_qualified_identity(&qualified_identity, &wallet_info)?;

        if let Some((wallet_seed_hash, identity_index)) = wallet_info
            && let Some(wallet_arc) = wallets.get(&wallet_seed_hash)
        {
            let mut wallet = wallet_arc.write().map_err(TaskError::from)?;
            wallet
                .identities
                .insert(identity_index, qualified_identity.identity.clone());
        }

        // FR-8: when a load-time password was supplied, seal the just-inserted
        // keyless keys Tier-2 through the existing per-identity protect
        // envelope. `insert_local_qualified_identity` migrated the resident
        // plaintext into the keyless vault, so `protect_identity_keys`
        // (validate → fail-closed guard → seal via the secret_seam chokepoint)
        // reloads from the DB and seals them — one path, no new crypto.
        if let Some(password) = encryption_password {
            self.protect_identity_keys(qualified_identity.identity.id(), password, None)?;
        }

        Ok(BackendTaskSuccessResult::LoadedIdentity(qualified_identity))
    }

    pub(super) async fn match_user_identity_keys_with_wallet(
        &self,
        identity: &Identity,
        wallets: &BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>,
        wallet_filter: Option<WalletSeedHash>,
    ) -> Result<WalletMatchResult, TaskError> {
        let highest_identity_key_id = identity.public_keys().keys().copied().max().unwrap_or(0);
        let top_bound = highest_identity_key_id.saturating_add(6).max(1);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TaskResult;
    use crate::app_dir::ensure_env_file;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::secret::Secret;
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::utils::tasks::TaskManager;
    use crate::wallet_backend::IdentityKeyView;
    use crate::wallet_backend::secret_seam::SecretScheme;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::Identity;
    use dash_sdk::dpp::identity::KeyID;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::IdentityPublicKey;
    use platform_wallet_storage::secrets::SecretString;

    const M: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
    const V: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnVoterIdentity;

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
        ks.private_keys.insert(
            (M, owner.id()),
            (
                QualifiedIdentityPublicKey::from(owner),
                PrivateKeyData::Clear([0xA0; 32]),
            ),
        );
        ks.private_keys.insert(
            (V, voter.id()),
            (
                QualifiedIdentityPublicKey::from(voter),
                PrivateKeyData::Clear([0xB0; 32]),
            ),
        );
        ks.private_keys.insert(
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

    /// TC-FR8-01/02/10 — a load-time encryption password seals ALL of a
    /// masternode's keys (voting, owner, and identity auth) Tier-2 through the
    /// existing per-identity protect envelope. Without a password the same keys
    /// stay keyless (Tier-1) after insert. Drives the exact call
    /// [`load_identity`] makes when `encryption_password` is `Some`, on an
    /// offline wired `AppContext` (no network I/O).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn load_time_password_seals_voting_owner_and_identity_keys() {
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

        let (qi, triple) = masternode_shaped_qi();
        let identity_id = qi.identity.id();
        // Insert migrates the resident-plaintext keys into the keyless vault
        // (Tier-1), exactly as the load path does before the optional seal.
        ctx.insert_local_qualified_identity(&qi, &None)
            .expect("insert masternode identity");

        let backend = ctx.wallet_backend().expect("backend wired");
        let view = IdentityKeyView::new(backend.secret_store(), identity_id.to_buffer());

        // No-password (None) path: every key is keyless after insert.
        for (t, k) in &triple {
            assert_eq!(
                view.scheme(t, *k).expect("scheme"),
                SecretScheme::Unprotected,
                "key ({t:?}, {k}) must be keyless before any load-time seal",
            );
        }

        // The exact call `load_identity` makes for `encryption_password = Some`.
        ctx.protect_identity_keys(identity_id, Secret::new("one-identity-password"), None)
            .expect("load-time seal must succeed");

        let pw = SecretString::new("one-identity-password");
        for (t, k) in &triple {
            assert_eq!(
                view.scheme(t, *k).expect("scheme"),
                SecretScheme::Protected,
                "key ({t:?}, {k}) must be sealed Tier-2 after the load-time password",
            );
            assert!(
                view.get_protected(t, *k, &pw)
                    .expect("get_protected")
                    .is_some(),
                "sealed key ({t:?}, {k}) must round-trip under the password",
            );
        }

        backend.shutdown().await;
    }

    /// QA-005 / §10.8 — the testable core of the "Add voting key" in-place
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
            .private_keys
            .get(&voter_key)
            .expect("existing voter key")
            .clone();
        new.private_keys.private_keys.insert(
            voter_key.clone(),
            (voter_pk, PrivateKeyData::Clear([0xEE; 32])),
        );

        merge_existing_keys_into(&mut new, existing);

        // Owner and identity-auth keys survive the voter-key-only update.
        assert!(
            new.private_keys.private_keys.contains_key(&owner_key),
            "owner key must survive a voting-key-only update",
        );
        assert!(
            new.private_keys.private_keys.contains_key(&idkey_key),
            "identity-auth key must survive a voting-key-only update",
        );
        // The resupplied voting key wins on collision (0xEE, not the old 0xB0).
        let (_, merged_voter) = new
            .private_keys
            .private_keys
            .get(&voter_key)
            .expect("voter key present after merge");
        assert!(
            matches!(merged_voter, PrivateKeyData::Clear(b) if *b == [0xEE; 32]),
            "the resupplied voting key must win on collision",
        );
    }

    /// QA-006 / §10.9 / TC-EDGE-07 — a fresh load (`RejectIfExists`) of a
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
                still.private_keys.private_keys.contains_key(&(t.clone(), *k)),
                "key ({t:?}, {k}) of the first node must survive a rejected duplicate load",
            );
        }

        ctx.wallet_backend().expect("backend").shutdown().await;
    }
}
