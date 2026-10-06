//! The per-target state machine, expressed as guarded read-modify-writes.
//!
//! Each transition names the status it will act on and leaves every other
//! status untouched, so a target can only ever advance along one path.

use super::keys::*;
use super::{load_operation, persist_operation};
use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::{
    DpnsVoteFailure, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteOutcome, DpnsVoteTargetKey,
    DpnsVoteTargetStatus, VoteTiming, dpns_schedule_is_overdue, failed_before_broadcast_outcome,
};
use crate::wallet_backend::DetKv;
use dash_sdk::dpp::dashcore::Network;

/// What [`with_target`] should do once its mutator has inspected an outcome.
pub(super) enum TargetUpdate<T> {
    /// Persist the mutated operation, then yield `T`.
    Persist(T),
    /// Leave the stored record untouched, but still yield `T`.
    Skip(T),
}

/// Run one guarded read-modify-write against a single journal target.
///
/// The caller must already hold the journal guard. `mutate` sees the outcome
/// matching `key` and decides whether its change is worth a write; returns
/// `Ok(None)` when the operation record or the target is gone, which is the
/// single place the missing-record policy lives.
pub(super) fn with_target<T>(
    kv: &DetKv,
    network: Network,
    operation_id: DpnsVoteOperationId,
    key: &DpnsVoteTargetKey,
    mutate: impl FnOnce(&mut DpnsVoteOutcome) -> TargetUpdate<T>,
) -> Result<Option<T>, TaskError> {
    let Some(mut operation): Option<DpnsVoteOperation> =
        load_operation(kv, &operation_key(network, operation_id))
            .map_err(unreadable_operation_err)?
    else {
        return Ok(None);
    };
    let Some(outcome) = operation
        .targets
        .iter_mut()
        .find(|outcome| outcome.target.key == *key)
    else {
        return Ok(None);
    };
    match mutate(outcome) {
        TargetUpdate::Persist(value) => {
            persist_operation(kv, network, &operation)?;
            Ok(Some(value))
        }
        TargetUpdate::Skip(value) => Ok(Some(value)),
    }
}

pub(super) fn transition_scheduled_target_to_queued(
    kv: &DetKv,
    network: Network,
    operation_id: DpnsVoteOperationId,
    key: &DpnsVoteTargetKey,
) -> Result<bool, TaskError> {
    Ok(with_target(kv, network, operation_id, key, |outcome| {
        if outcome.status != DpnsVoteTargetStatus::Scheduled {
            return TargetUpdate::Skip(false);
        }
        outcome.status = DpnsVoteTargetStatus::Queued;
        TargetUpdate::Persist(true)
    })?
    .unwrap_or(false))
}

pub(super) fn cancel_scheduled_target(
    kv: &DetKv,
    network: Network,
    operation_id: DpnsVoteOperationId,
    key: &DpnsVoteTargetKey,
) -> Result<bool, TaskError> {
    Ok(with_target(kv, network, operation_id, key, |outcome| {
        if outcome.status != DpnsVoteTargetStatus::Scheduled {
            return TargetUpdate::Skip(false);
        }
        outcome.status = DpnsVoteTargetStatus::Cancelled;
        outcome.failure = None;
        TargetUpdate::Persist(true)
    })?
    .unwrap_or(false))
}

pub(super) fn recover_interrupted_target_statuses(
    operation: &mut DpnsVoteOperation,
    includes: impl Fn(&DpnsVoteTargetKey) -> bool,
) -> bool {
    let mut changed = false;
    for outcome in &mut operation.targets {
        if !includes(&outcome.target.key) {
            continue;
        }
        match outcome.status {
            DpnsVoteTargetStatus::Submitting => {
                (outcome.status, outcome.failure) =
                    failed_before_broadcast_outcome(outcome.target.timing);
                changed = true;
            }
            DpnsVoteTargetStatus::Confirming => {
                outcome.status = DpnsVoteTargetStatus::Unconfirmed;
                outcome.target.current_choice = None;
                outcome.failure = Some(DpnsVoteFailure::ResultUnconfirmed);
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

/// Settle the immediate targets among `includes` that are still queued after
/// their executor ended with an error.
///
/// Nothing claimed them, so nothing was broadcast, but left queued they would
/// hold their lock with no executor until the next start. A queued schedule
/// is left alone: the sweep sends it again.
pub(super) fn fail_abandoned_queued_targets(
    operation: &mut DpnsVoteOperation,
    includes: impl Fn(&DpnsVoteTargetKey) -> bool,
) -> bool {
    let mut changed = false;
    for outcome in &mut operation.targets {
        if outcome.status == DpnsVoteTargetStatus::Queued
            && outcome.target.timing == VoteTiming::Now
            && includes(&outcome.target.key)
        {
            (outcome.status, outcome.failure) = failed_before_broadcast_outcome(VoteTiming::Now);
            changed = true;
        }
    }
    changed
}

/// Keep restart recovery from sending a queued target nobody is waiting for.
///
/// A queued target was never claimed, so nothing was broadcast. An immediate
/// vote reviewed longer ago than the lateness bound, or an admitted schedule
/// that far past its time, ends like an interrupted submission: a failure the
/// operator reviews, or a schedule shown as missed. Fresher targets stay
/// queued and are sent.
pub(super) fn expire_stale_queued_targets(operation: &mut DpnsVoteOperation, now_ms: u64) -> bool {
    let created_at = operation.created_at;
    let mut changed = false;
    for outcome in &mut operation.targets {
        if outcome.status != DpnsVoteTargetStatus::Queued {
            continue;
        }
        let due_at = match outcome.target.timing {
            VoteTiming::Now => created_at,
            VoteTiming::Scheduled(at) => at,
        };
        if dpns_schedule_is_overdue(due_at, now_ms) {
            (outcome.status, outcome.failure) =
                failed_before_broadcast_outcome(outcome.target.timing);
            changed = true;
        }
    }
    changed
}

pub(super) fn mark_target_broadcast(
    kv: &DetKv,
    network: Network,
    operation_id: DpnsVoteOperationId,
    key: &DpnsVoteTargetKey,
) -> Result<bool, TaskError> {
    Ok(with_target(kv, network, operation_id, key, |outcome| {
        if outcome.status != DpnsVoteTargetStatus::Submitting {
            return TargetUpdate::Skip(false);
        }
        outcome.status = DpnsVoteTargetStatus::Confirming;
        TargetUpdate::Persist(true)
    })?
    .unwrap_or(false))
}
