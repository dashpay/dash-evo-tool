use super::{BackendTaskSuccessResult, IdentityIndex};
use crate::backend_task::identity::{verify_key_input, IdentityInputToLoad};
use crate::context::AppContext;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::PrivateKeyTarget::{
    self, PrivateKeyOnMainIdentity, PrivateKeyOnVoterIdentity,
};
use crate::model::qualified_identity::{DPNSNameInfo, IdentityType, QualifiedIdentity};
use crate::model::wallet::Wallet;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::types::identity::PublicKeyHash;
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identifier, Identity};
use dash_sdk::Sdk;

impl AppContext {
    pub(super) async fn load_user_identity_from_wallet(
        &self,
        sdk: &Sdk,
        wallet: Wallet,
        identity_index: IdentityIndex,
    ) -> Result<BackendTaskSuccessResult, String> {
        let public_key =
            wallet.identity_authentication_ecdsa_public_key(self.network, identity_index, 0)?;

        let Some(identity) = Identity::fetch(
            &sdk,
            PublicKeyHash(public_key.pubkey_hash().to_byte_array()),
        )
        .await
        .map_err(|e| e.to_string())?
        else {
            return Ok(BackendTaskSuccessResult::None);
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
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
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
