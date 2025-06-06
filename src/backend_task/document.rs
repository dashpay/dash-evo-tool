use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::document_type::DocumentType;
use dash_sdk::dpp::document::{DocumentV0Getters, DocumentV0Setters};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::tokens::token_payment_info::TokenPaymentInfo;
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
use dash_sdk::{Error, Sdk};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DocumentTask {
    BroadcastDocument(
        Document,
        Option<TokenPaymentInfo>,
        [u8; 32],
        DocumentType,
        Arc<DataContract>,
        QualifiedIdentity,
        IdentityPublicKey,
    ),
    DeleteDocument(
        Identifier, // Document ID
        DocumentType,
        Arc<DataContract>,
        QualifiedIdentity,
        IdentityPublicKey,
        Option<TokenPaymentInfo>,
    ),
    ReplaceDocument(
        Document,
        DocumentType,
        Arc<DataContract>,
        QualifiedIdentity,
        IdentityPublicKey,
        Option<TokenPaymentInfo>,
    ),
    TransferDocument(
        Identifier, // Document ID
        Identifier, // New owner ID
        DocumentType,
        Arc<DataContract>,
        QualifiedIdentity,
        IdentityPublicKey,
        Option<TokenPaymentInfo>,
    ),
    PurchaseDocument(
        Credits,    // Price in credits
        Identifier, // Document ID
        DocumentType,
        Arc<DataContract>,
        QualifiedIdentity,
        IdentityPublicKey,
        Option<TokenPaymentInfo>,
    ),
    SetDocumentPrice(
        Credits,    // Price in credits
        Identifier, // Document ID
        DocumentType,
        Arc<DataContract>,
        QualifiedIdentity,
        IdentityPublicKey,
        Option<TokenPaymentInfo>,
    ),
    FetchDocuments(DocumentQuery),
    FetchDocumentsPage(DocumentQuery),
}

impl AppContext {
    pub async fn run_document_task(
        &self,
        task: DocumentTask,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            DocumentTask::FetchDocuments(document_query) => {
                Document::fetch_many(sdk, document_query)
                    .await
                    .map(BackendTaskSuccessResult::Documents)
                    .map_err(|e| format!("Error fetching documents: {}", e))
            }
            DocumentTask::FetchDocumentsPage(mut document_query) => {
                // Set the limit for each page
                document_query.limit = 100;

                // Initialize an empty IndexMap to accumulate documents for this page
                let mut page_docs: IndexMap<Identifier, Option<Document>> = IndexMap::new();

                // Fetch a single page
                let docs_batch_result = Document::fetch_many(sdk, document_query)
                    .await
                    .map_err(|e| format!("Error fetching documents: {}", e))?;

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
            DocumentTask::BroadcastDocument(
                document,
                token_payment_info,
                entropy,
                doc_type,
                data_contract,
                qualified_identity,
                identity_key,
            ) => {
                let mut builder = DocumentCreateTransitionBuilder::new(
                    data_contract,
                    doc_type.name().to_string(),
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
                    .map_err(|e| match e {
                        Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                            self.db
                                .insert_proof_log_item(ProofLogItem {
                                    request_type: RequestType::BroadcastStateTransition,
                                    request_bytes: vec![],
                                    verification_path_query_bytes: vec![],
                                    height: block_info.height,
                                    time_ms: block_info.time_ms,
                                    proof_bytes,
                                    error: Some(proof_error.to_string()),
                                })
                                .ok();
                            format!(
                                "Error broadcasting document: {}, proof error logged",
                                proof_error
                            )
                        }
                        e => format!("Error broadcasting document: {}", e),
                    })?;

                // Handle the result - DocumentCreateResult contains the created document
                match result {
                    DocumentCreateResult::Document(document) => {
                        Ok(BackendTaskSuccessResult::BroadcastedDocument(document))
                    }
                }
            }
            DocumentTask::DeleteDocument(
                document_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            ) => {
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
                    .map_err(|e| match e {
                        Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                            self.db
                                .insert_proof_log_item(ProofLogItem {
                                    request_type: RequestType::BroadcastStateTransition,
                                    request_bytes: vec![],
                                    verification_path_query_bytes: vec![],
                                    height: block_info.height,
                                    time_ms: block_info.time_ms,
                                    proof_bytes,
                                    error: Some(proof_error.to_string()),
                                })
                                .ok();
                            format!(
                                "Error deleting document: {}, proof error logged",
                                proof_error
                            )
                        }
                        e => format!("Error deleting document: {}", e),
                    })?;

                // Handle the result - DocumentDeleteResult contains the deleted document ID
                match result {
                    DocumentDeleteResult::Deleted(deleted_id) => {
                        Ok(BackendTaskSuccessResult::Message(format!(
                            "Document {} deleted successfully",
                            deleted_id
                        )))
                    }
                }
            }
            DocumentTask::ReplaceDocument(
                document,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            ) => {
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
                    .map_err(|e| match e {
                        Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                            self.db
                                .insert_proof_log_item(ProofLogItem {
                                    request_type: RequestType::BroadcastStateTransition,
                                    request_bytes: vec![],
                                    verification_path_query_bytes: vec![],
                                    height: block_info.height,
                                    time_ms: block_info.time_ms,
                                    proof_bytes,
                                    error: Some(proof_error.to_string()),
                                })
                                .ok();
                            format!(
                                "Error replacing document: {}, proof error logged",
                                proof_error
                            )
                        }
                        e => format!("Error replacing document: {}", e),
                    })?;

                // Handle the result - DocumentReplaceResult contains the replaced document
                match result {
                    DocumentReplaceResult::Document(document) => {
                        Ok(BackendTaskSuccessResult::Message(format!(
                            "Document {} replaced successfully",
                            document.id()
                        )))
                    }
                }
            }
            DocumentTask::TransferDocument(
                document_id,
                new_owner_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            ) => {
                // First fetch the document to transfer
                let document_query = DocumentQuery {
                    data_contract: data_contract.clone().into(),
                    document_type_name: document_type.name().to_string(),
                    where_clauses: vec![],
                    order_by_clauses: vec![],
                    limit: 1,
                    start: None,
                };
                let query_with_id = DocumentQuery::with_document_id(document_query, &document_id);
                let mut document = Document::fetch(sdk, query_with_id)
                    .await
                    .map_err(|e| format!("Error fetching document: {}", e))?
                    .ok_or_else(|| "Document not found".to_string())?;
                document.bump_revision();

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
                    .map_err(|e| match e {
                        Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                            self.db
                                .insert_proof_log_item(ProofLogItem {
                                    request_type: RequestType::BroadcastStateTransition,
                                    request_bytes: vec![],
                                    verification_path_query_bytes: vec![],
                                    height: block_info.height,
                                    time_ms: block_info.time_ms,
                                    proof_bytes,
                                    error: Some(proof_error.to_string()),
                                })
                                .ok();
                            format!(
                                "Error transferring document: {}, proof error logged",
                                proof_error
                            )
                        }
                        e => format!("Error transferring document: {}", e),
                    })?;

                // Handle the result - DocumentTransferResult contains the transferred document
                match result {
                    DocumentTransferResult::Document(document) => {
                        Ok(BackendTaskSuccessResult::Message(format!(
                            "Document {} transferred to {} successfully",
                            document.id(),
                            new_owner_id
                        )))
                    }
                }
            }
            DocumentTask::PurchaseDocument(
                price,
                document_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            ) => {
                // First fetch the document to purchase
                let document_query = DocumentQuery {
                    data_contract: data_contract.clone().into(),
                    document_type_name: document_type.name().to_string(),
                    where_clauses: vec![],
                    order_by_clauses: vec![],
                    limit: 1,
                    start: None,
                };
                let query_with_id = DocumentQuery::with_document_id(document_query, &document_id);
                let mut document = Document::fetch(sdk, query_with_id)
                    .await
                    .map_err(|e| format!("Error fetching document: {}", e))?
                    .ok_or_else(|| "Document not found".to_string())?;
                document.bump_revision();

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
                    .map_err(|e| match e {
                        Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                            self.db
                                .insert_proof_log_item(ProofLogItem {
                                    request_type: RequestType::BroadcastStateTransition,
                                    request_bytes: vec![],
                                    verification_path_query_bytes: vec![],
                                    height: block_info.height,
                                    time_ms: block_info.time_ms,
                                    proof_bytes,
                                    error: Some(proof_error.to_string()),
                                })
                                .ok();
                            format!(
                                "Error purchasing document: {}, proof error logged",
                                proof_error
                            )
                        }
                        e => format!("Error purchasing document: {}", e),
                    })?;

                // Handle the result - DocumentPurchaseResult contains the purchased document
                match result {
                    DocumentPurchaseResult::Document(document) => {
                        Ok(BackendTaskSuccessResult::Message(format!(
                            "Document {} purchased for {} credits",
                            document.id(),
                            price
                        )))
                    }
                }
            }
            DocumentTask::SetDocumentPrice(
                price,
                document_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            ) => {
                // First fetch the document to set price on
                let document_query = DocumentQuery {
                    data_contract: data_contract.clone().into(),
                    document_type_name: document_type.name().to_string(),
                    where_clauses: vec![],
                    order_by_clauses: vec![],
                    limit: 1,
                    start: None,
                };
                let query_with_id = DocumentQuery::with_document_id(document_query, &document_id);
                let mut document = Document::fetch(sdk, query_with_id)
                    .await
                    .map_err(|e| format!("Error fetching document: {}", e))?
                    .ok_or_else(|| "Document not found".to_string())?;
                document.bump_revision();

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
                    .map_err(|e| match e {
                        Error::DriveProofError(proof_error, proof_bytes, block_info) => {
                            self.db
                                .insert_proof_log_item(ProofLogItem {
                                    request_type: RequestType::BroadcastStateTransition,
                                    request_bytes: vec![],
                                    verification_path_query_bytes: vec![],
                                    height: block_info.height,
                                    time_ms: block_info.time_ms,
                                    proof_bytes,
                                    error: Some(proof_error.to_string()),
                                })
                                .ok();
                            format!(
                                "Error setting document price: {}, proof error logged",
                                proof_error
                            )
                        }
                        e => format!("Error setting document price: {}", e),
                    })?;

                // Handle the result - DocumentSetPriceResult contains the document with updated price
                match result {
                    DocumentSetPriceResult::Document(document) => {
                        Ok(BackendTaskSuccessResult::Message(format!(
                            "Document {} price set to {} credits",
                            document.id(),
                            price
                        )))
                    }
                }
            }
        }
    }
}
