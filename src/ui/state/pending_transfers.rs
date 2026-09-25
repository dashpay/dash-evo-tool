//! Scoped, coalesced read-only transfer assessments for the wallet screen.

use crate::backend_task::{BackendTask, wallet::WalletTask};
use crate::model::{pending_transfers::TransferAssessment, wallet::WalletSeedHash};
use dash_sdk::dpp::dashcore::Network;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
const REFRESH_INTERVAL: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Retains the last assessment while a refresh runs, rejecting stale responses.
#[derive(Debug, Default)]
pub struct PendingTransfersState {
    selection: Option<(Network, WalletSeedHash)>,
    assessment: Option<TransferAssessment>,
    pending: Option<(u64, Instant)>,
    last_attempt: Option<Instant>,
    failed: bool,
    refresh_requested: bool,
}

impl PendingTransfersState {
    /// Changing network or wallet invalidates data and in-flight responses together.
    pub fn select(&mut self, network: Network, seed_hash: WalletSeedHash) -> bool {
        if self.selection != Some((network, seed_hash)) {
            *self = Self {
                selection: Some((network, seed_hash)),
                ..Self::default()
            };
            return true;
        }
        false
    }

    /// Queue an explicit refresh without clearing the last successful assessment.
    pub fn refresh(&mut self) {
        if self.pending.is_none() {
            self.refresh_requested = true;
        }
    }

    /// Results may be discarded while a different screen is visible; fetch again on arrival.
    pub fn on_arrival(&mut self) {
        self.pending = None;
        self.refresh_requested = true;
    }

    /// Expire abandoned requests and return at most one assessment every thirty seconds.
    pub fn task(&mut self) -> Option<BackendTask> {
        if let Some((_, started)) = self.pending {
            if started.elapsed() < REQUEST_TIMEOUT {
                return None;
            }
            self.pending = None;
            self.failed = true;
        }
        let (_, seed_hash) = self.selection?;
        let due = self.last_attempt.is_none()
            || (!self.failed
                && self
                    .last_attempt
                    .is_some_and(|last| last.elapsed() >= REFRESH_INTERVAL));
        if !due && !self.refresh_requested {
            return None;
        }
        let request_id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();
        self.pending = Some((request_id, now));
        self.last_attempt = Some(now);
        self.refresh_requested = false;
        Some(BackendTask::WalletTask(
            WalletTask::AssessPlatformTransfers {
                seed_hash,
                request_id,
            },
        ))
    }

    /// Accept only the response belonging to the current network, wallet, and request.
    pub fn accept(
        &mut self,
        network: Network,
        seed_hash: WalletSeedHash,
        request_id: u64,
        assessment: TransferAssessment,
    ) -> bool {
        if self.selection == Some((network, seed_hash))
            && self.pending.is_some_and(|(id, _)| id == request_id)
        {
            self.assessment = Some(assessment);
            self.pending = None;
            self.failed = false;
            return true;
        }
        false
    }

    /// Whether this is the current wallet's in-flight request.
    pub fn matches_request(&self, seed_hash: WalletSeedHash, request_id: u64) -> bool {
        self.selection
            .is_some_and(|(_, wallet)| wallet == seed_hash)
            && self.pending.is_some_and(|(id, _)| id == request_id)
    }

    /// A scoped task failure leaves the last successful data available for inspection.
    pub fn fail(&mut self, seed_hash: WalletSeedHash, request_id: u64) {
        if self.matches_request(seed_hash, request_id) {
            self.pending = None;
            self.failed = true;
        }
    }

    /// Latest successfully loaded wallet observations.
    pub fn assessment(&self) -> Option<&TransferAssessment> {
        self.assessment.as_ref()
    }
    /// Whether an assessment is still running.
    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }
    /// Delay animation for fast local checks.
    pub fn show_progress(&self) -> bool {
        self.pending
            .is_some_and(|(_, started)| started.elapsed() >= Duration::from_secs(1))
    }
    /// Whether the latest request failed or timed out.
    pub fn is_failed(&self) -> bool {
        self.failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(state: &mut PendingTransfersState) -> u64 {
        let Some(BackendTask::WalletTask(WalletTask::AssessPlatformTransfers {
            request_id, ..
        })) = state.task()
        else {
            panic!("assessment task expected")
        };
        request_id
    }
    fn assessment() -> TransferAssessment {
        TransferAssessment {
            transfers: vec![],
            history_complete: true,
            checked_at: 123,
        }
    }

    #[test]
    fn pending_transfers_coalesce_refresh_and_ignore_unrelated_errors() {
        let mut state = PendingTransfersState::default();
        state.select(Network::Testnet, [1; 32]);
        let id = request(&mut state);
        state.refresh();
        assert!(state.task().is_none());
        state.fail([2; 32], id);
        assert!(state.is_loading());
        state.accept(Network::Testnet, [1; 32], id, assessment());
        assert!(!state.is_loading());
        assert!(state.task().is_none());
    }

    #[test]
    fn pending_transfers_reject_results_after_wallet_or_network_switch_and_return() {
        let mut state = PendingTransfersState::default();
        state.select(Network::Testnet, [1; 32]);
        let old = request(&mut state);
        state.select(Network::Mainnet, [1; 32]);
        let current = request(&mut state);
        state.accept(Network::Testnet, [1; 32], old, assessment());
        state.fail([1; 32], old);
        assert!(state.assessment().is_none());
        assert!(state.is_loading());
        state.select(Network::Testnet, [2; 32]);
        state.select(Network::Testnet, [1; 32]);
        let next = request(&mut state);
        assert_ne!(next, current);
        assert_ne!(next, old);
        state.accept(Network::Testnet, [1; 32], old, assessment());
        assert!(state.assessment().is_none());
    }

    #[test]
    fn pending_transfers_failed_refresh_retains_data_and_needs_explicit_retry() {
        let mut state = PendingTransfersState::default();
        state.select(Network::Testnet, [1; 32]);
        let id = request(&mut state);
        state.accept(Network::Testnet, [1; 32], id, assessment());
        state.refresh();
        let id = request(&mut state);
        state.fail([1; 32], id);
        assert!(state.is_failed());
        assert_eq!(state.assessment().unwrap().checked_at, 123);
        assert!(state.task().is_none());
        state.refresh();
        assert!(state.task().is_some());
    }

    #[test]
    fn pending_transfers_returning_to_screen_recovers_discarded_and_timed_out_requests() {
        let mut state = PendingTransfersState::default();
        state.select(Network::Testnet, [1; 32]);
        let old = request(&mut state);
        state.on_arrival();
        let current = request(&mut state);
        assert_ne!(old, current);
        state.accept(Network::Testnet, [1; 32], old, assessment());
        assert!(state.assessment().is_none());
        state.pending = Some((current, Instant::now() - REQUEST_TIMEOUT));
        assert!(state.task().is_none());
        assert!(state.is_failed());
        assert!(!state.is_loading());
        state.refresh();
        assert!(state.task().is_some());
    }
}
