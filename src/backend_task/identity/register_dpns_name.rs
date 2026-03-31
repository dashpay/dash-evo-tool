use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::model::fee_estimation::PlatformFeeEstimator;
use crate::{context::AppContext, model::qualified_identity::DPNSNameInfo};
use dash_sdk::{
    Sdk,
    dpp::{
        data_contract::accessors::v0::DataContractV0Getters,
        document::DocumentV0Getters,
        identity::accessors::IdentityGettersV0,
        platform_value::Value,
    },
    drive::query::{WhereClause, WhereOperator},
    platform::Fetch,
    platform::{Document, DocumentQuery, FetchMany},
};

use super::{BackendTaskSuccessResult, RegisterDpnsNameInput};
impl AppContext {
    pub(super) async fn register_dpns_name(
        &self,
        _sdk: &Sdk,
        input: RegisterDpnsNameInput,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let sdk = self.sdk.load().as_ref().clone();
        let mut qualified_identity = input.qualified_identity;

        let public_key = qualified_identity
            .document_signing_key(
                &self
                    .dpns_contract
                    .document_type_for_name("preorder")
                    .map_err(|_| TaskError::DataContractNotFound)?,
            )
            .ok_or(TaskError::NoDocumentSigningKey)?;

        let fee_estimator = PlatformFeeEstimator::new();
        let estimated_fee = fee_estimator.estimate_document_batch(2);

        let balance_before = qualified_identity.identity.balance();

        // Use platform-wallet's register_name_with_signer which handles
        // preorder + domain document creation and broadcasting internally.
        let platform_wallet = self.platform_wallet_for_identity(&qualified_identity)?;
        let identity_wallet = platform_wallet.identity();

        let _full_domain_name = identity_wallet
            .register_name_with_signer(
                qualified_identity.identity.clone(),
                &input.name_input,
                public_key.clone(),
                qualified_identity.clone(),
            )
            .await?;

        // Fetch owned DPNS names to update the local qualified identity.
        let dpns_names_document_query = DocumentQuery {
            data_contract: self.dpns_contract.clone(),
            document_type_name: "domain".to_string(),
            where_clauses: vec![WhereClause {
                field: "records.identity".to_string(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(qualified_identity.identity.id().into()),
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

        if qualified_identity.alias.is_none() {
            qualified_identity.alias = Some(format!("{}.dash", input.name_input));
        }

        let refreshed_identity = dash_sdk::platform::Identity::fetch_by_identifier(
            &sdk,
            qualified_identity.identity.id(),
        )
        .await?
        .ok_or(TaskError::IdentityNotFound)?;

        let balance_after = refreshed_identity.balance();
        let actual_fee = balance_before.saturating_sub(balance_after);

        tracing::info!(
            "DPNS registration complete: estimated fee {} credits, actual fee {} credits",
            estimated_fee,
            actual_fee
        );
        if actual_fee != estimated_fee {
            tracing::warn!(
                "Fee mismatch: estimated {} vs actual {} (diff: {})",
                estimated_fee,
                actual_fee,
                actual_fee as i64 - estimated_fee as i64
            );
        }

        qualified_identity.identity = refreshed_identity;

        self.update_local_qualified_identity(&qualified_identity)
            .map_err(|e| TaskError::Database { source: e })?;

        let fee_result = FeeResult::new(estimated_fee, actual_fee);
        Ok(BackendTaskSuccessResult::RegisteredDpnsName(fee_result))
    }
}
