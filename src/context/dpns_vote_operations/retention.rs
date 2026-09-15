//! Retention for completed vote batches and their dismissal sidecars.
//!
//! Only lock-releasing operations are ever retired: unresolved work is never
//! capped, so a target that still holds its lock cannot be pruned away.

use super::keys::*;
use super::{load_operations, rebuild_lock_index};
use crate::backend_task::error::TaskError;
use crate::context::identity_db::{delete_scheduled_vote_in, durable_scheduled_vote_keys};
use crate::model::dpns_voting::{
    DpnsScheduledVoteKey, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteTargetKey,
    DpnsVoteTargetStatus, VoteTiming,
};
use crate::wallet_backend::{DetKv, DetScope, network_prefix};
use dash_sdk::dpp::dashcore::Network;
use std::collections::BTreeSet;

/// Completed immediate batches retained per network; unresolved work is never capped.
pub(super) const DPNS_IMMEDIATE_HISTORY_LIMIT: usize = 256;
pub(super) const DPNS_SCHEDULED_HISTORY_LIMIT: usize = 256;
/// Completed records tolerated above a retention limit before pruning runs.
///
/// Every completing write would otherwise rescan the whole journal, so a bulk
/// vote across a large node fleet serialised one full scan per node against the
/// UI's own journal reads. Eviction is amortised over this many completions
/// instead; the cost is retaining up to that many extra completed records.
pub(super) const DPNS_HISTORY_PRUNE_SLACK: usize = 64;

pub(super) fn dismiss_scheduled_target(
    kv: &DetKv,
    network: Network,
    operation_id: DpnsVoteOperationId,
    key: &DpnsVoteTargetKey,
) -> Result<(), TaskError> {
    let storage_key = schedule_dismissal_key(network, operation_id);
    let mut dismissed: BTreeSet<DpnsVoteTargetKey> = kv
        .get(DetScope::Global, &storage_key)
        .map_err(unreadable_operation_err)?
        .unwrap_or_default();
    if dismissed.insert(key.clone()) {
        kv.put(DetScope::Global, &storage_key, &dismissed)
            .map_err(operation_err)?;
    }
    Ok(())
}

/// Dismiss confirmed schedule rows before clearing their mirrors, under the journal guard.
pub(crate) fn dismiss_confirmed_scheduled_targets(
    kv: &DetKv,
    network: Network,
) -> Result<(), TaskError> {
    for operation in load_operations(kv, network)? {
        for outcome in operation.targets.iter().filter(|outcome| {
            outcome.status == DpnsVoteTargetStatus::Confirmed
                && matches!(outcome.target.timing, VoteTiming::Scheduled(_))
        }) {
            dismiss_scheduled_target(kv, network, operation.id, &outcome.target.key)?;
        }
    }
    Ok(())
}

/// Which retention list a completed operation belongs to.
///
/// Scheduled batches are capped separately because retiring one must also drop
/// the compatibility mirror rows that back it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HistoryKind {
    Immediate,
    Scheduled,
}

impl HistoryKind {
    /// One scheduled target makes the whole batch scheduled history.
    pub(super) fn of(operation: &DpnsVoteOperation) -> Self {
        if operation
            .targets
            .iter()
            .any(|outcome| matches!(outcome.target.timing, VoteTiming::Scheduled(_)))
        {
            Self::Scheduled
        } else {
            Self::Immediate
        }
    }

    pub(super) fn key_prefix_and_limit(self) -> (&'static str, usize) {
        match self {
            Self::Immediate => (IMMEDIATE_HISTORY_KEY_PREFIX, DPNS_IMMEDIATE_HISTORY_LIMIT),
            Self::Scheduled => (SCHEDULED_HISTORY_KEY_PREFIX, DPNS_SCHEDULED_HISTORY_LIMIT),
        }
    }
}

pub(crate) fn prune_terminal_operations(kv: &DetKv, network: Network) -> Result<usize, TaskError> {
    let surviving_scheduled_votes = durable_scheduled_vote_keys(kv, network)?;
    let operations = load_operations(kv, network)?;
    let terminal_ids = operations
        .iter()
        .filter(|operation| {
            operation.is_complete()
                && !operation.targets.is_empty()
                && operation
                    .targets
                    .iter()
                    .any(|outcome| matches!(outcome.target.timing, VoteTiming::Scheduled(_)))
                && operation.targets.iter().all(|outcome| {
                    !matches!(outcome.target.timing, VoteTiming::Scheduled(_))
                        || !surviving_scheduled_votes.contains(&DpnsScheduledVoteKey {
                            network: outcome.target.key.network,
                            voter_id: outcome.target.key.voter_id,
                            contested_name: outcome.target.contested_name.clone(),
                        })
                })
        })
        .map(|operation| operation.id)
        .collect::<Vec<_>>();
    let scheduled_count = delete_terminal_operations(kv, network, &terminal_ids)?;
    let immediate_count = prune_completed_history(kv, network, None, HistoryKind::Immediate)?;
    prune_orphaned_schedule_dismissals(kv, network)?;
    Ok(scheduled_count + immediate_count)
}

pub(super) fn prune_completed_history(
    kv: &DetKv,
    network: Network,
    newly_completed: Option<DpnsVoteOperationId>,
    kind: HistoryKind,
) -> Result<usize, TaskError> {
    let (prefix, limit) = kind.key_prefix_and_limit();
    let history_key = format!("{prefix}{}", network_prefix(network));
    let previous: Vec<DpnsVoteOperationId> = kv
        .get(DetScope::Global, &history_key)
        .map_err(unreadable_operation_err)?
        .unwrap_or_default();
    // Reconciling the completion order against the journal costs a full scan, so
    // a write that merely completes an operation records its order and stops
    // while the slack lasts. Recovery (`newly_completed` is `None`) always
    // reconciles.
    if let Some(id) = newly_completed
        && previous.len() < limit + DPNS_HISTORY_PRUNE_SLACK
    {
        let mut ordered = previous.clone();
        ordered.retain(|existing| *existing != id);
        ordered.push(id);
        if ordered != previous {
            kv.put(DetScope::Global, &history_key, &ordered)
                .map_err(operation_err)?;
        }
        return Ok(0);
    }
    let operations = load_operations(kv, network)?;
    let mut terminal = operations
        .iter()
        .filter(|operation| operation.is_complete() && HistoryKind::of(operation) == kind)
        .collect::<Vec<_>>();
    terminal.sort_unstable_by_key(|operation| (operation.created_at, operation.id));
    let terminal_ids = terminal
        .iter()
        .map(|operation| operation.id)
        .collect::<BTreeSet<_>>();
    let mut ordered = terminal
        .iter()
        .map(|operation| operation.id)
        .filter(|id| !previous.contains(id))
        .collect::<Vec<_>>();
    ordered.extend(
        previous
            .iter()
            .filter(|id| terminal_ids.contains(id))
            .copied(),
    );
    if let Some(id) = newly_completed {
        ordered.retain(|existing| *existing != id);
        ordered.push(id);
    }
    // Persist deletion candidates as well as survivors before deleting records.
    // A partial failure retains the completion order and is retried at recovery.
    if ordered != previous {
        kv.put(DetScope::Global, &history_key, &ordered)
            .map_err(operation_err)?;
    }
    let remove_count = ordered.len().saturating_sub(limit);
    if kind == HistoryKind::Scheduled && remove_count > 0 {
        let removing = ordered[..remove_count]
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let surviving_pairs = operations
            .iter()
            .filter(|operation| !removing.contains(&operation.id))
            .flat_map(|operation| &operation.targets)
            .filter(|outcome| matches!(outcome.target.timing, VoteTiming::Scheduled(_)))
            .map(|outcome| {
                (
                    outcome.target.key.voter_id,
                    outcome.target.contested_name.as_str(),
                )
            })
            .collect::<BTreeSet<_>>();
        // Remove mirrors before their journals so partial cleanup cannot revive a schedule.
        for operation in terminal
            .iter()
            .filter(|operation| removing.contains(&operation.id))
        {
            for outcome in &operation.targets {
                let pair = (
                    outcome.target.key.voter_id,
                    outcome.target.contested_name.as_str(),
                );
                if matches!(outcome.target.timing, VoteTiming::Scheduled(_))
                    && !surviving_pairs.contains(&pair)
                {
                    delete_scheduled_vote_in(kv, &pair.0.to_buffer(), pair.1)?;
                }
            }
        }
    }
    let removed = delete_terminal_operations(kv, network, &ordered[..remove_count])?;
    if remove_count > 0 {
        ordered.drain(..remove_count);
        kv.put(DetScope::Global, &history_key, &ordered)
            .map_err(operation_err)?;
    }
    Ok(removed)
}

/// A record delete may succeed before its dismissal-sidecar delete fails.
/// Recover directly from the owned key namespaces so rebuilding the operation
/// index cannot make that cleanup obligation unreachable.
pub(super) fn prune_orphaned_schedule_dismissals(
    kv: &DetKv,
    network: Network,
) -> Result<(), TaskError> {
    let operation_prefix = operation_key_prefix(network);
    let dismissal_prefix = format!(
        "{SCHEDULE_DISMISSAL_KEY_PREFIX}{}:",
        network_prefix(network)
    );
    let live_suffixes = kv
        .list(DetScope::Global, Some(&operation_prefix))
        .map_err(unreadable_operation_err)?
        .into_iter()
        .filter_map(|key| key.strip_prefix(&operation_prefix).map(str::to_owned))
        .collect::<BTreeSet<_>>();
    for key in kv
        .list(DetScope::Global, Some(&dismissal_prefix))
        .map_err(unreadable_operation_err)?
    {
        if key
            .strip_prefix(&dismissal_prefix)
            .is_some_and(|suffix| !live_suffixes.contains(suffix))
        {
            kv.delete(DetScope::Global, &key).map_err(operation_err)?;
        }
    }
    Ok(())
}

pub(super) fn delete_terminal_operations(
    kv: &DetKv,
    network: Network,
    terminal_ids: &[DpnsVoteOperationId],
) -> Result<usize, TaskError> {
    if terminal_ids.is_empty() {
        return Ok(0);
    }
    kv.put(
        DetScope::Global,
        &operation_lock_index_dirty_key(network),
        &true,
    )
    .map_err(operation_err)?;
    for id in terminal_ids {
        kv.delete(DetScope::Global, &operation_key(network, *id))
            .map_err(operation_err)?;
        kv.delete(DetScope::Global, &schedule_dismissal_key(network, *id))
            .map_err(operation_err)?;
    }
    rebuild_lock_index(kv, network)?;
    Ok(terminal_ids.len())
}
