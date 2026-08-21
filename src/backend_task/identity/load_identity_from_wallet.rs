use super::{BackendTaskSuccessResult, IdentityIndex};
use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
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
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::key_wallet::bip32::{DerivationPath, KeyDerivationType};
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{SelectProjection, WhereClause, WhereOperator};
use dash_sdk::platform::types::identity::NonUniquePublicKeyHashQuery;
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identity};
use std::collections::BTreeMap;
use std::sync::Arc;

impl AppContext {
    pub(super) async fn load_user_identity_from_wallet(
        &self,
        sdk: &Sdk,
        wallet_arc_ref: WalletArcRef,
        identity_index: IdentityIndex,
        _sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        const AUTH_KEY_LOOKUP_WINDOW: u32 = 12;

        let mut fetched_identity: Option<Identity> = None;
        let mut queried_public_key = None;
        let mut queried_wallet_key_index = None;

        for key_index in 0..AUTH_KEY_LOOKUP_WINDOW {
            let public_key = self
                .resolve_identity_auth_pubkey(
                    &wallet_arc_ref.wallet,
                    true,
                    identity_index,
                    key_index,
                )
                .await?;

            let key_hash = public_key.pubkey_hash().into();
            let query = NonUniquePublicKeyHashQuery {
                key_hash,
                after: None,
            };

            match Identity::fetch(sdk, query).await {
                Ok(Some(identity)) => {
                    fetched_identity = Some(identity);
                    queried_public_key = Some(public_key);
                    queried_wallet_key_index = Some(key_index);
                    break;
                }
                Ok(None) => continue,
                Err(e) => return Err(TaskError::from(e)),
            }
        }

        let identity = match fetched_identity {
            Some(identity) => identity,
            None => {
                return Err(TaskError::WalletIdentityNotFound {
                    identity_index,
                    auth_key_count: AUTH_KEY_LOOKUP_WINDOW as usize,
                });
            }
        };

        let queried_public_key =
            queried_public_key.expect("queried public key should exist when identity is fetched");
        let queried_wallet_key_index = queried_wallet_key_index
            .expect("wallet key index should exist when identity is fetched");

        let queried_key_hash: [u8; 20] = queried_public_key.pubkey_hash().into();
        let matching_identity_key = identity.public_keys().values().find(|key| {
            key.public_key_hash()
                .ok()
                .map(|hash| hash == queried_key_hash)
                .unwrap_or(false)
        });

        let matching_identity_key = match matching_identity_key {
            Some(key) => key,
            None => {
                return Err(TaskError::WalletIdentityKeyMismatch);
            }
        };
        let matching_identity_key_id = matching_identity_key.id();

        let identity_id = identity.id();

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
            offset: None,
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

        let highest_identity_key_id = identity
            .public_keys()
            .keys()
            .copied()
            .max()
            .unwrap_or(matching_identity_key_id);

        let mut top_bound = highest_identity_key_id.saturating_add(1);
        top_bound = top_bound.max(queried_wallet_key_index.saturating_add(1));
        top_bound = top_bound.saturating_add(5);

        let wallet_seed_hash = wallet_arc_ref.wallet.read()?.seed_hash();
        let (public_key_result_map, public_key_hash_result_map) = self
            .resolve_identity_auth_pubkeys_data_map(
                &wallet_arc_ref.wallet,
                true,
                true,
                identity_index,
                0..top_bound,
            )
            .await?;

        let private_keys_map = identity
            .public_keys()
            .values()
            .filter_map(|public_key| {
                let index: u32 = match public_key.key_type() {
                    KeyType::ECDSA_SECP256K1 => public_key_result_map
                        .get(public_key.data().as_slice())
                        .cloned(),
                    KeyType::ECDSA_HASH160 => {
                        let hash: [u8; 20] = public_key.data().as_slice().try_into().ok()?;
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
                let wallet_derivation_path = WalletDerivationPath {
                    wallet_seed_hash,
                    derivation_path,
                };
                Some((
                    (PrivateKeyTarget::PrivateKeyOnMainIdentity, public_key.id()),
                    (
                        QualifiedIdentityPublicKey {
                            identity_public_key: public_key.clone(),
                            in_wallet_at_derivation_path: Some(wallet_derivation_path.clone()),
                        },
                        PrivateKeyData::AtWalletDerivationPath(wallet_derivation_path),
                    ),
                ))
            })
            .collect::<BTreeMap<_, _>>();

        if private_keys_map.is_empty() {
            return Err(TaskError::NoMatchingWalletKeys);
        }

        if !private_keys_map.contains_key(&(
            PrivateKeyTarget::PrivateKeyOnMainIdentity,
            matching_identity_key_id,
        )) {
            return Err(TaskError::WalletKeyDerivationPathNotFound);
        }

        let private_keys = private_keys_map.into();

        let wallet_seed_hash = wallet_arc_ref.wallet.read()?.seed_hash();

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
            secret_access: None,
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
        qualified_identity.secret_access = self.wallet_backend().ok().map(|b| b.secret_access());
        qualified_identity.wallet_index = Some(identity_index);
        qualified_identity.status = IdentityStatus::Active;
        qualified_identity.network = self.network;

        // Carry the user-assigned alias from any existing record so a re-load
        // refreshes keys/DPNS without wiping DET-only metadata.
        if let Some(existing) = self.get_identity_by_id(&identity_id)? {
            qualified_identity.alias = existing.alias;
            self.update_local_qualified_identity(&qualified_identity)?;
        } else {
            self.insert_local_qualified_identity(
                &qualified_identity,
                &Some((wallet_seed_hash, identity_index)),
            )?;
        }

        {
            let mut wallet = wallet_arc_ref.wallet.write()?;
            wallet
                .identities
                .insert(identity_index, qualified_identity.identity.clone());
        }

        Ok(BackendTaskSuccessResult::IdentitiesLoaded { count: 1 })
    }

    pub(super) async fn load_user_identities_up_to_index(
        self: &Arc<Self>,
        wallet_arc_ref: WalletArcRef,
        seed_identity_index: IdentityIndex,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // The interactive search seeds the rolling window from the user-supplied
        // index and may prompt for the passphrase on a cold cache.
        let summary = self
            .discover_identities_gap_limited(
                &wallet_arc_ref.wallet,
                seed_identity_index,
                true,
                Some(&sender),
            )
            .await?;

        if summary.found == 0 {
            return Err(TaskError::NoWalletIdentitiesFound {
                max_index: seed_identity_index,
            });
        }

        Ok(BackendTaskSuccessResult::IdentitiesLoaded {
            count: summary.found,
        })
    }
}
