//! What the vote progress drawer and the `Needs attention` row show
//! (VOTE-FR-083/084). Pure: callers pass journal operations and the clock.

use super::{
    DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteOutcome, DpnsVoteTargetStatus, VoteTiming,
    authoritative_dpns_vote_outcomes, dpns_schedule_is_overdue,
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

/// When a target was sent: its scheduled time for a scheduled vote, otherwise
/// when its operation was created. A vote scheduled in an earlier session
/// therefore counts as recent once it runs in this one, and so does a vote
/// from an earlier session that restart recovery stopped or sent again.
fn sent_at(operation: &DpnsVoteOperation, outcome: &DpnsVoteOutcome) -> u64 {
    let sent = match outcome.target.timing {
        VoteTiming::Scheduled(at) => at.max(operation.created_at),
        VoteTiming::Now => operation.created_at,
    };
    outcome
        .recovered_at_ms
        .map_or(sent, |recovered| recovered.max(sent))
}

/// Operations the drawer lists: anything still in flight, plus operations
/// with a target sent since `since_ms` that the operator has not dismissed.
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
                || (!dismissed.contains(&operation.id)
                    && operation.targets.iter().any(|outcome| {
                        progress_phase(outcome).is_some() && sent_at(operation, outcome) >= since_ms
                    }))
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
/// A failure counts only while it is the latest outcome for its target, the
/// target was sent since `since_ms` and the operator has not dismissed it in
/// the progress drawer. Unconfirmed and missed targets still hold their lock
/// and always count.
pub fn needs_attention(
    operations: &[DpnsVoteOperation],
    since_ms: u64,
    dismissed: &BTreeSet<DpnsVoteOperationId>,
    now_ms: u64,
) -> NeedsAttention {
    let mut attention = NeedsAttention::default();
    for (operation, outcome) in authoritative_dpns_vote_outcomes(operations, |_| true).into_values()
    {
        match (outcome.status, outcome.target.timing) {
            (DpnsVoteTargetStatus::Unconfirmed, _) => attention.checking += 1,
            (DpnsVoteTargetStatus::Scheduled, VoteTiming::Scheduled(at))
                if dpns_schedule_is_overdue(at, now_ms) =>
            {
                attention.missed_schedules += 1;
            }
            _ if sent_at(operation, outcome) >= since_ms
                && !dismissed.contains(&operation.id)
                && progress_phase(outcome) == Some(ProgressPhase::Failed) =>
            {
                attention.failed += 1;
            }
            _ => {}
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
            DpnsVoteOperationId::from_bytes((created_at as u128).to_be_bytes()),
            created_at,
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
        let old_settled = operation(2, &[(S::Confirmed, NOW)]);
        let recent_settled = operation(100, &[(S::Confirmed, NOW)]);
        let dismissed_settled = operation(101, &[(S::Rejected, NOW)]);
        let schedule_only = operation(102, &[(S::Scheduled, VoteTiming::Scheduled(500))]);
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
        let attention = needs_attention(&operations, 50, &BTreeSet::new(), 1_000 + 121_000);
        assert_eq!(
            attention,
            NeedsAttention {
                checking: 1,
                failed: 1,
                missed_schedules: 1,
            }
        );
        assert!(needs_attention(&[], 0, &BTreeSet::new(), 0).is_empty());
    }

    /// A failure stops needing attention once the same target is recast
    /// successfully, or once its operation is dismissed in the drawer.
    #[test]
    fn failures_clear_after_a_recast_or_dismiss() {
        let failed = operation(100, &[(S::FailedBeforeSubmission, NOW)]);
        let recast = operation(200, &[(S::Confirmed, NOW)]);
        let none = BTreeSet::new();
        assert_eq!(
            needs_attention(std::slice::from_ref(&failed), 0, &none, 300).failed,
            1
        );
        assert_eq!(
            needs_attention(&[failed.clone(), recast], 0, &none, 300).failed,
            0
        );
        let dismissed = BTreeSet::from([failed.id]);
        assert_eq!(needs_attention(&[failed], 0, &dismissed, 300).failed, 0);
    }

    /// A scheduled vote is created long before it runs, usually in an earlier
    /// session. Its rejection must still reach the operator when it runs now.
    #[test]
    fn a_vote_scheduled_in_an_earlier_session_is_surfaced_when_it_fails_now() {
        let session_start = 1_000;
        let due_this_session = VoteTiming::Scheduled(session_start + 500);
        let due_before_session = VoteTiming::Scheduled(session_start - 500);
        let rejected_now = operation(10, &[(S::Rejected, due_this_session)]);
        let ended_now = operation(
            11,
            &[
                (S::FailedBeforeSubmission, due_this_session),
                (S::Scheduled, VoteTiming::Scheduled(session_start + 900_000)),
            ],
        );
        let stale = operation(12, &[(S::Rejected, due_before_session)]);
        let mut operations = vec![rejected_now.clone(), ended_now.clone(), stale];
        // Distinct nodes, so no outcome supersedes another.
        for (node, outcome) in operations
            .iter_mut()
            .flat_map(|operation| &mut operation.targets)
            .enumerate()
        {
            outcome.target.key.voter_id = Identifier::from([node as u8 + 10; 32]);
        }
        let none = BTreeSet::new();

        let now = session_start + 1_000;
        assert_eq!(
            needs_attention(&operations, session_start, &none, now),
            NeedsAttention {
                checking: 0,
                failed: 2,
                missed_schedules: 0,
            }
        );
        let shown: Vec<_> = drawer_operations(&operations, session_start, &none)
            .into_iter()
            .map(|operation| operation.id)
            .collect();
        assert_eq!(shown, vec![rejected_now.id, ended_now.id]);

        let dismissed = BTreeSet::from([rejected_now.id, ended_now.id]);
        assert!(needs_attention(&operations, session_start, &dismissed, now).is_empty());
        assert!(drawer_operations(&operations, session_start, &dismissed).is_empty());
    }

    /// An immediate vote left over from an earlier session and settled after
    /// restart recovery picked it up is this session's news: the operator
    /// must see it as not submitted and be able to review it again.
    #[test]
    fn a_vote_recovered_after_a_restart_is_surfaced_in_the_new_session() {
        let session_start = 1_000;
        let mut stopped = operation(10, &[(S::FailedBeforeSubmission, NOW)]);
        stopped.targets[0].recovered_at_ms = Some(session_start + 60);
        let mut old_failure = operation(11, &[(S::FailedBeforeSubmission, NOW)]);
        old_failure.targets[0].target.key.voter_id = Identifier::from([42; 32]);
        let operations = vec![stopped.clone(), old_failure];
        let none = BTreeSet::new();

        assert_eq!(
            needs_attention(&operations, session_start, &none, session_start + 100).failed,
            1
        );
        let shown: Vec<_> = drawer_operations(&operations, session_start, &none)
            .into_iter()
            .map(|operation| operation.id)
            .collect();
        assert_eq!(shown, vec![stopped.id]);

        let later_session = session_start + 10_000;
        assert!(needs_attention(&operations, later_session, &none, later_session).is_empty());
        assert!(drawer_operations(&operations, later_session, &none).is_empty());
    }

    #[test]
    fn attention_uses_authority_for_equal_timestamps_and_lock_holders() {
        let mut failed = operation(100, &[(S::Rejected, NOW)]);
        failed.id = DpnsVoteOperationId([1; 16]);
        failed.targets[0].operation_id = failed.id;
        let mut confirmed = operation(100, &[(S::Confirmed, NOW)]);
        confirmed.id = DpnsVoteOperationId([2; 16]);
        confirmed.targets[0].operation_id = confirmed.id;
        for operations in [
            vec![failed.clone(), confirmed.clone()],
            vec![confirmed, failed.clone()],
        ] {
            assert!(needs_attention(&operations, 0, &BTreeSet::new(), 300).is_empty());
        }
        let checking = operation(50, &[(S::Unconfirmed, NOW)]);
        assert_eq!(
            needs_attention(&[failed, checking], 0, &BTreeSet::new(), 300),
            NeedsAttention {
                checking: 1,
                failed: 0,
                missed_schedules: 0
            }
        );
    }
}
