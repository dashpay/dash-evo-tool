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
//! refreshes a lock-free DET-typed snapshot off the UI thread (see
//! [`WalletBackend::refresh_token_balances`](super::WalletBackend::refresh_token_balances))
//! and the view reads that snapshot infallibly via [`ArcSwap`].
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
use dash_sdk::platform::Identifier;

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
}

/// Lock-free store of the published [`TokenBalanceSnapshot`]. Held by
/// [`WalletBackend`](super::WalletBackend); the refresh path publishes into
/// it off the UI thread and the view reads from it on the frame thread.
#[derive(Debug, Default)]
pub(super) struct TokenBalanceStore {
    snapshot: ArcSwap<TokenBalanceSnapshot>,
}

impl TokenBalanceStore {
    pub(super) fn new() -> Self {
        Self {
            snapshot: ArcSwap::from_pointee(TokenBalanceSnapshot::default()),
        }
    }

    /// Read the current snapshot. Lock-free, infallible.
    pub(super) fn load(&self) -> Arc<TokenBalanceSnapshot> {
        self.snapshot.load_full()
    }

    /// Atomically publish a freshly-built snapshot. Called by the refresh
    /// path after it has read upstream state and converted it to DET types.
    pub(super) fn publish(&self, snapshot: TokenBalanceSnapshot) {
        self.snapshot.store(Arc::new(snapshot));
    }

    /// Assemble a [`TokenBalanceSnapshot`] from already-converted, per-synced
    /// identity balances. Identities are passed in only when synced — the
    /// caller drops unsynced ones at the upstream boundary.
    pub(super) fn snapshot_from(
        synced: impl IntoIterator<Item = (Identifier, IdentityBalances)>,
    ) -> TokenBalanceSnapshot {
        TokenBalanceSnapshot {
            identities: synced.into_iter().collect(),
        }
    }

    /// Surgically set one `(identity, token)` balance in the published
    /// snapshot, leaving every other entry intact. Used to reflect a
    /// proof-derived post-transaction balance immediately, before the next
    /// upstream sync pass confirms it. The identity becomes present
    /// (no longer "syncing") for that token.
    pub(super) fn apply(&self, identity_id: Identifier, token_id: Identifier, balance: u64) {
        self.snapshot.rcu(|current| {
            let mut next = (**current).clone();
            next.identities
                .entry(identity_id)
                .or_default()
                .insert(token_id, balance);
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

    fn store_with(
        synced: impl IntoIterator<Item = (Identifier, IdentityBalances)>,
    ) -> TokenBalanceStore {
        let store = TokenBalanceStore::new();
        store.publish(TokenBalanceStore::snapshot_from(synced));
        store
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
        store.publish(TokenBalanceStore::snapshot_from([(
            id(1),
            BTreeMap::from([(id(2), 99)]),
        )]));
        // The pre-publish view still sees nothing.
        assert_eq!(view.get(&id(1), &id(2)), None);
        // A freshly-built view sees the new snapshot.
        assert_eq!(
            UpstreamTokenBalances::new(&store).get(&id(1), &id(2)),
            Some(99)
        );
    }
}
