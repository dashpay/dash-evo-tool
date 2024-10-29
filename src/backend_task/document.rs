use crate::context::AppContext;
use crate::backend_task::BackendTaskSuccessResult;
use dash_sdk::platform::{Document, DocumentQuery, FetchMany};
use dash_sdk::Sdk;

pub type DocumentTypeName = String;
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DocumentTask {
    FetchDocuments(DocumentQuery),
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
                .map_err(|e| e.to_string()),
        }
    }
}
