//! The per-target state machine, expressed as guarded read-modify-writes.
//!
//! Each transition names the status it will act on and leaves every other
//! status untouched, so a target can only ever advance along one path.

use super::keys::*;
use super::persist_operation;
use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::{
    DpnsVoteFailure, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteOutcome, DpnsVoteTargetKey,
    DpnsVoteTargetStatus, failed_before_broadcast_outcome,
};
use crate::wallet_backend::{DetKv, DetScope};
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
    let Some(mut operation): Option<DpnsVoteOperation> = kv
        .get(DetScope::Global, &operation_key(network, operation_id))
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

pub(super) fn recover_interrupted_target_statuses(operation: &mut DpnsVoteOperation) -> bool {
    let mut changed = false;
    for outcome in &mut operation.targets {
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
