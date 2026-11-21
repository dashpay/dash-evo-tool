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
use crate::model::token::{IdentityTokenIdentifier, tracked_token_ids};
use crate::wallet_backend::TokenBalanceSyncError;
use dash_sdk::Sdk;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::backend_task::TaskResult;

const TOKEN_BALANCE_REFRESH_MAX_LIFETIME: Duration = NETWORK_REQUEST_TIMEOUT.saturating_mul(2);

/// How long a refresh waits for another one to finish before running its own.
const TOKEN_BALANCE_REFRESH_JOIN_LIMIT: Duration = NETWORK_REQUEST_TIMEOUT;

/// How often a waiting refresh checks whether the running one has finished.
const TOKEN_BALANCE_REFRESH_JOIN_POLL: Duration = Duration::from_millis(100);

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
        self.ensure_token_balance_reads_allowed()?;
        let identities = self.load_local_qualified_identities()?;

        if identities.is_empty() {
            return Err(TaskError::NoIdentitiesFound);
        }

        let identity_ids: Vec<Identifier> = identities.iter().map(|qi| qi.identity.id()).collect();
        let watch_sets = self.token_watch_sets(identity_ids)?;

        let refresh_guard = self.begin_token_balance_refresh().await?;
        self.run_token_balance_refresh(refresh_guard, watch_sets, sender)
            .await
    }

    /// Track `pairs` again and refresh their identities' balances in one pass.
    /// Asking for a balance is intent to track it: every earlier "stop
    /// tracking" of these pairs is undone first, otherwise the watch set would
    /// omit the very tokens the caller asked about.
    pub async fn track_token_balances_again(
        self: &std::sync::Arc<Self>,
        _sdk: &Sdk,
        pairs: Vec<IdentityTokenIdentifier>,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let identity_ids: Vec<Identifier> = pairs
            .iter()
            .map(|pair| pair.identity_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if identity_ids.is_empty() {
            return Ok(BackendTaskSuccessResult::FetchedTokenBalances);
        }
        // Gate and guard come first: a refusal must leave the dismissals as they
        // were, or the rows would reappear with no refresh to fill them.
        self.ensure_token_balance_reads_allowed()?;
        let refresh_guard = self.begin_token_balance_refresh().await?;
        for pair in &pairs {
            self.clear_untracked_token_balance(*pair)?;
        }
        // The upstream watch list is per-identity and replaced wholesale, so
        // register each identity's whole watch set rather than single pairs.
        let watch_sets = self.token_watch_sets(identity_ids)?;
        self.run_token_balance_refresh(refresh_guard, watch_sets, sender)
            .await
    }

    /// Refresh `watch_sets` upstream under the refresh guard and the network
    /// timeout, then tell the screens to re-read the balances.
    async fn run_token_balance_refresh(
        self: &std::sync::Arc<Self>,
        refresh_guard: TokenBalanceRefreshGuard,
        watch_sets: Vec<(Identifier, Vec<Identifier>)>,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let context = Arc::clone(self);
        await_managed_network_request_with_timeout(
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
        self.notify_cache_changed(crate::backend_task::CacheDomain::Tokens, &sender)
            .await;
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
        self.notify_cache_changed(crate::backend_task::CacheDomain::Tokens, &sender)
            .await;
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
                let watched =
                    tracked_token_ids(token_ids.iter().copied(), &untracked, identity_id).collect();
                (identity_id, watched)
            })
            .collect())
    }

    /// Refuse a balance read until this context's wallet backend has synced
    /// its masternode list and is not stopping. Every caller goes through
    /// these tasks, so the gate lives here rather than in each screen: a
    /// proof-verifying read sent earlier fails and gets the queried node banned.
    fn ensure_token_balance_reads_allowed(&self) -> Result<(), TaskError> {
        self.wallet_backend()?
            .admit_token_balance_read()
            .map_err(token_balance_sync_task_error)
    }

    /// Whether a token balance refresh is running anywhere in the app. The
    /// guard is app-wide, so a second surface asking now waits for it.
    pub fn token_balance_refresh_in_flight(&self) -> bool {
        self.token_balance_refresh_in_flight.load(Ordering::Acquire)
    }

    async fn begin_token_balance_refresh(
        self: &Arc<Self>,
    ) -> Result<TokenBalanceRefreshGuard, TaskError> {
        self.begin_token_balance_refresh_after(TOKEN_BALANCE_REFRESH_JOIN_LIMIT)
            .await
    }

    /// Take the app-wide refresh guard, waiting up to `join_limit` for a
    /// refresh already running: it may not have seen this caller's changes,
    /// so this one runs its own pass after it.
    async fn begin_token_balance_refresh_after(
        self: &Arc<Self>,
        join_limit: Duration,
    ) -> Result<TokenBalanceRefreshGuard, TaskError> {
        let deadline = tokio::time::Instant::now() + join_limit;
        loop {
            match self.begin_token_balance_refresh_with_timeout(TOKEN_BALANCE_REFRESH_MAX_LIFETIME)
            {
                Err(TaskError::TokenBalanceRefreshInProgress)
                    if tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(TOKEN_BALANCE_REFRESH_JOIN_POLL).await;
                }
                outcome => return outcome,
            }
        }
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

    /// Register each identity's watch set with upstream, run a sync pass that
    /// sees it, and republish DET's balance snapshot.
    async fn refresh_upstream_token_balances(
        &self,
        watch_sets: Vec<(Identifier, Vec<Identifier>)>,
    ) -> Result<(), TaskError> {
        let backend = self.wallet_backend()?;
        backend
            .sync_token_balances_now(watch_sets)
            .await
            .map_err(token_balance_sync_task_error)
    }
}

fn token_balance_sync_task_error(error: TokenBalanceSyncError) -> TaskError {
    match error {
        TokenBalanceSyncError::SyncStopped => TaskError::TokenBalanceRefreshDisconnected,
        TokenBalanceSyncError::MasternodesNotReady => TaskError::MasternodeListSyncPending,
        TokenBalanceSyncError::UpstreamPassStillRunning => TaskError::TokenBalanceRefreshInProgress,
        TokenBalanceSyncError::NotRead(identities) => {
            TaskError::TokenBalancesNotChecked { identities }
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
    /// The receiver is held so the cache notification can be delivered.
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

    /// A refresh that meets another one waits for it and then runs, so a
    /// refresh after a token operation never fails on a collision.
    #[tokio::test]
    async fn a_second_refresh_waits_for_the_one_in_flight() {
        let f = fixture().await;
        let first_refresh = f
            .ctx
            .begin_token_balance_refresh()
            .await
            .expect("first refresh must acquire the single-flight guard");

        let ctx = Arc::clone(&f.ctx);
        let second = tokio::spawn(async move { ctx.begin_token_balance_refresh().await.is_ok() });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!second.is_finished(), "the second refresh waits");
        drop(first_refresh);
        let joined = tokio::time::timeout(std::time::Duration::from_secs(5), second)
            .await
            .expect("the second refresh starts once the first ends")
            .expect("join");
        assert!(joined);
    }

    /// The wait for another refresh is bounded.
    #[tokio::test]
    async fn a_refresh_stops_waiting_for_one_that_does_not_end() {
        let f = fixture().await;
        let _first_refresh = f
            .ctx
            .begin_token_balance_refresh()
            .await
            .expect("first refresh must acquire the single-flight guard");
        assert!(matches!(
            f.ctx
                .begin_token_balance_refresh_after(std::time::Duration::from_millis(200))
                .await,
            Err(TaskError::TokenBalanceRefreshInProgress)
        ));
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
                match f
                    .ctx
                    .begin_token_balance_refresh_with_timeout(TOKEN_BALANCE_REFRESH_MAX_LIFETIME)
                {
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

    /// Every balance read waits for the masternode list, whichever screen
    /// asks: a refusal happens before the refresh guard is taken and before
    /// any dismissal is undone.
    #[tokio::test]
    async fn balance_reads_are_refused_until_the_masternode_list_is_synced() {
        let f = fixture().await;
        f.ctx
            .wallet_backend()
            .expect("wired backend")
            .mark_started_for_test();
        let (alpha, beta) = tokens(&f.ctx);
        let identity = ident(10);
        f.ctx
            .stop_tracking_token_balance(pair(identity, alpha), f.sender.clone())
            .await
            .expect("stop tracking");
        assert!(!f.ctx.connection_status().masternodes_ready());
        let sdk = f.ctx.sdk();

        assert!(matches!(
            f.ctx.query_my_token_balances(&sdk, f.sender.clone()).await,
            Err(TaskError::MasternodeListSyncPending)
        ));
        assert!(matches!(
            f.ctx
                .track_token_balances_again(&sdk, vec![pair(identity, alpha)], f.sender.clone())
                .await,
            Err(TaskError::MasternodeListSyncPending)
        ));

        assert_eq!(
            watched(&f.ctx, identity),
            vec![beta],
            "a refused read leaves the dismissal in place"
        );
        assert!(!f.ctx.token_balance_refresh_in_flight());
    }

    /// Reads are admitted by the target backend's own session: a ready shared
    /// status does not admit a backend whose masternode list is not synced,
    /// and a disconnect re-arms that backend's gate.
    #[tokio::test]
    async fn balance_reads_wait_for_the_target_backend_session() {
        let f = fixture().await;
        let (alpha, _) = tokens(&f.ctx);
        let identity = ident(10);
        let sdk = f.ctx.sdk();
        let refused = |result: Result<BackendTaskSuccessResult, TaskError>| {
            matches!(result, Err(TaskError::MasternodeListSyncPending))
        };
        let backend = f.ctx.wallet_backend().expect("wired backend");
        backend.mark_started_for_test();
        f.ctx.connection_status().set_masternodes_ready(true);
        assert!(refused(
            f.ctx.query_my_token_balances(&sdk, f.sender.clone()).await
        ));
        assert!(refused(
            f.ctx
                .track_token_balances_again(&sdk, vec![pair(identity, alpha)], f.sender.clone())
                .await
        ));

        backend.mark_masternodes_ready_for_test();
        assert!(matches!(
            f.ctx.query_my_token_balances(&sdk, f.sender.clone()).await,
            Err(TaskError::NoIdentitiesFound)
        ));

        backend.stop_in_place().await;
        assert!(matches!(
            f.ctx.query_my_token_balances(&sdk, f.sender.clone()).await,
            Err(TaskError::TokenBalanceRefreshDisconnected)
        ));
    }

    /// Waiting does not help a read refused while disconnected or before the
    /// wallet backend is wired, so neither says the network is syncing.
    #[tokio::test]
    async fn a_refused_read_names_why_when_waiting_would_not_help() {
        let f = fixture().await;
        let sdk = f.ctx.sdk();
        assert!(matches!(
            f.ctx.query_my_token_balances(&sdk, f.sender.clone()).await,
            Err(TaskError::TokenBalanceRefreshDisconnected)
        ));

        let dir = tempfile::tempdir().expect("tempdir");
        let unwired = crate::context::test_support::test_app_context(dir.path());
        assert!(matches!(
            unwired
                .query_my_token_balances(&unwired.sdk(), f.sender.clone())
                .await,
            Err(TaskError::WalletBackendNotYetWired)
        ));
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
