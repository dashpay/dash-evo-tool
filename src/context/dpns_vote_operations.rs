//! Durable DPNS vote operation journal and exact-target lock registry.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::{
    DpnsVoteFailure, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteTarget, DpnsVoteTargetKey,
    DpnsVoteTargetStatus, VoteTiming,
};
use crate::wallet_backend::{DetKv, DetScope, KvAdapterError};
use dash_sdk::platform::Identifier;

const OPERATION_INDEX_KEY: &str = "det:dpns_vote_operations:v1";
const OPERATION_KEY_PREFIX: &str = "det:dpns_vote_operation:v1:";

fn operation_key(id: DpnsVoteOperationId) -> String {
    format!("{OPERATION_KEY_PREFIX}{id}")
}

fn operation_err(source: KvAdapterError) -> TaskError {
    TaskError::DpnsVoteOperationStorage { source }
}

fn load_operation_ids(kv: &DetKv) -> Result<Vec<[u8; 16]>, TaskError> {
    kv.get(DetScope::Global, OPERATION_INDEX_KEY)
        .map(|ids| ids.unwrap_or_default())
        .map_err(operation_err)
}

fn load_operations(kv: &DetKv) -> Result<Vec<DpnsVoteOperation>, TaskError> {
    let mut operations = Vec::new();
    for bytes in load_operation_ids(kv)? {
        let id = DpnsVoteOperationId::from_bytes(bytes);
        match kv.get(DetScope::Global, &operation_key(id)) {
            Ok(Some(operation)) => operations.push(operation),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(
                    operation_id = %id,
                    error = ?error,
                    "Skipping unreadable DPNS vote operation"
                );
            }
        }
    }
    Ok(operations)
}

fn persist_operation(kv: &DetKv, operation: &DpnsVoteOperation) -> Result<(), TaskError> {
    let conflict = load_operations(kv)?.iter().any(|existing| {
        existing.id != operation.id
            && existing.targets.iter().any(|existing_outcome| {
                existing_outcome.status.holds_lock()
                    && operation.targets.iter().any(|outcome| {
                        outcome.status.holds_lock()
                            && outcome.target.key == existing_outcome.target.key
                    })
            })
    });
    if conflict {
        return Err(TaskError::DpnsVoteTargetBusy);
    }

    kv.put(DetScope::Global, &operation_key(operation.id), operation)
        .map_err(operation_err)?;
    let mut ids = load_operation_ids(kv)?;
    if !ids.contains(&operation.id.to_bytes()) {
        ids.push(operation.id.to_bytes());
        kv.put(DetScope::Global, OPERATION_INDEX_KEY, &ids)
            .map_err(operation_err)?;
    }
    Ok(())
}

impl AppContext {
    /// Persist a reviewed operation and atomically acquire all unresolved locks.
    pub fn insert_dpns_vote_operation(
        &self,
        operation: &DpnsVoteOperation,
    ) -> Result<(), TaskError> {
        if operation
            .targets
            .iter()
            .any(|outcome| outcome.target.key.network != self.network)
        {
            return Err(TaskError::DpnsVoteTargetBusy);
        }
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kv = self.det_kv()?;
        let mut existing_operations = load_operations(&kv)?;
        for existing in &mut existing_operations {
            let mut changed = false;
            for existing_outcome in &mut existing.targets {
                if existing_outcome.status == DpnsVoteTargetStatus::Scheduled
                    && operation.targets.iter().any(|new_outcome| {
                        new_outcome.status == DpnsVoteTargetStatus::Scheduled
                            && new_outcome.target.key == existing_outcome.target.key
                    })
                {
                    existing_outcome.status = DpnsVoteTargetStatus::NotApplied;
                    changed = true;
                }
            }
            if changed {
                kv.put(DetScope::Global, &operation_key(existing.id), existing)
                    .map_err(operation_err)?;
            }
        }
        persist_operation(&kv, operation)
    }

    /// Persist updated target statuses while retaining the original operation ID.
    pub fn update_dpns_vote_operation(
        &self,
        operation: &DpnsVoteOperation,
    ) -> Result<(), TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        persist_operation(&self.det_kv()?, operation)
    }

    /// Load every operation for this network, including completed history.
    pub fn dpns_vote_operations(&self) -> Result<Vec<DpnsVoteOperation>, TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kv = self.det_kv()?;
        let mut operations = load_operations(&kv)?;
        for legacy in self.get_scheduled_votes()? {
            if operations.iter().any(|operation| {
                operation.targets.iter().any(|outcome| {
                    outcome.target.key.voter_id == legacy.voter_id
                        && outcome.target.contested_name == legacy.contested_name
                })
            }) {
                continue;
            }
            let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
                key: DpnsVoteTargetKey {
                    network: self.network,
                    voter_id: legacy.voter_id,
                    vote_poll_id: self.dpns_vote_poll_id(&legacy.contested_name)?,
                },
                voter_alias: None,
                contested_name: legacy.contested_name,
                requested_choice: legacy.choice,
                current_choice: None,
                timing: VoteTiming::Scheduled(legacy.unix_timestamp),
            }]);
            if legacy.executed_successfully {
                operation.targets[0].status = DpnsVoteTargetStatus::Confirmed;
            }
            persist_operation(&kv, &operation)?;
            operations.push(operation);
        }
        Ok(operations)
    }

    /// Load one operation by its stable ID.
    pub fn dpns_vote_operation(
        &self,
        id: DpnsVoteOperationId,
    ) -> Result<Option<DpnsVoteOperation>, TaskError> {
        self.det_kv()?
            .get(DetScope::Global, &operation_key(id))
            .map_err(operation_err)
    }

    /// Return the unresolved status that currently locks an exact target.
    pub fn dpns_vote_target_status(
        &self,
        key: &DpnsVoteTargetKey,
    ) -> Result<Option<DpnsVoteTargetStatus>, TaskError> {
        Ok(self
            .dpns_vote_operations()?
            .into_iter()
            .flat_map(|operation| operation.targets)
            .find(|outcome| outcome.target.key == *key && outcome.status.holds_lock())
            .map(|outcome| outcome.status))
    }

    /// Atomically update one target in the durable journal.
    pub(crate) fn update_dpns_vote_target(
        &self,
        operation_id: DpnsVoteOperationId,
        key: &DpnsVoteTargetKey,
        status: DpnsVoteTargetStatus,
        failure: Option<DpnsVoteFailure>,
    ) -> Result<(), TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kv = self.det_kv()?;
        let Some(mut operation): Option<DpnsVoteOperation> = kv
            .get(DetScope::Global, &operation_key(operation_id))
            .map_err(operation_err)?
        else {
            return Ok(());
        };
        if let Some(outcome) = operation
            .targets
            .iter_mut()
            .find(|outcome| outcome.target.key == *key)
        {
            outcome.status = status;
            outcome.failure = failure;
        }
        persist_operation(&kv, &operation)
    }

    /// Release a not-yet-submitting scheduled target after explicit cancellation.
    pub(crate) fn cancel_scheduled_dpns_vote_target(
        &self,
        voter_id: Identifier,
        contested_name: &str,
    ) -> Result<(), TaskError> {
        let operations = self.dpns_vote_operations()?;
        for mut operation in operations {
            let mut changed = false;
            for outcome in &mut operation.targets {
                if outcome.target.key.voter_id == voter_id
                    && outcome.target.contested_name == contested_name
                    && outcome.status == DpnsVoteTargetStatus::Scheduled
                {
                    outcome.status = DpnsVoteTargetStatus::NotApplied;
                    changed = true;
                }
            }
            if changed {
                self.update_dpns_vote_operation(&operation)?;
            }
        }
        Ok(())
    }

    /// Release every not-yet-submitting scheduled target on this network.
    pub(crate) fn cancel_all_scheduled_dpns_vote_targets(&self) -> Result<(), TaskError> {
        let operations = self.dpns_vote_operations()?;
        for mut operation in operations {
            let mut changed = false;
            for outcome in &mut operation.targets {
                if outcome.status == DpnsVoteTargetStatus::Scheduled {
                    outcome.status = DpnsVoteTargetStatus::NotApplied;
                    changed = true;
                }
            }
            if changed {
                self.update_dpns_vote_operation(&operation)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dpns_voting::{DpnsVoteTarget, VoteTiming};
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
    use dash_sdk::platform::Identifier;
    use std::sync::Arc;

    fn kv() -> DetKv {
        DetKv::from_store(Arc::new(InMemoryKv::default()))
    }

    fn operation(status: DpnsVoteTargetStatus) -> DpnsVoteOperation {
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id: Identifier::from([1; 32]),
                vote_poll_id: Identifier::from([2; 32]),
            },
            voter_alias: Some("Eve".to_owned()),
            contested_name: "dominguez".to_owned(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Now,
        }]);
        operation.targets[0].status = status;
        operation
    }

    /// VOTE-TC-040/041: one unresolved target cannot be inserted twice.
    #[test]
    fn unresolved_target_rejects_a_competing_operation() {
        let kv = kv();
        persist_operation(&kv, &operation(DpnsVoteTargetStatus::Submitting)).unwrap();

        let error = persist_operation(&kv, &operation(DpnsVoteTargetStatus::Queued))
            .expect_err("the exact target must stay locked");
        assert!(matches!(error, TaskError::DpnsVoteTargetBusy));
    }

    /// VOTE-TC-044: reloading the journal reconstructs an Unconfirmed lock.
    #[test]
    fn unconfirmed_lock_survives_journal_reload() {
        let kv = kv();
        let operation = operation(DpnsVoteTargetStatus::Unconfirmed);
        persist_operation(&kv, &operation).unwrap();

        let restored = load_operations(&kv).unwrap();
        assert_eq!(restored, vec![operation]);
        assert!(restored[0].targets[0].status.holds_lock());
    }

    /// VOTE-TC-042: a different poll remains usable.
    #[test]
    fn unrelated_target_can_be_persisted() {
        let kv = kv();
        persist_operation(&kv, &operation(DpnsVoteTargetStatus::Confirming)).unwrap();
        let mut unrelated = operation(DpnsVoteTargetStatus::Queued);
        unrelated.targets[0].target.key.vote_poll_id = Identifier::from([3; 32]);

        persist_operation(&kv, &unrelated).unwrap();
        assert_eq!(load_operations(&kv).unwrap().len(), 2);
    }
}
