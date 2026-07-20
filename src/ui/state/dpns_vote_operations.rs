//! Per-screen DPNS vote-operation snapshot for immediate-mode render paths.

use std::collections::BTreeMap;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dpns_voting::{
    DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteTargetKey, DpnsVoteTargetStatus,
};

#[derive(Debug, Clone, Default)]
pub struct DpnsVoteOperationSnapshot {
    operations: Vec<DpnsVoteOperation>,
    target_statuses: BTreeMap<DpnsVoteTargetKey, DpnsVoteTargetStatus>,
    loaded: bool,
}

impl DpnsVoteOperationSnapshot {
    pub fn load(app_context: &AppContext) -> Result<Self, TaskError> {
        let mut snapshot = Self::default();
        snapshot.refresh(app_context)?;
        Ok(snapshot)
    }

    pub fn refresh(&mut self, app_context: &AppContext) -> Result<(), TaskError> {
        self.replace(app_context.dpns_vote_operations()?);
        Ok(())
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
    use crate::model::dpns_voting::{DpnsVoteTarget, VoteTiming};
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
    use dash_sdk::platform::Identifier;

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
}
