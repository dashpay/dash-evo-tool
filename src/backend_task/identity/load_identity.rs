use super::BackendTaskSuccessResult;
use crate::backend_task::identity::{verify_key_input, IdentityInputToLoad};
use crate::context::AppContext;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::PrivateKeyTarget::{
    self, PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::{DPNSNameInfo, IdentityType, QualifiedIdentity};
use dash_sdk::dashcore_rpc::dashcore::key::Secp256k1;
use dash_sdk::dashcore_rpc::dashcore::PrivateKey;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identifier::MasternodeIdentifiers;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier, Identity};
use dash_sdk::Sdk;
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

        let wallets = self.wallets.read().unwrap();

        if identity_type != IdentityType::User && owner_private_key_bytes.is_some() {
            let owner_private_key_bytes = owner_private_key_bytes.unwrap();
            let key =
                self.verify_owner_key_exists_on_identity(&identity, &owner_private_key_bytes)?;
            let key_id = key.id();
            let qualified_key =
                QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
                    key,
                    wallets.as_slice(),
                );
            encrypted_private_keys.insert(
                (PrivateKeyOnMainIdentity, key_id),
                (qualified_key, owner_private_key_bytes),
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
                    wallets.as_slice(),
                );
            encrypted_private_keys.insert(
                (PrivateKeyOnMainIdentity, key_id),
                (qualified_key, payout_address_private_key_bytes),
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
                        wallets.as_slice(),
                    );
                encrypted_private_keys.insert(
                    (PrivateKeyOnVoterIdentity, key.id()),
                    (qualified_key, voting_private_key_bytes),
                );
                Some((voter_identity, key))
            } else {
                return Err("Voting private key is not valid".to_string());
            }
        } else {
            None
        };

        if identity_type == IdentityType::User {
            for (i, private_key_input) in keys_input.into_iter().enumerate() {
                let key_id = i as u32;
                let public_key = match identity.public_keys().get(&key_id) {
                    Some(key) => key,
                    None => return Err("No public key matching key id {key_id}".to_string()),
                };
                let private_key_bytes = match verify_key_input(
                    private_key_input,
                    &public_key.key_type().to_string(),
                )? {
                    Some(bytes) => bytes,
                    None => {
                        return Err("Private key input length is 0 for key id {key_id}".to_string())
                    }
                };
                let qualified_key =
                    QualifiedIdentityPublicKey::from_identity_public_key_with_wallets_check(
                        public_key.clone(),
                        wallets.as_slice(),
                    );
                encrypted_private_keys.insert(
                    (PrivateKeyTarget::PrivateKeyOnMainIdentity, key_id),
                    (qualified_key, private_key_bytes),
                );
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
                                .get("normalizedLabel")
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
        };

        // Insert qualified identity into the database
        self.insert_local_qualified_identity(&qualified_identity, None)
            .map_err(|e| format!("Database error: {}", e))?;

        Ok(BackendTaskSuccessResult::Message(
            "Successfully loaded identity".to_string(),
        ))
    }
}
