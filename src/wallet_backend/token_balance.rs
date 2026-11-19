//! Per-`(identity, token)` balance view (T6 seam), read live from upstream.
//!
//! [`UpstreamTokenBalances`] is the doorway DET code uses to read the raw
//! `u64` Platform token balance for an `(identity, token)` pair. Upstream's
//! [`IdentitySyncManager`](platform_wallet::manager::identity_sync::IdentitySyncManager)
//! owns the authoritative balances; DET no longer keeps a `det:token_balance`
//! cache of its own.
//!
//! ## Read path
//!
//! The upstream readers (`all_state` / `state_for_identity`) are async, but
//! DET reads token balances from the synchronous egui frame. So the backend
//! refreshes a lock-free DET-typed snapshot off the UI thread after explicit
//! and periodic sync passes. The frame reads that snapshot via [`ArcSwap`].
//!
//! ## Syncing vs zero
//!
//! An identity that has never completed a sync pass (absent from upstream
//! state, or `last_sync_unix == 0`) is reported as **not present** — [`get`]
//! returns `None` and [`list`] omits it. The UI renders this as the
//! "balance unknown / Check" state, never as a misleading `0`. Only a synced
//! identity contributes its `(token, balance)` rows.
//!
//! ## Type boundary
//!
//! Upstream `IdentityTokenSyncState` / `IdentityTokenSyncInfo` / `TokenAmount`
//! are converted to DET / SDK domain types ([`Identifier`], `u64`) at the
//! publish boundary (rust-best-practices M-DONT-LEAK-TYPES). No upstream
//! manager type crosses the seam.
//!
//! [`get`]: UpstreamTokenBalances::get
//! [`list`]: UpstreamTokenBalances::list

use std::collections::BTreeMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::platform::Identifier;
use platform_wallet::manager::identity_sync::IdentitySyncManager;
use platform_wallet_storage::SqlitePersister;
use tokio_util::sync::CancellationToken;

use crate::backend_task::{CacheDomain, CacheUpdate, TaskResult};
use crate::utils::egui_mpsc::SenderAsync;

/// One synced identity's token balances. Presence in
/// [`TokenBalanceSnapshot::identities`] means the identity has completed at
/// least one sync pass; an unsynced identity is simply absent.
type IdentityBalances = BTreeMap<Identifier, u64>;

/// DET-typed, lock-free token-balance snapshot. Maps each *synced* identity
/// to its per-token balances. Cheap to clone-share via the enclosing `Arc`.
#[derive(Debug, Clone, Default)]
pub struct TokenBalanceSnapshot {
    /// Synced identities → (token → balance). An identity absent here has
    /// not completed a sync pass yet and must be reported as "syncing".
    identities: BTreeMap<Identifier, IdentityBalances>,
    /// Proof-derived updates and unwatch operations get distinct markers,
    /// even when they leave the visible amount unchanged.
    local_writes: BTreeMap<(Identifier, Identifier), Arc<()>>,
}

impl TokenBalanceSnapshot {
    /// Balance for one `(identity, token)` pair. `None` when the identity is
    /// not yet synced *or* the token is not among its synced balances.
    fn get(&self, identity_id: &Identifier, token_id: &Identifier) -> Option<u64> {
        self.identities.get(identity_id)?.get(token_id).copied()
    }

    /// Every `(identity, token, balance)` triple across synced identities.
    fn list(&self) -> Vec<(Identifier, Identifier, u64)> {
        self.entries().collect()
    }

    fn entries(&self) -> impl Iterator<Item = (Identifier, Identifier, u64)> {
        self.identities.iter().flat_map(|(identity_id, tokens)| {
            tokens
                .iter()
                .map(move |(token_id, balance)| (*identity_id, *token_id, *balance))
        })
    }

    fn set(&mut self, identity_id: Identifier, token_id: Identifier, balance: Option<u64>) {
        if let Some(balance) = balance {
            self.identities
                .entry(identity_id)
                .or_default()
                .insert(token_id, balance);
        } else if let Some(tokens) = self.identities.get_mut(&identity_id) {
            tokens.remove(&token_id);
            if tokens.is_empty() {
                self.identities.remove(&identity_id);
            }
        }
    }

    fn locally_written_since(&self, before: &Self, pair: (Identifier, Identifier)) -> bool {
        self.local_writes.get(&pair).is_some_and(|current| {
            before
                .local_writes
                .get(&pair)
                .is_none_or(|old| !Arc::ptr_eq(old, current))
        })
    }
}

/// Lock-free store of the published [`TokenBalanceSnapshot`]. Held by
/// [`WalletBackend`](super::WalletBackend); the refresh path publishes into
/// it off the UI thread and the view reads from it on the frame thread.
#[derive(Debug, Default)]
pub(super) struct TokenBalanceStore {
    snapshot: ArcSwap<TokenBalanceSnapshot>,
    /// Serializes foreground and periodic sync, including watch registration.
    /// Frame reads and local writes never take this lock.
    pub(super) sync_lock: tokio::sync::Mutex<Option<PendingTokenBalanceRead>>,
}

/// A foreground read can time out after upstream changed some balances.
/// Keep its starting point until those changes can be reconciled.
#[derive(Debug)]
pub(super) struct PendingTokenBalanceRead {
    pub(super) published: Arc<TokenBalanceSnapshot>,
    pub(super) upstream: TokenBalanceSnapshot,
}

impl TokenBalanceStore {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Read the current snapshot. Lock-free, infallible.
    pub(super) fn load(&self) -> Arc<TokenBalanceSnapshot> {
        self.snapshot.load_full()
    }

    /// Replace the snapshot after explicit refresh. Keep local writes that
    /// landed after this network read started.
    pub(super) fn replace(&self, incoming: TokenBalanceSnapshot, before: &TokenBalanceSnapshot) {
        self.snapshot.rcu(|current| {
            let mut next = incoming.clone();
            for (&pair, marker) in &current.local_writes {
                if current.locally_written_since(before, pair) {
                    let (identity_id, token_id) = pair;
                    next.set(identity_id, token_id, current.get(&identity_id, &token_id));
                    next.local_writes.insert(pair, Arc::clone(marker));
                }
            }
            next
        });
    }

    /// Publish only changed upstream pairs after a periodic pass. An
    /// unchanged value may have been retained after a failed batch, so it
    /// cannot replace a local proof. A proof written during the pass wins.
    fn publish_changes(
        &self,
        before: &TokenBalanceSnapshot,
        upstream_before: &TokenBalanceSnapshot,
        upstream_after: &TokenBalanceSnapshot,
    ) -> bool {
        let changed: Vec<_> = upstream_after
            .entries()
            .filter(|(identity, token, amount)| {
                upstream_before.get(identity, token) != Some(*amount)
            })
            .map(|(identity, token, amount)| (identity, token, Some(amount)))
            .chain(
                upstream_before
                    .entries()
                    .filter(|(identity, token, _)| upstream_after.get(identity, token).is_none())
                    .map(|(identity, token, _)| (identity, token, None)),
            )
            .collect();
        if changed.is_empty() {
            return false;
        }

        let mut balances_changed = false;
        self.snapshot.rcu(|current| {
            let mut next = (**current).clone();
            balances_changed = false;
            for &(identity, token, amount) in &changed {
                if !current.locally_written_since(before, (identity, token)) {
                    balances_changed |= current.get(&identity, &token) != amount;
                    next.set(identity, token, amount);
                    next.local_writes.remove(&(identity, token));
                }
            }
            next
        });
        balances_changed
    }

    /// Apply changes from a read that ended before it could publish them.
    pub(super) fn reconcile_pending(
        &self,
        pending: &mut Option<PendingTokenBalanceRead>,
        upstream: &TokenBalanceSnapshot,
    ) -> bool {
        pending
            .take()
            .is_some_and(|read| self.publish_changes(&read.published, &read.upstream, upstream))
    }

    /// Assemble a [`TokenBalanceSnapshot`] from already-converted, per-synced
    /// identity balances. Identities are passed in only when synced — the
    /// caller drops unsynced ones at the upstream boundary.
    pub(super) fn snapshot_from(
        synced: impl IntoIterator<Item = (Identifier, IdentityBalances)>,
    ) -> TokenBalanceSnapshot {
        TokenBalanceSnapshot {
            identities: synced.into_iter().collect(),
            ..Default::default()
        }
    }

    /// Read the upstream cache and expose only its DET-facing balance fields.
    pub(super) async fn read_upstream(
        sync: &IdentitySyncManager<SqlitePersister>,
    ) -> TokenBalanceSnapshot {
        Self::snapshot_from(sync.all_state().await.into_values().filter_map(|state| {
            if state.last_sync_unix == 0 {
                return None;
            }
            let balances = state
                .tokens
                .into_iter()
                .map(|info| (info.token_id, info.balance))
                .collect();
            Some((state.identity_id, balances))
        }))
    }

    /// Surgically set one `(identity, token)` balance in the published
    /// snapshot, leaving every other entry intact. Used to reflect a
    /// proof-derived post-transaction balance immediately, before the next
    /// upstream sync pass confirms it. The identity becomes present
    /// (no longer "syncing") for that token.
    pub(super) fn apply(&self, identity_id: Identifier, token_id: Identifier, balance: u64) {
        self.write_local(identity_id, token_id, Some(balance));
    }

    /// Drop an unwatched pair without waiting for an in-flight network read.
    pub(super) fn remove(&self, identity_id: Identifier, token_id: Identifier) {
        self.write_local(identity_id, token_id, None);
    }

    fn write_local(&self, identity_id: Identifier, token_id: Identifier, amount: Option<u64>) {
        self.snapshot.rcu(|current| {
            let mut next = (**current).clone();
            next.set(identity_id, token_id, amount);
            next.local_writes
                .insert((identity_id, token_id), Arc::new(()));
            next
        });
    }
}

/// The wallet backend owns one periodic token sync session while connected.
/// Owns the timer and joins its task before ending the session.
pub(super) struct PeriodicTokenBalanceSync {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl PeriodicTokenBalanceSync {
    pub(super) fn start(
        manager: Arc<IdentitySyncManager<SqlitePersister>>,
        store: Arc<TokenBalanceStore>,
        network: Network,
        sender: SenderAsync<TaskResult>,
    ) -> Self {
        let cancel = CancellationToken::new();
        let session_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            while !session_cancel.is_cancelled() {
                let changed = store.sync_periodically(&manager, &session_cancel).await;
                if changed {
                    let update = TaskResult::CacheUpdated(CacheUpdate {
                        network,
                        domain: CacheDomain::Tokens,
                    });
                    tokio::select! {
                        biased;
                        _ = session_cancel.cancelled() => break,
                        result = sender.send(update) => {
                            if result.is_err() {
                                break;
                            }
                        }
                    }
                }
                tokio::select! {
                    _ = session_cancel.cancelled() => break,
                    _ = tokio::time::sleep(manager.interval()) => {}
                }
            }
        });
        Self { cancel, task }
    }

    pub(super) async fn stop(mut self) {
        self.cancel.cancel();
        if let Err(error) = (&mut self.task).await {
            tracing::warn!(%error, "Could not join the periodic token balance sync");
        }
    }
}

impl Drop for PeriodicTokenBalanceSync {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl TokenBalanceStore {
    async fn sync_periodically(
        &self,
        sync: &IdentitySyncManager<SqlitePersister>,
        cancel: &CancellationToken,
    ) -> bool {
        let mut pending = tokio::select! {
            biased;
            _ = cancel.cancelled() => return false,
            guard = self.sync_lock.lock() => guard,
        };
        let upstream_before = Self::read_upstream(sync).await;
        let recovered = self.reconcile_pending(&mut pending, &upstream_before);
        let before = self.load();
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {},
            _ = sync.sync_now() => {},
        }
        // A canceled pass may already have fetched some identities. Import
        // those changes before joining the old session. Otherwise a later
        // pass that reads the same amounts would have no delta to publish.
        let upstream_after = Self::read_upstream(sync).await;
        self.publish_changes(&before, &upstream_before, &upstream_after) || recovered
    }
}

/// Read the raw per-`(identity, token)` token balance from the lock-free
/// upstream-fed snapshot. Registry decoration (alias / config / order list)
/// stays with the caller; this view owns only the balance slot.
pub struct UpstreamTokenBalances {
    snapshot: Arc<TokenBalanceSnapshot>,
}

impl UpstreamTokenBalances {
    pub(super) fn new(store: &TokenBalanceStore) -> Self {
        Self {
            snapshot: store.load(),
        }
    }

    /// Balance for one `(identity, token)` pair, or `None` when the identity
    /// is not yet synced or the token has no synced balance.
    pub fn get(&self, identity_id: &Identifier, token_id: &Identifier) -> Option<u64> {
        self.snapshot.get(identity_id, token_id)
    }

    /// Every `(identity, token, balance)` triple across synced identities.
    pub fn list(&self) -> Vec<(Identifier, Identifier, u64)> {
        self.snapshot.list()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> Identifier {
        Identifier::from([b; 32])
    }

    /// A periodic pass must reach the frame-readable snapshot without another
    /// foreground refresh, and later passes with the same amounts stay quiet.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn periodic_identity_sync_publishes_changed_token_balances() {
        use crate::backend_task::{CacheDomain, CacheUpdate, TaskResult};
        use crate::context::test_support::test_app_context_for_network;
        use crate::utils::egui_mpsc::SenderAsync;
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::platform::tokens::identity_token_balances::{
            IdentityTokenBalances, IdentityTokenBalancesQuery,
        };
        use std::time::Duration;
        use tokio_util::sync::CancellationToken;

        let directory = tempfile::tempdir().expect("app data directory");
        let network = Network::Regtest;
        let ctx = test_app_context_for_network(directory.path(), network);
        let identity_id = id(1);
        let token_id = id(2);
        let second_token = id(3);
        let reconnect_token = id(4);
        let mut sdk = dash_sdk::SdkBuilder::new_mock()
            .with_network(network)
            .build()
            .expect("mock Platform SDK");
        for (token_ids, balances) in [
            (vec![token_id], vec![(token_id, Some(10))]),
            (
                vec![token_id, second_token],
                vec![(token_id, Some(20)), (second_token, Some(0))],
            ),
            (
                vec![token_id, reconnect_token],
                vec![(token_id, Some(25)), (reconnect_token, Some(0))],
            ),
        ] {
            sdk.mock()
                .expect_fetch_many::<Identifier, u64, _, IdentityTokenBalances>(
                    IdentityTokenBalancesQuery {
                        identity_id,
                        token_ids,
                    },
                    Some(balances.into_iter().collect()),
                )
                .await
                .expect("token balance mock response");
        }
        ctx.sdk.store(Arc::new(sdk));
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        ctx.ensure_wallet_backend(SenderAsync::new(tx, ctx.egui_ctx().clone()))
            .await
            .expect("wallet backend");
        let backend = ctx.wallet_backend().expect("wired wallet backend");
        let upstream = backend.inner.pwm.identity_sync();
        upstream.set_interval(Duration::from_secs(1));
        backend
            .sync_token_balances_now(
                vec![(identity_id, vec![token_id])],
                Duration::from_secs(5),
                &CancellationToken::new(),
            )
            .await
            .expect("initial balance check");
        assert_eq!(
            backend.token_balances().get(&identity_id, &token_id),
            Some(10)
        );

        // A second watch changes the request key so the mock can return the
        // newer amount without reconfiguring a mock used by the running worker.
        backend
            .register_identity_tokens(identity_id, vec![token_id, second_token])
            .await;
        backend.inner.coordinator_gate.on_masternodes_ready();
        backend.start().await.expect("start wallet backend");

        let updated_upstream = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(row) = upstream.state_for_identity(&identity_id).await
                    && row
                        .tokens
                        .iter()
                        .any(|token| token.token_id == token_id && token.balance == 20)
                {
                    break row.last_sync_unix;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        async fn next_update(rx: &mut tokio::sync::mpsc::Receiver<TaskResult>) -> CacheUpdate {
            loop {
                if let TaskResult::CacheUpdated(update) =
                    rx.recv().await.expect("frame loop sender")
                    && update.domain == CacheDomain::Tokens
                {
                    return update;
                }
            }
        }
        let published = tokio::time::timeout(Duration::from_secs(4), next_update(&mut rx)).await;
        let balance = backend.token_balances().get(&identity_id, &token_id);
        let checked_again = tokio::time::timeout(Duration::from_secs(5), async {
            let since = *updated_upstream
                .as_ref()
                .expect("upstream worker fetched the new amount");
            loop {
                if upstream.last_sync_unix_for_identity(&identity_id).await > Some(since) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(
            updated_upstream.is_ok(),
            "upstream worker fetched the new amount"
        );
        assert_eq!(
            balance,
            Some(20),
            "the frame still reads the previous balance"
        );
        assert_eq!(
            published.expect("changed snapshot notifies the frame loop"),
            CacheUpdate {
                network,
                domain: CacheDomain::Tokens,
            }
        );
        assert!(
            checked_again.is_ok(),
            "the upstream worker ran another pass"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(500), next_update(&mut rx))
                .await
                .is_err(),
            "equal balances must not send another notification"
        );

        // A session with a pending pass must stop even while the sync gate is
        // held. The new watch does not publish until the new session starts.
        let serial = backend.inner.token_balances.sync_lock.lock().await;
        backend
            .register_identity_tokens(identity_id, vec![token_id, reconnect_token])
            .await;
        tokio::time::timeout(Duration::from_secs(5), backend.stop_in_place())
            .await
            .expect("disconnect joins the pending worker");
        drop(serial);
        assert_eq!(
            backend.token_balances().get(&identity_id, &token_id),
            Some(20)
        );
        assert!(
            std::iter::from_fn(|| rx.try_recv().ok()).all(|result| !matches!(
                result,
                TaskResult::CacheUpdated(update) if update.domain == CacheDomain::Tokens
            ))
        );
        backend.inner.coordinator_gate.on_masternodes_ready();
        backend.start().await.expect("restart wallet backend");
        let updated = tokio::time::timeout(Duration::from_secs(5), next_update(&mut rx)).await;
        backend.shutdown().await;
        assert_eq!(
            updated
                .expect("reconnect publishes the latest balance")
                .network,
            network
        );
        assert_eq!(
            backend.token_balances().get(&identity_id, &token_id),
            Some(25)
        );
    }

    fn store_with(
        synced: impl IntoIterator<Item = (Identifier, IdentityBalances)>,
    ) -> TokenBalanceStore {
        let store = TokenBalanceStore::new();
        store.replace(TokenBalanceStore::snapshot_from(synced), &store.load());
        store
    }

    #[test]
    fn a_local_proof_wins_over_an_older_pass_but_a_later_change_is_published() {
        let (identity, token, other) = (id(1), id(2), id(3));
        let initial = [(identity, BTreeMap::from([(token, 5), (other, 5)]))];
        let store = store_with(initial.clone());
        let before = store.load();
        // The older pass completed after the local proof and had already
        // fetched a lower amount. Its change to the other token still lands.
        store.apply(identity, token, 20);
        let old_result = TokenBalanceStore::snapshot_from([(
            identity,
            BTreeMap::from([(token, 10), (other, 6)]),
        )]);
        assert!(store.publish_changes(
            &before,
            &TokenBalanceStore::snapshot_from(initial),
            &old_result
        ));
        assert_eq!(store.load().get(&identity, &token), Some(20));
        assert_eq!(store.load().get(&identity, &other), Some(6));

        let before = store.load();
        assert!(!store.publish_changes(&before, &old_result, &old_result));
        assert_eq!(store.load().get(&identity, &token), Some(20));
        let new_result = TokenBalanceStore::snapshot_from([(
            identity,
            BTreeMap::from([(token, 25), (other, 6)]),
        )]);
        assert!(store.publish_changes(&before, &old_result, &new_result));
        assert_eq!(store.load().get(&identity, &token), Some(25));
        assert!(!store.publish_changes(&store.load(), &new_result, &new_result));
    }

    #[test]
    fn a_partial_foreground_pass_is_reconciled_before_an_unchanged_periodic_pass() {
        let (identity, token) = (id(1), id(2));
        let initial = [(identity, BTreeMap::from([(token, 10)]))];
        let store = store_with(initial.clone());
        let mut pending = Some(PendingTokenBalanceRead {
            published: store.load(),
            upstream: TokenBalanceStore::snapshot_from(initial),
        });
        let advanced =
            TokenBalanceStore::snapshot_from([(identity, BTreeMap::from([(token, 25)]))]);
        assert!(store.reconcile_pending(&mut pending, &advanced));
        assert_eq!(store.load().get(&identity, &token), Some(25));
        assert!(!store.reconcile_pending(&mut pending, &advanced));
        assert!(!store.publish_changes(&store.load(), &advanced, &advanced));
    }

    /// TB1: a synced identity's balance is readable through the view.
    #[test]
    fn synced_balance_is_readable() {
        let store = store_with([(id(1), BTreeMap::from([(id(2), 5_000)]))]);
        let view = UpstreamTokenBalances::new(&store);
        assert_eq!(view.get(&id(1), &id(2)), Some(5_000));
    }

    /// TB2: an unsynced identity reads as `None` (syncing), never `0`.
    #[test]
    fn unsynced_identity_reads_none() {
        let store = store_with([(id(1), BTreeMap::from([(id(2), 5_000)]))]);
        let view = UpstreamTokenBalances::new(&store);
        // id(9) never synced — absent from the snapshot.
        assert_eq!(view.get(&id(9), &id(2)), None);
    }

    /// TB3: a synced identity that lacks a balance for a specific token reads
    /// as `None`, not `0` — distinguishing "not fetched" from "zero".
    #[test]
    fn synced_identity_missing_token_reads_none() {
        let store = store_with([(id(1), BTreeMap::from([(id(2), 5_000)]))]);
        let view = UpstreamTokenBalances::new(&store);
        assert_eq!(view.get(&id(1), &id(7)), None);
    }

    /// TB4: a synced identity with an explicit zero balance reads as
    /// `Some(0)` — a real, fetched zero is distinct from "syncing".
    #[test]
    fn explicit_zero_balance_reads_some_zero() {
        let store = store_with([(id(1), BTreeMap::from([(id(2), 0)]))]);
        let view = UpstreamTokenBalances::new(&store);
        assert_eq!(view.get(&id(1), &id(2)), Some(0));
    }

    /// TB5: `list` returns every synced triple and omits unsynced identities.
    #[test]
    fn list_returns_synced_triples_only() {
        let store = store_with([
            (id(1), BTreeMap::from([(id(2), 10), (id(3), 20)])),
            (id(4), BTreeMap::from([(id(5), 30)])),
        ]);
        let view = UpstreamTokenBalances::new(&store);
        let mut got = view.list();
        got.sort_by_key(|(_, _, bal)| *bal);
        assert_eq!(
            got,
            vec![(id(1), id(2), 10), (id(1), id(3), 20), (id(4), id(5), 30)]
        );
    }

    /// TB6: a freshly-constructed store yields an empty snapshot — every
    /// read is "syncing" until the first publish.
    #[test]
    fn empty_store_yields_no_balances() {
        let store = TokenBalanceStore::new();
        let view = UpstreamTokenBalances::new(&store);
        assert_eq!(view.get(&id(1), &id(2)), None);
        assert!(view.list().is_empty());
    }

    /// TB8: `apply` sets one balance without disturbing other entries, and
    /// makes a previously-unsynced identity present for that token.
    #[test]
    fn apply_sets_one_balance_in_place() {
        let store = store_with([(id(1), BTreeMap::from([(id(2), 10)]))]);
        // Update an existing pair and add a brand-new (unsynced) identity.
        store.apply(id(1), id(2), 42);
        store.apply(id(8), id(9), 7);
        let view = UpstreamTokenBalances::new(&store);
        assert_eq!(view.get(&id(1), &id(2)), Some(42));
        assert_eq!(view.get(&id(8), &id(9)), Some(7));
    }

    /// TB7: the view captures the snapshot at construction; a later publish
    /// is not seen by an already-built view (callers build one per read).
    #[test]
    fn view_is_a_point_in_time_read() {
        let store = TokenBalanceStore::new();
        let view = UpstreamTokenBalances::new(&store);
        store.replace(
            TokenBalanceStore::snapshot_from([(id(1), BTreeMap::from([(id(2), 99)]))]),
            &store.load(),
        );
        // The pre-publish view still sees nothing.
        assert_eq!(view.get(&id(1), &id(2)), None);
        // A freshly-built view sees the new snapshot.
        assert_eq!(
            UpstreamTokenBalances::new(&store).get(&id(1), &id(2)),
            Some(99)
        );
    }
}
