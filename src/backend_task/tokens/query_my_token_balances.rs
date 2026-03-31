//! Query token balances from Platform via TokenWallet

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use dash_sdk::Sdk;

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

            // Get the platform wallet for this identity (if available)
            let platform_wallet = match self.platform_wallet_for_identity(&identity) {
                Ok(pw) => pw,
                Err(_) => {
                    // Identity may not have a wallet (e.g. single-key identity);
                    // fall back to direct SDK query for this identity
                    self.query_token_balances_direct(sdk, identity_id, &sender)
                        .await?;
                    continue;
                }
            };

            let token_wallet = platform_wallet.tokens();

            let token_infos = self
                .identity_token_balances()?
                .values()
                .filter(|t| t.identity_id == identity_id)
                .map(|t| (t.token_id, t.data_contract_id, t.token_position))
                .collect::<Vec<_>>();

            if token_infos.is_empty() {
                continue;
            }

            // Register all tokens with the TokenWallet's watch list
            for (token_id, _, _) in &token_infos {
                token_wallet.watch(identity_id, *token_id).await;
            }

            // Sync balances from Platform
            token_wallet.sync().await.map_err(|e| {
                TaskError::TokenQueryError {
                    detail: format!("Failed to sync token balances: {}", e),
                }
            })?;

            // Read synced balances from cache and persist to local DB
            for (token_id, _, _) in &token_infos {
                let balance = token_wallet
                    .balance(&identity_id, token_id)
                    .await
                    .unwrap_or(0);

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

        Ok(BackendTaskSuccessResult::FetchedTokenBalances)
    }

    /// Fallback direct SDK query for identities without a platform wallet
    /// (e.g. single-key identities).
    async fn query_token_balances_direct(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        sender: &crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<(), TaskError> {
        use dash_sdk::dpp::balances::credits::TokenAmount;
        use dash_sdk::platform::FetchMany;
        use dash_sdk::platform::tokens::identity_token_balances::{
            IdentityTokenBalances, IdentityTokenBalancesQuery,
        };

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
            return Ok(());
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

        Ok(())
    }

    pub async fn query_token_balance(
        &self,
        sdk: &Sdk,
        identity_id: Identifier,
        token_id: Identifier,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        use dash_sdk::dpp::balances::credits::TokenAmount;
        use dash_sdk::platform::FetchMany;
        use dash_sdk::platform::tokens::identity_token_balances::{
            IdentityTokenBalances, IdentityTokenBalancesQuery,
        };

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
