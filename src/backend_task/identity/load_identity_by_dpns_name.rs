use super::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
use crate::model::qualified_identity::{DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity};
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::util::strings::convert_to_homograph_safe_chars;
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier, Identity, Fetch};

impl AppContext {
    /// Load an identity by its DPNS name
    pub(super) async fn load_identity_by_dpns_name(
        &self,
        sdk: &Sdk,
        dpns_name: String,
    ) -> Result<BackendTaskSuccessResult, String> {
        // Normalize the name (convert to lowercase and handle homoglyphs)
        let normalized_name = convert_to_homograph_safe_chars(&dpns_name);

        // Query the DPNS contract for the domain document
        let domain_query = DocumentQuery {
            data_contract: self.dpns_contract.clone(),
            document_type_name: "domain".to_string(),
            where_clauses: vec![
                WhereClause {
                    field: "normalizedParentDomainName".to_string(),
                    operator: WhereOperator::Equal,
                    value: Value::Text("dash".to_string()),
                },
                WhereClause {
                    field: "normalizedLabel".to_string(),
                    operator: WhereOperator::Equal,
                    value: Value::Text(normalized_name.clone()),
                },
            ],
            order_by_clauses: vec![],
            limit: 1,
            start: None,
        };

        let documents = Document::fetch_many(sdk, domain_query)
            .await
            .map_err(|e| format!("Error querying DPNS: {}", e))?;

        // Get the first (and should be only) document
        let domain_doc = documents
            .values()
            .filter_map(|maybe_doc| maybe_doc.as_ref())
            .next()
            .ok_or_else(|| format!("No identity found with DPNS name '{}.dash'", dpns_name))?;

        // Extract the identity ID from the records.identity field
        let identity_id = domain_doc
            .get("records")
            .and_then(|records| {
                if let Value::Map(map) = records {
                    map.iter()
                        .find(|(k, _)| {
                            if let Value::Text(key) = k {
                                key == "identity"
                            } else {
                                false
                            }
                        })
                        .map(|(_, v)| v.clone())
                } else {
                    None
                }
            })
            .and_then(|id_value| {
                if let Value::Identifier(id_bytes) = id_value {
                    Some(Identifier::from(id_bytes))
                } else {
                    None
                }
            })
            .ok_or_else(|| "DPNS domain document does not contain a valid identity reference".to_string())?;

        // Fetch the identity
        let identity = match Identity::fetch_by_identifier(sdk, identity_id).await {
            Ok(Some(identity)) => identity,
            Ok(None) => return Err("Identity referenced by DPNS name not found".to_string()),
            Err(e) => return Err(format!("Error fetching identity: {}", e)),
        };

        // Get the label from the document for display
        let label = domain_doc
            .get("label")
            .and_then(|l| l.to_str().ok())
            .unwrap_or(&dpns_name)
            .to_string();

        // Fetch all DPNS names owned by this identity
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

        let owned_dpns_names = Document::fetch_many(sdk, dpns_names_document_query)
            .await
            .map(|document_map| {
                document_map
                    .values()
                    .filter_map(|maybe_doc| {
                        maybe_doc.as_ref().and_then(|doc| {
                            let name = doc
                                .get("label")
                                .map(|l| l.to_str().unwrap_or_default());
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

        let wallets = self.wallets.read().unwrap().clone();

        let qualified_identity = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: Some(label),
            private_keys: KeyStorage::default(),
            dpns_names: owned_dpns_names,
            associated_wallets: wallets
                .values()
                .map(|wallet| (wallet.read().unwrap().seed_hash(), wallet.clone()))
                .collect(),
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: self.network,
        };
        let wallet_info = qualified_identity.determine_wallet_info()?;

        // Insert qualified identity into the database
        self.insert_local_qualified_identity(&qualified_identity, &wallet_info)
            .map_err(|e| format!("Database error: {}", e))?;

        Ok(BackendTaskSuccessResult::LoadedIdentity(qualified_identity))
    }
}
