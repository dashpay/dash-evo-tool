use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use dash_sdk::dpp::data_contract::document_type::DocumentType;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::transition::put_document::PutDocument;
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier, IdentityPublicKey};
use dash_sdk::query_types::IndexMap;
use dash_sdk::Sdk;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DocumentTask {
    BroadcastDocument(
        Document,
        [u8; 32],
        DocumentType,
        QualifiedIdentity,
        IdentityPublicKey,
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
                    None,
                    &qualified_identity,
                    None,
                )
                .await
                .map(BackendTaskSuccessResult::Document)
                .map_err(|e| format!("Error fetching documents: {}", e.to_string())),
        }
    }
}
