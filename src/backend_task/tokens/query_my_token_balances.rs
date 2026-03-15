//! Query token balances from Platform

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::tokens::identity_token_balances::{
    IdentityTokenBalances, IdentityTokenBalancesQuery,
};
use dash_sdk::platform::{FetchMany, Identifier};
use dash_sdk::{Sdk, dpp::balances::credits::TokenAmount};

use crate::app::TaskResult;

impl AppContext {
    pub async fn query_my_token_balances(
        &self,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let identities = self.load_local_qualified_identities()?;

        if identities.is_empty() {
            return Err(TaskError::NoIdentitiesFound);
        }

        for identity in identities {
            let identity_id = identity.identity.id();
            let token_infos = self
                .identity_token_balances()?
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

            let balances_result: Result<IdentityTokenBalances, _> =
                TokenAmount::fetch_many(sdk, query).await;

            match balances_result {
                Ok(token_balances) => {
                    for balance in token_balances.iter() {
                        let token_id = balance.0;
                        let balance = match balance.1 {
                            Some(b) => *b,
                            None => 0,
                        };
                        self.db.insert_identity_token_balance(
                            token_id,
                            &identity_id,
                            balance,
                            self,
                        )?;
                        sender
                            .send(TaskResult::Refresh)
                            .await
                            .map_err(|_| TaskError::InternalSendError)?;
                    }
                }
                Err(e) => {
                    return Err(TaskError::TokenQueryError {
                        detail: format!("Failed to query token balances: {}", e),
                    });
                }
            }
        }

        Ok(BackendTaskSuccessResult::FetchedTokenBalances)
    }

    pub async fn query_token_balance(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        token_id: Identifier,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let query = IdentityTokenBalancesQuery {
            identity_id,
            token_ids: vec![token_id],
        };

        let balances_result: Result<IdentityTokenBalances, _> =
            TokenAmount::fetch_many(sdk, query).await;

        match balances_result {
            Ok(token_balances) => {
                for balance in token_balances.iter() {
                    let token_id = balance.0;
                    let balance = match balance.1 {
                        Some(b) => *b,
                        None => 0,
                    };
                    self.db
                        .insert_identity_token_balance(token_id, &identity_id, balance, self)?;
                    sender
                        .send(TaskResult::Refresh)
                        .await
                        .map_err(|_| TaskError::InternalSendError)?;
                }
            }
            Err(e) => {
                return Err(TaskError::TokenQueryError {
                    detail: format!("Failed to query token balances: {}", e),
                });
            }
        }

        Ok(BackendTaskSuccessResult::FetchedTokenBalances)
    }
}
