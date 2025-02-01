use super::BackendTaskSuccessResult;
use crate::{app::TaskResult, context::AppContext};
use dash_sdk::{platform::Identifier, Sdk};
use std::sync::Arc;
use tokio::sync::mpsc;

mod mint_token;
mod query_my_token_balances;
mod query_tokens;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TokenTask {
    QueryMyTokenBalances,
    QueryTokensByKeyword(String),
    QueryTokensByKeywordPage(String, Option<Identifier>),
    MintToken(Identifier),
}

impl AppContext {
    pub async fn run_token_task(
        self: &Arc<Self>,
        task: TokenTask,
        sdk: &Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match &task {
            TokenTask::QueryMyTokenBalances => {
                // Placeholder
                // Ok(BackendTaskSuccessResult::Message(
                //     "QueryMyTokenBalances".to_string(),
                // ))

                // Actually do this
                self.query_my_token_balances(sdk, sender).await
            }
            TokenTask::QueryTokensByKeyword(query) => {
                // Placeholder
                Ok(BackendTaskSuccessResult::Message("QueryTokens".to_string()))

                // Actually do this
                // self.query_tokens(query, sdk, sender).await
            }
            TokenTask::MintToken(id) => {
                // Placeholder
                Ok(BackendTaskSuccessResult::Message("MintToken".to_string()))

                // Actually do this
                // self.mint_token(id, sdk, sender).await
            }
            TokenTask::QueryTokensByKeywordPage(query, cursor) => {
                // Placeholder
                Ok(BackendTaskSuccessResult::Message(
                    "QueryTokensByKeywordPage".to_string(),
                ))

                // Actually do this
                // self.query_tokens_page(query, cursor, sdk, sender).await
            }
        }
    }
}
