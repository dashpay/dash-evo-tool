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
use std::collections::BTreeMap;

/// Per-wallet fetch state. The absence of an entry is the implicit
/// `NotRequested` state — only `NotRequested` dispatches a fetch.
enum FetchState {
    /// A fetch has been dispatched and no result has arrived yet.
    Loading,
    /// The fetch completed; holds the wallet's tracked asset locks.
    Loaded(Vec<TrackedAssetLock>),
    /// The fetch failed. Does not auto-redispatch; the user retries explicitly.
    Failed,
}

/// Cached tracked asset locks keyed by wallet, fetched via backend tasks.
///
/// Each wallet moves through `NotRequested → Loading → Loaded | Failed`. A fetch
/// is dispatched only from `NotRequested`, so neither an empty result, a slow
/// fetch, nor a failure re-dispatches every frame (the F94 retry-storm guard).
/// [`Self::invalidate`] / [`Self::invalidate_one`] return a wallet to
/// `NotRequested` to re-arm a fetch (refresh and the Retry affordance).
#[derive(Default)]
pub struct TrackedAssetLockCache {
    states: BTreeMap<WalletSeedHash, FetchState>,
}

impl TrackedAssetLockCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the backend task to dispatch when this wallet is `NotRequested`,
    /// or `None` once it is `Loading`, `Loaded`, or `Failed`. The caller
    /// dispatches the returned task as an
    /// [`AppAction::BackendTask`](crate::app::AppAction::BackendTask).
    pub fn ensure_requested(&mut self, seed_hash: WalletSeedHash) -> Option<BackendTask> {
        if self.states.contains_key(&seed_hash) {
            return None;
        }
        self.states.insert(seed_hash, FetchState::Loading);
        Some(BackendTask::WalletTask(WalletTask::ListTrackedAssetLocks {
            seed_hash,
        }))
    }

    /// Marks every `NotRequested` wallet in `seed_hashes` as `Loading` and
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

    /// Record a completed fetch for one wallet (`→ Loaded`).
    pub fn store(&mut self, seed_hash: WalletSeedHash, locks: Vec<TrackedAssetLock>) {
        self.states.insert(seed_hash, FetchState::Loaded(locks));
    }

    /// Move every wallet currently `Loading` to `Failed`. The error
    /// `TaskResult` carries no `seed_hash`, so the screen's error handler routes
    /// here; a single lock fetch is normally in flight, so attribution is
    /// correct. Worst case an unrelated error flips a concurrent fetch to a
    /// retryable "couldn't load" state — graceful, no data loss.
    pub fn mark_loading_failed(&mut self) {
        for state in self.states.values_mut() {
            if matches!(state, FetchState::Loading) {
                *state = FetchState::Failed;
            }
        }
    }

    /// Return one wallet to `NotRequested` so the next render re-dispatches its
    /// fetch (the Retry affordance).
    pub fn invalidate_one(&mut self, seed_hash: &WalletSeedHash) {
        self.states.remove(seed_hash);
    }

    /// Return all wallets to `NotRequested` so the next render re-fetches every
    /// wallet (explicit refresh).
    pub fn invalidate(&mut self) {
        self.states.clear();
    }

    /// Whether the wallet's fetch is in flight (`Loading`). Drives the
    /// "Loading asset locks…" state.
    pub fn is_loading(&self, seed_hash: &WalletSeedHash) -> bool {
        matches!(self.states.get(seed_hash), Some(FetchState::Loading))
    }

    /// Whether the wallet's fetch failed (`Failed`). Drives the retryable
    /// "couldn't load" state.
    pub fn is_failed(&self, seed_hash: &WalletSeedHash) -> bool {
        matches!(self.states.get(seed_hash), Some(FetchState::Failed))
    }

    /// The cached locks for `seed_hash` once `Loaded`, or `None` in every other
    /// state.
    pub fn get(&self, seed_hash: &WalletSeedHash) -> Option<&[TrackedAssetLock]> {
        match self.states.get(seed_hash) {
            Some(FetchState::Loaded(locks)) => Some(locks),
            _ => None,
        }
    }

    /// Whether the wallet has at least one still-actionable tracked asset lock
    /// (any status other than `Consumed`). Returns `false` in every state but
    /// `Loaded`.
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

    /// `invalidate` clears every wallet's state so an explicit refresh re-fetches.
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

    /// A failed fetch must not strand the screen on an infinite spinner.
    /// From `Loading`, `mark_loading_failed` moves to `Failed`; the wallet then
    /// reports failed (not loading) and does NOT re-dispatch — so the screen can
    /// render a retryable "couldn't load" state instead of a stuck spinner.
    #[test]
    fn failed_fetch_is_terminal_until_retry() {
        let mut cache = TrackedAssetLockCache::new();
        cache.ensure_requested(SEED_A);
        assert!(
            cache.is_loading(&SEED_A),
            "precondition: the fetch is loading"
        );

        cache.mark_loading_failed();
        assert!(
            cache.is_failed(&SEED_A),
            "a failed fetch must report failed"
        );
        assert!(
            !cache.is_loading(&SEED_A),
            "a failed fetch must NOT still report loading (no stuck spinner)"
        );
        assert!(
            cache.ensure_requested(SEED_A).is_none(),
            "a failed fetch must not auto-redispatch (no retry storm)"
        );

        // The explicit Retry affordance re-arms the fetch.
        cache.invalidate_one(&SEED_A);
        assert!(
            cache.ensure_requested(SEED_A).is_some(),
            "invalidate_one must re-arm a failed wallet so Retry re-fetches"
        );
        assert!(
            cache.is_loading(&SEED_A),
            "the retry puts the wallet back to loading"
        );
    }

    /// `mark_loading_failed` only touches in-flight fetches: an already-`Loaded`
    /// wallet keeps its data when an unrelated error fires.
    #[test]
    fn mark_loading_failed_spares_loaded_wallets() {
        let mut cache = TrackedAssetLockCache::new();
        cache.ensure_requested(SEED_A);
        cache.store(SEED_A, Vec::new()); // SEED_A is Loaded
        cache.ensure_requested(SEED_B); // SEED_B is Loading

        cache.mark_loading_failed();

        assert!(
            cache.get(&SEED_A).is_some(),
            "a Loaded wallet must keep its data when an unrelated fetch fails"
        );
        assert!(
            !cache.is_failed(&SEED_A),
            "a Loaded wallet must not flip to failed"
        );
        assert!(
            cache.is_failed(&SEED_B),
            "the in-flight fetch flips to failed"
        );
    }
}
