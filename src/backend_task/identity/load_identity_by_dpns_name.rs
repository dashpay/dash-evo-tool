use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity,
};
use crate::model::wallet::WalletSeedHash;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{SelectProjection, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, Fetch, FetchMany, Identity};

impl AppContext {
    /// Load an identity by its DPNS name
    pub(super) async fn load_identity_by_dpns_name(
        &self,
        sdk: &Sdk,
        dpns_name: String,
        selected_wallet_seed_hash: Option<WalletSeedHash>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // The name's own record names its identity, which after a transfer is
        // not the document's owner.
        let identity_id = sdk
            .resolve_dpns_name(dpns_name.trim())
            .await
            .map_err(TaskError::from)?
            .ok_or(TaskError::IdentityNotFound)?;

        // Fetch the identity
        let identity = match Identity::fetch_by_identifier(sdk, identity_id).await {
            Ok(Some(identity)) => identity,
            Ok(None) => return Err(TaskError::IdentityNotFound),
            Err(e) => return Err(TaskError::from(e)),
        };

        // Fetch all DPNS names owned by this identity
        let dpns_names_document_query = DocumentQuery {
            sub_queries: Vec::new(),
            select: SelectProjection::documents(),
            data_contract: self.dpns_contract.clone(),
            document_type_name: "domain".to_string(),
            where_clauses: vec![WhereClause {
                field: "records.identity".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(identity_id.into()),
            }],
            time_range_clauses: Vec::new(),
            group_by: Vec::new(),
            having: Vec::new(),
            order_by_clauses: vec![],
            limit: 100,
            offset: None,
            start: None,
        };

        let owned_dpns_names = Document::fetch_many(sdk, dpns_names_document_query)
            .await
            .map(|document_map| {
                document_map
                    .values()
                    .filter_map(|maybe_doc| {
                        maybe_doc.as_ref().and_then(|doc| {
                            let name = doc.get("label").map(|l| l.to_str().unwrap_or_default());
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
            .map_err(TaskError::from)?;

        let wallets = self.wallet_context().wallets();

        // Try to derive keys from wallets if requested
        let mut encrypted_private_keys = std::collections::BTreeMap::new();

        if let Some((_, _, wallet_private_keys)) = self
            .match_user_identity_keys_with_wallet(&identity, &wallets, selected_wallet_seed_hash)
            .await?
        {
            encrypted_private_keys.extend(wallet_private_keys);
        }

        let qualified_identity = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: encrypted_private_keys.into(),
            dpns_names: owned_dpns_names,
            associated_wallets: wallets
                .values()
                .map(|wallet| {
                    let w = wallet.read()?;
                    Ok::<_, TaskError>((w.seed_hash(), wallet.clone()))
                })
                .collect::<Result<_, _>>()?,
            secret_access: self.wallet_backend().ok().map(|b| b.secret_access()),
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: self.network,
        };
        let wallet_info = qualified_identity
            .determine_wallet_info()
            .map_err(|e| TaskError::WalletInfoDeterminationFailed { detail: e })?;

        // Insert qualified identity into the database
        self.insert_local_qualified_identity(&qualified_identity, &wallet_info)?;

        Ok(BackendTaskSuccessResult::LoadedIdentity(qualified_identity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::document::DocumentV0;
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;
    use dash_sdk::query_types::Documents;
    use std::collections::BTreeMap;

    /// A registered-name document for `label` whose record points at `identity`.
    fn domain_document(label: &str, identity: Value) -> Document {
        Document::V0(DocumentV0 {
            id: Identifier::from([0x5D; 32]),
            owner_id: Identifier::from([0x5E; 32]),
            creator_id: None,
            properties: BTreeMap::from([
                ("label".to_string(), Value::Text(label.to_string())),
                (
                    "records".to_string(),
                    Value::Map(vec![(Value::Text("identity".to_string()), identity)]),
                ),
            ]),
            revision: None,
            created_at: Some(100),
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
            contract_version: None,
        })
    }

    fn documents(found: Document) -> Documents {
        Documents::from_iter([(found.id(), Some(found))])
    }

    /// The name's record, not its document owner, says whose identity to load;
    /// the network may hand that record back as plain bytes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn loading_by_name_follows_the_identity_the_name_points_at() {
        let staged =
            crate::context::test_staging::stage_identity_with_vaulted_keys([0x11; 32], [0x22; 32])
                .await;
        let ctx = &staged.ctx;
        let target = Identifier::from([7; 32]);
        let name = domain_document("Alice", Value::Bytes32(target.to_buffer()));
        let mut sdk = crate::context::test_support::mock_sdk_for(ctx);
        sdk.mock()
            .expect_fetch_many::<Identifier, Document, _, Documents>(
                ctx.dpns_domain_query("a11ce"),
                Some(documents(name.clone())),
            )
            .await
            .expect("name lookup expectation");
        let identity =
            Identity::create_basic_identity(target, PlatformVersion::latest()).expect("identity");
        sdk.mock()
            .expect_fetch(target, Some(identity))
            .await
            .expect("identity expectation");
        sdk.mock()
            .expect_fetch_many::<Identifier, Document, _, Documents>(
                ctx.owned_dpns_names_query(target),
                Some(documents(name)),
            )
            .await
            .expect("owned names expectation");

        let loaded = ctx
            .load_identity_by_dpns_name(&sdk, " Alice.dash ".to_string(), None)
            .await
            .expect("the identity loads");

        let BackendTaskSuccessResult::LoadedIdentity(loaded) = loaded else {
            panic!("expected a loaded identity, got {loaded:?}");
        };
        assert_eq!(loaded.identity.id(), target);
        let names: Vec<&str> = loaded.dpns_names.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["Alice"]);
    }
}
