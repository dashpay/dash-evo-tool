//! Query token balances from Platform
use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::proto::get_identities_token_balances_response::get_identities_token_balances_response_v0::IdentityTokenBalances;
use dash_sdk::platform::tokens::identity_token_balances::IdentityTokenBalancesQuery;
use dash_sdk::platform::{FetchMany, Query};
use dash_sdk::{dpp::balances::credits::TokenAmount, Sdk};
use tokio::sync::mpsc;

use crate::app::TaskResult;
impl AppContext {
    pub async fn query_my_token_balances(
        &self,
        sdk: &Sdk,
        sender: mpsc::Sender<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let identities = self
            .load_local_qualified_identities()
            .expect("Failed to load identities");

        for identity in identities {
            let identity_id = identity.identity.id();
            let token_ids = self
                .identity_token_balances()
                .expect("Expected token balances")
                .iter()
                .map(|t| t.token_identifier.clone())
                .collect();

            let query_struct = IdentityTokenBalancesQuery {
                identity_id,
                token_ids,
            };

            let query = query_struct.query(false).expect("Failed to create query");

            let token_balances: Result<IdentityTokenBalances, _> =
                TokenAmount::fetch_many(sdk, query).await;

            match token_balances {
                Ok(_balances) => {
                    sender
                        .send(TaskResult::Refresh)
                        .await
                        .map_err(|e| format!("Failed to send token balances: {:?}", e))?;
                }
                Err(e) => {
                    return Err(format!("Failed to query token balances: {:?}", e));
                }
            }
        }

        Ok(BackendTaskSuccessResult::Message(
            "QueryMyTokenBalances".to_string(),
        ))
    }
}
