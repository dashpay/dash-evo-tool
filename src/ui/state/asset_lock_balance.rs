//! Per-screen cache of live asset-lock builder maximum amounts.

use crate::backend_task::BackendTask;
use crate::backend_task::wallet::WalletTask;
use crate::model::wallet::WalletSeedHash;
use std::collections::BTreeMap;

/// Preserves result ordering within one generation sequence without making old
/// high-water marks or unresolved requests permanent dispatch barriers.
struct FetchState {
    request_generation: u64,
    request_final_funds_duffs: u64,
    request_utxo_revision: u64,
    loaded: Option<(u64, u64, u64, u64)>,
    in_flight: Option<(u64, u64, u64)>,
    failed: Option<(u64, u64, u64)>,
}

/// Async fetch state for asset-lock maximum amounts, keyed by wallet.
#[derive(Default)]
pub struct AssetLockBalanceCache {
    states: BTreeMap<WalletSeedHash, FetchState>,
}

impl AssetLockBalanceCache {
    /// Dispatch at most one live-builder query per relevant wallet snapshot.
    ///
    /// A final-funds or eligible-UTXO-composition change supersedes unresolved
    /// work. Irrelevant generation churn keeps the existing request, while a
    /// lower generation starts a fresh sequence after a counter reset.
    pub fn ensure_requested(
        &mut self,
        seed_hash: WalletSeedHash,
        snapshot_generation: u64,
        final_funds_duffs: u64,
        utxo_revision: u64,
    ) -> Option<BackendTask> {
        let state = self.states.entry(seed_hash).or_insert(FetchState {
            request_generation: snapshot_generation,
            request_final_funds_duffs: final_funds_duffs,
            request_utxo_revision: utxo_revision,
            loaded: None,
            in_flight: None,
            failed: None,
        });
        let generation_restarted = snapshot_generation < state.request_generation;
        let final_funds_changed = final_funds_duffs != state.request_final_funds_duffs;
        let utxo_composition_changed = utxo_revision != state.request_utxo_revision;
        if generation_restarted || final_funds_changed || utxo_composition_changed {
            state.request_generation = snapshot_generation;
            state.request_final_funds_duffs = final_funds_duffs;
            state.request_utxo_revision = utxo_revision;
            state.in_flight = None;
            state.failed = None;
            if generation_restarted {
                state.loaded = None;
            }
        } else if snapshot_generation != state.request_generation {
            return None;
        }
        let request_key = (snapshot_generation, final_funds_duffs, utxo_revision);
        if state.in_flight == Some(request_key)
            || state
                .loaded
                .is_some_and(|(generation, final_funds, revision, _)| {
                    (generation, final_funds, revision) == request_key
                })
            || state.failed == Some(request_key)
        {
            return None;
        }
        state.in_flight = Some(request_key);
        state.failed = None;
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
        if let Some((in_flight_generation, in_flight_final_funds, in_flight_revision)) =
            state.in_flight
            && in_flight_generation == snapshot_generation
        {
            state.in_flight = None;
            if (
                state.request_generation,
                state.request_final_funds_duffs,
                state.request_utxo_revision,
            ) == (
                in_flight_generation,
                in_flight_final_funds,
                in_flight_revision,
            ) {
                state.loaded = Some((
                    in_flight_generation,
                    in_flight_final_funds,
                    in_flight_revision,
                    amount_duffs,
                ));
                state.failed = None;
            }
        }
    }

    /// Mark one in-flight wallet query retryable after a backend failure.
    pub fn mark_loading_failed(&mut self, seed_hash: &WalletSeedHash, snapshot_generation: u64) {
        let Some(state) = self.states.get_mut(seed_hash) else {
            return;
        };
        if let Some((in_flight_generation, in_flight_final_funds, in_flight_revision)) =
            state.in_flight
            && in_flight_generation == snapshot_generation
        {
            state.in_flight = None;
            if (
                state.request_generation,
                state.request_final_funds_duffs,
                state.request_utxo_revision,
            ) == (
                in_flight_generation,
                in_flight_final_funds,
                in_flight_revision,
            ) {
                state.failed = Some((
                    in_flight_generation,
                    in_flight_final_funds,
                    in_flight_revision,
                ));
            }
        }
    }

    /// Return the most recent builder maximum, including during revalidation.
    pub fn get(&self, seed_hash: &WalletSeedHash) -> Option<u64> {
        self.states
            .get(seed_hash)
            .and_then(|state| state.loaded.map(|(_, _, _, amount_duffs)| amount_duffs))
    }

    /// Return a quote only when it matches the current validation inputs.
    pub fn get_current(
        &self,
        seed_hash: &WalletSeedHash,
        final_funds_duffs: u64,
        utxo_revision: u64,
    ) -> Option<u64> {
        self.states.get(seed_hash).and_then(|state| {
            state
                .loaded
                .filter(|(_, loaded_final_funds, loaded_revision, _)| {
                    (*loaded_final_funds, *loaded_revision) == (final_funds_duffs, utxo_revision)
                })
                .map(|(_, _, _, amount_duffs)| amount_duffs)
        })
    }

    /// Whether the query failed and needs an explicit retry.
    pub fn is_failed(&self, seed_hash: &WalletSeedHash) -> bool {
        self.states.get(seed_hash).is_some_and(|state| {
            state.failed
                == Some((
                    state.request_generation,
                    state.request_final_funds_duffs,
                    state.request_utxo_revision,
                ))
        })
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
    use crate::wallet_backend::DetWalletBalance;

    #[test]
    fn asset_lock_balance_cache_requeries_and_rejects_stale_results_after_snapshot_change() {
        let seed_hash = [0x29; 32];
        let mut cache = AssetLockBalanceCache::default();

        assert!(matches!(
            cache.ensure_requested(seed_hash, 7, 1_000, 1),
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
            cache.ensure_requested(seed_hash, 8, 900, 2),
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
            cache.ensure_requested(seed_hash, 8, 900, 2).is_none(),
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

        assert!(cache.ensure_requested(seed_hash, 7, 1_000, 1).is_some());
        cache.store(seed_hash, 7, 1_000);
        assert_eq!(cache.get(&seed_hash), Some(1_000));

        assert!(matches!(
            cache.ensure_requested(seed_hash, 2, 1_000, 1),
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
            cache.ensure_requested(seed_hash, 2, 1_000, 1).is_none(),
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

        assert!(cache.ensure_requested(seed_hash, 4, 1_000, 1).is_some());
        assert!(matches!(
            cache.ensure_requested(seed_hash, 5, 900, 2),
            Some(BackendTask::WalletTask(
                WalletTask::GetAssetLockMaxAmount {
                    seed_hash: requested_seed,
                    snapshot_generation: 5,
                }
            )) if requested_seed == seed_hash
        ));
        assert!(
            cache.ensure_requested(seed_hash, 5, 900, 2).is_none(),
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

    #[test]
    fn asset_lock_balance_cache_ignores_generation_change_when_spendable_is_unchanged() {
        let seed_hash = [0x2c; 32];
        let mut cache = AssetLockBalanceCache::default();

        assert!(cache.ensure_requested(seed_hash, 7, 1_000, 1).is_some());
        assert!(
            cache.ensure_requested(seed_hash, 8, 1_000, 1).is_none(),
            "a generation-only change must not restart the live-builder probe"
        );

        cache.store(seed_hash, 7, 900);
        assert_eq!(
            cache.get(&seed_hash),
            Some(900),
            "the original request must remain current after a generation-only change"
        );
    }

    #[test]
    fn asset_lock_balance_cache_requeries_when_spendable_changes() {
        let seed_hash = [0x2d; 32];
        let mut cache = AssetLockBalanceCache::default();

        assert!(cache.ensure_requested(seed_hash, 7, 1_000, 1).is_some());
        assert!(matches!(
            cache.ensure_requested(seed_hash, 8, 1_500, 2),
            Some(BackendTask::WalletTask(
                WalletTask::GetAssetLockMaxAmount {
                    seed_hash: requested_seed,
                    snapshot_generation: 8,
                }
            )) if requested_seed == seed_hash
        ));
    }

    #[test]
    fn asset_lock_balance_cache_requeries_when_unconfirmed_funds_become_final() {
        let seed_hash = [0x2e; 32];
        let mut cache = AssetLockBalanceCache::default();
        let unconfirmed = DetWalletBalance {
            confirmed: 0,
            unconfirmed: 1_000,
            total: 1_000,
        };
        let confirmed = DetWalletBalance {
            confirmed: 1_000,
            unconfirmed: 0,
            total: 1_000,
        };

        assert_eq!(unconfirmed.spendable(), confirmed.spendable());
        assert!(
            cache
                .ensure_requested(seed_hash, 7, unconfirmed.confirmed, 1)
                .is_some()
        );
        cache.store(seed_hash, 7, 0);

        assert!(
            cache
                .ensure_requested(seed_hash, 8, confirmed.confirmed, 2)
                .is_some(),
            "confirmation must re-arm the builder probe even when spendable() is unchanged"
        );
    }

    #[test]
    fn asset_lock_balance_cache_requeries_same_generation_after_loaded_signal_changes() {
        let seed_hash = [0x2f; 32];
        let mut cache = AssetLockBalanceCache::default();

        assert!(cache.ensure_requested(seed_hash, 7, 1_000, 1).is_some());
        cache.store(seed_hash, 7, 900);
        assert_eq!(cache.get(&seed_hash), Some(900));

        assert!(
            cache.ensure_requested(seed_hash, 7, 1_500, 2).is_some(),
            "a changed debounce signal must supersede a loaded result even at the same generation"
        );
        cache.store(seed_hash, 7, 1_400);
        assert_eq!(cache.get(&seed_hash), Some(1_400));
        assert!(
            cache.ensure_requested(seed_hash, 7, 1_500, 2).is_none(),
            "the replacement result must deduplicate its own generation and signal"
        );
    }

    #[test]
    fn asset_lock_balance_cache_blocks_stale_validation_after_utxo_composition_change() {
        let seed_hash = [0x30; 32];
        let mut cache = AssetLockBalanceCache::default();

        assert!(cache.ensure_requested(seed_hash, 7, 1_000, 1).is_some());
        cache.store(seed_hash, 7, 900);
        assert_eq!(cache.get(&seed_hash), Some(900));

        assert!(
            cache.ensure_requested(seed_hash, 8, 1_000, 2).is_some(),
            "a different eligible-UTXO composition must re-arm the builder probe"
        );
        assert_eq!(
            cache.get(&seed_hash),
            Some(900),
            "stale-while-revalidate must keep the prior quote displayable"
        );
        assert_eq!(
            cache.get_current(&seed_hash, 1_000, 2),
            None,
            "validation must not use the stale higher quote for the new composition"
        );

        cache.store(seed_hash, 8, 700);
        assert_eq!(cache.get_current(&seed_hash, 1_000, 2), Some(700));
    }
}
