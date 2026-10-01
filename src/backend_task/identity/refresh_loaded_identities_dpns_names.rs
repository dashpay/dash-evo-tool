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

        for mut qualified_identity in qualified_identities {
            qualified_identity.dpns_names = self
                .fetch_owned_dpns_names(&sdk, qualified_identity.identity.id())
                .await?;
            let main_username = self.main_username(&qualified_identity);
            qualified_identity.initialize_node_alias(None, main_username.as_deref());
            self.update_local_qualified_identity(&qualified_identity)?;
        }

        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|_| TaskError::InternalSendError)?;

        Ok(BackendTaskSuccessResult::RefreshedOwnedDpnsNames)
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
