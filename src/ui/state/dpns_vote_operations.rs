//! Per-screen DPNS vote-operation snapshot for immediate-mode render paths.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dpns_voting::{
    DpnsVoteFailure, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteTargetKey,
    DpnsVoteTargetStatus, VoteTiming,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScheduledDpnsVoteRow {
    pub vote: ScheduledDPNSVote,
    pub journal_target: Option<(DpnsVoteOperationId, DpnsVoteTargetKey)>,
    pub status: DpnsVoteTargetStatus,
    pub failure: Option<DpnsVoteFailure>,
}

#[derive(Debug, Clone, Default)]
pub struct DpnsVoteOperationSnapshot {
    operations: Vec<DpnsVoteOperation>,
    recent_operations: Arc<[DpnsVoteOperation]>,
    dismissed_schedules: BTreeSet<(DpnsVoteOperationId, DpnsVoteTargetKey)>,
    target_statuses: BTreeMap<DpnsVoteTargetKey, DpnsVoteTargetStatus>,
    loaded: bool,
    read_error: Option<Arc<TaskError>>,
}

impl DpnsVoteOperationSnapshot {
    pub fn load(app_context: &AppContext) -> Self {
        let mut snapshot = Self::default();
        let _ = snapshot.refresh(app_context);
        snapshot
    }

    pub fn refresh(&mut self, app_context: &AppContext) -> Result<(), Arc<TaskError>> {
        let result = (|| {
            Ok::<_, TaskError>((
                app_context.dpns_vote_operations()?,
                app_context.dismissed_dpns_vote_schedules()?,
            ))
        })();
        let (operations, dismissals) = result.map_err(|error| {
            let error = Arc::new(error);
            self.read_error = Some(error.clone());
            error
        })?;
        self.replace(operations);
        self.dismissed_schedules = dismissals;
        self.read_error = None;
        Ok(())
    }

    pub(crate) fn read_error(&self) -> Option<&Arc<TaskError>> {
        self.read_error.as_ref()
    }

    pub fn operations(&self) -> &[DpnsVoteOperation] {
        &self.operations
    }

    pub(crate) fn recent_operations(&self) -> Arc<[DpnsVoteOperation]> {
        Arc::clone(&self.recent_operations)
    }

    pub fn operation(&self, id: DpnsVoteOperationId) -> Option<&DpnsVoteOperation> {
        self.operations.iter().find(|operation| operation.id == id)
    }

    pub fn target_status(&self, key: &DpnsVoteTargetKey) -> Option<DpnsVoteTargetStatus> {
        self.target_statuses.get(key).copied()
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn scheduled_vote_rows(
        &self,
        legacy_votes: &[ScheduledDPNSVote],
    ) -> Vec<ScheduledDpnsVoteRow> {
        let mut journal_rows = BTreeMap::<
            (dash_sdk::platform::Identifier, String),
            ((u64, usize), ScheduledDpnsVoteRow),
        >::new();
        for (operation_index, operation) in self.operations.iter().enumerate() {
            for outcome in &operation.targets {
                let VoteTiming::Scheduled(timestamp) = outcome.target.timing else {
                    continue;
                };
                let pair = (
                    outcome.target.key.voter_id,
                    outcome.target.contested_name.clone(),
                );
                let rank = (operation.created_at, operation_index);
                if journal_rows
                    .get(&pair)
                    .is_some_and(|(current_rank, _)| *current_rank > rank)
                {
                    continue;
                }
                journal_rows.insert(
                    pair,
                    (
                        rank,
                        ScheduledDpnsVoteRow {
                            vote: ScheduledDPNSVote {
                                contested_name: outcome.target.contested_name.clone(),
                                voter_id: outcome.target.key.voter_id,
                                choice: outcome.target.requested_choice,
                                unix_timestamp: timestamp,
                                executed_successfully: outcome.status
                                    == DpnsVoteTargetStatus::Confirmed,
                            },
                            journal_target: Some((operation.id, outcome.target.key.clone())),
                            status: outcome.status,
                            failure: outcome.failure,
                        },
                    ),
                );
            }
        }

        let journal_pairs = journal_rows.keys().cloned().collect::<BTreeSet<_>>();
        // A cancelled target is a dismissed schedule. Its pair stays in
        // `journal_pairs` so a mirror row that outlived a best-effort delete
        // cannot bring the dismissed schedule back.
        let mut rows = journal_rows
            .into_values()
            .map(|(_, row)| row)
            .filter(|row| row.status != DpnsVoteTargetStatus::Cancelled)
            .filter(|row| {
                !row.journal_target
                    .as_ref()
                    .is_some_and(|key| self.dismissed_schedules.contains(key))
            })
            .collect::<Vec<_>>();
        rows.extend(
            legacy_votes
                .iter()
                .filter(|vote| {
                    !journal_pairs.contains(&(vote.voter_id, vote.contested_name.clone()))
                })
                .map(|vote| ScheduledDpnsVoteRow {
                    vote: vote.clone(),
                    journal_target: None,
                    failure: None,
                    status: if vote.executed_successfully {
                        DpnsVoteTargetStatus::Confirmed
                    } else {
                        DpnsVoteTargetStatus::Scheduled
                    },
                }),
        );
        rows
    }

    fn replace(&mut self, operations: Vec<DpnsVoteOperation>) {
        self.target_statuses = operations
            .iter()
            .flat_map(|operation| &operation.targets)
            .filter(|outcome| outcome.status.holds_lock())
            .map(|outcome| (outcome.target.key.clone(), outcome.status))
            .collect();
        let mut recent = operations
            .iter()
            .filter(|operation| !operation.targets.is_empty())
            .collect::<Vec<_>>();
        recent.sort_by_key(|operation| operation.created_at);
        // Pending operations must remain reachable even after newer batches complete.
        let mut completed_count = 0;
        self.recent_operations = recent
            .into_iter()
            .rev()
            .filter(|operation| {
                if operation
                    .targets
                    .iter()
                    .any(|outcome| outcome.status.holds_lock())
                {
                    true
                } else {
                    completed_count += 1;
                    completed_count <= 5
                }
            })
            .cloned()
            .collect();
        self.operations = operations;
        self.loaded = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::contested_names::ScheduledDPNSVote;
    use crate::model::dpns_voting::{DpnsVoteTarget, VoteTiming};
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
    use dash_sdk::platform::Identifier;

    #[test]
    fn voting_ui_unresolved_activity_survives_newer_completed_batches() {
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        let mut unresolved = operation(DpnsVoteTargetStatus::Unconfirmed);
        unresolved.created_at = 0;
        let unresolved_id = unresolved.id;
        let mut operations = vec![unresolved];
        for created_at in 1..=8 {
            let mut completed = operation(DpnsVoteTargetStatus::Confirmed);
            completed.created_at = created_at;
            operations.push(completed);
        }
        snapshot.replace(operations);
        let visible = snapshot.recent_operations();
        assert!(
            visible
                .iter()
                .any(|operation| operation.id == unresolved_id)
        );
        assert_eq!(visible.len(), 6);
    }

    #[test]
    fn voting_ui_recent_activity_is_bounded_ordered_and_shared_between_frames() {
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        let operations = (0..20)
            .map(|created_at| {
                let mut operation = operation(DpnsVoteTargetStatus::Confirmed);
                operation.created_at = created_at;
                operation
            })
            .collect();
        snapshot.replace(operations);
        let first = snapshot.recent_operations();
        let second = snapshot.recent_operations();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            first
                .iter()
                .map(|operation| operation.created_at)
                .collect::<Vec<_>>(),
            vec![19, 18, 17, 16, 15]
        );
        assert_eq!(snapshot.operations().len(), 20);
    }

    #[test]
    fn voting_ui_dismissed_terminal_row_suppresses_its_mirror_and_preserves_siblings() {
        let mut operation = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Confirmed,
            ResourceVoteChoice::Lock,
            100,
        );
        let dismissed = (operation.id, operation.targets[0].target.key.clone());
        let original = operation.targets[0].clone();
        let mut sibling = original.clone();
        sibling.target.key.vote_poll_id = Identifier::from([12; 32]);
        sibling.target.contested_name = "bob".into();
        operation.targets.push(sibling);
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![operation]);
        snapshot.dismissed_schedules.insert(dismissed);
        let rows = snapshot.scheduled_vote_rows(&[legacy_vote(
            original.target.key.voter_id,
            "alice",
            ResourceVoteChoice::Lock,
            100,
            true,
        )]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].vote.contested_name, "bob");
        assert_eq!(snapshot.operations()[0].targets[0], original);
    }

    fn operation(status: DpnsVoteTargetStatus) -> DpnsVoteOperation {
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id: Identifier::from([1; 32]),
                vote_poll_id: Identifier::from([2; 32]),
            },
            voter_alias: None,
            contested_name: "dominguez".to_owned(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Now,
        }]);
        operation.targets[0].status = status;
        operation
    }

    #[test]
    fn snapshot_indexes_only_lock_holding_targets() {
        let live = operation(DpnsVoteTargetStatus::Submitting);
        let mut terminal = operation(DpnsVoteTargetStatus::Confirmed);
        terminal.targets[0].target.key.vote_poll_id = Identifier::from([3; 32]);
        let live_key = live.targets[0].target.key.clone();
        let terminal_key = terminal.targets[0].target.key.clone();
        let mut snapshot = DpnsVoteOperationSnapshot::default();

        snapshot.replace(vec![live.clone(), terminal.clone()]);

        assert_eq!(
            snapshot.target_status(&live_key),
            Some(DpnsVoteTargetStatus::Submitting)
        );
        assert_eq!(snapshot.target_status(&terminal_key), None);
        assert_eq!(snapshot.operation(live.id), Some(&live));
        assert_eq!(snapshot.operations(), &[live, terminal]);
        assert!(snapshot.is_loaded());
    }

    fn scheduled_operation(
        created_at: u64,
        status: DpnsVoteTargetStatus,
        choice: ResourceVoteChoice,
        timestamp: u64,
    ) -> DpnsVoteOperation {
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id: Identifier::from([7; 32]),
                vote_poll_id: Identifier::from([8; 32]),
            },
            voter_alias: Some("node-7".to_owned()),
            contested_name: "alice".to_owned(),
            requested_choice: choice,
            current_choice: None,
            timing: VoteTiming::Scheduled(timestamp),
        }]);
        operation.created_at = created_at;
        operation.targets[0].status = status;
        operation
    }

    fn legacy_vote(
        voter_id: Identifier,
        name: &str,
        choice: ResourceVoteChoice,
        timestamp: u64,
        executed_successfully: bool,
    ) -> ScheduledDPNSVote {
        ScheduledDPNSVote {
            contested_name: name.to_owned(),
            voter_id,
            choice,
            unix_timestamp: timestamp,
            executed_successfully,
        }
    }

    #[test]
    fn scheduled_rows_prefer_the_newest_journal_outcome_over_legacy_data() {
        let older = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Confirmed,
            ResourceVoteChoice::Lock,
            100,
        );
        let newer = scheduled_operation(
            20,
            DpnsVoteTargetStatus::Rejected,
            ResourceVoteChoice::Abstain,
            200,
        );
        let expected_id = newer.id;
        let expected_key = newer.targets[0].target.key.clone();
        let legacy = legacy_vote(
            Identifier::from([7; 32]),
            "alice",
            ResourceVoteChoice::Lock,
            999,
            true,
        );
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![older, newer]);

        let rows = snapshot.scheduled_vote_rows(&[legacy]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].journal_target, Some((expected_id, expected_key)));
        assert_eq!(rows[0].status, DpnsVoteTargetStatus::Rejected);
        assert_eq!(rows[0].vote.choice, ResourceVoteChoice::Abstain);
        assert_eq!(rows[0].vote.unix_timestamp, 200);
        assert!(!rows[0].vote.executed_successfully);
    }

    #[test]
    fn scheduled_rows_use_later_persisted_order_when_created_times_match() {
        let first = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Rejected,
            ResourceVoteChoice::Lock,
            100,
        );
        let second = scheduled_operation(
            10,
            DpnsVoteTargetStatus::NotApplied,
            ResourceVoteChoice::Abstain,
            200,
        );
        let expected_id = second.id;
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![first, second]);

        let rows = snapshot.scheduled_vote_rows(&[]);

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].journal_target.as_ref().map(|(id, _)| *id),
            Some(expected_id)
        );
        assert_eq!(rows[0].status, DpnsVoteTargetStatus::NotApplied);
        assert_eq!(rows[0].vote.unix_timestamp, 200);
    }

    /// Removing a scheduled vote cancels its journal target. The row must then
    /// disappear instead of returning with a Remove button that does nothing.
    #[test]
    fn cancelled_scheduled_targets_stop_projecting_a_row() {
        let cancelled = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Cancelled,
            ResourceVoteChoice::Lock,
            100,
        );
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![cancelled]);

        assert!(snapshot.scheduled_vote_rows(&[]).is_empty());
    }

    /// The compatibility-mirror delete is best effort, so a cancelled target
    /// must keep suppressing its legacy row even when the mirror survived.
    #[test]
    fn cancelled_targets_still_suppress_their_legacy_mirror_row() {
        let cancelled = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Cancelled,
            ResourceVoteChoice::Lock,
            100,
        );
        let voter_id = cancelled.targets[0].target.key.voter_id;
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![cancelled]);
        let legacy = [
            legacy_vote(voter_id, "alice", ResourceVoteChoice::Lock, 100, false),
            legacy_vote(
                Identifier::from([11; 32]),
                "unrelated",
                ResourceVoteChoice::Abstain,
                300,
                false,
            ),
        ];

        let rows = snapshot.scheduled_vote_rows(&legacy);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].vote.contested_name, "unrelated");
    }

    /// A bulk schedule is one operation with many targets, so cancelling one of
    /// them leaves the operation live. Only the cancelled row may disappear.
    #[test]
    fn cancelling_one_target_keeps_the_other_rows_of_its_operation() {
        let mut operation = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Cancelled,
            ResourceVoteChoice::Lock,
            100,
        );
        let mut surviving = operation.targets[0].clone();
        surviving.target.key.vote_poll_id = Identifier::from([12; 32]);
        surviving.target.contested_name = "bob".to_owned();
        surviving.status = DpnsVoteTargetStatus::Scheduled;
        operation.targets.push(surviving);
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![operation]);

        let rows = snapshot.scheduled_vote_rows(&[]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].vote.contested_name, "bob");
        assert_eq!(rows[0].status, DpnsVoteTargetStatus::Scheduled);
    }

    #[test]
    fn scheduled_rows_derive_compatibility_execution_only_from_confirmed_status() {
        let confirmed = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Confirmed,
            ResourceVoteChoice::Lock,
            100,
        );
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![confirmed]);

        let rows = snapshot.scheduled_vote_rows(&[]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, DpnsVoteTargetStatus::Confirmed);
        assert!(rows[0].vote.executed_successfully);
    }

    #[test]
    fn scheduled_rows_append_only_unseen_legacy_pairs() {
        let journal = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Scheduled,
            ResourceVoteChoice::Lock,
            100,
        );
        let voter_id = journal.targets[0].target.key.voter_id;
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![journal]);
        let legacy = [
            legacy_vote(voter_id, "alice", ResourceVoteChoice::Abstain, 999, true),
            legacy_vote(
                Identifier::from([9; 32]),
                "confirmed-legacy",
                ResourceVoteChoice::Lock,
                300,
                true,
            ),
            legacy_vote(
                Identifier::from([10; 32]),
                "pending-legacy",
                ResourceVoteChoice::Abstain,
                400,
                false,
            ),
        ];

        let rows = snapshot.scheduled_vote_rows(&legacy);

        assert_eq!(rows.len(), 3);
        assert!(rows.iter().any(|row| {
            row.vote.contested_name == "alice"
                && row.status == DpnsVoteTargetStatus::Scheduled
                && row.journal_target.is_some()
        }));
        assert!(rows.iter().any(|row| {
            row.vote.contested_name == "confirmed-legacy"
                && row.status == DpnsVoteTargetStatus::Confirmed
                && row.journal_target.is_none()
        }));
        assert!(rows.iter().any(|row| {
            row.vote.contested_name == "pending-legacy"
                && row.status == DpnsVoteTargetStatus::Scheduled
                && row.journal_target.is_none()
        }));
    }
}
