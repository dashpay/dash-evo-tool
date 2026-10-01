//! What the vote progress drawer and the `Needs attention` row show
//! (VOTE-FR-083/084). Pure: callers pass journal operations and the clock.

use super::{
    DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteOutcome, DpnsVoteTargetStatus, VoteTiming,
    dpns_schedule_is_overdue,
};
use std::collections::BTreeSet;

/// Where one sent target stands, as the drawer groups it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressPhase {
    /// Queued, submitting or confirming.
    Sending,
    /// Sent, result not confirmed yet; DET keeps checking.
    Checking,
    Confirmed,
    /// Rejected, not applied, or failed before submission.
    Failed,
}

/// The drawer phase of an outcome; `None` for scheduled or cancelled targets,
/// which belong to the Scheduled view.
pub fn progress_phase(outcome: &DpnsVoteOutcome) -> Option<ProgressPhase> {
    match outcome.status {
        DpnsVoteTargetStatus::Queued
        | DpnsVoteTargetStatus::Submitting
        | DpnsVoteTargetStatus::Confirming => Some(ProgressPhase::Sending),
        DpnsVoteTargetStatus::Unconfirmed => Some(ProgressPhase::Checking),
        DpnsVoteTargetStatus::Confirmed => Some(ProgressPhase::Confirmed),
        DpnsVoteTargetStatus::Rejected
        | DpnsVoteTargetStatus::FailedBeforeSubmission
        | DpnsVoteTargetStatus::NotApplied => Some(ProgressPhase::Failed),
        DpnsVoteTargetStatus::Scheduled | DpnsVoteTargetStatus::Cancelled => None,
    }
}

/// Header counts of the drawer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProgressCounts {
    pub total: usize,
    pub confirmed: usize,
    pub failed: usize,
    pub sending: usize,
    pub checking: usize,
}

impl ProgressCounts {
    /// Settled targets: confirmed or failed.
    pub fn done(&self) -> usize {
        self.confirmed + self.failed
    }

    /// Whether anything is still on its way or being checked.
    pub fn in_flight(&self) -> bool {
        self.sending + self.checking > 0
    }
}

/// Count the drawer phases across `operations`.
pub fn progress_counts<'a>(
    operations: impl IntoIterator<Item = &'a DpnsVoteOperation>,
) -> ProgressCounts {
    let mut counts = ProgressCounts::default();
    for phase in operations
        .into_iter()
        .flat_map(|operation| &operation.targets)
        .filter_map(progress_phase)
    {
        counts.total += 1;
        match phase {
            ProgressPhase::Sending => counts.sending += 1,
            ProgressPhase::Checking => counts.checking += 1,
            ProgressPhase::Confirmed => counts.confirmed += 1,
            ProgressPhase::Failed => counts.failed += 1,
        }
    }
    counts
}

/// Operations the drawer lists: anything still in flight, plus settled
/// operations created since `since_ms` that the operator has not dismissed.
pub fn drawer_operations<'a>(
    operations: &'a [DpnsVoteOperation],
    since_ms: u64,
    dismissed: &BTreeSet<DpnsVoteOperationId>,
) -> Vec<&'a DpnsVoteOperation> {
    operations
        .iter()
        .filter(|operation| {
            let counts = progress_counts([*operation]);
            if counts.total == 0 {
                return false;
            }
            counts.in_flight()
                || (operation.created_at >= since_ms && !dismissed.contains(&operation.id))
        })
        .collect()
}

/// What the `Needs attention` row summarizes (VOTE-FR-084).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NeedsAttention {
    pub checking: usize,
    pub failed: usize,
    pub missed_schedules: usize,
}

impl NeedsAttention {
    pub fn is_empty(&self) -> bool {
        self.checking + self.failed + self.missed_schedules == 0
    }
}

/// Targets that need the operator: unconfirmed, failed, or schedules missed.
///
/// Failed targets count only from operations created since `since_ms`, so a
/// stale failure from an earlier session does not nag forever; unconfirmed
/// and missed targets still hold their lock and always count.
pub fn needs_attention(
    operations: &[DpnsVoteOperation],
    since_ms: u64,
    now_ms: u64,
) -> NeedsAttention {
    let mut attention = NeedsAttention::default();
    for operation in operations {
        for outcome in &operation.targets {
            match (outcome.status, outcome.target.timing) {
                (DpnsVoteTargetStatus::Unconfirmed, _) => attention.checking += 1,
                (DpnsVoteTargetStatus::Scheduled, VoteTiming::Scheduled(at))
                    if dpns_schedule_is_overdue(at, now_ms) =>
                {
                    attention.missed_schedules += 1;
                }
                _ if operation.created_at >= since_ms
                    && progress_phase(outcome) == Some(ProgressPhase::Failed) =>
                {
                    attention.failed += 1;
                }
                _ => {}
            }
        }
    }
    attention
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dpns_voting::{DpnsVoteTarget, DpnsVoteTargetKey};
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
    use dash_sdk::platform::Identifier;

    fn operation(
        created_at: u64,
        statuses: &[(DpnsVoteTargetStatus, VoteTiming)],
    ) -> DpnsVoteOperation {
        let mut operation = DpnsVoteOperation::new(
            statuses
                .iter()
                .enumerate()
                .map(|(index, (_, timing))| DpnsVoteTarget {
                    key: DpnsVoteTargetKey {
                        network: Network::Testnet,
                        voter_id: Identifier::from([index as u8; 32]),
                        vote_poll_id: Identifier::from([9; 32]),
                    },
                    voter_alias: None,
                    contested_name: "alice".to_owned(),
                    requested_choice: ResourceVoteChoice::Lock,
                    current_choice: None,
                    timing: *timing,
                })
                .collect(),
        );
        operation.created_at = created_at;
        for (outcome, (status, _)) in operation.targets.iter_mut().zip(statuses) {
            outcome.status = *status;
        }
        operation
    }

    use DpnsVoteTargetStatus as S;
    const NOW: VoteTiming = VoteTiming::Now;

    /// VOTE-TC-101: the drawer header counts done, sending and checking.
    #[test]
    fn counts_group_targets_by_phase_and_skip_schedules() {
        let op = operation(
            0,
            &[
                (S::Confirmed, NOW),
                (S::Rejected, NOW),
                (S::Submitting, NOW),
                (S::Queued, NOW),
                (S::Unconfirmed, NOW),
                (S::Scheduled, VoteTiming::Scheduled(10)),
                (S::Cancelled, VoteTiming::Scheduled(10)),
            ],
        );
        let counts = progress_counts([&op]);
        assert_eq!(
            counts,
            ProgressCounts {
                total: 5,
                confirmed: 1,
                failed: 1,
                sending: 2,
                checking: 1,
            }
        );
        assert_eq!(counts.done(), 2);
        assert!(counts.in_flight());
    }

    /// In-flight operations always show; settled ones only from this session
    /// and until dismissed; schedule-only operations never.
    #[test]
    fn drawer_lists_in_flight_and_recent_undismissed_operations() {
        let old_in_flight = operation(1, &[(S::Unconfirmed, NOW)]);
        let old_settled = operation(1, &[(S::Confirmed, NOW)]);
        let recent_settled = operation(100, &[(S::Confirmed, NOW)]);
        let dismissed_settled = operation(100, &[(S::Rejected, NOW)]);
        let schedule_only = operation(100, &[(S::Scheduled, VoteTiming::Scheduled(500))]);
        let operations = vec![
            old_in_flight.clone(),
            old_settled,
            recent_settled.clone(),
            dismissed_settled.clone(),
            schedule_only,
        ];
        let dismissed = BTreeSet::from([dismissed_settled.id, old_in_flight.id]);
        let shown: Vec<_> = drawer_operations(&operations, 50, &dismissed)
            .into_iter()
            .map(|operation| operation.id)
            .collect();
        assert_eq!(shown, vec![old_in_flight.id, recent_settled.id]);
    }

    /// VOTE-TC-102: unconfirmed, failed and missed targets need attention.
    #[test]
    fn needs_attention_counts_each_kind() {
        let operations = vec![
            operation(
                100,
                &[
                    (S::Unconfirmed, NOW),
                    (S::FailedBeforeSubmission, NOW),
                    (S::Scheduled, VoteTiming::Scheduled(1_000)),
                    (S::Confirmed, NOW),
                ],
            ),
            operation(1, &[(S::Rejected, NOW)]),
        ];
        let attention = needs_attention(&operations, 50, 1_000 + 121_000);
        assert_eq!(
            attention,
            NeedsAttention {
                checking: 1,
                failed: 1,
                missed_schedules: 1,
            }
        );
        assert!(needs_attention(&[], 0, 0).is_empty());
    }
}
