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

        for qualified_identity in qualified_identities {
            let names = self
                .fetch_owned_dpns_names(&sdk, qualified_identity.identity.id())
                .await?;
            self.finish_owned_names_refresh(qualified_identity, names)?;
        }

        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|_| TaskError::InternalSendError)?;

        Ok(BackendTaskSuccessResult::RefreshedOwnedDpnsNames)
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
        let query = DocumentQuery {
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
        let documents =
            Document::fetch_many(sdk, query)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bare_identity(byte: u8) -> crate::model::qualified_identity::QualifiedIdentity {
        use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
        use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
        QualifiedIdentity {
            identity: dash_sdk::platform::Identity::create_basic_identity(
                [byte; 32].into(),
                dash_sdk::dpp::version::PlatformVersion::latest(),
            )
            .expect("identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: Vec::new(),
            associated_wallets: Default::default(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        }
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
}
