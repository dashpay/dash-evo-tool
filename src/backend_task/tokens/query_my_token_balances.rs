//! Query token balances from Platform

use crate::backend_task::{BackendTaskSuccessResult, NO_IDENTITIES_FOUND};
use crate::context::AppContext;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::tokens::identity_token_balances::{
    IdentityTokenBalances, IdentityTokenBalancesQuery,
};
use dash_sdk::platform::{FetchMany, Identifier};
use dash_sdk::{dpp::balances::credits::TokenAmount, Sdk};

use crate::app::TaskResult;

impl AppContext {
    pub async fn query_my_token_balances(
        &self,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let identities = self
            .load_local_qualified_identities()
            .map_err(|e| format!("Failed to load identities: {e}"))?;

        if identities.is_empty() {
            return Err(NO_IDENTITIES_FOUND.to_string());
        }

        for identity in identities {
            let identity_id = identity.identity.id();
            let token_infos = self
                .identity_token_balances()
                .map_err(|e| format!("Failed to load identity token balances: {e}"))?
                .values()
                .filter(|t| t.identity_id == identity_id)
                .map(|t| (t.token_id, t.data_contract_id, t.token_position))
                .collect::<Vec<_>>();

            let token_ids: Vec<Identifier> = token_infos
                .iter()
                .map(|(token_id, _, _)| *token_id)
                .collect();

            if token_ids.is_empty() {
                continue;
            }

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
                                "Failed to insert token balance into local database: {}",
                                e
                            ));
                        };
                        sender
                            .send(TaskResult::Refresh)
                            .await
                            .map_err(|e| format!("Failed to send refresh message after successful Platform query and local database insert: {}", e))?;
                    }
                }
                Err(e) => {
                    return Err(format!("Failed to query token balances: {}", e));
                }
            }
        }

        Ok(BackendTaskSuccessResult::Message(
            "Successfully fetched token balances".to_string(),
        ))
    }

    pub async fn query_token_balance(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        token_id: Identifier,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, String> {
        let query = IdentityTokenBalancesQuery {
            identity_id,
            token_ids: vec![token_id],
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
                    if let Err(e) =
                        self.db
                            .insert_identity_token_balance(token_id, &identity_id, balance, self)
                    {
                        return Err(format!(
                            "Failed to insert token balance into local database: {}",
                            e
                        ));
                    };
                    sender
                            .send(TaskResult::Refresh)
                            .await
                            .map_err(|e| format!("Failed to send refresh message after successful Platform query and local database insert: {}", e))?;
                }
            }
            Err(e) => {
                return Err(format!("Failed to query token balances: {}", e));
            }
        }

        Ok(BackendTaskSuccessResult::Message(
            "Successfully fetched token balances".to_string(),
        ))
    }
}
