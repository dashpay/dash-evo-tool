use super::{BackendTaskSuccessResult, IdentityIndex};
use crate::context::AppContext;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, PrivateKeyTarget, QualifiedIdentity,
};
use crate::model::wallet::WalletArcRef;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyID, KeyType};
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, KeyDerivationType};
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::types::identity::NonUniquePublicKeyHashQuery;
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identity};
use std::collections::BTreeMap;

impl AppContext {
    pub(super) async fn load_user_identity_from_wallet(
        &self,
        sdk: &Sdk,
        wallet_arc_ref: WalletArcRef,
        identity_index: IdentityIndex,
    ) -> Result<BackendTaskSuccessResult, String> {
        tracing::info!(
            "Loading user identity at index {} from wallet {}",
            identity_index,
            wallet_arc_ref.wallet.read().unwrap().alias.clone().unwrap()
        );
        let public_key = {
            let wallet = wallet_arc_ref.wallet.write().unwrap();
            wallet.identity_authentication_ecdsa_public_key(self.network, identity_index, 0)?
        };

        tracing::info!(
            "Fetched public key at index 0 for identity index {}: {:?}",
            identity_index,
            public_key
        );

        let key_hash = public_key.pubkey_hash();
        let query = NonUniquePublicKeyHashQuery {
            key_hash: key_hash.into(),
            after: None,
        };

        tracing::info!(
            "Derived public key hash for identity index {}: {}",
            identity_index,
            hex::encode(key_hash)
        );

        let identity = match Identity::fetch(sdk, query).await {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                return Err(format!(
                    "No identity found for public key at index {}",
                    identity_index
                ));
            }
            Err(e) => return Err(e.to_string()),
        };

        let identity_id = identity.id();

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

        let sdk_guard = {
            let guard = self.sdk.read().unwrap();
            guard.clone()
        };

        let maybe_owned_dpns_names = Document::fetch_many(&sdk_guard, dpns_names_document_query)
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

        let top_bound = identity.public_keys().len() as u32 + 5;

        let wallet_seed_hash;
        let (public_key_result_map, public_key_hash_result_map) = {
            let mut wallet = wallet_arc_ref.wallet.write().unwrap();
            wallet_seed_hash = wallet.seed_hash();
            wallet.identity_authentication_ecdsa_public_keys_data_map(
                self.network,
                identity_index,
                0..top_bound,
                Some(self),
            )?
        };

        let private_keys = identity.public_keys().values().filter_map(|public_key| {
            let index: u32 = match public_key.key_type() {
                KeyType::ECDSA_SECP256K1 => {
                    public_key_result_map.get(public_key.data().as_slice()).cloned()
                }
                KeyType::ECDSA_HASH160 => {
                    let hash: [u8;20] = public_key.data().as_slice().try_into().ok()?;
                    public_key_hash_result_map.get(&hash).cloned()
                }
                _ => None,
            }?;
            let derivation_path = DerivationPath::identity_authentication_path(
                self.network,
                KeyDerivationType::ECDSA,
                identity_index,
                index,
            );
            let wallet_derivation_path = WalletDerivationPath { wallet_seed_hash, derivation_path};
            Some(((PrivateKeyTarget::PrivateKeyOnMainIdentity, public_key.id()), (QualifiedIdentityPublicKey { identity_public_key: public_key.clone(), in_wallet_at_derivation_path: Some(wallet_derivation_path.clone()) }, PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path))))
        }).collect::<BTreeMap<(PrivateKeyTarget, KeyID), (QualifiedIdentityPublicKey, PrivateKeyData)>>().into();

        let wallet_seed_hash = wallet_arc_ref.wallet.read().unwrap().seed_hash();

        let mut qualified_identity = QualifiedIdentity {
            identity: identity.clone(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: Vec::new(),
            associated_wallets: BTreeMap::new(),
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: self.network,
        };

        qualified_identity.identity = identity;
        qualified_identity.private_keys = private_keys;
        qualified_identity.dpns_names = maybe_owned_dpns_names;
        qualified_identity.associated_wallets =
            BTreeMap::from([(wallet_seed_hash, wallet_arc_ref.wallet.clone())]);
        qualified_identity.wallet_index = Some(identity_index);
        qualified_identity.status = IdentityStatus::Active;
        qualified_identity.network = self.network;

        // Insert qualified identity into the database
        self.insert_local_qualified_identity(
            &qualified_identity,
            &Some((wallet_seed_hash, identity_index)),
        )
        .map_err(|e| format!("Database error: {}", e))?;

        {
            let mut wallet = wallet_arc_ref.wallet.write().unwrap();
            wallet
                .identities
                .insert(identity_index, qualified_identity.identity.clone());
        }

        Ok(BackendTaskSuccessResult::Message(
            "Successfully loaded identity".to_string(),
        ))
    }
}
