//! Per-screen cache of live asset-lock builder maximum amounts.

use crate::backend_task::BackendTask;
use crate::backend_task::wallet::WalletTask;
use crate::model::wallet::WalletSeedHash;
use std::collections::BTreeMap;

/// Preserves result ordering within one generation sequence without making old
/// high-water marks or unresolved requests permanent dispatch barriers.
struct FetchState {
    latest_snapshot_generation: u64,
    loaded: Option<(u64, u64)>,
    in_flight_generation: Option<u64>,
    failed_generation: Option<u64>,
}

/// Async fetch state for asset-lock maximum amounts, keyed by wallet.
#[derive(Default)]
pub struct AssetLockBalanceCache {
    states: BTreeMap<WalletSeedHash, FetchState>,
}

impl AssetLockBalanceCache {
    /// Dispatch at most one live-builder query per wallet snapshot generation.
    ///
    /// Equal generations deduplicate, higher generations supersede unresolved
    /// work, and a lower generation starts a fresh sequence after a counter reset.
    pub fn ensure_requested(
        &mut self,
        seed_hash: WalletSeedHash,
        snapshot_generation: u64,
    ) -> Option<BackendTask> {
        let state = self.states.entry(seed_hash).or_insert(FetchState {
            latest_snapshot_generation: snapshot_generation,
            loaded: None,
            in_flight_generation: None,
            failed_generation: None,
        });
        if snapshot_generation != state.latest_snapshot_generation {
            let generation_restarted = snapshot_generation < state.latest_snapshot_generation;
            state.latest_snapshot_generation = snapshot_generation;
            state.in_flight_generation = None;
            state.failed_generation = None;
            if generation_restarted {
                state.loaded = None;
            }
        }
        if state.in_flight_generation == Some(snapshot_generation)
            || state
                .loaded
                .is_some_and(|(generation, _)| generation == snapshot_generation)
            || state.failed_generation == Some(snapshot_generation)
        {
            return None;
        }
        state.in_flight_generation = Some(snapshot_generation);
        state.failed_generation = None;
        Some(BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount {
            seed_hash,
            snapshot_generation,
        }))
    }

    /// Store the maximum returned by the live wallet backend.
    pub fn store(
        &mut self,
        seed_hash: WalletSeedHash,
        snapshot_generation: u64,
        amount_duffs: u64,
    ) {
        let Some(state) = self.states.get_mut(&seed_hash) else {
            return;
        };
        if state.in_flight_generation == Some(snapshot_generation) {
            state.in_flight_generation = None;
            if state.latest_snapshot_generation == snapshot_generation {
                state.loaded = Some((snapshot_generation, amount_duffs));
                state.failed_generation = None;
            }
        }
    }

    /// Mark one in-flight wallet query retryable after a backend failure.
    pub fn mark_loading_failed(&mut self, seed_hash: &WalletSeedHash, snapshot_generation: u64) {
        let Some(state) = self.states.get_mut(seed_hash) else {
            return;
        };
        if state.in_flight_generation == Some(snapshot_generation) {
            state.in_flight_generation = None;
            if state.latest_snapshot_generation == snapshot_generation {
                state.failed_generation = Some(snapshot_generation);
            }
        }
    }

    /// Return the builder maximum once loaded.
    pub fn get(&self, seed_hash: &WalletSeedHash) -> Option<u64> {
        self.states
            .get(seed_hash)
            .and_then(|state| state.loaded.map(|(_, amount_duffs)| amount_duffs))
    }

    /// Whether the query failed and needs an explicit retry.
    pub fn is_failed(&self, seed_hash: &WalletSeedHash) -> bool {
        self.states
            .get(seed_hash)
            .is_some_and(|state| state.failed_generation == Some(state.latest_snapshot_generation))
    }

    /// Re-arm one wallet's query.
    pub fn invalidate_one(&mut self, seed_hash: &WalletSeedHash) {
        self.states.remove(seed_hash);
    }

    /// Re-arm all wallet queries after a refresh or context change.
    pub fn invalidate(&mut self) {
        self.states.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::AssetLockBalanceCache;
    use crate::backend_task::BackendTask;
    use crate::backend_task::wallet::WalletTask;

    #[test]
    fn asset_lock_balance_cache_requeries_and_rejects_stale_results_after_snapshot_change() {
        let seed_hash = [0x29; 32];
        let mut cache = AssetLockBalanceCache::default();

        assert!(matches!(
            cache.ensure_requested(seed_hash, 7),
            Some(BackendTask::WalletTask(
                WalletTask::GetAssetLockMaxAmount {
                    seed_hash: requested_seed,
                    snapshot_generation: 7,
                }
            )) if requested_seed == seed_hash
        ));
        cache.store(seed_hash, 7, 1_000);
        assert_eq!(cache.get(&seed_hash), Some(1_000));

        assert!(matches!(
            cache.ensure_requested(seed_hash, 8),
            Some(BackendTask::WalletTask(
                WalletTask::GetAssetLockMaxAmount {
                    seed_hash: requested_seed,
                    snapshot_generation: 8,
                }
            )) if requested_seed == seed_hash
        ));
        assert_eq!(
            cache.get(&seed_hash),
            Some(1_000),
            "the last loaded value must remain displayable while generation 8 refreshes"
        );
        assert!(
            cache.ensure_requested(seed_hash, 8).is_none(),
            "an in-flight refresh for the current generation must not dispatch twice"
        );

        cache.store(seed_hash, 7, 1_000);
        assert_eq!(
            cache.get(&seed_hash),
            Some(1_000),
            "a stale response must not overwrite or erase the last displayable value"
        );

        cache.store(seed_hash, 8, 900);
        assert_eq!(cache.get(&seed_hash), Some(900));
    }

    #[test]
    fn asset_lock_balance_cache_recovers_after_snapshot_generation_regression() {
        let seed_hash = [0x2a; 32];
        let mut cache = AssetLockBalanceCache::default();

        assert!(cache.ensure_requested(seed_hash, 7).is_some());
        cache.store(seed_hash, 7, 1_000);
        assert_eq!(cache.get(&seed_hash), Some(1_000));

        assert!(matches!(
            cache.ensure_requested(seed_hash, 2),
            Some(BackendTask::WalletTask(
                WalletTask::GetAssetLockMaxAmount {
                    seed_hash: requested_seed,
                    snapshot_generation: 2,
                }
            )) if requested_seed == seed_hash
        ));
        assert_eq!(
            cache.get(&seed_hash),
            None,
            "a restarted generation sequence must discard data from the previous sequence"
        );
        assert!(
            cache.ensure_requested(seed_hash, 2).is_none(),
            "the replacement request must still deduplicate its own generation"
        );

        cache.store(seed_hash, 7, 2_000);
        assert_eq!(
            cache.get(&seed_hash),
            None,
            "a late result from before the generation restart must be ignored"
        );
        cache.store(seed_hash, 2, 800);
        assert_eq!(cache.get(&seed_hash), Some(800));
    }

    #[test]
    fn asset_lock_balance_cache_supersedes_stuck_in_flight_request_on_newer_snapshot() {
        let seed_hash = [0x2b; 32];
        let mut cache = AssetLockBalanceCache::default();

        assert!(cache.ensure_requested(seed_hash, 4).is_some());
        assert!(matches!(
            cache.ensure_requested(seed_hash, 5),
            Some(BackendTask::WalletTask(
                WalletTask::GetAssetLockMaxAmount {
                    seed_hash: requested_seed,
                    snapshot_generation: 5,
                }
            )) if requested_seed == seed_hash
        ));
        assert!(
            cache.ensure_requested(seed_hash, 5).is_none(),
            "the superseding request must deduplicate its own generation"
        );

        cache.store(seed_hash, 4, 1_000);
        assert_eq!(
            cache.get(&seed_hash),
            None,
            "the superseded request must not populate the cache"
        );
        cache.store(seed_hash, 5, 900);
        assert_eq!(cache.get(&seed_hash), Some(900));
    }
}
