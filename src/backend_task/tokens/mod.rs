use super::BackendTaskSuccessResult;
use crate::{app::TaskResult, context::AppContext, model::qualified_identity::QualifiedIdentity};
use dash_sdk::{
    platform::{DataContract, Identifier, IdentityPublicKey},
    Sdk,
};
use std::sync::Arc;
use tokio::sync::mpsc;

mod mint_token;
mod query_my_token_balances;
mod query_tokens;
mod transfer_tokens;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TokenTask {
    QueryMyTokenBalances,
    QueryTokensByKeyword(String),
    QueryTokensByKeywordPage(String, Option<Identifier>),
    MintToken(Identifier),
    TransferTokens {
        sending_identity: QualifiedIdentity,
        recipient_id: Identifier,
        amount: u64,
        data_contract: DataContract,
        token_position: u16,
        signing_key: IdentityPublicKey,
    },
}

impl AppContext {
    pub async fn run_token_task(
        self: &Arc<Self>,
        task: TokenTask,
        sdk: &Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        match &task {
            TokenTask::QueryMyTokenBalances => self
                .query_my_token_balances(sdk, sender)
                .await
                .map_err(|e| format!("Failed to fetch token balances: {e}")),
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
            TokenTask::TransferTokens {
                sending_identity,
                recipient_id,
                amount,
                data_contract,
                token_position,
                signing_key,
            } => self
                .transfer_tokens(
                    &sending_identity,
                    *recipient_id,
                    *amount,
                    data_contract,
                    *token_position,
                    signing_key.clone(),
                    sdk,
                    sender,
                )
                .await
                .map_err(|e| format!("Failed to transfer tokens: {e}")),
        }
    }
}
