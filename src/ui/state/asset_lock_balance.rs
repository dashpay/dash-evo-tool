//! Per-screen cache of live asset-lock builder maximum amounts.

use crate::backend_task::BackendTask;
use crate::backend_task::wallet::WalletTask;
use crate::model::wallet::WalletSeedHash;
use std::collections::BTreeMap;

enum FetchState {
    Loading {
        snapshot_generation: u64,
    },
    Loaded {
        snapshot_generation: u64,
        amount_duffs: u64,
    },
    Failed {
        snapshot_generation: u64,
    },
}

impl FetchState {
    fn snapshot_generation(&self) -> u64 {
        match self {
            Self::Loading {
                snapshot_generation,
            }
            | Self::Loaded {
                snapshot_generation,
                ..
            }
            | Self::Failed {
                snapshot_generation,
            } => *snapshot_generation,
        }
    }
}

/// Async fetch state for asset-lock maximum amounts, keyed by wallet.
#[derive(Default)]
pub struct AssetLockBalanceCache {
    states: BTreeMap<WalletSeedHash, FetchState>,
}

impl AssetLockBalanceCache {
    /// Dispatch at most one live-builder query per wallet snapshot generation.
    pub fn ensure_requested(
        &mut self,
        seed_hash: WalletSeedHash,
        snapshot_generation: u64,
    ) -> Option<BackendTask> {
        if self
            .states
            .get(&seed_hash)
            .is_some_and(|state| state.snapshot_generation() == snapshot_generation)
        {
            return None;
        }
        self.states.insert(
            seed_hash,
            FetchState::Loading {
                snapshot_generation,
            },
        );
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
        if self
            .states
            .get(&seed_hash)
            .is_some_and(|state| state.snapshot_generation() == snapshot_generation)
        {
            self.states.insert(
                seed_hash,
                FetchState::Loaded {
                    snapshot_generation,
                    amount_duffs,
                },
            );
        }
    }

    /// Mark one in-flight wallet query retryable after a backend failure.
    pub fn mark_loading_failed(&mut self, seed_hash: &WalletSeedHash, snapshot_generation: u64) {
        if matches!(
            self.states.get(seed_hash),
            Some(FetchState::Loading {
                snapshot_generation: active_generation,
            }) if *active_generation == snapshot_generation
        ) {
            self.states.insert(
                *seed_hash,
                FetchState::Failed {
                    snapshot_generation,
                },
            );
        }
    }

    /// Return the builder maximum once loaded.
    pub fn get(&self, seed_hash: &WalletSeedHash) -> Option<u64> {
        match self.states.get(seed_hash) {
            Some(FetchState::Loaded { amount_duffs, .. }) => Some(*amount_duffs),
            _ => None,
        }
    }

    /// Whether the query failed and needs an explicit retry.
    pub fn is_failed(&self, seed_hash: &WalletSeedHash) -> bool {
        matches!(self.states.get(seed_hash), Some(FetchState::Failed { .. }))
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
        assert_eq!(cache.get(&seed_hash), None);

        cache.store(seed_hash, 7, 1_000);
        assert_eq!(
            cache.get(&seed_hash),
            None,
            "a response for the prior wallet snapshot must not refill the cache"
        );

        cache.store(seed_hash, 8, 900);
        assert_eq!(cache.get(&seed_hash), Some(900));
    }
}
