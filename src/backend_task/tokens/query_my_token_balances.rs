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
use crate::backend_task::{NETWORK_REQUEST_TIMEOUT, await_managed_network_request_with_timeout};
use crate::context::AppContext;
use crate::ui::tokens::tokens_screen::IdentityTokenIdentifier;
use crate::wallet_backend::TokenBalanceSyncOutcome;
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::app::TaskResult;

const TOKEN_BALANCE_REFRESH_MAX_LIFETIME: Duration = NETWORK_REQUEST_TIMEOUT.saturating_mul(2);

struct TokenBalanceRefreshLease {
    context: Arc<AppContext>,
    released: AtomicBool,
    released_signal: CancellationToken,
}

impl TokenBalanceRefreshLease {
    fn release(&self) -> bool {
        if self.released.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.context
            .token_balance_refresh_in_flight
            .store(false, Ordering::Release);
        self.released_signal.cancel();
        true
    }
}

struct TokenBalanceRefreshGuard {
    lease: Arc<TokenBalanceRefreshLease>,
}

impl Drop for TokenBalanceRefreshGuard {
    fn drop(&mut self) {
        self.lease.release();
    }
}

impl AppContext {
    pub async fn query_my_token_balances(
        self: &std::sync::Arc<Self>,
        _sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let identities = self.load_local_qualified_identities()?;

        if identities.is_empty() {
            return Err(TaskError::NoIdentitiesFound);
        }

        let identity_ids: Vec<Identifier> = identities.iter().map(|qi| qi.identity.id()).collect();
        let watch_sets = self.token_watch_sets(identity_ids)?;

        let refresh_guard = self.begin_token_balance_refresh()?;
        let context = Arc::clone(self);
        let outcome = await_managed_network_request_with_timeout(
            self.subtasks.clone(),
            "token_balance_refresh_reaper",
            NETWORK_REQUEST_TIMEOUT,
            async move {
                let _refresh_guard = refresh_guard;
                context.refresh_upstream_token_balances(watch_sets).await
            },
            |source| TaskError::TokenBalanceRefreshTimeout { source },
        )
        .await??;
        let result = Self::token_balance_sync_result(outcome);
        if matches!(result, BackendTaskSuccessResult::FetchedTokenBalances) {
            sender
                .send(TaskResult::Refresh)
                .await
                .map_err(|_| TaskError::InternalSendError)?;
        }
        Ok(result)
    }

    pub async fn query_token_balance(
        self: &std::sync::Arc<Self>,
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
        let refresh_guard = self.begin_token_balance_refresh()?;
        let context = Arc::clone(self);
        let outcome = await_managed_network_request_with_timeout(
            self.subtasks.clone(),
            "token_balance_refresh_reaper",
            NETWORK_REQUEST_TIMEOUT,
            async move {
                let _refresh_guard = refresh_guard;
                context.refresh_upstream_token_balances(watch_sets).await
            },
            |source| TaskError::TokenBalanceRefreshTimeout { source },
        )
        .await??;
        let result = Self::token_balance_sync_result(outcome);
        if matches!(result, BackendTaskSuccessResult::FetchedTokenBalances) {
            sender
                .send(TaskResult::Refresh)
                .await
                .map_err(|_| TaskError::InternalSendError)?;
        }
        Ok(result)
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

    fn begin_token_balance_refresh(
        self: &Arc<Self>,
    ) -> Result<TokenBalanceRefreshGuard, TaskError> {
        self.begin_token_balance_refresh_with_timeout(TOKEN_BALANCE_REFRESH_MAX_LIFETIME)
    }

    fn begin_token_balance_refresh_with_timeout(
        self: &Arc<Self>,
        max_lifetime: Duration,
    ) -> Result<TokenBalanceRefreshGuard, TaskError> {
        self.token_balance_refresh_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| TaskError::TokenBalanceRefreshInProgress)?;

        let lease = Arc::new(TokenBalanceRefreshLease {
            context: Arc::clone(self),
            released: AtomicBool::new(false),
            released_signal: CancellationToken::new(),
        });
        let timeout_lease = Arc::clone(&lease);
        let released_signal = lease.released_signal.clone();
        let deadline = tokio::time::Instant::now() + max_lifetime;
        tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => {
                    if timeout_lease.release() {
                        tracing::debug!(
                            timeout_ms = max_lifetime.as_millis() as u64,
                            "Released a token balance refresh guard after its maximum lifetime"
                        );
                    }
                }
                _ = released_signal.cancelled() => {}
            }
        });

        Ok(TokenBalanceRefreshGuard { lease })
    }

    /// Register each identity's watch set with upstream, force an immediate
    /// sync pass, then republish DET's balance snapshot and nudge the UI to
    /// re-read it.
    async fn refresh_upstream_token_balances(
        &self,
        watch_sets: Vec<(Identifier, Vec<Identifier>)>,
    ) -> Result<TokenBalanceSyncOutcome, TaskError> {
        let backend = self.wallet_backend()?;
        for (identity_id, token_ids) in watch_sets {
            backend
                .register_identity_tokens(identity_id, token_ids)
                .await;
        }
        Ok(backend.sync_token_balances_now().await)
    }

    fn token_balance_sync_result(outcome: TokenBalanceSyncOutcome) -> BackendTaskSuccessResult {
        match outcome {
            TokenBalanceSyncOutcome::Performed => BackendTaskSuccessResult::FetchedTokenBalances,
            TokenBalanceSyncOutcome::AlreadyInFlight => {
                BackendTaskSuccessResult::TokenBalanceRefreshAlreadyInFlight
            }
        }
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
            crate::model::user_role::UserRoleCell::default(),
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

    #[tokio::test]
    async fn overlapping_token_refresh_is_rejected_until_the_first_finishes() {
        let f = fixture().await;
        let first_refresh = f
            .ctx
            .begin_token_balance_refresh()
            .expect("first refresh must acquire the single-flight guard");

        assert!(matches!(
            f.ctx.begin_token_balance_refresh(),
            Err(TaskError::TokenBalanceRefreshInProgress)
        ));

        drop(first_refresh);
        assert!(f.ctx.begin_token_balance_refresh().is_ok());
    }

    #[tokio::test]
    async fn unbounded_refresh_hang_eventually_releases_single_flight_guard() {
        let f = fixture().await;
        let refresh_guard = f
            .ctx
            .begin_token_balance_refresh_with_timeout(std::time::Duration::from_millis(10))
            .expect("hung refresh must acquire the single-flight guard");

        let result = await_managed_network_request_with_timeout(
            f.ctx.subtasks.clone(),
            "pending_token_balance_refresh_test",
            std::time::Duration::from_millis(1),
            async move {
                let _refresh_guard = refresh_guard;
                std::future::pending::<Result<(), TaskError>>().await
            },
            |source| TaskError::TokenBalanceRefreshTimeout { source },
        )
        .await;
        assert!(matches!(
            result,
            Err(TaskError::TokenBalanceRefreshTimeout { .. })
        ));

        let reacquired_guard = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                match f.ctx.begin_token_balance_refresh() {
                    Ok(guard) => break guard,
                    Err(TaskError::TokenBalanceRefreshInProgress) => {
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                    Err(error) => panic!("unexpected refresh error: {error:?}"),
                }
            }
        })
        .await
        .expect("the hung refresh must release its single-flight guard");
        drop(reacquired_guard);
    }

    #[tokio::test]
    async fn expired_lease_does_not_report_success_while_upstream_refresh_is_still_running() {
        let f = fixture().await;
        let old_refresh_guard = f
            .ctx
            .begin_token_balance_refresh_with_timeout(std::time::Duration::from_millis(10))
            .expect("old refresh must acquire the single-flight guard");
        let old_result = await_managed_network_request_with_timeout(
            f.ctx.subtasks.clone(),
            "detached_pending_token_balance_refresh_test",
            std::time::Duration::from_millis(1),
            async move {
                let _old_refresh_guard = old_refresh_guard;
                std::future::pending::<Result<(), TaskError>>().await
            },
            |source| TaskError::TokenBalanceRefreshTimeout { source },
        )
        .await;
        assert!(matches!(
            old_result,
            Err(TaskError::TokenBalanceRefreshTimeout { .. })
        ));

        let retry_guard = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                match f.ctx.begin_token_balance_refresh() {
                    Ok(guard) => break guard,
                    Err(TaskError::TokenBalanceRefreshInProgress) => {
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                    Err(error) => panic!("unexpected refresh error: {error:?}"),
                }
            }
        })
        .await
        .expect("the expired lease must allow a retry attempt");

        let result =
            AppContext::token_balance_sync_result(TokenBalanceSyncOutcome::AlreadyInFlight);

        assert!(matches!(
            result,
            BackendTaskSuccessResult::TokenBalanceRefreshAlreadyInFlight
        ));
        drop(retry_guard);
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
