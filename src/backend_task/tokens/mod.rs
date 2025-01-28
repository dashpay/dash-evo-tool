use super::BackendTaskSuccessResult;
use crate::{app::TaskResult, context::AppContext};
use dash_sdk::Sdk;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TokenTask {
    QueryMyTokenBalances,
    QueryTokens(String),
}

impl AppContext {
    pub async fn run_token_task(
        self: &Arc<Self>,
        task: TokenTask,
        sdk: &Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match &task {
            TokenTask::QueryMyTokenBalances => self.query_my_token_balances(sdk, sender).await,
            TokenTask::QueryTokens(query) => self.query_tokens(sdk, sender, query).await,
        }
    }
}
