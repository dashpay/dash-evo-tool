//! Read-only detection of schedules that require a new voting decision.

use super::AppContext;
use crate::backend_task::error::TaskError;
use std::collections::BTreeSet;

impl AppContext {
    /// Whether unexecuted SQLite schedules still need an explicit voting decision.
    pub(crate) fn has_legacy_scheduled_votes(&self) -> Result<bool, TaskError> {
        let old = crate::database::legacy_import::read_scheduled_votes(
            &self.db.locked_conn(),
            self.network,
        )
        .map_err(|source| TaskError::LegacyScheduledVotesRead { source })?;
        if old.unreadable > 0 {
            return Ok(true);
        }
        if old.votes.iter().all(|vote| vote.executed_successfully) {
            return Ok(false);
        }
        let operations = self.dpns_vote_operations()?;
        let represented: BTreeSet<_> = operations
            .iter()
            .flat_map(|operation| &operation.targets)
            .map(|outcome| {
                (
                    outcome.target.key.voter_id,
                    outcome.target.contested_name.as_str(),
                )
            })
            .collect();
        Ok(old.votes.iter().any(|vote| {
            !vote.executed_successfully
                && !represented.contains(&(vote.voter_id, vote.contested_name.as_str()))
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::database::test_helpers::{
        create_legacy_scheduled_votes_table, seed_legacy_scheduled_vote_row,
    };
    use crate::model::dpns_voting::{
        DpnsVoteOperation, DpnsVoteTarget, DpnsVoteTargetKey, VoteTiming,
    };
    use crate::wallet_backend::{DetKv, kv_test_support::InMemoryKv};
    use dash_sdk::dpp::{
        dashcore::Network, voting::vote_choices::resource_vote_choice::ResourceVoteChoice,
    };
    use dash_sdk::platform::Identifier;
    use std::sync::Arc;

    #[test]
    fn startup_notice_detects_old_schedules_without_importing_them() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        context.set_det_kv_override_for_test(DetKv::from_store(Arc::new(InMemoryKv::default())));
        assert!(!context.has_legacy_scheduled_votes().unwrap());
        create_legacy_scheduled_votes_table(&context.db).unwrap();
        seed_legacy_scheduled_vote_row(&context.db, &[1; 32], "alice", "Lock", Network::Testnet)
            .unwrap();
        assert!(context.has_legacy_scheduled_votes().unwrap());
        assert!(context.dpns_vote_operations().unwrap().is_empty());
        assert!(
            context.has_legacy_scheduled_votes().unwrap(),
            "detection leaves the old record intact"
        );

        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id: Identifier::from([1; 32]),
                vote_poll_id: Identifier::from([2; 32]),
            },
            voter_alias: None,
            contested_name: "alice".into(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Scheduled(u64::MAX),
        }]);
        context
            .insert_dpns_vote_operation(&mut operation, None)
            .unwrap();
        assert!(
            !context.has_legacy_scheduled_votes().unwrap(),
            "already represented votes do not need a new decision"
        );
    }

    #[test]
    fn startup_notice_ignores_executed_votes_and_other_networks() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        create_legacy_scheduled_votes_table(&context.db).unwrap();
        seed_legacy_scheduled_vote_row(
            &context.db,
            &[1; 32],
            "other-network",
            "Lock",
            Network::Mainnet,
        )
        .unwrap();
        seed_legacy_scheduled_vote_row(&context.db, &[1; 32], "executed", "Lock", Network::Testnet)
            .unwrap();
        context
            .db
            .execute(
                "UPDATE scheduled_votes SET executed = 1 WHERE contested_name = 'executed'",
                [],
            )
            .unwrap();
        assert!(!context.has_legacy_scheduled_votes().unwrap());
    }

    #[test]
    fn startup_notice_reports_unreadable_schedules() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        create_legacy_scheduled_votes_table(&context.db).unwrap();
        seed_legacy_scheduled_vote_row(
            &context.db,
            &[1; 32],
            "alice",
            "invalid choice",
            Network::Testnet,
        )
        .unwrap();
        assert!(context.has_legacy_scheduled_votes().unwrap());
    }
}
