//! Per-network voting maintenance policy. AppState supplies readiness and dispatches tasks.

use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult};
use crate::context::migration_status::MigrationState;
use crate::model::dpns_voting::operator::{
    background_refresh_due, background_refresh_incomplete, background_refresh_interval,
    background_refresh_retry_delay,
};
use dash_sdk::dpp::dashcore::Network;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Scheduled voting can resume only after every migration pass has finished.
pub(super) fn migration_allows_scheduled_vote_sweep(state: &MigrationState) -> bool {
    matches!(
        state,
        MigrationState::Ready
            | MigrationState::Success
            | MigrationState::SucceededWithUnreadableData { .. }
    )
}

fn clear_scheduled_vote_sweep_guard_on_error(
    in_progress: &mut BTreeSet<Network>,
    context: &BackendTaskContext,
    error: &TaskError,
) {
    let network = match (context, error) {
        (_, TaskError::ScheduledVoteSweepFailed { network, .. }) => Some(*network),
        (_, TaskError::ScheduledVoteSweepAllAddressesExhausted { network, .. }) => Some(*network),
        (
            BackendTaskContext::ScheduledVoteSweep { network },
            TaskError::BackendTaskFailed { .. }
            | TaskError::BackendTaskPanicked
            | TaskError::BackendTaskBatchAborted,
        ) => Some(*network),
        _ => None,
    };
    if let Some(network) = network {
        in_progress.remove(&network);
    }
}

/// How often local state re-derives the voting attention summary.
const DPNS_ATTENTION_RECOMPUTE_INTERVAL: Duration = Duration::from_secs(60);

/// Sweeps that re-check unconfirmed votes right after startup or vote activity.
const UNCONFIRMED_VOTE_QUICK_CHECKS: u8 = 3;

/// How often unconfirmed votes are re-checked once the quick checks are used.
const UNCONFIRMED_VOTE_RECHECK_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// Paces the sweep's re-check of unconfirmed votes. A vote can stay
/// unconfirmed until its contest closes, so it is queried on a few sweeps and
/// then only every [`UNCONFIRMED_VOTE_RECHECK_INTERVAL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnconfirmedVoteChecks {
    quick_left: u8,
    last_check: Option<Instant>,
}

impl Default for UnconfirmedVoteChecks {
    fn default() -> Self {
        Self {
            quick_left: UNCONFIRMED_VOTE_QUICK_CHECKS,
            last_check: None,
        }
    }
}

impl UnconfirmedVoteChecks {
    /// Whether the sweep starting at `now` re-checks; records the check if so.
    fn take_due(&mut self, now: Instant) -> bool {
        let due = self.quick_left > 0
            || self
                .last_check
                .is_none_or(|last| now.duration_since(last) >= UNCONFIRMED_VOTE_RECHECK_INTERVAL);
        if due {
            self.quick_left = self.quick_left.saturating_sub(1);
            self.last_check = Some(now);
        }
        due
    }
}

/// Whether `result` reports vote activity that can leave a vote unconfirmed.
fn restarts_unconfirmed_vote_checks(result: &BackendTaskSuccessResult) -> bool {
    matches!(
        result,
        BackendTaskSuccessResult::DpnsVoteOperationUpdated { .. }
            | BackendTaskSuccessResult::ScheduledVotesInProgress(_)
    )
}

/// Reads the wall-clock timestamp used by persisted voting state and the drawer.
pub(super) fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn clear_confirmed_vote_recovery_cutoff(
    deferred: &mut BTreeMap<Network, u64>,
    network: Network,
    confirmed_cutoff: Option<u64>,
) -> bool {
    if confirmed_cutoff.is_some() && deferred.get(&network).copied() == confirmed_cutoff {
        deferred.remove(&network);
        true
    } else {
        false
    }
}

/// Owns pacing, in-flight guards, and migration recovery for voting maintenance.
pub(super) struct DpnsBackground {
    last_sweep: Instant,
    deferred_since_ms: BTreeMap<Network, u64>,
    sweeps_in_progress: BTreeSet<Network>,
    unconfirmed_checks: BTreeMap<Network, UnconfirmedVoteChecks>,
    refresh_dispatched_at_ms: BTreeMap<Network, u64>,
    refresh_retry_at_ms: BTreeMap<Network, u64>,
    attention_recomputed_at: BTreeMap<Network, Instant>,
    recovery_last_attempt: BTreeMap<Network, Instant>,
}

impl DpnsBackground {
    /// Starts the periodic sweep clock; migration recovery can dispatch sooner.
    pub(super) fn new(now: Instant) -> Self {
        Self {
            last_sweep: now,
            deferred_since_ms: BTreeMap::new(),
            sweeps_in_progress: BTreeSet::new(),
            unconfirmed_checks: BTreeMap::new(),
            refresh_dispatched_at_ms: BTreeMap::new(),
            refresh_retry_at_ms: BTreeMap::new(),
            attention_recomputed_at: BTreeMap::new(),
            recovery_last_attempt: BTreeMap::new(),
        }
    }

    /// Claims one due sweep while retaining the first migration eligibility cutoff.
    pub(super) fn scheduled_vote_task(
        &mut self,
        network: Network,
        migration_ready: bool,
        network_selection_required: bool,
        now: Instant,
        now_ms: u64,
    ) -> Option<ContestedResourceTask> {
        if !migration_ready {
            self.deferred_since_ms.entry(network).or_insert(now_ms);
            return None;
        }
        if network_selection_required || self.sweeps_in_progress.contains(&network) {
            return None;
        }
        let preserve_eligibility_since_ms = self.deferred_since_ms.get(&network).copied();
        let recovery_due = preserve_eligibility_since_ms.is_some()
            && self
                .recovery_last_attempt
                .get(&network)
                .is_none_or(|last| now.duration_since(*last) > Duration::from_secs(60));
        let periodic_due = preserve_eligibility_since_ms.is_none()
            && now.duration_since(self.last_sweep) > Duration::from_secs(60);
        if !recovery_due && !periodic_due {
            return None;
        }
        self.last_sweep = now;
        if preserve_eligibility_since_ms.is_some() {
            self.recovery_last_attempt.insert(network, now);
        }
        self.sweeps_in_progress.insert(network);
        let reconcile_unconfirmed = self
            .unconfirmed_checks
            .entry(network)
            .or_default()
            .take_due(now);
        Some(ContestedResourceTask::CastDueScheduledVotes {
            preserve_eligibility_since_ms,
            reconcile_unconfirmed,
        })
    }

    /// Settles sweep guards and restarts quick checks after vote activity.
    pub(super) fn record_vote_result(&mut self, result: &BackendTaskSuccessResult) {
        if restarts_unconfirmed_vote_checks(result) {
            self.unconfirmed_checks.clear();
        }
        if let BackendTaskSuccessResult::ScheduledVoteSweepCompleted {
            network,
            preserve_eligibility_since_ms,
        } = result
        {
            self.sweeps_in_progress.remove(network);
            if clear_confirmed_vote_recovery_cutoff(
                &mut self.deferred_since_ms,
                *network,
                *preserve_eligibility_since_ms,
            ) {
                self.recovery_last_attempt.remove(network);
            }
        }
    }

    /// Releases only the failed sweep's network, retaining its recovery cutoff.
    pub(super) fn sweep_failed(&mut self, context: &BackendTaskContext, error: &TaskError) {
        clear_scheduled_vote_sweep_guard_on_error(&mut self.sweeps_in_progress, context, error);
    }

    /// Claims a local summary update without advancing the clock while gated.
    pub(super) fn attention_recompute_due(
        &mut self,
        network: Network,
        available: bool,
        now: Instant,
    ) -> bool {
        if available
            && self
                .attention_recomputed_at
                .get(&network)
                .is_none_or(|at| now.duration_since(*at) >= DPNS_ATTENTION_RECOMPUTE_INTERVAL)
        {
            self.attention_recomputed_at.insert(network, now);
            true
        } else {
            false
        }
    }

    /// Claims a contest refresh when its normal interval or incomplete-work retry is due.
    pub(super) fn refresh_task(
        &mut self,
        network: Network,
        available: bool,
        completed_at_ms: Option<u64>,
        now_ms: u64,
    ) -> Option<ContestedResourceTask> {
        if !available {
            return None;
        }
        let retry_due = self
            .refresh_retry_at_ms
            .get(&network)
            .is_some_and(|at| now_ms >= *at);
        if retry_due
            || background_refresh_due(
                completed_at_ms,
                self.refresh_dispatched_at_ms.get(&network).copied(),
                now_ms,
                background_refresh_interval(network),
            )
        {
            self.refresh_retry_at_ms.remove(&network);
            self.refresh_dispatched_at_ms.insert(network, now_ms);
            Some(ContestedResourceTask::RefreshContestsInBackground)
        } else {
            None
        }
    }

    /// Schedules a short retry if the originating network did not finish its refresh.
    pub(super) fn refresh_completed(
        &mut self,
        network: Network,
        completed_at_ms: Option<u64>,
        now_ms: u64,
    ) {
        let Some(dispatched_at) = self.refresh_dispatched_at_ms.get(&network).copied() else {
            return;
        };
        if background_refresh_incomplete(completed_at_ms, dispatched_at) {
            let delay = u64::try_from(background_refresh_retry_delay(network).as_millis())
                .unwrap_or(u64::MAX);
            self.refresh_retry_at_ms
                .insert(network, now_ms.saturating_add(delay));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dpns_voting::DpnsVoteOperationId;

    #[test]
    fn aborted_batch_releases_only_its_sweep_guard() {
        let start = Instant::now();
        let mut scheduler = DpnsBackground::new(start);
        for network in [Network::Testnet, Network::Mainnet] {
            assert!(
                scheduler
                    .scheduled_vote_task(network, false, false, start, 42)
                    .is_none()
            );
            assert!(
                scheduler
                    .scheduled_vote_task(network, true, false, start, 43)
                    .is_some()
            );
        }
        scheduler.sweep_failed(
            &BackendTaskContext::ScheduledVoteSweep {
                network: Network::Testnet,
            },
            &TaskError::BackendTaskBatchAborted,
        );
        let retry = start + Duration::from_secs(61);
        assert!(
            scheduler
                .scheduled_vote_task(Network::Testnet, true, false, retry, 44)
                .is_some()
        );
        assert!(
            scheduler
                .scheduled_vote_task(Network::Mainnet, true, false, retry, 44)
                .is_none()
        );
    }

    #[test]
    fn migration_cutoff_survives_failure_and_other_network_completion() {
        let start = Instant::now();
        let mut scheduler = DpnsBackground::new(start);
        assert!(
            scheduler
                .scheduled_vote_task(Network::Testnet, false, false, start, 42)
                .is_none()
        );
        assert!(
            scheduler
                .scheduled_vote_task(Network::Testnet, false, false, start, 99)
                .is_none()
        );
        assert!(
            scheduler
                .scheduled_vote_task(Network::Testnet, true, true, start, 100)
                .is_none()
        );
        let task = scheduler.scheduled_vote_task(Network::Testnet, true, false, start, 101);
        assert!(matches!(
            task,
            Some(ContestedResourceTask::CastDueScheduledVotes {
                preserve_eligibility_since_ms: Some(42),
                ..
            })
        ));
        scheduler.record_vote_result(&BackendTaskSuccessResult::ScheduledVoteSweepCompleted {
            network: Network::Mainnet,
            preserve_eligibility_since_ms: Some(42),
        });
        assert!(
            scheduler
                .scheduled_vote_task(
                    Network::Testnet,
                    true,
                    false,
                    start + Duration::from_secs(61),
                    102
                )
                .is_none()
        );
        scheduler.sweep_failed(
            &BackendTaskContext::ScheduledVoteSweep {
                network: Network::Testnet,
            },
            &TaskError::NoVotingIdentity {
                identity_id: "voter".to_string(),
            },
        );
        assert!(
            scheduler
                .scheduled_vote_task(
                    Network::Testnet,
                    true,
                    false,
                    start + Duration::from_secs(61),
                    103
                )
                .is_none()
        );
        scheduler.sweep_failed(
            &BackendTaskContext::ScheduledVoteSweep {
                network: Network::Testnet,
            },
            &TaskError::ScheduledVoteSweepFailed {
                network: Network::Testnet,
                source: Box::new(TaskError::NoVotingIdentity {
                    identity_id: "voter".to_string(),
                }),
            },
        );
        // A failed recovery retains the original cutoff and still waits for its retry interval.
        assert!(
            scheduler
                .scheduled_vote_task(
                    Network::Testnet,
                    true,
                    false,
                    start + Duration::from_secs(60),
                    103
                )
                .is_none()
        );
        assert!(matches!(
            scheduler.scheduled_vote_task(
                Network::Testnet,
                true,
                false,
                start + Duration::from_secs(61),
                103
            ),
            Some(ContestedResourceTask::CastDueScheduledVotes {
                preserve_eligibility_since_ms: Some(42),
                ..
            })
        ));
        scheduler.record_vote_result(&BackendTaskSuccessResult::ScheduledVoteSweepCompleted {
            network: Network::Testnet,
            preserve_eligibility_since_ms: Some(41),
        });
        assert!(matches!(
            scheduler.scheduled_vote_task(
                Network::Testnet,
                true,
                false,
                start + Duration::from_secs(122),
                104
            ),
            Some(ContestedResourceTask::CastDueScheduledVotes {
                preserve_eligibility_since_ms: Some(42),
                ..
            })
        ));
        scheduler.record_vote_result(&BackendTaskSuccessResult::ScheduledVoteSweepCompleted {
            network: Network::Testnet,
            preserve_eligibility_since_ms: Some(42),
        });
        assert!(matches!(
            scheduler.scheduled_vote_task(
                Network::Testnet,
                true,
                false,
                start + Duration::from_secs(183),
                105
            ),
            Some(ContestedResourceTask::CastDueScheduledVotes {
                preserve_eligibility_since_ms: None,
                ..
            })
        ));
    }

    #[test]
    fn attention_timer_is_per_network_and_does_not_advance_while_gated() {
        let start = Instant::now();
        let mut scheduler = DpnsBackground::new(start);
        assert!(!scheduler.attention_recompute_due(Network::Testnet, false, start));
        assert!(scheduler.attention_recompute_due(Network::Testnet, true, start));
        assert!(!scheduler.attention_recompute_due(
            Network::Testnet,
            true,
            start + Duration::from_secs(59)
        ));
        assert!(scheduler.attention_recompute_due(
            Network::Mainnet,
            true,
            start + Duration::from_secs(59)
        ));
        assert!(!scheduler.attention_recompute_due(
            Network::Testnet,
            false,
            start + Duration::from_secs(60)
        ));
        assert!(scheduler.attention_recompute_due(
            Network::Testnet,
            true,
            start + Duration::from_secs(60)
        ));
    }

    #[test]
    fn background_refresh_retry_respects_gate_and_origin_network() {
        let mut scheduler = DpnsBackground::new(Instant::now());
        assert!(
            scheduler
                .refresh_task(Network::Testnet, false, None, 1)
                .is_none()
        );
        assert!(
            scheduler
                .refresh_task(Network::Testnet, true, None, 1)
                .is_some()
        );
        scheduler.refresh_completed(Network::Testnet, None, 2);
        assert!(
            scheduler
                .refresh_task(Network::Testnet, true, None, 2)
                .is_none()
        );
        let retry_at =
            2 + u64::try_from(background_refresh_retry_delay(Network::Testnet).as_millis())
                .unwrap();
        assert!(
            scheduler
                .refresh_task(Network::Testnet, false, None, retry_at)
                .is_none()
        );
        assert!(
            scheduler
                .refresh_task(Network::Mainnet, true, None, 3)
                .is_some()
        );
        assert!(
            scheduler
                .refresh_task(Network::Testnet, true, None, retry_at)
                .is_some()
        );
    }

    #[test]
    fn scheduled_vote_sweep_waits_for_successful_migration_completion() {
        use crate::context::migration_status::{MigrationState, MigrationStep};

        assert!(!migration_allows_scheduled_vote_sweep(
            &MigrationState::Idle
        ));
        assert!(migration_allows_scheduled_vote_sweep(
            &MigrationState::Ready
        ));
        assert!(!migration_allows_scheduled_vote_sweep(
            &MigrationState::Running {
                step: MigrationStep::Identities,
            },
        ));
        assert!(!migration_allows_scheduled_vote_sweep(
            &MigrationState::AwaitingWalletPasswords {
                wallets: Vec::new(),
            },
        ));
        assert!(migration_allows_scheduled_vote_sweep(
            &MigrationState::Success,
        ));
        assert!(migration_allows_scheduled_vote_sweep(
            &MigrationState::SucceededWithUnreadableData {
                identities: 1,
                top_ups: 1,
            },
        ));
    }

    #[test]
    fn deferred_vote_cutoff_clears_only_after_matching_success() {
        let mut deferred = BTreeMap::from([(Network::Testnet, 42)]);

        assert!(!clear_confirmed_vote_recovery_cutoff(
            &mut deferred,
            Network::Testnet,
            None,
        ));
        assert!(!clear_confirmed_vote_recovery_cutoff(
            &mut deferred,
            Network::Testnet,
            Some(41),
        ));
        assert_eq!(deferred.get(&Network::Testnet), Some(&42));

        assert!(clear_confirmed_vote_recovery_cutoff(
            &mut deferred,
            Network::Testnet,
            Some(42),
        ));
        assert!(!deferred.contains_key(&Network::Testnet));
    }

    #[test]
    fn unconfirmed_votes_are_rechecked_a_few_times_then_rarely() {
        let start = Instant::now();
        let minute = Duration::from_secs(60);
        let mut checks = UnconfirmedVoteChecks::default();
        let due: Vec<bool> = (0..15u32)
            .map(|sweep| checks.take_due(start + minute * sweep))
            .collect();
        let mut expected = vec![false; 15];
        expected[..usize::from(UNCONFIRMED_VOTE_QUICK_CHECKS)].fill(true);
        // The last quick check ran at minute 2; the next is one interval later.
        expected[12] = true;
        assert_eq!(due, expected);

        assert!(restarts_unconfirmed_vote_checks(
            &BackendTaskSuccessResult::DpnsVoteOperationUpdated {
                network: Network::Testnet,
                operation_id: DpnsVoteOperationId::from_bytes([7; 16]),
            }
        ));
        assert!(restarts_unconfirmed_vote_checks(
            &BackendTaskSuccessResult::ScheduledVotesInProgress(Vec::new())
        ));
        assert!(!restarts_unconfirmed_vote_checks(
            &BackendTaskSuccessResult::ScheduledVoteSweepCompleted {
                network: Network::Testnet,
                preserve_eligibility_since_ms: None,
            }
        ));
    }
}
