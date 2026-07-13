//! Refresh token balances from upstream.
//!
//! Balances are owned by the upstream `IdentitySyncManager`: DET registers
//! each local identity's watched-token list, forces a sync pass, then
//! republishes the lock-free balance snapshot the My Tokens screen reads. DET
//! no longer fetches or caches balances itself.
//!
//! A watch set is the local token registry minus the `(identity, token)` pairs
//! the user stopped tracking. Upstream holds the watch set in memory only, so
//! the dismissals are persisted DET-side and re-applied on every refresh.

use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::ui::tokens::tokens_screen::IdentityTokenIdentifier;
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

        let identity_ids: Vec<Identifier> = identities.iter().map(|qi| qi.identity.id()).collect();
        let watch_sets = self.token_watch_sets(identity_ids)?;

        self.refresh_upstream_token_balances(watch_sets, &sender)
            .await?;

        Ok(BackendTaskSuccessResult::FetchedTokenBalances)
    }

    pub async fn query_token_balance(
        &self,
        _sdk: &Sdk,
        pair: IdentityTokenIdentifier,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        // Asking for a balance is intent to track it: undo any earlier "stop
        // tracking" for this pair, otherwise the watch set would omit the very
        // token the caller asked about.
        self.clear_untracked_token_balance(pair)?;

        // The upstream watch list is per-identity and replaced wholesale, so
        // register the identity's whole watch set rather than a single pair.
        let watch_sets = self.token_watch_sets(vec![pair.identity_id])?;
        self.refresh_upstream_token_balances(watch_sets, &sender)
            .await?;

        Ok(BackendTaskSuccessResult::FetchedTokenBalances)
    }

    /// Stop tracking one identity-token balance. Un-watches the pair in the
    /// upstream sync loop so its background pass stops fetching the balance and
    /// the pair leaves the published snapshot, records the dismissal so later
    /// refreshes do not re-watch it, then drops it from the saved My Tokens
    /// ordering and nudges the UI to re-read the snapshot. The token stays in
    /// DET's registry: re-importing it, or explicitly checking that identity's
    /// balance, tracks the pair again.
    pub async fn stop_tracking_token_balance(
        &self,
        pair: IdentityTokenIdentifier,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.wallet_backend()?
            .unwatch_identity_token(pair.identity_id, pair.token_id)
            .await;
        self.mark_token_balance_untracked(pair)?;
        self.remove_token_balance(pair)?;
        sender
            .send(TaskResult::Refresh)
            .await
            .map_err(|_| TaskError::InternalSendError)?;
        Ok(BackendTaskSuccessResult::FetchedTokenBalances)
    }

    /// The tokens DET watches upstream for each identity: every token in the
    /// local registry, minus the pairs the user stopped tracking.
    ///
    /// Upstream's watch set is in-memory and replaced wholesale per identity,
    /// so each refresh must rebuild it from DET's persisted state.
    fn token_watch_sets(
        &self,
        identity_ids: Vec<Identifier>,
    ) -> Result<Vec<(Identifier, Vec<Identifier>)>, TaskError> {
        let token_ids: Vec<Identifier> = self.get_all_known_tokens()?.keys().copied().collect();
        let untracked = self.untracked_token_balances()?;

        Ok(identity_ids
            .into_iter()
            .map(|identity_id| {
                let watched = token_ids
                    .iter()
                    .copied()
                    .filter(|token_id| {
                        !untracked.contains(&IdentityTokenIdentifier {
                            identity_id,
                            token_id: *token_id,
                        })
                    })
                    .collect();
                (identity_id, watched)
            })
            .collect())
    }

    /// Register each identity's watch set with upstream, force an immediate
    /// sync pass, then republish DET's balance snapshot and nudge the UI to
    /// re-read it.
    async fn refresh_upstream_token_balances(
        &self,
        watch_sets: Vec<(Identifier, Vec<Identifier>)>,
        sender: &crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        for (identity_id, token_ids) in watch_sets {
            backend
                .register_identity_tokens(identity_id, token_ids)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_dir::ensure_env_file;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::TokenConfiguration;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tokio::sync::mpsc::Receiver;

    /// Offline, wired context — real k/v store and wallet backend, no network.
    /// The receiver is held so the `TaskResult::Refresh` nudge does not fail.
    struct Fixture {
        ctx: Arc<AppContext>,
        sender: SenderAsync<TaskResult>,
        _rx: Receiver<TaskResult>,
        _dir: tempfile::TempDir,
    }

    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            Arc::new(AtomicBool::new(false)),
        )
        .expect("offline testnet AppContext");

        let (tx, rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender.clone())
            .await
            .expect("wire wallet backend offline");

        Fixture {
            ctx,
            sender,
            _rx: rx,
            _dir: dir,
        }
    }

    fn ident(byte: u8) -> Identifier {
        Identifier::from([byte; 32])
    }

    /// Put a token in DET's local registry, as importing one would.
    fn register_token(ctx: &AppContext, token_id: Identifier, alias: &str) {
        ctx.insert_token(
            &token_id,
            alias,
            TokenConfiguration::V0(TokenConfigurationV0::default_most_restrictive()),
            &ident(200),
            0,
        )
        .expect("insert token");
    }

    /// The tokens a balance refresh would re-register upstream for `identity`.
    fn watched(ctx: &AppContext, identity: Identifier) -> Vec<Identifier> {
        ctx.token_watch_sets(vec![identity])
            .expect("watch sets")
            .pop()
            .expect("one watch set per identity")
            .1
    }

    /// The registry is sorted by alias, so watch sets come back in that order.
    fn tokens(ctx: &AppContext) -> (Identifier, Identifier) {
        let (alpha, beta) = (ident(1), ident(2));
        register_token(ctx, alpha, "Alpha");
        register_token(ctx, beta, "Beta");
        (alpha, beta)
    }

    fn pair(identity_id: Identifier, token_id: Identifier) -> IdentityTokenIdentifier {
        IdentityTokenIdentifier {
            identity_id,
            token_id,
        }
    }

    /// Dismissing a balance must survive "Refresh My Tokens": the pair stays
    /// out of the identity's watch set, and only that identity is affected.
    #[tokio::test]
    async fn stopped_pair_is_not_rewatched_by_a_refresh() {
        let f = fixture().await;
        let (alpha, beta) = tokens(&f.ctx);
        let (identity, other_identity) = (ident(10), ident(11));

        f.ctx
            .stop_tracking_token_balance(pair(identity, alpha), f.sender.clone())
            .await
            .expect("stop tracking");

        assert_eq!(watched(&f.ctx, identity), vec![beta]);
        assert_eq!(watched(&f.ctx, other_identity), vec![alpha, beta]);
    }

    /// Explicitly checking a dismissed balance tracks that one pair again.
    #[tokio::test]
    async fn retracking_a_pair_restores_it_to_the_watch_set() {
        let f = fixture().await;
        let (alpha, beta) = tokens(&f.ctx);
        let identity = ident(10);

        f.ctx
            .stop_tracking_token_balance(pair(identity, alpha), f.sender.clone())
            .await
            .expect("stop tracking");
        f.ctx
            .clear_untracked_token_balance(pair(identity, alpha))
            .expect("re-track pair");

        assert_eq!(watched(&f.ctx, identity), vec![alpha, beta]);
    }

    /// Re-importing a token tracks it again for every identity that dismissed
    /// it — the "you can re-add it later" promise the remove dialog makes.
    #[tokio::test]
    async fn reimporting_a_token_retracks_it_for_every_identity() {
        let f = fixture().await;
        let (alpha, beta) = tokens(&f.ctx);
        let (first, second) = (ident(10), ident(11));

        for identity in [first, second] {
            f.ctx
                .stop_tracking_token_balance(pair(identity, alpha), f.sender.clone())
                .await
                .expect("stop tracking");
        }
        f.ctx
            .clear_untracked_token(&alpha)
            .expect("re-import token");

        assert_eq!(watched(&f.ctx, first), vec![alpha, beta]);
        assert_eq!(watched(&f.ctx, second), vec![alpha, beta]);
    }
}
