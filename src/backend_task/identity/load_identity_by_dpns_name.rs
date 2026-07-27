use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::{
    DPNSNameInfo, IdentityStatus, IdentityType, QualifiedIdentity,
};
use crate::model::wallet::WalletSeedHash;
use dash_sdk::Sdk;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
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
        let normalized_name = crate::model::dpns::normalize_dpns_label(&dpns_name);

        // Query the DPNS contract for the domain document
        let domain_query = DocumentQuery {
            select: SelectProjection::documents(),
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
            group_by: Vec::new(),
            having: Vec::new(),
            order_by_clauses: vec![],
            limit: 1,
            start: None,
        };

        let documents = Document::fetch_many(sdk, domain_query)
            .await
            .map_err(TaskError::from)?;

        // Get the first (and should be only) document
        let domain_doc = documents
            .values()
            .filter_map(|maybe_doc| maybe_doc.as_ref())
            .next()
            .ok_or(TaskError::IdentityNotFound)?;

        // Extract the identity ID from the records.identity field
        let identity_id = crate::model::dpns::extract_identity_id_from_dpns_document(domain_doc)
            .ok_or(TaskError::IdentityNotFound)?;

        // Fetch the identity
        let identity = match Identity::fetch_by_identifier(sdk, identity_id).await {
            Ok(Some(identity)) => identity,
            Ok(None) => return Err(TaskError::IdentityNotFound),
            Err(e) => return Err(TaskError::from(e)),
        };
        let load_guard =
            self.begin_identity_load_and_validate_type(IdentityType::User, &identity, None)?;

        // Get the label from the document for display
        let label = domain_doc
            .get("label")
            .and_then(|l| l.to_str().ok())
            .unwrap_or(&dpns_name)
            .to_string();

        // Fetch all DPNS names owned by this identity
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

        let wallets = self.wallets.read().map_err(TaskError::from)?.clone();

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
            alias: Some(format!("{}.dash", label)),
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
        self.persist_identity_loaded_by_dpns_name(qualified_identity, load_guard)
    }

    fn persist_identity_loaded_by_dpns_name(
        &self,
        qualified_identity: QualifiedIdentity,
        load_guard: crate::context::identity_load_registry::IdentityLoadGuard,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let identity_id = qualified_identity.identity.id();
        let wallet_info = qualified_identity
            .determine_wallet_info()
            .map_err(|e| TaskError::WalletInfoDeterminationFailed { detail: e })?;

        // Insert qualified identity into the database
        self.prepare_stuck_unload_cleanup_for_reload(&identity_id.to_buffer())?;
        self.insert_local_qualified_identity(&qualified_identity, &wallet_info)?;
        self.finish_identity_load_after_persist(&identity_id, load_guard);

        Ok(BackendTaskSuccessResult::LoadedIdentity(qualified_identity))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::test_support::test_app_context;
    use crate::model::qualified_identity::PrivateKeyTarget;
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::wallet_backend::IdentityKeyView;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dpns_load_clears_repaired_unload_ghost_key() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = test_app_context(temp_dir.path());
        let (tx, _rx) = tokio::sync::mpsc::channel::<crate::app::TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        let backend = ctx.wallet_backend().expect("wallet backend");

        let platform_version = PlatformVersion::latest();
        let identity_id = Identifier::from([0xD1; 32]);
        let identity =
            Identity::create_basic_identity(identity_id, platform_version).expect("identity");
        let old_key_id = ctx.install_repaired_unload_ghost_for_test(identity.clone(), [0xD2; 32]);
        let view = IdentityKeyView::new(backend.secret_store(), identity_id.to_buffer());
        assert!(
            view.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, old_key_id,)
                .expect("read old vault key before reload")
                .is_some(),
            "precondition: the interrupted unload retains its old vault key",
        );

        let load_guard = ctx
            .begin_identity_load_and_validate_type(IdentityType::User, &identity, None)
            .expect("claim replacement load");
        let qualified_identity = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: Some("alice.dash".to_string()),
            private_keys: Default::default(),
            dpns_names: Vec::new(),
            associated_wallets: Default::default(),
            secret_access: Some(backend.secret_access()),
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: ctx.network(),
        };
        let result = ctx.persist_identity_loaded_by_dpns_name(qualified_identity, load_guard);
        assert!(
            matches!(
                result,
                Ok(BackendTaskSuccessResult::LoadedIdentity(ref loaded))
                    if loaded.identity.id() == identity_id
            ),
            "the repaired ghost must reload by DPNS name: {result:?}",
        );
        assert!(
            view.get(&PrivateKeyTarget::PrivateKeyOnMainIdentity, old_key_id,)
                .expect("read old vault key after reload")
                .is_none(),
            "the old vault key must be cleared before the DPNS load writes its blob",
        );
        assert!(
            !ctx.is_identity_forgotten(&identity_id)
                .expect("read marker after reload"),
            "a successful DPNS load must retire the forgotten marker",
        );

        backend.shutdown().await;
    }
}
