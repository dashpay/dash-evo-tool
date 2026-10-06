use super::BackendTaskSuccessResult;
use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::DPNSNameInfo;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{SelectProjection, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};

impl AppContext {
    pub(super) async fn refresh_loaded_identities_dpns_names(
        &self,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let qualified_identities = self.load_local_qualified_identities()?;

        let sdk = self.sdk.load().as_ref().clone();

        let refreshed = self
            .refresh_owned_names_of(&sdk, qualified_identities)
            .await;

        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|_| TaskError::InternalSendError)?;

        refreshed.map(|()| BackendTaskSuccessResult::RefreshedOwnedDpnsNames)
    }

    /// Refresh the owned names of every identity in the `identities` snapshot.
    ///
    /// An identity removed since the snapshot is skipped. Any other failure is
    /// returned only after the remaining identities were refreshed.
    async fn refresh_owned_names_of(
        &self,
        sdk: &dash_sdk::Sdk,
        identities: Vec<crate::model::qualified_identity::QualifiedIdentity>,
    ) -> Result<(), TaskError> {
        let mut first_error = None;
        for identity in identities {
            let refreshed = match self
                .fetch_owned_dpns_names(sdk, identity.identity.id())
                .await
            {
                Ok(names) => self.finish_owned_names_refresh(identity, names),
                Err(error) => Err(error),
            };
            match refreshed {
                Ok(()) | Err(TaskError::IdentityNotFoundLocally) => {}
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn finish_owned_names_refresh(
        &self,
        identity: crate::model::qualified_identity::QualifiedIdentity,
        names: Vec<DPNSNameInfo>,
    ) -> Result<(), TaskError> {
        self.edit_local_qualified_identity(&identity.identity.id(), |fresh| {
            fresh.dpns_names = names;
            let main_username = self.main_username(fresh);
            fresh.initialize_node_alias(None, main_username.as_deref());
            Ok(())
        })?;
        Ok(())
    }

    /// Every DPNS name whose records point at `identity_id`.
    pub(crate) async fn fetch_owned_dpns_names(
        &self,
        sdk: &dash_sdk::Sdk,
        identity_id: Identifier,
    ) -> Result<Vec<DPNSNameInfo>, TaskError> {
        let documents = Document::fetch_many(sdk, self.owned_dpns_names_query(identity_id))
            .await
            .map_err(|e| TaskError::DpnsFetchError {
                source: Box::new(e),
            })?;
        Ok(documents
            .values()
            .flatten()
            .filter_map(|doc| {
                let name = doc.get("label")?.to_str().ok()?.to_owned();
                let acquired_at = doc
                    .created_at()
                    .into_iter()
                    .chain(doc.transferred_at())
                    .max()?;
                Some(DPNSNameInfo { name, acquired_at })
            })
            .collect())
    }

    /// The lookup for every DPNS name whose records point at `identity_id`.
    pub(super) fn owned_dpns_names_query(&self, identity_id: Identifier) -> DocumentQuery {
        DocumentQuery {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bare_identity(byte: u8) -> crate::model::qualified_identity::QualifiedIdentity {
        crate::context::test_support::bare_user_identity(
            [byte; 32].into(),
            dash_sdk::dpp::dashcore::Network::Testnet,
        )
    }

    #[test]
    fn followup_owned_names_refresh_preserves_concurrent_identity_edits() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let stale = bare_identity(8);
        let mut current = stale.clone();
        current.alias = Some("Updated during registration".into());
        let key = dash_sdk::platform::IdentityPublicKey::random_key(
            9,
            Some(8),
            dash_sdk::dpp::version::PlatformVersion::latest(),
        );
        current.private_keys.insert_at((crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnMainIdentity, 9), (
            crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey::from(key.clone()),
            crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData::InVault,
        ));
        ctx.update_local_qualified_identity(&current).unwrap();
        ctx.finish_owned_names_refresh(
            stale,
            vec![DPNSNameInfo {
                name: "alice-123".into(),
                acquired_at: 1_000,
            }],
        )
        .unwrap();
        let saved = ctx
            .get_local_qualified_identity(&current.identity.id())
            .unwrap()
            .unwrap();
        assert_eq!(saved.alias, current.alias);
        assert!(saved.private_keys.first_live_candidate(&key).is_some());
        assert!(saved.dpns_names.iter().any(|name| name.name == "alice-123"));
    }

    /// Mock the owned-names lookup of `owner` to return one name.
    async fn expect_owned_name(sdk: &mut dash_sdk::Sdk, ctx: &AppContext, owner: Identifier) {
        let document = Document::V0(dash_sdk::dpp::document::DocumentV0 {
            id: owner,
            owner_id: owner,
            creator_id: None,
            properties: std::collections::BTreeMap::from([(
                "label".to_string(),
                Value::Text("owned-name-2".to_string()),
            )]),
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
        });
        let documents: dash_sdk::query_types::Documents =
            [(document.id(), Some(document))].into_iter().collect();
        sdk.mock()
            .expect_fetch_many::<Identifier, Document, _, dash_sdk::query_types::Documents>(
                ctx.owned_dpns_names_query(owner),
                Some(documents),
            )
            .await
            .expect("owned names expectation");
    }

    fn owned_names(ctx: &AppContext, id: &Identifier) -> Vec<String> {
        ctx.get_local_qualified_identity(id)
            .unwrap()
            .expect("identity is stored")
            .dpns_names
            .into_iter()
            .map(|name| name.name)
            .collect()
    }

    /// An identity removed while the refresh runs is skipped without an error,
    /// and the identities after it are still refreshed.
    #[tokio::test]
    async fn owned_names_refresh_skips_an_identity_removed_mid_run() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let (removed, kept) = (bare_identity(5), bare_identity(6));
        let (removed_id, kept_id) = (removed.identity.id(), kept.identity.id());
        for identity in [&removed, &kept] {
            ctx.update_local_qualified_identity(identity).unwrap();
        }
        let mut sdk = dash_sdk::Sdk::new_mock();
        expect_owned_name(&mut sdk, &ctx, removed_id).await;
        expect_owned_name(&mut sdk, &ctx, kept_id).await;
        // Removed after the snapshot below was taken, before its write lands.
        ctx.delete_local_qualified_identity_with_outcome(&removed_id)
            .unwrap();

        ctx.refresh_owned_names_of(&sdk, vec![removed, kept])
            .await
            .expect("a removed identity is not a refresh failure");

        assert_eq!(owned_names(&ctx, &kept_id), ["owned-name-2"]);
        assert!(
            ctx.get_local_qualified_identity(&removed_id)
                .unwrap()
                .is_none(),
            "the refresh must not bring a removed identity back"
        );
    }

    /// One identity whose names cannot be read does not stop the others from
    /// being refreshed; the failure is still reported.
    #[tokio::test]
    async fn owned_names_refresh_continues_past_a_failed_identity() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let (unreadable, readable) = (bare_identity(5), bare_identity(6));
        let readable_id = readable.identity.id();
        for identity in [&unreadable, &readable] {
            ctx.update_local_qualified_identity(identity).unwrap();
        }
        // Only the second identity's lookup is mocked; the first one fails.
        let mut sdk = dash_sdk::Sdk::new_mock();
        expect_owned_name(&mut sdk, &ctx, readable_id).await;

        let result = ctx
            .refresh_owned_names_of(&sdk, vec![unreadable, readable])
            .await;

        assert!(
            matches!(result, Err(TaskError::DpnsFetchError { .. })),
            "{result:?}"
        );
        assert_eq!(owned_names(&ctx, &readable_id), ["owned-name-2"]);
    }
}
