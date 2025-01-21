use super::BackendTaskSuccessResult;
use crate::backend_task::identity::{verify_key_input, IdentityInputToLoad};
use crate::context::AppContext;
use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::PrivateKeyTarget::{
    self, PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::{DPNSNameInfo, IdentityType, QualifiedIdentity};
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dashcore_rpc::dashcore::PrivateKey;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identifier::MasternodeIdentifiers;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::SecurityLevel;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier, Identity};
use dash_sdk::Sdk;
use egui::ahash::HashMap;
use std::collections::BTreeMap;

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

        let wallets = self.wallets.read().unwrap().clone();

        if identity_type != IdentityType::User && owner_private_key_bytes.is_some() {
            let owner_private_key_bytes = owner_private_key_bytes.unwrap();
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

        if identity_type != IdentityType::User && payout_address_private_key_bytes.is_some() {
            let payout_address_private_key_bytes = payout_address_private_key_bytes.unwrap();
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
        let associated_voter_identity = if identity_type != IdentityType::User
            && voting_private_key_bytes.is_some()
        {
            let voting_private_key_bytes = voting_private_key_bytes.unwrap();
            if let Ok(private_key) =
                PrivateKey::from_slice(voting_private_key_bytes.as_slice(), self.network)
            {
                // Make the vote identifier
                let address = private_key.public_key(&Secp256k1::new()).pubkey_hash();
                let voter_identifier =
                    Identifier::create_voter_identifier(identity_id.as_bytes(), address.as_ref());

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
                                PrivateKey::from_slice(sk.as_slice(), self.network)
                                    .map_err(|e| e.to_string())
                            }),
                    )
                })
                .collect::<Result<Vec<PrivateKey>, String>>()?;

            let secp = Secp256k1::new();
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
                let qualified_key =
                    QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
                        public_key.clone(),
                        self.network,
                        &wallets.values().collect::<Vec<_>>(),
                    );

                if let Some(wallet_derivation_path) =
                    qualified_key.in_wallet_at_derivation_path.clone()
                {
                    encrypted_private_keys.insert(
                        (PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id),
                        (
                            qualified_key,
                            PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path),
                        ),
                    );
                } else if let Some(private_key_bytes) =
                    public_key_lookup.get(public_key.data().0.as_slice())
                {
                    let private_data = match public_key.security_level() {
                        SecurityLevel::MEDIUM => PrivateKeyData::AlwaysClear(*private_key_bytes),
                        _ => PrivateKeyData::Clear(*private_key_bytes),
                    };
                    encrypted_private_keys.insert(
                        (PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id),
                        (qualified_key, private_data),
                    );
                } else if let Some(private_key_bytes) =
                    public_key_hash_lookup.get(public_key.data().0.as_slice())
                {
                    let private_data = match public_key.security_level() {
                        SecurityLevel::MEDIUM => PrivateKeyData::AlwaysClear(*private_key_bytes),
                        _ => PrivateKeyData::Clear(*private_key_bytes),
                    };
                    encrypted_private_keys.insert(
                        (PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id),
                        (qualified_key, private_data),
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

        let maybe_owned_dpns_names = Document::fetch_many(&self.sdk, dpns_names_document_query)
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
                    .into()
            })
            .map_err(|e| format!("Error fetching DPNS names: {}", e))?;

        let qualified_identity = QualifiedIdentity {
            identity,
            associated_voter_identity,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type,
            alias: if alias_input.is_empty() {
                None
            } else {
                Some(alias_input)
            },
            private_keys: encrypted_private_keys.into(),
            dpns_names: maybe_owned_dpns_names,
            associated_wallets: wallets
                .values()
                .map(|wallet| (wallet.read().unwrap().seed_hash(), wallet.clone()))
                .collect(),
            wallet_index: None, //todo
            top_ups: Default::default(),
        };

        // Insert qualified identity into the database
        self.insert_local_qualified_identity(&qualified_identity, None)
            .map_err(|e| format!("Database error: {}", e))?;

        Ok(BackendTaskSuccessResult::Message(
            "Successfully loaded identity".to_string(),
        ))
    }
}
