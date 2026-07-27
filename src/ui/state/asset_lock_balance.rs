//! Per-screen cache of live asset-lock builder maximum amounts.

use crate::backend_task::BackendTask;
use crate::backend_task::wallet::WalletTask;
use crate::model::wallet::WalletSeedHash;
use std::collections::BTreeMap;

enum FetchState {
    Loading,
    Loaded(u64),
    Failed,
}

/// Async fetch state for asset-lock maximum amounts, keyed by wallet.
#[derive(Default)]
pub struct AssetLockBalanceCache {
    states: BTreeMap<WalletSeedHash, FetchState>,
}

impl AssetLockBalanceCache {
    /// Dispatch at most one live-builder query per wallet until invalidated.
    pub fn ensure_requested(&mut self, seed_hash: WalletSeedHash) -> Option<BackendTask> {
        if self.states.contains_key(&seed_hash) {
            return None;
        }
        self.states.insert(seed_hash, FetchState::Loading);
        Some(BackendTask::WalletTask(WalletTask::GetAssetLockMaxAmount {
            seed_hash,
        }))
    }

    /// Store the maximum returned by the live wallet backend.
    pub fn store(&mut self, seed_hash: WalletSeedHash, amount_duffs: u64) {
        self.states
            .insert(seed_hash, FetchState::Loaded(amount_duffs));
    }

    /// Mark one in-flight wallet query retryable after a backend failure.
    pub fn mark_loading_failed(&mut self, seed_hash: &WalletSeedHash) {
        if let Some(state @ FetchState::Loading) = self.states.get_mut(seed_hash) {
            *state = FetchState::Failed;
        }
    }

    /// Return the builder maximum once loaded.
    pub fn get(&self, seed_hash: &WalletSeedHash) -> Option<u64> {
        match self.states.get(seed_hash) {
            Some(FetchState::Loaded(amount_duffs)) => Some(*amount_duffs),
            _ => None,
        }
    }

    /// Whether the query failed and needs an explicit retry.
    pub fn is_failed(&self, seed_hash: &WalletSeedHash) -> bool {
        matches!(self.states.get(seed_hash), Some(FetchState::Failed))
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
