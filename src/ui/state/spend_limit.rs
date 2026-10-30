//! Per-screen cache of backend-quoted spending limits ("Max" amounts).
//!
//! A limit is only trusted for the wallet state it was quoted against: the
//! screen names that state with a revision (a snapshot generation, a balance
//! fingerprint), and a quote for any other revision is treated as absent until
//! a fresh one arrives.

use crate::backend_task::BackendTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::wallet::WalletTask;
use crate::model::spend_limit::{CoreSendQuoteFee, CoreSendShortfall, SpendLimitKind};
use crate::model::wallet::WalletSeedHash;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// How long an unanswered request blocks a re-dispatch for the same state.
const SPEND_LIMIT_REQUEST_DEADLINE: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    InFlight {
        request_id: u64,
        started_at: Instant,
    },
    Loaded {
        max_credits: u64,
        core_fee: Option<CoreSendQuoteFee>,
    },
    Failed,
    /// Another transaction from the wallet is in progress; no limit is
    /// offered until the revision changes.
    Busy,
    /// The wallet answered that a Core send has nothing to offer, and why.
    Unavailable(CoreSendShortfall),
}

#[derive(Debug, Clone, Copy)]
struct State {
    revision: u64,
    phase: Phase,
}

/// Async fetch state for spending limits, keyed by wallet and kind.
#[derive(Debug, Default)]
pub struct SpendLimitCache {
    states: BTreeMap<(WalletSeedHash, SpendLimitKind), State>,
    next_request_id: u64,
}

impl SpendLimitCache {
    /// The task to dispatch for `revision`, unless one is already loaded,
    /// failed, or recently dispatched for it. A failure is not retried until
    /// the revision changes or [`Self::invalidate`] runs.
    pub fn ensure_requested(
        &mut self,
        seed_hash: WalletSeedHash,
        kind: SpendLimitKind,
        revision: u64,
    ) -> Option<BackendTask> {
        if let Some(state) = self.states.get(&(seed_hash, kind))
            && state.revision == revision
        {
            match state.phase {
                Phase::Loaded { .. } | Phase::Failed | Phase::Busy | Phase::Unavailable(_) => {
                    return None;
                }
                Phase::InFlight { started_at, .. }
                    if started_at.elapsed() < SPEND_LIMIT_REQUEST_DEADLINE =>
                {
                    return None;
                }
                Phase::InFlight { .. } => {}
            }
        }
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request_id = self.next_request_id;
        self.states.insert(
            (seed_hash, kind),
            State {
                revision,
                phase: Phase::InFlight {
                    request_id,
                    started_at: Instant::now(),
                },
            },
        );
        Some(BackendTask::WalletTask(WalletTask::GetSpendLimit {
            seed_hash,
            kind,
            request_id,
        }))
    }

    /// The quoted limit, in credits, if one is loaded for `revision`.
    pub fn get(
        &self,
        seed_hash: &WalletSeedHash,
        kind: SpendLimitKind,
        revision: u64,
    ) -> Option<u64> {
        match self.states.get(&(*seed_hash, kind)) {
            Some(State {
                revision: loaded_for,
                phase: Phase::Loaded { max_credits, .. },
            }) if *loaded_for == revision => Some(*max_credits),
            _ => None,
        }
    }

    /// For a Core send, what the loaded Max for `revision` was proven
    /// against.
    pub fn core_fee(
        &self,
        seed_hash: &WalletSeedHash,
        kind: SpendLimitKind,
        revision: u64,
    ) -> Option<CoreSendQuoteFee> {
        match self.states.get(&(*seed_hash, kind)) {
            Some(State {
                revision: loaded_for,
                phase: Phase::Loaded { core_fee, .. },
            }) if *loaded_for == revision => *core_fee,
            _ => None,
        }
    }

    /// Whether the last request for `revision` failed.
    pub fn is_failed(
        &self,
        seed_hash: &WalletSeedHash,
        kind: SpendLimitKind,
        revision: u64,
    ) -> bool {
        matches!(
            self.states.get(&(*seed_hash, kind)),
            Some(State {
                revision: failed_for,
                phase: Phase::Failed,
            }) if *failed_for == revision
        )
    }

    /// Take a reply. Only the answer to the request currently awaited is
    /// kept; a superseded one is dropped.
    pub fn store(
        &mut self,
        seed_hash: WalletSeedHash,
        kind: SpendLimitKind,
        request_id: u64,
        max_credits: u64,
        core_fee: Option<CoreSendQuoteFee>,
    ) {
        self.settle(
            seed_hash,
            kind,
            request_id,
            Phase::Loaded {
                max_credits,
                core_fee,
            },
        );
    }

    /// Whether the wallet reported another transaction in progress for
    /// `revision`, so no limit is offered.
    pub fn is_busy(&self, seed_hash: &WalletSeedHash, kind: SpendLimitKind, revision: u64) -> bool {
        matches!(
            self.states.get(&(*seed_hash, kind)),
            Some(State {
                revision: busy_for,
                phase: Phase::Busy,
            }) if *busy_for == revision
        )
    }

    /// Why the wallet offers no limit for `revision`, when it said so.
    pub fn unavailable(
        &self,
        seed_hash: &WalletSeedHash,
        kind: SpendLimitKind,
        revision: u64,
    ) -> Option<CoreSendShortfall> {
        match self.states.get(&(*seed_hash, kind)) {
            Some(State {
                revision: answered_for,
                phase: Phase::Unavailable(reason),
            }) if *answered_for == revision => Some(*reason),
            _ => None,
        }
    }

    /// Record why the awaited request returned no limit.
    pub fn mark_request_error(
        &mut self,
        seed_hash: WalletSeedHash,
        kind: SpendLimitKind,
        request_id: u64,
        error: &TaskError,
    ) {
        let phase = match error {
            TaskError::SpendInProgress => Phase::Busy,
            TaskError::CoreSendMaxUnavailable { reason } => Phase::Unavailable(*reason),
            _ => Phase::Failed,
        };
        self.settle(seed_hash, kind, request_id, phase);
    }

    /// Record that the awaited request failed.
    pub fn mark_failed(
        &mut self,
        seed_hash: WalletSeedHash,
        kind: SpendLimitKind,
        request_id: u64,
    ) {
        self.settle(seed_hash, kind, request_id, Phase::Failed);
    }

    /// Forget every quote, so the next frame asks again.
    pub fn invalidate(&mut self) {
        self.states.clear();
    }

    fn settle(
        &mut self,
        seed_hash: WalletSeedHash,
        kind: SpendLimitKind,
        request_id: u64,
        phase: Phase,
    ) {
        if let Some(state) = self.states.get_mut(&(seed_hash, kind))
            && matches!(state.phase, Phase::InFlight { request_id: awaited, .. } if awaited == request_id)
        {
            state.phase = phase;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: WalletSeedHash = [7; 32];

    fn request_id(task: Option<BackendTask>) -> u64 {
        match task {
            Some(BackendTask::WalletTask(WalletTask::GetSpendLimit { request_id, .. })) => {
                request_id
            }
            other => panic!("expected a spend-limit request, got {other:?}"),
        }
    }

    #[test]
    fn a_quote_is_used_only_for_the_revision_it_was_requested_for() {
        let mut cache = SpendLimitCache::default();
        let kind = SpendLimitKind::CoreSend {
            destination: Default::default(),
        };
        let id = request_id(cache.ensure_requested(SEED, kind, 1));
        assert_eq!(
            cache.get(&SEED, kind, 1),
            None,
            "nothing is offered while loading"
        );
        assert!(
            cache.ensure_requested(SEED, kind, 1).is_none(),
            "an in-flight request is not repeated"
        );

        cache.store(SEED, kind, id, 5_000, None);
        assert_eq!(cache.get(&SEED, kind, 1), Some(5_000));
        assert!(cache.ensure_requested(SEED, kind, 1).is_none());

        assert_eq!(
            cache.get(&SEED, kind, 2),
            None,
            "a quote for an older wallet state is not offered"
        );
        assert!(cache.ensure_requested(SEED, kind, 2).is_some());
    }

    #[test]
    fn a_superseded_reply_is_dropped() {
        let mut cache = SpendLimitCache::default();
        let kind = SpendLimitKind::ShieldFromPlatform;
        let stale = request_id(cache.ensure_requested(SEED, kind, 1));
        let current = request_id(cache.ensure_requested(SEED, kind, 2));

        cache.store(SEED, kind, stale, 1, None);
        assert_eq!(cache.get(&SEED, kind, 2), None);
        assert_eq!(cache.get(&SEED, kind, 1), None);

        cache.store(SEED, kind, current, 2, None);
        assert_eq!(cache.get(&SEED, kind, 2), Some(2));
    }

    /// A Core send the wallet says has nothing to offer keeps that reason
    /// for the revision, and is not asked again for it.
    #[test]
    fn an_unavailable_core_send_max_keeps_its_reason() {
        let mut cache = SpendLimitCache::default();
        let kind = SpendLimitKind::CoreSend {
            destination: Default::default(),
        };
        let id = request_id(cache.ensure_requested(SEED, kind, 1));
        cache.mark_request_error(
            SEED,
            kind,
            id,
            &TaskError::CoreSendMaxUnavailable {
                reason: CoreSendShortfall::NothingSpendable,
            },
        );

        assert_eq!(
            cache.unavailable(&SEED, kind, 1),
            Some(CoreSendShortfall::NothingSpendable)
        );
        assert!(!cache.is_failed(&SEED, kind, 1), "no retry is offered");
        assert!(cache.ensure_requested(SEED, kind, 1).is_none());
        assert!(cache.ensure_requested(SEED, kind, 2).is_some());
    }

    #[test]
    fn a_failure_waits_for_new_wallet_state_or_an_invalidation() {
        let mut cache = SpendLimitCache::default();
        let kind = SpendLimitKind::CoreSend {
            destination: Default::default(),
        };
        let id = request_id(cache.ensure_requested(SEED, kind, 1));
        cache.mark_failed(SEED, kind, id);

        assert!(cache.is_failed(&SEED, kind, 1));
        assert_eq!(cache.get(&SEED, kind, 1), None);
        assert!(cache.ensure_requested(SEED, kind, 1).is_none());

        cache.invalidate();
        assert!(!cache.is_failed(&SEED, kind, 1));
        assert!(cache.ensure_requested(SEED, kind, 1).is_some());
    }

    #[test]
    fn kinds_are_cached_independently() {
        let mut cache = SpendLimitCache::default();
        let core = request_id(cache.ensure_requested(
            SEED,
            SpendLimitKind::CoreSend {
                destination: Default::default(),
            },
            1,
        ));
        assert!(
            cache
                .ensure_requested(SEED, SpendLimitKind::ShieldFromPlatform, 1)
                .is_some()
        );
        cache.store(
            SEED,
            SpendLimitKind::CoreSend {
                destination: Default::default(),
            },
            core,
            9,
            None,
        );
        assert_eq!(
            cache.get(
                &SEED,
                SpendLimitKind::CoreSend {
                    destination: Default::default()
                },
                1
            ),
            Some(9)
        );
        assert_eq!(
            cache.get(&SEED, SpendLimitKind::ShieldFromPlatform, 1),
            None
        );
    }
}
