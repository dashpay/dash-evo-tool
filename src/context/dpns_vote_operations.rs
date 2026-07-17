//! Durable DPNS vote operation journal and exact-target lock registry.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsVoteFailure, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteTarget,
    DpnsVoteTargetKey, DpnsVoteTargetStatus, VoteTiming, unavailable_preflight_outcome,
};
use crate::wallet_backend::{DetKv, DetScope, KvAdapterError};
use dash_sdk::dpp::dashcore::Network;
use std::sync::Arc;

const LEGACY_OPERATION_INDEX_KEY: &str = "det:dpns_vote_operations:v1";
const LEGACY_OPERATION_KEY_PREFIX: &str = "det:dpns_vote_operation:v1:";
const OPERATION_INDEX_KEY_PREFIX: &str = "det:dpns_vote_operations:v2:";
const OPERATION_KEY_PREFIX: &str = "det:dpns_vote_operation:v2:";

fn network_tag(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
        Network::Devnet => "devnet",
        Network::Regtest => "regtest",
    }
}

fn operation_index_key(network: Network) -> String {
    format!("{OPERATION_INDEX_KEY_PREFIX}{}", network_tag(network))
}

fn operation_key(network: Network, id: DpnsVoteOperationId) -> String {
    format!("{OPERATION_KEY_PREFIX}{}:{id}", network_tag(network))
}

fn legacy_operation_key(id: DpnsVoteOperationId) -> String {
    format!("{LEGACY_OPERATION_KEY_PREFIX}{id}")
}

fn operation_err(source: KvAdapterError) -> TaskError {
    TaskError::DpnsVoteOperationStorage { source }
}

fn unreadable_operation_err(source: KvAdapterError) -> TaskError {
    TaskError::DpnsVoteOperationUnreadable { source }
}

fn load_operation_ids(kv: &DetKv, network: Network) -> Result<Vec<[u8; 16]>, TaskError> {
    kv.get(DetScope::Global, &operation_index_key(network))
        .map(|ids| ids.unwrap_or_default())
        .map_err(unreadable_operation_err)
}

fn operation_matches_network(
    operation: &DpnsVoteOperation,
    network: Network,
) -> Result<bool, TaskError> {
    if operation
        .targets
        .iter()
        .all(|outcome| outcome.target.key.network == network)
    {
        return Ok(true);
    }
    if operation
        .targets
        .iter()
        .any(|outcome| outcome.target.key.network != network && outcome.status.holds_lock())
    {
        return Err(TaskError::DpnsVoteJournalNetworkMismatch);
    }
    Ok(false)
}

fn migrate_legacy_operations(kv: &DetKv, network: Network) -> Result<(), TaskError> {
    let legacy_ids: Vec<[u8; 16]> = kv
        .get(DetScope::Global, LEGACY_OPERATION_INDEX_KEY)
        .map_err(unreadable_operation_err)?
        .unwrap_or_default();
    if legacy_ids.is_empty() {
        return Ok(());
    }

    let mut qualified_ids = load_operation_ids(kv, network)?;
    let mut changed = false;
    for bytes in legacy_ids {
        let id = DpnsVoteOperationId::from_bytes(bytes);
        if qualified_ids.contains(&bytes) {
            continue;
        }
        let operation: DpnsVoteOperation = kv
            .get(DetScope::Global, &legacy_operation_key(id))
            .map_err(unreadable_operation_err)?
            .ok_or(TaskError::DpnsVoteOperationRecordMissing)?;
        if !operation_matches_network(&operation, network)? {
            continue;
        }
        kv.put(DetScope::Global, &operation_key(network, id), &operation)
            .map_err(operation_err)?;
        qualified_ids.push(bytes);
        changed = true;
    }
    if changed {
        kv.put(
            DetScope::Global,
            &operation_index_key(network),
            &qualified_ids,
        )
        .map_err(operation_err)?;
    }
    Ok(())
}

fn load_operations(kv: &DetKv, network: Network) -> Result<Vec<DpnsVoteOperation>, TaskError> {
    migrate_legacy_operations(kv, network)?;
    let mut operations = Vec::new();
    for bytes in load_operation_ids(kv, network)? {
        let id = DpnsVoteOperationId::from_bytes(bytes);
        let operation: DpnsVoteOperation = kv
            .get(DetScope::Global, &operation_key(network, id))
            .map_err(unreadable_operation_err)?
            .ok_or(TaskError::DpnsVoteOperationRecordMissing)?;
        if operation_matches_network(&operation, network)? {
            operations.push(operation);
        }
    }
    Ok(operations)
}

fn persist_operation(
    kv: &DetKv,
    network: Network,
    operation: &DpnsVoteOperation,
) -> Result<(), TaskError> {
    if !operation_matches_network(operation, network)? {
        return Err(TaskError::DpnsVoteJournalNetworkMismatch);
    }
    let conflict = load_operations(kv, network)?.iter().any(|existing| {
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

    kv.put(
        DetScope::Global,
        &operation_key(network, operation.id),
        operation,
    )
    .map_err(operation_err)?;
    let mut ids = load_operation_ids(kv, network)?;
    if !ids.contains(&operation.id.to_bytes()) {
        ids.push(operation.id.to_bytes());
        kv.put(DetScope::Global, &operation_index_key(network), &ids)
            .map_err(operation_err)?;
    }
    Ok(())
}

fn write_existing_operation(
    kv: &DetKv,
    network: Network,
    operation: &DpnsVoteOperation,
) -> Result<(), TaskError> {
    kv.put(
        DetScope::Global,
        &operation_key(network, operation.id),
        operation,
    )
    .map_err(operation_err)
}

fn transition_scheduled_target_to_queued(
    kv: &DetKv,
    network: Network,
    operation_id: DpnsVoteOperationId,
    key: &DpnsVoteTargetKey,
) -> Result<bool, TaskError> {
    let Some(mut operation): Option<DpnsVoteOperation> = kv
        .get(DetScope::Global, &operation_key(network, operation_id))
        .map_err(unreadable_operation_err)?
    else {
        return Ok(false);
    };
    let Some(outcome) = operation
        .targets
        .iter_mut()
        .find(|outcome| outcome.target.key == *key)
    else {
        return Ok(false);
    };
    if outcome.status != DpnsVoteTargetStatus::Scheduled {
        return Ok(false);
    }
    outcome.status = DpnsVoteTargetStatus::Queued;
    write_existing_operation(kv, network, &operation)?;
    Ok(true)
}

fn is_authorized_scheduled_replacement(
    existing_status: DpnsVoteTargetStatus,
    existing_key: &DpnsVoteTargetKey,
    operation: &DpnsVoteOperation,
    replacing_scheduled_key: Option<&DpnsVoteTargetKey>,
) -> bool {
    existing_status == DpnsVoteTargetStatus::Scheduled
        && replacing_scheduled_key == Some(existing_key)
        && operation.targets.iter().any(|outcome| {
            outcome.target.key == *existing_key
                && outcome.status == DpnsVoteTargetStatus::Scheduled
                && matches!(outcome.target.timing, VoteTiming::Scheduled(_))
        })
}

fn replace_scheduled_operation(
    kv: &DetKv,
    network: Network,
    operation: &mut DpnsVoteOperation,
    replacing_scheduled_key: &DpnsVoteTargetKey,
) -> Result<(), TaskError> {
    if operation.targets.len() != 1
        || !is_authorized_scheduled_replacement(
            DpnsVoteTargetStatus::Scheduled,
            replacing_scheduled_key,
            operation,
            Some(replacing_scheduled_key),
        )
    {
        return Err(TaskError::DpnsVoteTargetBusy);
    }
    let replacement = operation.targets[0].clone();
    for mut existing in load_operations(kv, network)? {
        let Some(outcome) = existing
            .targets
            .iter_mut()
            .find(|outcome| outcome.target.key == *replacing_scheduled_key)
        else {
            continue;
        };
        if outcome.status != DpnsVoteTargetStatus::Scheduled {
            continue;
        }
        *outcome = replacement;
        outcome.operation_id = existing.id;
        write_existing_operation(kv, network, &existing)?;
        *operation = existing;
        return Ok(());
    }
    Err(TaskError::DpnsVoteTargetBusy)
}

fn cancel_scheduled_target(
    kv: &DetKv,
    network: Network,
    operation_id: DpnsVoteOperationId,
    key: &DpnsVoteTargetKey,
) -> Result<bool, TaskError> {
    let Some(mut operation): Option<DpnsVoteOperation> = kv
        .get(DetScope::Global, &operation_key(network, operation_id))
        .map_err(unreadable_operation_err)?
    else {
        return Ok(false);
    };
    let Some(outcome) = operation
        .targets
        .iter_mut()
        .find(|outcome| outcome.target.key == *key)
    else {
        return Ok(false);
    };
    if outcome.status != DpnsVoteTargetStatus::Scheduled {
        return Ok(false);
    }
    outcome.status = DpnsVoteTargetStatus::NotApplied;
    write_existing_operation(kv, network, &operation)?;
    Ok(true)
}

fn cancel_all_scheduled_targets(kv: &DetKv, network: Network) -> Result<usize, TaskError> {
    let mut cancelled = 0;
    for mut operation in load_operations(kv, network)? {
        let mut changed = false;
        for outcome in &mut operation.targets {
            if outcome.status == DpnsVoteTargetStatus::Scheduled {
                outcome.status = DpnsVoteTargetStatus::NotApplied;
                changed = true;
                cancelled += 1;
            }
        }
        if changed {
            write_existing_operation(kv, network, &operation)?;
        }
    }
    Ok(cancelled)
}

fn mark_interrupted_targets_unconfirmed(operation: &mut DpnsVoteOperation) -> bool {
    let mut changed = false;
    for outcome in &mut operation.targets {
        if matches!(
            outcome.status,
            DpnsVoteTargetStatus::Submitting | DpnsVoteTargetStatus::Confirming
        ) {
            outcome.status = DpnsVoteTargetStatus::Unconfirmed;
            outcome.failure = Some(DpnsVoteFailure::ResultUnconfirmed);
            changed = true;
        }
    }
    changed
}

impl AppContext {
    /// Durably claim one due scheduled target before any executor can observe it.
    pub(crate) fn queue_scheduled_dpns_vote_target(
        &self,
        operation_id: DpnsVoteOperationId,
        key: &DpnsVoteTargetKey,
    ) -> Result<bool, TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        transition_scheduled_target_to_queued(&self.det_kv()?, self.network, operation_id, key)
    }

    /// Persist a reviewed operation and atomically acquire all unresolved locks.
    pub fn insert_dpns_vote_operation(
        &self,
        operation: &mut DpnsVoteOperation,
        replacing_scheduled_key: Option<&DpnsVoteTargetKey>,
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
        if let Some(key) = replacing_scheduled_key {
            return replace_scheduled_operation(&kv, self.network, operation, key);
        }
        persist_operation(&kv, self.network, operation)
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
        persist_operation(&self.det_kv()?, self.network, operation)
    }

    /// Load every operation for this network, including completed history.
    pub fn dpns_vote_operations(&self) -> Result<Vec<DpnsVoteOperation>, TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kv = self.det_kv()?;
        let mut operations = load_operations(&kv, self.network)?;
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
            persist_operation(&kv, self.network, &operation)?;
            operations.push(operation);
        }
        Ok(operations)
    }

    /// Load one operation by its stable ID.
    pub fn dpns_vote_operation(
        &self,
        id: DpnsVoteOperationId,
    ) -> Result<Option<DpnsVoteOperation>, TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kv = self.det_kv()?;
        migrate_legacy_operations(&kv, self.network)?;
        let operation = kv
            .get(DetScope::Global, &operation_key(self.network, id))
            .map_err(unreadable_operation_err)?;
        match operation {
            Some(operation) if operation_matches_network(&operation, self.network)? => {
                Ok(Some(operation))
            }
            Some(_) | None => Ok(None),
        }
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
            .get(DetScope::Global, &operation_key(self.network, operation_id))
            .map_err(unreadable_operation_err)?
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
        persist_operation(&kv, self.network, &operation)
    }

    /// Atomically claim a queued target before any network or nonce work.
    pub(crate) fn claim_dpns_vote_target(
        &self,
        operation_id: DpnsVoteOperationId,
        key: &DpnsVoteTargetKey,
    ) -> Result<bool, TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kv = self.det_kv()?;
        let Some(mut operation): Option<DpnsVoteOperation> = kv
            .get(DetScope::Global, &operation_key(self.network, operation_id))
            .map_err(unreadable_operation_err)?
        else {
            return Ok(false);
        };
        let Some(outcome) = operation
            .targets
            .iter_mut()
            .find(|outcome| outcome.target.key == *key)
        else {
            return Ok(false);
        };
        if outcome.status != DpnsVoteTargetStatus::Queued {
            return Ok(false);
        }
        outcome.status = DpnsVoteTargetStatus::Submitting;
        outcome.failure = None;
        persist_operation(&kv, self.network, &operation)?;
        Ok(true)
    }

    /// Apply fresh proved state only while the target is still queued.
    ///
    /// Returning `false` means another executor already advanced the target;
    /// callers must not write their stale operation snapshot back.
    pub(crate) fn revalidate_queued_dpns_vote_target(
        &self,
        operation_id: DpnsVoteOperationId,
        key: &DpnsVoteTargetKey,
        state: DpnsCurrentVoteState,
    ) -> Result<bool, TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kv = self.det_kv()?;
        let Some(mut operation): Option<DpnsVoteOperation> = kv
            .get(DetScope::Global, &operation_key(self.network, operation_id))
            .map_err(unreadable_operation_err)?
        else {
            return Ok(false);
        };
        let Some(outcome) = operation
            .targets
            .iter_mut()
            .find(|outcome| outcome.target.key == *key)
        else {
            return Ok(false);
        };
        if outcome.status != DpnsVoteTargetStatus::Queued {
            return Ok(false);
        }
        match state {
            DpnsCurrentVoteState::Available(current) => {
                outcome.target.current_choice = current;
                if current == Some(outcome.target.requested_choice) {
                    outcome.status = DpnsVoteTargetStatus::Confirmed;
                    outcome.failure = None;
                }
            }
            DpnsCurrentVoteState::Checking | DpnsCurrentVoteState::Unavailable => {
                (outcome.status, outcome.failure) =
                    unavailable_preflight_outcome(outcome.target.timing);
            }
        }
        let still_queued = outcome.status == DpnsVoteTargetStatus::Queued;
        persist_operation(&kv, self.network, &operation)?;
        Ok(still_queued)
    }

    /// Convert crash-interrupted transitions into conservative reconciliation.
    pub(crate) fn recover_interrupted_dpns_vote_operations(&self) -> Result<(), TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kv = self.det_kv()?;
        for mut operation in load_operations(&kv, self.network)? {
            if mark_interrupted_targets_unconfirmed(&mut operation) {
                persist_operation(&kv, self.network, &operation)?;
            }
        }
        Ok(())
    }

    /// Conservatively recover one operation after its executor has returned.
    pub(crate) fn recover_interrupted_dpns_vote_operation(
        &self,
        operation_id: DpnsVoteOperationId,
    ) -> Result<(), TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let kv = self.det_kv()?;
        let Some(mut operation): Option<DpnsVoteOperation> = kv
            .get(DetScope::Global, &operation_key(self.network, operation_id))
            .map_err(unreadable_operation_err)?
        else {
            return Ok(());
        };
        if mark_interrupted_targets_unconfirmed(&mut operation) {
            persist_operation(&kv, self.network, &operation)?;
        }
        Ok(())
    }

    pub(crate) fn record_dpns_vote_diagnostic(
        &self,
        operation_id: DpnsVoteOperationId,
        key: DpnsVoteTargetKey,
        error: TaskError,
    ) {
        self.dpns_vote_diagnostics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert((operation_id, key), Arc::new(error));
    }

    pub(crate) fn dpns_vote_operation_diagnostics(
        &self,
        operation_id: DpnsVoteOperationId,
    ) -> Vec<Arc<TaskError>> {
        self.dpns_vote_diagnostics
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|((id, _), _)| *id == operation_id)
            .map(|(_, error)| Arc::clone(error))
            .collect()
    }

    /// Release a not-yet-submitting scheduled target after explicit cancellation.
    pub(crate) fn cancel_scheduled_dpns_vote_target(
        &self,
        operation_id: DpnsVoteOperationId,
        key: &DpnsVoteTargetKey,
    ) -> Result<(), TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cancel_scheduled_target(&self.det_kv()?, self.network, operation_id, key)?;
        Ok(())
    }

    /// Release every not-yet-submitting scheduled target on this network.
    pub(crate) fn cancel_all_scheduled_dpns_vote_targets(&self) -> Result<(), TaskError> {
        let _guard = self
            .dpns_vote_operation_guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cancel_all_scheduled_targets(&self.det_kv()?, self.network)?;
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
    use platform_wallet_storage::{KvStore, ObjectId};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingKv {
        inner: InMemoryKv,
        puts: AtomicUsize,
    }

    impl CountingKv {
        fn reset_puts(&self) {
            self.puts.store(0, Ordering::Relaxed);
        }

        fn put_count(&self) -> usize {
            self.puts.load(Ordering::Relaxed)
        }
    }

    impl KvStore for CountingKv {
        fn get(
            &self,
            scope: &ObjectId,
            key: &str,
        ) -> Result<Option<Vec<u8>>, platform_wallet_storage::KvError> {
            self.inner.get(scope, key)
        }

        fn put(
            &self,
            scope: &ObjectId,
            key: &str,
            value: &[u8],
        ) -> Result<(), platform_wallet_storage::KvError> {
            self.puts.fetch_add(1, Ordering::Relaxed);
            self.inner.put(scope, key, value)
        }

        fn delete(
            &self,
            scope: &ObjectId,
            key: &str,
        ) -> Result<(), platform_wallet_storage::KvError> {
            self.inner.delete(scope, key)
        }

        fn list_keys(
            &self,
            scope: &ObjectId,
            prefix: Option<&str>,
        ) -> Result<Vec<String>, platform_wallet_storage::KvError> {
            self.inner.list_keys(scope, prefix)
        }
    }

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
        persist_operation(
            &kv,
            Network::Testnet,
            &operation(DpnsVoteTargetStatus::Submitting),
        )
        .unwrap();

        let error = persist_operation(
            &kv,
            Network::Testnet,
            &operation(DpnsVoteTargetStatus::Queued),
        )
        .expect_err("the exact target must stay locked");
        assert!(matches!(error, TaskError::DpnsVoteTargetBusy));
    }

    /// VOTE-TC-044: reloading the journal reconstructs an Unconfirmed lock.
    #[test]
    fn unconfirmed_lock_survives_journal_reload() {
        let kv = kv();
        let operation = operation(DpnsVoteTargetStatus::Unconfirmed);
        persist_operation(&kv, Network::Testnet, &operation).unwrap();

        let restored = load_operations(&kv, Network::Testnet).unwrap();
        assert_eq!(restored, vec![operation]);
        assert!(restored[0].targets[0].status.holds_lock());
    }

    /// VOTE-TC-042: a different poll remains usable.
    #[test]
    fn unrelated_target_can_be_persisted() {
        let kv = kv();
        persist_operation(
            &kv,
            Network::Testnet,
            &operation(DpnsVoteTargetStatus::Confirming),
        )
        .unwrap();
        let mut unrelated = operation(DpnsVoteTargetStatus::Queued);
        unrelated.targets[0].target.key.vote_poll_id = Identifier::from([3; 32]);

        persist_operation(&kv, Network::Testnet, &unrelated).unwrap();
        assert_eq!(load_operations(&kv, Network::Testnet).unwrap().len(), 2);
    }

    #[test]
    fn scheduled_replacement_requires_the_exact_edit_key() {
        let mut replacement = operation(DpnsVoteTargetStatus::Scheduled);
        replacement.targets[0].target.timing = VoteTiming::Scheduled(42);
        let key = replacement.targets[0].target.key.clone();
        let other = DpnsVoteTargetKey {
            vote_poll_id: Identifier::from([9; 32]),
            ..key.clone()
        };

        assert!(is_authorized_scheduled_replacement(
            DpnsVoteTargetStatus::Scheduled,
            &key,
            &replacement,
            Some(&key),
        ));
        assert!(!is_authorized_scheduled_replacement(
            DpnsVoteTargetStatus::Scheduled,
            &key,
            &replacement,
            None,
        ));
        assert!(!is_authorized_scheduled_replacement(
            DpnsVoteTargetStatus::Scheduled,
            &key,
            &replacement,
            Some(&other),
        ));
    }

    #[test]
    fn scheduled_replacement_reuses_the_existing_record_in_one_write() {
        let store = Arc::new(CountingKv::default());
        let kv = DetKv::from_store(store.clone());
        let mut scheduled = operation(DpnsVoteTargetStatus::Scheduled);
        scheduled.targets[0].target.timing = VoteTiming::Scheduled(42);
        let key = scheduled.targets[0].target.key.clone();
        persist_operation(&kv, Network::Testnet, &scheduled).unwrap();

        let mut replacement = operation(DpnsVoteTargetStatus::Scheduled);
        replacement.targets[0].target.timing = VoteTiming::Scheduled(84);
        store.reset_puts();

        replace_scheduled_operation(&kv, Network::Testnet, &mut replacement, &key).unwrap();

        assert_eq!(store.put_count(), 1);
        assert_eq!(replacement.id, scheduled.id);
        assert_eq!(
            load_operations(&kv, Network::Testnet).unwrap(),
            vec![replacement]
        );
    }

    #[test]
    fn due_schedule_is_durably_queued_once_through_the_kv_seam() {
        let kv = kv();
        let mut scheduled = operation(DpnsVoteTargetStatus::Scheduled);
        scheduled.targets[0].target.timing = VoteTiming::Scheduled(42);
        let key = scheduled.targets[0].target.key.clone();
        persist_operation(&kv, Network::Testnet, &scheduled).unwrap();

        assert!(
            transition_scheduled_target_to_queued(&kv, Network::Testnet, scheduled.id, &key,)
                .unwrap()
        );
        assert_eq!(
            load_operations(&kv, Network::Testnet).unwrap()[0].targets[0].status,
            DpnsVoteTargetStatus::Queued
        );
        assert!(
            !transition_scheduled_target_to_queued(&kv, Network::Testnet, scheduled.id, &key,)
                .unwrap(),
            "a second executor must not claim the same schedule"
        );
    }

    #[test]
    fn unavailable_preflight_preserves_retryability_by_timing() {
        assert_eq!(
            unavailable_preflight_outcome(VoteTiming::Scheduled(42)),
            (
                DpnsVoteTargetStatus::Scheduled,
                Some(DpnsVoteFailure::CurrentVoteUnavailable),
            )
        );
        assert_eq!(
            unavailable_preflight_outcome(VoteTiming::Now),
            (
                DpnsVoteTargetStatus::FailedBeforeSubmission,
                Some(DpnsVoteFailure::CurrentVoteUnavailable),
            )
        );
    }

    #[test]
    fn cancellation_does_not_overwrite_a_queued_target() {
        let kv = kv();
        let mut scheduled = operation(DpnsVoteTargetStatus::Scheduled);
        scheduled.targets[0].target.timing = VoteTiming::Scheduled(42);
        let key = scheduled.targets[0].target.key.clone();
        persist_operation(&kv, Network::Testnet, &scheduled).unwrap();
        transition_scheduled_target_to_queued(&kv, Network::Testnet, scheduled.id, &key).unwrap();

        assert!(!cancel_scheduled_target(&kv, Network::Testnet, scheduled.id, &key).unwrap());
        assert_eq!(
            load_operations(&kv, Network::Testnet).unwrap()[0].targets[0].status,
            DpnsVoteTargetStatus::Queued
        );
    }

    #[test]
    fn cancel_all_preserves_targets_that_are_already_queued() {
        let kv = kv();
        let mut scheduled = operation(DpnsVoteTargetStatus::Scheduled);
        scheduled.targets[0].target.timing = VoteTiming::Scheduled(42);
        let mut queued = operation(DpnsVoteTargetStatus::Queued);
        queued.targets[0].target.key.vote_poll_id = Identifier::from([3; 32]);
        persist_operation(&kv, Network::Testnet, &scheduled).unwrap();
        persist_operation(&kv, Network::Testnet, &queued).unwrap();

        assert_eq!(
            cancel_all_scheduled_targets(&kv, Network::Testnet).unwrap(),
            1
        );
        let operations = load_operations(&kv, Network::Testnet).unwrap();
        assert!(operations.iter().any(|operation| {
            operation.targets[0].target.key == queued.targets[0].target.key
                && operation.targets[0].status == DpnsVoteTargetStatus::Queued
        }));
    }

    /// A corrupt indexed row must block lock reconstruction rather than being skipped.
    #[test]
    fn unreadable_indexed_operation_fails_closed() {
        let store = Arc::new(InMemoryKv::default());
        let kv = DetKv::from_store(store.clone());
        let operation = operation(DpnsVoteTargetStatus::Unconfirmed);
        persist_operation(&kv, Network::Testnet, &operation).unwrap();
        store
            .put(
                &ObjectId::Global,
                &operation_key(Network::Testnet, operation.id),
                &[0xff, 0x00],
            )
            .unwrap();

        assert!(
            load_operations(&kv, Network::Testnet).is_err(),
            "an unreadable lock record must never be treated as absent"
        );
    }

    #[test]
    fn network_qualified_journals_are_isolated() {
        let kv = kv();
        persist_operation(
            &kv,
            Network::Testnet,
            &operation(DpnsVoteTargetStatus::Unconfirmed),
        )
        .unwrap();

        assert_eq!(load_operations(&kv, Network::Testnet).unwrap().len(), 1);
        assert!(load_operations(&kv, Network::Mainnet).unwrap().is_empty());
    }

    #[test]
    fn legacy_journal_migrates_idempotently_into_network_namespace() {
        let kv = kv();
        let operation = operation(DpnsVoteTargetStatus::Scheduled);
        kv.put(
            DetScope::Global,
            &legacy_operation_key(operation.id),
            &operation,
        )
        .unwrap();
        kv.put(
            DetScope::Global,
            LEGACY_OPERATION_INDEX_KEY,
            &vec![operation.id.to_bytes()],
        )
        .unwrap();

        assert_eq!(
            load_operations(&kv, Network::Testnet).unwrap(),
            vec![operation.clone()]
        );
        assert_eq!(
            load_operations(&kv, Network::Testnet).unwrap(),
            vec![operation],
            "repeating migration must not duplicate the operation"
        );
    }

    #[test]
    fn mismatched_unresolved_legacy_record_fails_closed() {
        let kv = kv();
        let operation = operation(DpnsVoteTargetStatus::Unconfirmed);
        kv.put(
            DetScope::Global,
            &legacy_operation_key(operation.id),
            &operation,
        )
        .unwrap();
        kv.put(
            DetScope::Global,
            LEGACY_OPERATION_INDEX_KEY,
            &vec![operation.id.to_bytes()],
        )
        .unwrap();

        assert!(matches!(
            load_operations(&kv, Network::Mainnet),
            Err(TaskError::DpnsVoteJournalNetworkMismatch)
        ));
    }

    #[test]
    fn mismatched_terminal_legacy_record_is_safely_ignored() {
        let kv = kv();
        let operation = operation(DpnsVoteTargetStatus::Confirmed);
        kv.put(
            DetScope::Global,
            &legacy_operation_key(operation.id),
            &operation,
        )
        .unwrap();
        kv.put(
            DetScope::Global,
            LEGACY_OPERATION_INDEX_KEY,
            &vec![operation.id.to_bytes()],
        )
        .unwrap();

        assert!(load_operations(&kv, Network::Mainnet).unwrap().is_empty());
    }

    #[test]
    fn interrupted_submission_recovers_to_unconfirmed() {
        let mut operation = operation(DpnsVoteTargetStatus::Submitting);
        assert!(mark_interrupted_targets_unconfirmed(&mut operation));
        assert_eq!(
            operation.targets[0].status,
            DpnsVoteTargetStatus::Unconfirmed
        );
        assert_eq!(
            operation.targets[0].failure,
            Some(DpnsVoteFailure::ResultUnconfirmed)
        );
    }
}
