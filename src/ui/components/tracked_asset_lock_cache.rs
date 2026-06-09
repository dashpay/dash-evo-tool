//! Per-screen cache of wallets' tracked asset locks.
//!
//! Tracked asset locks live behind the wallet backend's async accessor, which
//! must not be driven from the egui frame loop. Screens fetch them through the
//! App Task System ([`WalletTask::ListTrackedAssetLocks`]) and render from this
//! cache. Entries are keyed by `WalletSeedHash`, so a screen that lists several
//! wallets (e.g. the top-up funding-method gate) fetches each independently.
//!
//! [`WalletTask::ListTrackedAssetLocks`]: crate::backend_task::wallet::WalletTask::ListTrackedAssetLocks

use crate::backend_task::BackendTask;
use crate::backend_task::wallet::WalletTask;
use crate::model::wallet::WalletSeedHash;
use platform_wallet::wallet::asset_lock::tracked::{AssetLockStatus, TrackedAssetLock};
use std::collections::{BTreeMap, BTreeSet};

/// Cached tracked asset locks keyed by wallet, fetched via backend tasks.
///
/// Each wallet fetches at most once: a `seed_hash` is recorded in `requested`
/// at dispatch time, so a slow, empty, or failed fetch never re-dispatches every
/// frame. [`Self::invalidate`] clears both guards to allow a fresh fetch.
#[derive(Default)]
pub struct TrackedAssetLockCache {
    /// Wallets a fetch was dispatched for. A wallet present here is never
    /// re-requested until [`Self::invalidate`] runs.
    requested: BTreeSet<WalletSeedHash>,
    cached: BTreeMap<WalletSeedHash, Vec<TrackedAssetLock>>,
}

impl TrackedAssetLockCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the backend task to dispatch when this wallet has not yet been
    /// fetched, or `None` when a fetch for it was already dispatched. The caller
    /// dispatches the returned task as an
    /// [`AppAction::BackendTask`](crate::app::AppAction::BackendTask).
    pub fn ensure_requested(&mut self, seed_hash: WalletSeedHash) -> Option<BackendTask> {
        if !self.requested.insert(seed_hash) {
            return None;
        }
        Some(BackendTask::WalletTask(WalletTask::ListTrackedAssetLocks {
            seed_hash,
        }))
    }

    /// Marks every not-yet-requested wallet in `seed_hashes` as requested and
    /// returns one fetch task per newly-requested wallet. Use this when a screen
    /// reads several wallets at once (e.g. a funding-method gate) so all fetches
    /// dispatch together as a single
    /// [`AppAction::BackendTasks`](crate::app::AppAction::BackendTasks) — a loop
    /// of `ensure_requested` would lose all but the last task because
    /// `AppAction`'s `|=` keeps only the most recent value.
    pub fn ensure_requested_many(
        &mut self,
        seed_hashes: impl IntoIterator<Item = WalletSeedHash>,
    ) -> Vec<BackendTask> {
        seed_hashes
            .into_iter()
            .filter_map(|seed_hash| self.ensure_requested(seed_hash))
            .collect()
    }

    /// Store a completed fetch for one wallet.
    pub fn store(&mut self, seed_hash: WalletSeedHash, locks: Vec<TrackedAssetLock>) {
        self.requested.insert(seed_hash);
        self.cached.insert(seed_hash, locks);
    }

    /// Drop all cached locks and dispatch guards so the next render re-fetches
    /// every wallet (explicit refresh).
    pub fn invalidate(&mut self) {
        self.requested.clear();
        self.cached.clear();
    }

    /// Whether the locks for `seed_hash` have not arrived yet (a fetch is
    /// pending or in flight). Drives the "Loading asset locks…" state.
    pub fn is_loading(&self, seed_hash: &WalletSeedHash) -> bool {
        !self.cached.contains_key(seed_hash)
    }

    /// The cached locks for `seed_hash`, or `None` until the fetch arrives.
    pub fn get(&self, seed_hash: &WalletSeedHash) -> Option<&[TrackedAssetLock]> {
        self.cached.get(seed_hash).map(Vec::as_slice)
    }

    /// Whether the wallet has at least one still-actionable tracked asset lock
    /// (any status other than `Consumed`). Returns `false` until the fetch
    /// arrives.
    pub fn has_unused(&self, seed_hash: &WalletSeedHash) -> bool {
        self.get(seed_hash).is_some_and(|locks| {
            locks
                .iter()
                .any(|l| !matches!(l.status, AssetLockStatus::Consumed))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED_A: WalletSeedHash = [1u8; 32];
    const SEED_B: WalletSeedHash = [2u8; 32];

    /// The fetch fires exactly once per wallet: the first `ensure_requested`
    /// yields a task, and every subsequent call for the same wallet yields
    /// `None` — even before any result arrives and even after an empty result.
    /// This is the F94 retry-storm guard: an empty or failed fetch must not
    /// re-dispatch every frame.
    #[test]
    fn ensure_requested_fires_once_per_wallet() {
        let mut cache = TrackedAssetLockCache::new();

        assert!(
            cache.ensure_requested(SEED_A).is_some(),
            "first request for a wallet must dispatch"
        );
        assert!(
            cache.is_loading(&SEED_A),
            "no result yet means the loading state holds"
        );
        assert!(
            cache.ensure_requested(SEED_A).is_none(),
            "a second request before the result must not re-dispatch"
        );

        // An empty result (the common case) must stop further dispatches.
        cache.store(SEED_A, Vec::new());
        assert!(
            cache.ensure_requested(SEED_A).is_none(),
            "an empty result must not trigger a retry storm"
        );
        assert!(!cache.is_loading(&SEED_A), "stored result clears loading");
        assert!(cache.get(&SEED_A).is_some_and(|l| l.is_empty()));
    }

    /// Distinct wallets fetch independently — one being cached does not satisfy
    /// or suppress the fetch of another.
    #[test]
    fn wallets_fetch_independently() {
        let mut cache = TrackedAssetLockCache::new();
        cache.ensure_requested(SEED_A);
        cache.store(SEED_A, Vec::new());

        assert!(
            cache.ensure_requested(SEED_B).is_some(),
            "a different wallet must dispatch its own fetch"
        );
        assert!(
            cache.is_loading(&SEED_B),
            "the second wallet is still loading until its result arrives"
        );
        assert!(
            cache.get(&SEED_A).is_some(),
            "the first wallet's cache must remain available"
        );
    }

    /// `ensure_requested_many` returns one task per not-yet-requested wallet and
    /// marks them all, so a multi-wallet screen dispatches every fetch in one
    /// batch. Already-requested wallets and duplicates are skipped.
    #[test]
    fn ensure_requested_many_batches_unrequested_wallets() {
        let mut cache = TrackedAssetLockCache::new();
        cache.ensure_requested(SEED_A); // already requested

        let tasks = cache.ensure_requested_many([SEED_A, SEED_B, SEED_B]);
        assert_eq!(
            tasks.len(),
            1,
            "only the new, de-duplicated wallet (SEED_B) yields a task"
        );
        assert!(
            cache.ensure_requested_many([SEED_A, SEED_B]).is_empty(),
            "a second pass over the same wallets dispatches nothing"
        );
    }

    /// `invalidate` clears both the dispatch guards and the cache so an explicit
    /// refresh re-fetches.
    #[test]
    fn invalidate_allows_refetch() {
        let mut cache = TrackedAssetLockCache::new();
        cache.ensure_requested(SEED_A);
        cache.store(SEED_A, Vec::new());
        assert!(cache.ensure_requested(SEED_A).is_none());

        cache.invalidate();
        assert!(
            cache.ensure_requested(SEED_A).is_some(),
            "invalidate must allow the same wallet to re-fetch"
        );
    }
}
