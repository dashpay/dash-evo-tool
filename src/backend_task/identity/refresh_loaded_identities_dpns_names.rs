use super::BackendTaskSuccessResult;
use crate::app::TaskResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::qualified_identity::DPNSNameInfo;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::{WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany};

impl AppContext {
    pub(super) async fn refresh_loaded_identities_dpns_names(
        &self,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let qualified_identities = self.load_local_qualified_identities()?;

        let sdk = self.sdk.load().as_ref().clone();

        for mut qualified_identity in qualified_identities {
            let identity_id = qualified_identity.identity.id();

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

            let owned_dpns_names = Document::fetch_many(&sdk, dpns_names_document_query)
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

            qualified_identity.dpns_names = owned_dpns_names;

            if qualified_identity.alias.is_none() && !qualified_identity.dpns_names.is_empty() {
                let dpns_name = &qualified_identity.dpns_names[0].name;
                qualified_identity.alias = Some(format!("{}.dash", dpns_name));
            }

            self.update_local_qualified_identity(&qualified_identity)
                .map_err(|e| TaskError::Database { source: e })?;
        }

        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|_| TaskError::InternalSendError)?;

        Ok(BackendTaskSuccessResult::RefreshedOwnedDpnsNames)
    }
}
