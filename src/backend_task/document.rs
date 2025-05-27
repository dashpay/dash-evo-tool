use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::document_type::DocumentType;
use dash_sdk::dpp::document::DocumentV0Setters;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::prelude::UserFeeIncrease;
use dash_sdk::dpp::state_transition::batch_transition::methods::v0::DocumentsBatchTransitionMethodsV0;
use dash_sdk::dpp::state_transition::batch_transition::BatchTransitionV0;
use dash_sdk::dpp::state_transition::proof_result::StateTransitionProofResult;
use dash_sdk::dpp::tokens::token_payment_info::TokenPaymentInfo;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::transition::broadcast::BroadcastStateTransition;
use dash_sdk::platform::transition::put_document::PutDocument;
use dash_sdk::platform::{
    DataContract, Document, DocumentQuery, Fetch, FetchMany, Identifier, IdentityPublicKey,
};
use dash_sdk::query_types::IndexMap;
use dash_sdk::Sdk;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DocumentTask {
    BroadcastDocument(
        Document,
        Option<TokenPaymentInfo>,
        [u8; 32],
        DocumentType,
        QualifiedIdentity,
        IdentityPublicKey,
    ),
    DeleteDocument(
        Identifier, // Document ID
        DocumentType,
        DataContract,
        QualifiedIdentity,
        IdentityPublicKey,
        Option<TokenPaymentInfo>,
    ),
    ReplaceDocument(
        Document,
        DocumentType,
        DataContract,
        QualifiedIdentity,
        IdentityPublicKey,
        Option<TokenPaymentInfo>,
    ),
    TransferDocument(
        Identifier, // Document ID
        Identifier, // New owner ID
        DocumentType,
        DataContract,
        QualifiedIdentity,
        IdentityPublicKey,
        Option<TokenPaymentInfo>,
    ),
    PurchaseDocument(
        Credits,    // Price in credits
        Identifier, // Document ID
        DocumentType,
        DataContract,
        QualifiedIdentity,
        IdentityPublicKey,
        Option<TokenPaymentInfo>,
    ),
    SetDocumentPrice(
        Credits,    // Price in credits
        Identifier, // Document ID
        DocumentType,
        DataContract,
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
                    .map_err(|e| format!("Error fetching documents: {}", e.to_string()))
            }
            DocumentTask::FetchDocumentsPage(mut document_query) => {
                // Set the limit for each page
                document_query.limit = 100;

                // Initialize an empty IndexMap to accumulate documents for this page
                let mut page_docs: IndexMap<Identifier, Option<Document>> = IndexMap::new();

                // Fetch a single page
                let docs_batch_result = Document::fetch_many(sdk, document_query.clone())
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
                entropy, // you usually don’t need it here
                doc_type,
                qualified_identity,
                identity_key,
            ) => document
                .put_to_platform_and_wait_for_response(
                    sdk,
                    doc_type,
                    entropy,
                    identity_key,
                    token_payment_info,
                    &qualified_identity,
                    None,
                )
                .await
                .map(BackendTaskSuccessResult::Document)
                .map_err(|e| format!("Error broadcasting document: {}", e.to_string())),
            DocumentTask::DeleteDocument(
                document_id,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            ) => {
                // ---------- 1. get & bump identity nonce ----------
                let new_nonce = sdk
                    .get_identity_contract_nonce(
                        qualified_identity.identity.id(),
                        data_contract.id(),
                        true,
                        None,
                    )
                    .await
                    .map_err(|e| format!("Fetch nonce error: {e}"))?;

                // ---------- 2. fetch document ----------
                let document_query = DocumentQuery {
                    data_contract: data_contract.into(),
                    document_type_name: document_type.name().to_string(),
                    where_clauses: vec![],
                    order_by_clauses: vec![],
                    limit: 1,
                    start: None,
                };
                let query_with_id = DocumentQuery::with_document_id(document_query, &document_id);
                let document = Document::fetch(sdk, query_with_id)
                    .await
                    .map_err(|e| format!("Error fetching document: {}", e))?
                    .ok_or_else(|| "Document not found".to_string())?;

                let state_transition =
                    BatchTransitionV0::new_document_deletion_transition_from_document(
                        document,
                        document_type.as_ref(),
                        &identity_key,
                        new_nonce,
                        UserFeeIncrease::default(),
                        token_payment_info,
                        &qualified_identity,
                        sdk.version(),
                        None,
                    )
                    .map_err(|e| format!("Error creating batch transition: {}", e))?;

                // ---------- 4. broadcast ----------
                state_transition
                    .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
                    .await
                    .map(|_| {
                        BackendTaskSuccessResult::Message(
                            "Document deleted successfully".to_string(),
                        )
                    })
                    .map_err(|e| format!("Broadcasting error: {}", e.to_string()))
            }
            DocumentTask::ReplaceDocument(
                document,
                document_type,
                data_contract,
                qualified_identity,
                identity_key,
                token_payment_info,
            ) => {
                // ---------- 1. get & bump identity nonce ----------
                let new_nonce = sdk
                    .get_identity_contract_nonce(
                        qualified_identity.identity.id(),
                        data_contract.id(),
                        true,
                        None,
                    )
                    .await
                    .map_err(|e| format!("Fetch nonce error: {e}"))?;

                let state_transition =
                    BatchTransitionV0::new_document_replacement_transition_from_document(
                        document,
                        document_type.as_ref(),
                        &identity_key,
                        new_nonce,
                        UserFeeIncrease::default(),
                        token_payment_info,
                        &qualified_identity,
                        sdk.version(),
                        None,
                    )
                    .map_err(|e| format!("Error creating batch transition: {}", e))?;

                // ---------- 4. broadcast ----------
                state_transition
                    .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
                    .await
                    .map(|_| {
                        BackendTaskSuccessResult::Message(
                            "Document replaced successfully".to_string(),
                        )
                    })
                    .map_err(|e| format!("Broadcasting error: {}", e.to_string()))
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
                // ---------- 1. get & bump identity nonce ----------
                let new_nonce = sdk
                    .get_identity_contract_nonce(
                        qualified_identity.identity.id(),
                        data_contract.id(),
                        true,
                        None,
                    )
                    .await
                    .map_err(|e| format!("Fetch nonce error: {e}"))?;

                // ---------- 2. fetch document ----------
                let document_query = DocumentQuery {
                    data_contract: data_contract.into(),
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

                // ---------- 3. create transfer transition ----------
                let state_transition =
                    BatchTransitionV0::new_document_transfer_transition_from_document(
                        document,
                        document_type.as_ref(),
                        new_owner_id,
                        &identity_key,
                        new_nonce,
                        UserFeeIncrease::default(),
                        token_payment_info,
                        &qualified_identity,
                        sdk.version(),
                        None, // options
                    )
                    .map_err(|e| format!("Error creating batch transition: {}", e))?;

                // ---------- 4. broadcast ----------
                state_transition
                    .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
                    .await
                    .map(|_| {
                        BackendTaskSuccessResult::Message(
                            "Document transferred successfully".to_string(),
                        )
                    })
                    .map_err(|e| format!("Broadcasting error: {}", e.to_string()))
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
                // ---------- 1. get & bump identity nonce ----------
                let new_nonce = sdk
                    .get_identity_contract_nonce(
                        qualified_identity.identity.id(),
                        data_contract.id(),
                        true,
                        None,
                    )
                    .await
                    .map_err(|e| format!("Fetch nonce error: {e}"))?;

                // ---------- 2. fetch document ----------
                let document_query = DocumentQuery {
                    data_contract: data_contract.into(),
                    document_type_name: document_type.name().to_string(), // Not needed for purchase
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

                // ---------- 3. create purchase transition ----------
                let state_transition =
                    BatchTransitionV0::new_document_purchase_transition_from_document(
                        document,
                        document_type.as_ref(),
                        qualified_identity.identity.id(),
                        price,
                        &identity_key,
                        new_nonce,
                        UserFeeIncrease::default(),
                        token_payment_info,
                        &qualified_identity,
                        sdk.version(),
                        None, // options
                    )
                    .map_err(|e| format!("Error creating batch transition: {}", e))?;

                // ---------- 4. broadcast ----------
                state_transition
                    .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
                    .await
                    .map(|_| {
                        BackendTaskSuccessResult::Message(
                            "Document purchased successfully".to_string(),
                        )
                    })
                    .map_err(|e| format!("Broadcasting error: {}", e.to_string()))
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
                // ---------- 1. get & bump identity nonce ----------
                let new_nonce = sdk
                    .get_identity_contract_nonce(
                        qualified_identity.identity.id(),
                        data_contract.id(),
                        true,
                        None,
                    )
                    .await
                    .map_err(|e| format!("Fetch nonce error: {e}"))?;

                // ---------- 2. fetch document ----------
                let document_query = DocumentQuery {
                    data_contract: data_contract.into(),
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

                // ---------- 3. create set price transition ----------
                let state_transition =
                    BatchTransitionV0::new_document_update_price_transition_from_document(
                        document,
                        document_type.as_ref(),
                        price,
                        &identity_key,
                        new_nonce,
                        UserFeeIncrease::default(),
                        token_payment_info,
                        &qualified_identity,
                        sdk.version(),
                        None, // options
                    )
                    .map_err(|e| format!("Error creating batch transition: {}", e))?;

                // ---------- 4. broadcast ----------
                state_transition
                    .broadcast_and_wait::<StateTransitionProofResult>(sdk, None)
                    .await
                    .map(|_| {
                        BackendTaskSuccessResult::Message(
                            "Document price set successfully".to_string(),
                        )
                    })
                    .map_err(|e| format!("Broadcasting error: {}", e.to_string()))
            }
        }
    }
}
