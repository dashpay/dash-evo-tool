use std::sync::Arc;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::platform::{DataContract, Document, DriveDocumentQuery, Identifier};
use dash_sdk::Sdk;
use crate::context::AppContext;
use crate::platform::contract::ContractTask;

pub type DocumentTypeName = String;
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DocumentTask {
    FetchDocument(Arc<DataContract>, DocumentTypeName, Identifier),
    FetchDocuments(DriveDocumentQuery),
}

impl AppContext {
    pub async fn run_document_task(&self, task: ContractTask, sdk: &Sdk) -> Result<Value, String> {
        match task {
            DocumentTask::FetchDocument(identifier, name) => {

            }
        }
    }
}