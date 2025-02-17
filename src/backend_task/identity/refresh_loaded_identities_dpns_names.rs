use super::BackendTaskSuccessResult;
use crate::app::TaskResult;
use crate::context::AppContext;
use crate::model::qualified_identity::DPNSNameInfo;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany};
use tokio::sync::mpsc;

impl AppContext {
    pub(super) async fn refresh_loaded_identities_dpns_names(
        &self,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let qualified_identities = self
            .load_local_qualified_identities()
            .map_err(|e| format!("Error refreshing owned DPNS names: Database error: {}", e))?;

        for mut qualified_identity in qualified_identities {
            let identity_id = qualified_identity.identity.id();

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

            let owned_dpns_names = Document::fetch_many(&sdk_guard, dpns_names_document_query)
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
                .map_err(|e| format!("Error refreshing owned DPNS names: {}", e))?;

            qualified_identity.dpns_names = owned_dpns_names;

            // Update qualified identity in the database
            self.update_local_qualified_identity(&qualified_identity)
                .map_err(|e| format!("Error refreshing owned DPNS names: Database error: {}", e))?;
        }

        sender.send(TaskResult::Refresh).await.map_err(|e| {
            format!(
                "Error refreshing owned DPNS names. Sender failed to send TaskResult: {}",
                e.to_string()
            )
        })?;

        Ok(BackendTaskSuccessResult::Message(
            "Successfully refreshed loaded identities dpns names".to_string(),
        ))
    }
}
