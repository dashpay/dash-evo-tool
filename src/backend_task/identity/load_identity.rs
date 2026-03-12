use super::BackendTaskSuccessResult;
use crate::backend_task::identity::{IdentityInputToLoad, verify_key_input};
use crate::context::AppContext;
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
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier, Identity};
use egui::ahash::HashMap;
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::sync::{Arc, RwLock};

type WalletKeyMap = BTreeMap<(PrivateKeyTarget, u32), (QualifiedIdentityPublicKey, PrivateKeyData)>;
type WalletMatchResult = Option<(WalletSeedHash, u32, WalletKeyMap)>;

impl AppContext {
    pub(super) async fn load_identity(
        &self,
        sdk: &Sdk,
        input: IdentityInputToLoad,
    ) -> Result<BackendTaskSuccessResult, String> {
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
        } = input;

        // Verify the voting private key
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
            Err(e) => return Err(format!("Identifier error: {}", e)),
        };

        // Fetch the identity using the SDK
        let identity = match Identity::fetch_by_identifier(sdk, identity_id).await {
            Ok(Some(identity)) => identity,
            Ok(None) => return Err("Identity not found".to_string()),
            Err(e) => return Err(format!("Error fetching identity: {}", e)),
        };

        let mut encrypted_private_keys = BTreeMap::new();

        let wallets = self
            .wallets
            .read()
            .map_err(|_| "Wallets lock poisoned".to_string())?
            .clone();

        if identity_type == IdentityType::User
            && derive_keys_from_wallets
            && let Some((_, _, wallet_private_keys)) = self.match_user_identity_keys_with_wallet(
                &identity,
                &wallets,
                selected_wallet_seed_hash,
            )?
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
                            Ok(None) => return Err("Voter Identity not found".to_string()),
                            Err(e) => return Err(format!("Error fetching voter identity: {}", e)),
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
                    return Err("Voting private key is not valid".to_string());
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
                            .transpose()?
                            .and_then(|sk| {
                                PrivateKey::from_byte_array(&sk, self.network)
                                    .map_err(|e| e.to_string())
                            }),
                    )
                })
                .collect::<Result<Vec<PrivateKey>, String>>()?;

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
            data_contract: self.dpns_contract.clone(),
            document_type_name: "domain".to_string(),
            where_clauses: vec![WhereClause {
                field: "records.identity".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(identity_id.into()),
            }],
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
            .map_err(|e| format!("Error fetching DPNS names: {}", e))?;

        // Determine alias: use user input, or fall back to first DPNS name if available
        let alias = if !alias_input.is_empty() {
            Some(alias_input)
        } else if !maybe_owned_dpns_names.is_empty() {
            Some(format!("{}.dash", maybe_owned_dpns_names[0].name))
        } else {
            None
        };

        let qualified_identity = QualifiedIdentity {
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
                .filter_map(|wallet| {
                    wallet.read().ok().map(|w| (w.seed_hash(), wallet.clone()))
                })
                .collect(),
            wallet_index: None, //todo
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: self.network,
        };
        let wallet_info = qualified_identity.determine_wallet_info()?;

        // Insert qualified identity into the database
        self.insert_local_qualified_identity(&qualified_identity, &wallet_info)
            .map_err(|e| e.to_string())?;

        if let Some((wallet_seed_hash, identity_index)) = wallet_info
            && let Some(wallet_arc) = wallets.get(&wallet_seed_hash)
        {
            let mut wallet = wallet_arc
                .write()
                .map_err(|_| "Wallet lock poisoned".to_string())?;
            wallet
                .identities
                .insert(identity_index, qualified_identity.identity.clone());
        }

        Ok(BackendTaskSuccessResult::LoadedIdentity(qualified_identity))
    }

    pub(super) fn match_user_identity_keys_with_wallet(
        &self,
        identity: &Identity,
        wallets: &BTreeMap<WalletSeedHash, Arc<RwLock<Wallet>>>,
        wallet_filter: Option<WalletSeedHash>,
    ) -> Result<WalletMatchResult, String> {
        let highest_identity_key_id = identity.public_keys().keys().copied().max().unwrap_or(0);
        let top_bound = highest_identity_key_id.saturating_add(6).max(1);

        for (&wallet_seed_hash, wallet_arc) in wallets.iter() {
            if wallet_filter.is_some_and(|filter| filter != wallet_seed_hash) {
                continue;
            }
            let mut wallet = match wallet_arc.write() {
                Ok(guard) => guard,
                Err(_) => continue, // Skip poisoned wallets rather than failing the whole operation
            };
            if !wallet.is_open() {
                continue;
            }

            if let Some((identity_index, wallet_private_keys)) = self
                .attempt_match_identity_with_wallet(
                    identity,
                    &mut wallet,
                    wallet_seed_hash,
                    top_bound,
                )?
            {
                drop(wallet);
                return Ok(Some((
                    wallet_seed_hash,
                    identity_index,
                    wallet_private_keys,
                )));
            }
        }

        Ok(None)
    }

    fn attempt_match_identity_with_wallet(
        &self,
        identity: &Identity,
        wallet: &mut Wallet,
        wallet_seed_hash: WalletSeedHash,
        top_bound: u32,
    ) -> Result<Option<(u32, WalletKeyMap)>, String> {
        let identity_id = identity.id();

        if let Some((&identity_index, _)) = wallet
            .identities
            .iter()
            .find(|(_, existing)| existing.id() == identity_id)
        {
            let (public_key_map, public_key_hash_map) = wallet
                .identity_authentication_ecdsa_public_keys_data_map(
                    self,
                    true,
                    self.network,
                    identity_index,
                    0..top_bound,
                )?;
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
            let (public_key_map, public_key_hash_map) = wallet
                .identity_authentication_ecdsa_public_keys_data_map(
                    self,
                    false,
                    self.network,
                    candidate_index,
                    0..top_bound,
                )?;

            if !Self::identity_matches_wallet_key_material(
                identity,
                &public_key_map,
                &public_key_hash_map,
            ) {
                continue;
            }

            let (public_key_map, public_key_hash_map) = wallet
                .identity_authentication_ecdsa_public_keys_data_map(
                    self,
                    true,
                    self.network,
                    candidate_index,
                    0..top_bound,
                )?;

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
