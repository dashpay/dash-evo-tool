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
//! refreshes a lock-free DET-typed snapshot off the UI thread after each
//! requested sync pass. The frame reads that snapshot via [`ArcSwap`].
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

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use arc_swap::ArcSwap;
use dash_sdk::platform::Identifier;
use platform_wallet::manager::identity_sync::IdentitySyncManager;
use platform_wallet_storage::SqlitePersister;

/// One synced identity's token balances. Presence in
/// [`TokenBalanceSnapshot::identities`] means the identity has completed at
/// least one sync pass; an unsynced identity is simply absent.
type IdentityBalances = BTreeMap<Identifier, u64>;

/// When upstream last completed a pass per identity, in Unix seconds.
pub(super) type SyncStamps = BTreeMap<Identifier, u64>;

/// The identities an upstream pass that ran between `started` and `finished`
/// read. Upstream leaves an identity's stamp alone when its read fails.
pub(super) fn refreshed_between(
    started: &SyncStamps,
    finished: &SyncStamps,
) -> BTreeSet<Identifier> {
    finished
        .iter()
        .filter(|(identity_id, at)| **at > started.get(*identity_id).copied().unwrap_or(0))
        .map(|(identity_id, _)| *identity_id)
        .collect()
}

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
        self.identities
            .iter()
            .flat_map(|(identity_id, tokens)| {
                tokens
                    .iter()
                    .map(move |(token_id, balance)| (*identity_id, *token_id, *balance))
            })
            .collect()
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
    /// A local write [`Self::load_with`] makes as its upstream read starts.
    #[cfg(test)]
    write_during_baseline_read: std::sync::Mutex<Option<(Identifier, Identifier, u64)>>,
}

impl TokenBalanceStore {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Read the current snapshot. Lock-free, infallible.
    pub(super) fn load(&self) -> Arc<TokenBalanceSnapshot> {
        self.snapshot.load_full()
    }

    /// Load the snapshot that [`Self::replace`] compares against, then run the
    /// upstream `read`, so local writes made during the read are kept.
    pub(super) async fn load_with<T>(
        &self,
        read: impl std::future::Future<Output = T>,
    ) -> (Arc<TokenBalanceSnapshot>, T) {
        let before = self.load();
        #[cfg(test)]
        if let Some((identity_id, token_id, balance)) =
            crate::wallet_backend::poison::MutexRecover::lock_recover(
                &self.write_during_baseline_read,
            )
            .take()
        {
            self.apply(identity_id, token_id, balance);
        }
        (before, read.await)
    }

    /// Replace the snapshot after explicit refresh. A local write stays until
    /// a pass that started after it reads its identity: keep the ones that
    /// landed after this network read started, and every one whose identity
    /// is not in `refreshed`.
    pub(super) fn replace(
        &self,
        incoming: TokenBalanceSnapshot,
        before: &TokenBalanceSnapshot,
        refreshed: &BTreeSet<Identifier>,
    ) {
        self.snapshot.rcu(|current| {
            let mut next = incoming.clone();
            for (&pair, marker) in &current.local_writes {
                if current.locally_written_since(before, pair) || !refreshed.contains(&pair.0) {
                    let (identity_id, token_id) = pair;
                    next.set(identity_id, token_id, current.get(&identity_id, &token_id));
                    next.local_writes.insert(pair, Arc::clone(marker));
                }
            }
            next
        });
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

    /// Read the upstream cache: its DET-facing balance fields and when each
    /// identity was last read.
    pub(super) async fn read_upstream(
        sync: &IdentitySyncManager<SqlitePersister>,
    ) -> (TokenBalanceSnapshot, SyncStamps) {
        let state = sync.all_state().await;
        let stamps = state
            .values()
            .map(|state| (state.identity_id, state.last_sync_unix))
            .collect();
        let snapshot = Self::snapshot_from(state.into_values().filter_map(|state| {
            if state.last_sync_unix == 0 {
                return None;
            }
            let balances = state
                .tokens
                .into_iter()
                .map(|info| (info.token_id, info.balance))
                .collect();
            Some((state.identity_id, balances))
        }));
        (snapshot, stamps)
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

    /// A refresh takes its baseline before the upstream pass, so a local
    /// write made during the pass wins over the balances it read.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refresh_keeps_a_local_write_made_during_its_upstream_read() {
        use crate::context::test_support::test_app_context_for_network;
        use crate::utils::egui_mpsc::SenderAsync;
        use dash_sdk::dpp::dashcore::Network;
        use dash_sdk::platform::tokens::identity_token_balances::{
            IdentityTokenBalances, IdentityTokenBalancesQuery,
        };

        let directory = tempfile::tempdir().expect("app data directory");
        let network = Network::Regtest;
        let ctx = test_app_context_for_network(directory.path(), network);
        let (identity_id, token_id) = (id(1), id(2));
        let mut sdk = dash_sdk::SdkBuilder::new_mock()
            .with_network(network)
            .build()
            .expect("mock Platform SDK");
        sdk.mock()
            .expect_fetch_many::<Identifier, u64, _, IdentityTokenBalances>(
                IdentityTokenBalancesQuery {
                    identity_id,
                    token_ids: vec![token_id],
                },
                Some([(token_id, Some(10))].into_iter().collect()),
            )
            .await
            .expect("token balance mock response");
        ctx.sdk.store(Arc::new(sdk));
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        ctx.ensure_wallet_backend(SenderAsync::new(tx, ctx.egui_ctx().clone()))
            .await
            .expect("wallet backend");
        let backend = ctx.wallet_backend().expect("wired wallet backend");
        backend.mark_masternodes_ready_for_test();

        *crate::wallet_backend::poison::MutexRecover::lock_recover(
            &backend.inner.token_balances.write_during_baseline_read,
        ) = Some((identity_id, token_id, 20));
        backend
            .sync_token_balances_now(vec![(identity_id, vec![token_id])])
            .await
            .expect("refresh");
        let refreshed = backend.token_balances().get(&identity_id, &token_id);
        backend.shutdown().await;
        assert_eq!(refreshed, Some(20));
    }

    /// A transaction's proof-derived balance stays until an upstream pass
    /// that ran after it reads the pair: a refresh whose read fails does not
    /// bring back the balance from before the transaction.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_refresh_keeps_an_earlier_local_write() {
        use crate::context::test_support::test_app_context_for_network;
        use crate::utils::egui_mpsc::SenderAsync;
        use dash_sdk::dpp::dashcore::Network;

        let directory = tempfile::tempdir().expect("app data directory");
        let network = Network::Regtest;
        let ctx = test_app_context_for_network(directory.path(), network);
        let (identity_id, token_id) = (id(1), id(2));
        // No expected query: every balance read fails.
        let sdk = dash_sdk::SdkBuilder::new_mock()
            .with_network(network)
            .build()
            .expect("mock Platform SDK");
        ctx.sdk.store(Arc::new(sdk));
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        ctx.ensure_wallet_backend(SenderAsync::new(tx, ctx.egui_ctx().clone()))
            .await
            .expect("wallet backend");
        let backend = ctx.wallet_backend().expect("wired wallet backend");
        backend.mark_masternodes_ready_for_test();

        backend.apply_known_token_balance(identity_id, token_id, 20);
        let outcome = backend
            .sync_token_balances_now(vec![(identity_id, vec![token_id])])
            .await;
        let refreshed = backend.token_balances().get(&identity_id, &token_id);
        backend.shutdown().await;
        assert_eq!(outcome, Ok(()));
        assert_eq!(refreshed, Some(20));
    }

    fn store_with(
        synced: impl IntoIterator<Item = (Identifier, IdentityBalances)>,
    ) -> TokenBalanceStore {
        let store = TokenBalanceStore::new();
        let synced = TokenBalanceStore::snapshot_from(synced);
        let refreshed = synced.identities.keys().copied().collect();
        store.replace(synced, &store.load(), &refreshed);
        store
    }

    /// A local write that lands while upstream is read is newer than the
    /// balances that read returns.
    #[tokio::test]
    async fn a_local_write_during_the_baseline_read_survives_the_pass() {
        let (identity, token) = (id(1), id(2));
        let initial = [(identity, BTreeMap::from([(token, 5)]))];
        let fetched = TokenBalanceStore::snapshot_from([(identity, BTreeMap::from([(token, 10)]))]);

        let store = store_with(initial);
        let (before, _) = store
            .load_with(async { store.apply(identity, token, 20) })
            .await;
        store.replace(fetched, &before, &BTreeSet::from([identity]));
        assert_eq!(store.load().get(&identity, &token), Some(20));
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
            &BTreeSet::from([id(1)]),
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
