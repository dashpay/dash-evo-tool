use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use dash_sdk::query_types::IndexMap;
use dash_sdk::Sdk;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DocumentTask {
    FetchDocuments(DocumentQuery),
    FetchAllDocuments(DocumentQuery),
}

impl AppContext {
    pub async fn run_document_task(
        &self,
        task: DocumentTask,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, String> {
        match task {
            DocumentTask::FetchDocuments(drive_query) => Document::fetch_many(sdk, drive_query)
                .await
                .map(BackendTaskSuccessResult::Documents)
                .map_err(|e| format!("Error fetching documents: {}", e.to_string())),
            DocumentTask::FetchAllDocuments(mut document_query) => {
                // Initialize an empty IndexMap to accumulate documents
                let mut all_docs: IndexMap<Identifier, Option<Document>> = IndexMap::new();

                loop {
                    // Fetch a batch
                    let docs_batch_result = Document::fetch_many(sdk, document_query.clone())
                        .await
                        .map_err(|e| format!("Error fetching documents: {}", e))?;

                    let batch_len = docs_batch_result.len();

                    // Insert the batch into our master map
                    for (id, doc_opt) in docs_batch_result {
                        all_docs.insert(id, doc_opt);
                    }

                    // If fewer than 100 results, we're done
                    if batch_len < 100 {
                        break;
                    }

                    // Otherwise, set 'start' to the last document's identifier bytes
                    if let Some(last_doc_id) = all_docs.keys().last().cloned() {
                        // Convert the Identifier to bytes
                        let id_bytes = last_doc_id.to_buffer();
                        document_query.start = Some(Start::StartAfter(id_bytes.to_vec()));
                    } else {
                        break;
                    }
                }

                // Return all accumulated documents
                Ok(BackendTaskSuccessResult::Documents(all_docs))
            }
        }
    }
}
