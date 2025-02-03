//! Query token balances from Platform
use std::collections::BTreeSet;

use crate::backend_task::BackendTaskSuccessResult;
use crate::context::AppContext;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::tokens::identity_token_balances::{
    IdentityTokenBalances, IdentityTokenBalancesQuery,
};
use dash_sdk::platform::FetchMany;
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
            .map_err(|e| format!("Failed to load identities: {e}"))?;

        for identity in identities {
            let identity_id = identity.identity.id();
            let token_ids = self
                .identity_token_balances()
                .map_err(|e| format!("Failed to load identity token balances: {e}"))?
                .iter()
                .map(|t| t.token_identifier.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();

            println!("Identity ID: {:?}", identity_id);
            println!("Token IDs: {:?}", token_ids);

            let query = IdentityTokenBalancesQuery {
                identity_id,
                token_ids,
            };

            let balances_result: Result<IdentityTokenBalances, String> =
                TokenAmount::fetch_many(sdk, query)
                    .await
                    .map_err(|e| e.to_string());

            match balances_result {
                Ok(token_balances) => {
                    for balance in token_balances.iter() {
                        let token_id = balance.0;
                        let balance = match balance.1 {
                            Some(b) => *b,
                            None => 0,
                        };
                        if let Err(e) = self.db.insert_identity_token_balance(
                            token_id,
                            &identity_id,
                            balance,
                            self,
                        ) {
                            return Err(format!(
                                "Failed to insert token balance into local database: {:?}",
                                e
                            ));
                        };
                        sender
                            .send(TaskResult::Refresh)
                            .await
                            .map_err(|e| format!("Failed to send refresh message after successful Platform query and local database insert: {:?}", e))?;
                    }
                }
                Err(e) => {
                    return Err(format!("Failed to query token balances: {:?}", e));
                }
            }
        }

        Ok(BackendTaskSuccessResult::Message(
            "Successfully fetched token balances".to_string(),
        ))
    }
}
