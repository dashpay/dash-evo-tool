//! Refresh token balances from upstream.
//!
//! Balances are owned by the upstream `IdentitySyncManager`: DET registers
//! each local identity's watched-token list (its full local token registry),
//! forces a sync pass, then republishes the lock-free balance snapshot the My
//! Tokens screen reads. DET no longer fetches or caches balances itself.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;

use crate::app::TaskResult;

impl AppContext {
    pub async fn query_my_token_balances(
        &self,
        _sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let identities = self.load_local_qualified_identities()?;

        if identities.is_empty() {
            return Err(TaskError::NoIdentitiesFound);
        }

        let token_ids = self.known_token_ids()?;
        let identity_ids: Vec<Identifier> = identities.iter().map(|qi| qi.identity.id()).collect();

        self.refresh_upstream_token_balances(identity_ids, token_ids, &sender)
            .await?;

        Ok(BackendTaskSuccessResult::FetchedTokenBalances)
    }

    pub async fn query_token_balance(
        &self,
        _sdk: &Sdk,
        identity_id: Identifier,
        _token_id: Identifier,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // The upstream watch list is per-identity and replaced wholesale, so
        // register the identity's full local token set (the requested token is
        // part of it) rather than a single pair.
        let token_ids = self.known_token_ids()?;
        self.refresh_upstream_token_balances(vec![identity_id], token_ids, &sender)
            .await?;

        Ok(BackendTaskSuccessResult::FetchedTokenBalances)
    }

    /// Token ids in DET's local registry — the watch set every local identity
    /// tracks upstream.
    fn known_token_ids(&self) -> Result<Vec<Identifier>, TaskError> {
        Ok(self.get_all_known_tokens()?.keys().copied().collect())
    }

    /// Register each identity's watched tokens with upstream, force an
    /// immediate sync pass, then republish DET's balance snapshot and nudge
    /// the UI to re-read it.
    async fn refresh_upstream_token_balances(
        &self,
        identity_ids: Vec<Identifier>,
        token_ids: Vec<Identifier>,
        sender: &crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        for identity_id in identity_ids {
            backend
                .register_identity_tokens(identity_id, token_ids.clone())
                .await;
        }
        backend.sync_token_balances_now().await;
        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|_| TaskError::InternalSendError)?;
        Ok(())
    }
}
