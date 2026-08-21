use crate::backend_task::error::TaskError;
use crate::backend_task::{
    BackendTaskSuccessResult, FeeResult, NETWORK_REQUEST_TIMEOUT,
    await_network_request_with_timeout,
};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::request_type::RequestType;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::document_type::DocumentType;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::document::{DocumentV0Getters, DocumentV0Setters};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::tokens::token_payment_info::TokenPaymentInfo;
use dash_sdk::drive::query::SelectProjection;
use dash_sdk::platform::documents::transitions::DocumentCreateResult;
use dash_sdk::platform::documents::transitions::DocumentCreateTransitionBuilder;
use dash_sdk::platform::documents::transitions::DocumentDeleteResult;
use dash_sdk::platform::documents::transitions::DocumentDeleteTransitionBuilder;
use dash_sdk::platform::documents::transitions::DocumentPurchaseResult;
use dash_sdk::platform::documents::transitions::DocumentPurchaseTransitionBuilder;
use dash_sdk::platform::documents::transitions::DocumentReplaceResult;
use dash_sdk::platform::documents::transitions::DocumentReplaceTransitionBuilder;
use dash_sdk::platform::documents::transitions::DocumentSetPriceResult;
use dash_sdk::platform::documents::transitions::DocumentSetPriceTransitionBuilder;
use dash_sdk::platform::documents::transitions::DocumentTransferResult;
use dash_sdk::platform::documents::transitions::DocumentTransferTransitionBuilder;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{
    DataContract, Document, DocumentQuery, Fetch, FetchMany, Identifier, IdentityPublicKey,
};
use dash_sdk::query_types::IndexMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum DocumentTask {
    BroadcastDocument {
        document: Document,
        token_payment_info: Option<TokenPaymentInfo>,
        entropy: [u8; 32],
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
    },
    DeleteDocument {
        document_id: Identifier,
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
    },
    ReplaceDocument {
        document: Document,
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
    },
    TransferDocument {
        document_id: Identifier,
        new_owner_id: Identifier,
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
    },
    PurchaseDocument {
        price: Credits,
        document_id: Identifier,
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
    },
    SetDocumentPrice {
        price: Credits,
        document_id: Identifier,
        document_type: DocumentType,
        data_contract: Arc<DataContract>,
        qualified_identity: QualifiedIdentity,
        identity_key: IdentityPublicKey,
        token_payment_info: Option<TokenPaymentInfo>,
    },
    FetchDocuments(DocumentQuery),
    FetchDocumentsPage(DocumentQuery),
}

impl AppContext {
    /// Fetches a single document by id and bumps its revision, preparing it for
    /// a replace/transfer/purchase/set-price mutation.
    async fn fetch_document_for_mutation(
        &self,
        sdk: &Sdk,
        data_contract: Arc<DataContract>,
        document_type: &DocumentType,
        document_id: Identifier,
    ) -> Result<Document, TaskError> {
        let document_query = DocumentQuery {
            select: SelectProjection::documents(),
            data_contract,
            document_type_name: document_type.name().to_string(),
            where_clauses: vec![],
            group_by: Vec::new(),
            having: Vec::new(),
            order_by_clauses: vec![],
            limit: 1,
            offset: None,
            start: None,
        };
        let query_with_id = DocumentQuery::with_document_id(document_query, &document_id);
        let mut document = await_network_request_with_timeout(
            NETWORK_REQUEST_TIMEOUT,
            Document::fetch(sdk, query_with_id),
            |source| TaskError::DocumentFetchTimeout { source },
        )
        .await?
        .map_err(TaskError::from)?
        .ok_or(TaskError::DocumentNotFound)?;
        document.bump_revision();
        Ok(document)
    }

    pub async fn run_document_task(
        &self,
        task: DocumentTask,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        match task {
            DocumentTask::FetchDocuments(document_query) => await_network_request_with_timeout(
                NETWORK_REQUEST_TIMEOUT,
                Document::fetch_many(sdk, document_query),
                |source| TaskError::DocumentFetchTimeout { source },
            )
            .await?
            .map(BackendTaskSuccessResult::Documents)
            .map_err(TaskError::from),
            DocumentTask::FetchDocumentsPage(mut document_query) => {
                // Set the limit for each page
                document_query.limit = 100;

                // Initialize an empty IndexMap to accumulate documents for this page
                let mut page_docs: IndexMap<Identifier, Option<Document>> = IndexMap::new();

                // Fetch a single page
                let docs_batch_result = await_network_request_with_timeout(
                    NETWORK_REQUEST_TIMEOUT,
                    Document::fetch_many(sdk, document_query),
                    |source| TaskError::DocumentFetchTimeout { source },
                )
                .await?
                .map_err(TaskError::from)?;

                let batch_len = docs_batch_result.len();

                // Insert the batch into the page map
                for (id, doc_opt) in docs_batch_result {
                    page_docs.insert(id, doc_opt);
                }

                // Determine if there's a next page
                let has_next_page = batch_len == 100;

                // If there's a next page, set the 'start' parameter for the next cursor
                let next_cursor = if has_next_page {
                    page_docs.keys().last().cloned().map(|last_doc_id| {
                        let id_bytes = last_doc_id.to_buffer();
                        Start::StartAfter(id_bytes.to_vec())
                    })
                } else {
                    None
                };

                Ok(BackendTaskSuccessResult::PageDocuments(
                    page_docs,
                    next_cursor,
                ))
            }
            DocumentTask::BroadcastDocument {
                document,
                token_payment_info,
                entropy,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
            } => {
                let mut builder = DocumentCreateTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document,
                    entropy,
                );

                if let Some(token_payment) = token_payment_info {
                    builder = builder.with_token_payment_info(token_payment);
                }

                let maybe_options = self.state_transition_options();
                if let Some(options) = maybe_options {
                    builder = builder.with_state_transition_creation_options(options);
                }

                let result = sdk
                    .document_create(builder, &identity_key, &qualified_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })?;

                // Handle the result - DocumentCreateResult contains the created document
                match result {
                    DocumentCreateResult::Document(document) => {
                        Ok(BackendTaskSuccessResult::BroadcastedDocument(document))
                    }
                }
            }
            DocumentTask::DeleteDocument {
                document_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            } => {
                let mut builder = DocumentDeleteTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document_id,
                    qualified_identity.identity.id(),
                );

                if let Some(token_payment) = token_payment_info {
                    builder = builder.with_token_payment_info(token_payment);
                }

                let maybe_options = self.state_transition_options();
                if let Some(options) = maybe_options {
                    builder = builder.with_state_transition_creation_options(options);
                }

                let result = sdk
                    .document_delete(builder, &identity_key, &qualified_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })?;

                // Handle the result - DocumentDeleteResult contains the deleted document ID
                let estimated_fee = self.fee_estimator().estimate_document_batch(1);
                let fee_result = FeeResult::estimated_only(estimated_fee);
                match result {
                    DocumentDeleteResult::Deleted(deleted_id) => Ok(
                        BackendTaskSuccessResult::DeletedDocument(deleted_id, fee_result),
                    ),
                }
            }
            DocumentTask::ReplaceDocument {
                document,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            } => {
                let mut builder = DocumentReplaceTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document,
                );

                if let Some(token_payment) = token_payment_info {
                    builder = builder.with_token_payment_info(token_payment);
                }

                let maybe_options = self.state_transition_options();
                if let Some(options) = maybe_options {
                    builder = builder.with_state_transition_creation_options(options);
                }

                let result = sdk
                    .document_replace(builder, &identity_key, &qualified_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })?;

                // Handle the result - DocumentReplaceResult contains the replaced document
                let estimated_fee = self.fee_estimator().estimate_document_batch(1);
                let fee_result = FeeResult::estimated_only(estimated_fee);
                match result {
                    DocumentReplaceResult::Document(document) => Ok(
                        BackendTaskSuccessResult::ReplacedDocument(document.id(), fee_result),
                    ),
                }
            }
            DocumentTask::TransferDocument {
                document_id,
                new_owner_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            } => {
                let document = self
                    .fetch_document_for_mutation(
                        sdk,
                        data_contract.clone(),
                        &document_type,
                        document_id,
                    )
                    .await?;

                let mut builder = DocumentTransferTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document,
                    new_owner_id,
                );

                if let Some(token_payment) = token_payment_info {
                    builder = builder.with_token_payment_info(token_payment);
                }

                let maybe_options = self.state_transition_options();
                if let Some(options) = maybe_options {
                    builder = builder.with_state_transition_creation_options(options);
                }

                let result = sdk
                    .document_transfer(builder, &identity_key, &qualified_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })?;

                // Handle the result - DocumentTransferResult contains the transferred document
                let estimated_fee = self.fee_estimator().estimate_document_batch(1);
                let fee_result = FeeResult::estimated_only(estimated_fee);
                match result {
                    DocumentTransferResult::Document(document) => Ok(
                        BackendTaskSuccessResult::TransferredDocument(document.id(), fee_result),
                    ),
                }
            }
            DocumentTask::PurchaseDocument {
                price,
                document_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            } => {
                let document = self
                    .fetch_document_for_mutation(
                        sdk,
                        data_contract.clone(),
                        &document_type,
                        document_id,
                    )
                    .await?;

                let mut builder = DocumentPurchaseTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document,
                    qualified_identity.identity.id(),
                    price,
                );

                if let Some(token_payment) = token_payment_info {
                    builder = builder.with_token_payment_info(token_payment);
                }

                let maybe_options = self.state_transition_options();
                if let Some(options) = maybe_options {
                    builder = builder.with_state_transition_creation_options(options);
                }

                let result = sdk
                    .document_purchase(builder, &identity_key, &qualified_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })?;

                // Handle the result - DocumentPurchaseResult contains the purchased document
                let estimated_fee = self.fee_estimator().estimate_document_batch(1);
                let fee_result = FeeResult::estimated_only(estimated_fee);
                match result {
                    DocumentPurchaseResult::Document(document) => Ok(
                        BackendTaskSuccessResult::PurchasedDocument(document.id(), fee_result),
                    ),
                }
            }
            DocumentTask::SetDocumentPrice {
                price,
                document_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            } => {
                let document = self
                    .fetch_document_for_mutation(
                        sdk,
                        data_contract.clone(),
                        &document_type,
                        document_id,
                    )
                    .await?;

                let mut builder = DocumentSetPriceTransitionBuilder::new(
                    data_contract,
                    document_type.name().to_string(),
                    document,
                    price,
                );

                if let Some(token_payment) = token_payment_info {
                    builder = builder.with_token_payment_info(token_payment);
                }

                let maybe_options = self.state_transition_options();
                if let Some(options) = maybe_options {
                    builder = builder.with_state_transition_creation_options(options);
                }

                let result = sdk
                    .document_set_price(builder, &identity_key, &qualified_identity)
                    .await
                    .map_err(|e| {
                        self.log_drive_proof_error(e, RequestType::BroadcastStateTransition)
                    })?;

                // Handle the result - DocumentSetPriceResult contains the document with updated price
                let estimated_fee = self.fee_estimator().estimate_document_batch(1);
                let fee_result = FeeResult::estimated_only(estimated_fee);
                match result {
                    DocumentSetPriceResult::Document(document) => Ok(
                        BackendTaskSuccessResult::SetDocumentPrice(document.id(), fee_result),
                    ),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn document_network_timeout_is_typed_and_actionable() {
        let error = crate::backend_task::await_network_request_with_timeout(
            std::time::Duration::from_millis(1),
            std::future::pending::<()>(),
            |source| TaskError::DocumentFetchTimeout { source },
        )
        .await
        .expect_err("a pending document request must time out");

        assert!(matches!(error, TaskError::DocumentFetchTimeout { .. }));
        assert!(error.to_string().contains("Check your connection"));
    }
}
