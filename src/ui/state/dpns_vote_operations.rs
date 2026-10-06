//! Per-screen DPNS vote-operation snapshot for immediate-mode render paths.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dpns_voting::{
    DpnsVoteFailure, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteOutcome, DpnsVoteTargetKey,
    DpnsVoteTargetStatus, VoteTiming, authoritative_dpns_vote_outcomes,
};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScheduledDpnsVoteRow {
    pub vote: ScheduledDPNSVote,
    pub journal_target: (DpnsVoteOperationId, DpnsVoteTargetKey),
    pub status: DpnsVoteTargetStatus,
    pub failure: Option<DpnsVoteFailure>,
}

/// Scheduled rows sharing one decision: name × choice × time (VOTE-FR-088).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScheduledDecisionGroup {
    pub contested_name: String,
    pub choice: dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice,
    pub unix_timestamp: u64,
    pub rows: Vec<ScheduledDpnsVoteRow>,
}

/// Group scheduled rows by decision, ordered by name then time; rows keep
/// their input order inside a group.
pub(crate) fn group_scheduled_rows(rows: Vec<ScheduledDpnsVoteRow>) -> Vec<ScheduledDecisionGroup> {
    let mut groups: Vec<ScheduledDecisionGroup> = Vec::new();
    let mut indices = BTreeMap::new();
    for row in rows {
        let key = (
            row.vote.contested_name.clone(),
            row.vote.choice,
            row.vote.unix_timestamp,
        );
        let index = *indices.entry(key).or_insert_with(|| {
            groups.push(ScheduledDecisionGroup {
                contested_name: row.vote.contested_name.clone(),
                choice: row.vote.choice,
                unix_timestamp: row.vote.unix_timestamp,
                rows: Vec::new(),
            });
            groups.len() - 1
        });
        groups[index].rows.push(row);
    }
    groups.sort_by(|a, b| {
        a.contested_name
            .cmp(&b.contested_name)
            .then(a.unix_timestamp.cmp(&b.unix_timestamp))
    });
    groups
}

#[derive(Debug, Clone, Default)]
pub struct DpnsVoteOperationSnapshot {
    operations: Vec<DpnsVoteOperation>,
    published: Option<Arc<[DpnsVoteOperation]>>,
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

    /// Adopt backend progress without reading storage on a render frame.
    ///
    /// Returns each changed outcome with the status it had before, `None` for
    /// a target seen for the first time.
    pub(crate) fn sync_progress(
        &mut self,
        app_context: &AppContext,
    ) -> Option<Vec<(Option<DpnsVoteTargetStatus>, DpnsVoteOutcome)>> {
        let published = app_context.dpns_vote_progress();
        if self
            .published
            .as_ref()
            .is_some_and(|previous| Arc::ptr_eq(previous, &published))
        {
            return None;
        }
        let previous: BTreeMap<_, _> = self
            .operations
            .iter()
            .flat_map(|operation| {
                operation
                    .targets
                    .iter()
                    .map(move |outcome| ((operation.id, &outcome.target.key), outcome))
            })
            .collect();
        let changed = published
            .iter()
            .flat_map(|operation| operation.targets.iter())
            .filter_map(|outcome| {
                let old = previous.get(&(outcome.operation_id, &outcome.target.key));
                old.is_none_or(|old| **old != *outcome)
                    .then(|| (old.map(|old| old.status), outcome.clone()))
            })
            .collect();
        self.replace(published.to_vec());
        self.published = Some(published);
        Some(changed)
    }

    pub(crate) fn read_error(&self) -> Option<&Arc<TaskError>> {
        self.read_error.as_ref()
    }

    pub fn operations(&self) -> &[DpnsVoteOperation] {
        &self.operations
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

    pub(crate) fn scheduled_vote_rows(&self) -> Vec<ScheduledDpnsVoteRow> {
        authoritative_dpns_vote_outcomes(&self.operations, |outcome| {
            matches!(outcome.target.timing, VoteTiming::Scheduled(_))
        })
        .into_values()
        .filter_map(|(operation, outcome)| {
            let VoteTiming::Scheduled(timestamp) = outcome.target.timing else {
                return None;
            };
            Some(ScheduledDpnsVoteRow {
                vote: ScheduledDPNSVote {
                    contested_name: outcome.target.contested_name.clone(),
                    voter_id: outcome.target.key.voter_id,
                    choice: outcome.target.requested_choice,
                    unix_timestamp: timestamp,
                    executed_successfully: outcome.status == DpnsVoteTargetStatus::Confirmed,
                },
                journal_target: (operation.id, outcome.target.key.clone()),
                status: outcome.status,
                failure: outcome.failure,
            })
        })
        .filter(|row| row.status != DpnsVoteTargetStatus::Cancelled)
        .filter(|row| !self.dismissed_schedules.contains(&row.journal_target))
        .collect()
    }

    fn replace(&mut self, operations: Vec<DpnsVoteOperation>) {
        self.target_statuses = operations
            .iter()
            .flat_map(|operation| &operation.targets)
            .filter(|outcome| outcome.status.holds_lock())
            .map(|outcome| (outcome.target.key.clone(), outcome.status))
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
    fn screen_adopts_live_progress_without_reading_storage() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let store = Arc::new(crate::wallet_backend::kv_test_support::FailingKv::default());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(store.clone()));
        let mut snapshot = DpnsVoteOperationSnapshot::load(&ctx);
        let mut operation = operation(DpnsVoteTargetStatus::Queued);
        let key = operation.targets[0].target.key.clone();
        ctx.insert_dpns_vote_operation(&mut operation, None)
            .unwrap();
        store.fail_all_reads(true);
        assert!(snapshot.sync_progress(&ctx).is_some());
        assert_eq!(
            snapshot.target_status(&key),
            Some(DpnsVoteTargetStatus::Queued)
        );
        assert!(snapshot.sync_progress(&ctx).is_none());
        store.fail_all_reads(false);
        ctx.claim_dpns_vote_target(operation.id, &key).unwrap();
        store.fail_all_reads(true);
        assert!(snapshot.sync_progress(&ctx).is_some());
        assert_eq!(
            snapshot.target_status(&key),
            Some(DpnsVoteTargetStatus::Submitting)
        );
        assert!(snapshot.read_error().is_none());
    }

    #[test]
    fn voting_ui_dismissed_terminal_row_preserves_siblings() {
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
        let rows = snapshot.scheduled_vote_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].vote.contested_name, "bob");
        assert_eq!(snapshot.operations()[0].targets[0], original);
    }

    fn operation(status: DpnsVoteTargetStatus) -> DpnsVoteOperation {
        let mut operation = AppContext::new_dpns_vote_operation(vec![DpnsVoteTarget {
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

    /// VOTE-TC-069: one decision on many nodes is one group.
    #[test]
    fn scheduled_rows_group_by_decision() {
        let row = |voter: u8, name: &str, choice, timestamp| ScheduledDpnsVoteRow {
            vote: scheduled_vote(
                Identifier::from([voter; 32]),
                name,
                choice,
                timestamp,
                false,
            ),
            journal_target: (
                DpnsVoteOperationId::from_bytes([1; 16]),
                DpnsVoteTargetKey {
                    network: Network::Testnet,
                    voter_id: Identifier::from([1; 32]),
                    vote_poll_id: Identifier::from([2; 32]),
                },
            ),
            status: DpnsVoteTargetStatus::Scheduled,
            failure: None,
        };
        let lock = ResourceVoteChoice::Lock;
        let mut rows: Vec<_> = (0..24).map(|voter| row(voter, "bob", lock, 50)).collect();
        rows.push(row(1, "alice", lock, 90));
        rows.push(row(2, "alice", ResourceVoteChoice::Abstain, 90));
        rows.push(row(3, "alice", lock, 10));
        let groups = group_scheduled_rows(rows);
        assert_eq!(
            groups
                .iter()
                .map(|group| (
                    group.contested_name.as_str(),
                    group.unix_timestamp,
                    group.rows.len()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("alice", 10, 1),
                ("alice", 90, 1),
                ("alice", 90, 1),
                ("bob", 50, 24)
            ]
        );
    }

    /// A schedule that still holds its lock outranks a newer terminal record
    /// for the same target, as everywhere else that picks an authority.
    #[test]
    fn scheduled_rows_prefer_the_lock_holder_over_newer_history() {
        let pending = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Scheduled,
            ResourceVoteChoice::Lock,
            100,
        );
        let newer_rejected = scheduled_operation(
            20,
            DpnsVoteTargetStatus::Rejected,
            ResourceVoteChoice::Abstain,
            200,
        );
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![pending.clone(), newer_rejected]);

        let rows = snapshot.scheduled_vote_rows();

        assert_eq!(rows.len(), 1);
        assert_eq!(Some(rows[0].journal_target.0), Some(pending.id));
        assert_eq!(rows[0].status, DpnsVoteTargetStatus::Scheduled);
    }

    fn scheduled_operation(
        created_at: u64,
        status: DpnsVoteTargetStatus,
        choice: ResourceVoteChoice,
        timestamp: u64,
    ) -> DpnsVoteOperation {
        let mut operation = AppContext::new_dpns_vote_operation(vec![DpnsVoteTarget {
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

    fn scheduled_vote(
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
    fn scheduled_rows_keep_distinct_polls_and_networks_with_the_same_name() {
        let first = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Scheduled,
            ResourceVoteChoice::Lock,
            100,
        );
        let mut second = first.clone();
        second.targets[0].target.key.vote_poll_id = Identifier::from([99; 32]);
        let mut third = first.clone();
        third.targets[0].target.key.network = Network::Mainnet;
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![first, second, third]);
        assert_eq!(snapshot.scheduled_vote_rows().len(), 3);
    }

    #[test]
    fn scheduled_rows_prefer_the_newest_journal_outcome() {
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
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![older, newer]);

        let rows = snapshot.scheduled_vote_rows();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].journal_target, (expected_id, expected_key));
        assert_eq!(rows[0].status, DpnsVoteTargetStatus::Rejected);
        assert_eq!(rows[0].vote.choice, ResourceVoteChoice::Abstain);
        assert_eq!(rows[0].vote.unix_timestamp, 200);
        assert!(!rows[0].vote.executed_successfully);
    }

    /// Operations stamped in the same millisecond break the tie on the
    /// operation id, not on persisted order — the rule every other site uses.
    #[test]
    fn scheduled_rows_break_created_time_ties_on_the_operation_id() {
        let mut first = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Rejected,
            ResourceVoteChoice::Lock,
            100,
        );
        first.id = DpnsVoteOperationId::from_bytes([2; 16]);
        let mut second = scheduled_operation(
            10,
            DpnsVoteTargetStatus::NotApplied,
            ResourceVoteChoice::Abstain,
            200,
        );
        second.id = DpnsVoteOperationId::from_bytes([1; 16]);
        let expected_id = first.id;
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![first, second]);

        let rows = snapshot.scheduled_vote_rows();

        assert_eq!(rows.len(), 1);
        assert_eq!(Some(rows[0].journal_target.0), Some(expected_id));
        assert_eq!(rows[0].status, DpnsVoteTargetStatus::Rejected);
        assert_eq!(rows[0].vote.unix_timestamp, 100);
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

        assert!(snapshot.scheduled_vote_rows().is_empty());
    }

    #[test]
    fn cancelled_targets_are_not_shown() {
        let cancelled = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Cancelled,
            ResourceVoteChoice::Lock,
            100,
        );
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![cancelled]);

        let rows = snapshot.scheduled_vote_rows();

        assert!(rows.is_empty());
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

        let rows = snapshot.scheduled_vote_rows();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].vote.contested_name, "bob");
        assert_eq!(rows[0].status, DpnsVoteTargetStatus::Scheduled);
    }

    #[test]
    fn scheduled_rows_mark_only_confirmed_votes_as_executed() {
        let confirmed = scheduled_operation(
            10,
            DpnsVoteTargetStatus::Confirmed,
            ResourceVoteChoice::Lock,
            100,
        );
        let mut snapshot = DpnsVoteOperationSnapshot::default();
        snapshot.replace(vec![confirmed]);

        let rows = snapshot.scheduled_vote_rows();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, DpnsVoteTargetStatus::Confirmed);
        assert!(rows[0].vote.executed_successfully);
    }
}
