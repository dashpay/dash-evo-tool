//! Read-only detection of schedules that require a new voting decision.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::utils::time::now_ms;
use crate::wallet_backend::{DetScope, network_prefix};
use std::collections::BTreeSet;
use std::time::Duration;

const NOTICED_AT_KEY_PREFIX: &str = "det:legacy_scheduled_votes_noticed_at:v1:";

/// Whether a notice first raised at `first_noticed_at_ms` is still worth
/// showing: once a full contest has passed, every contest an old schedule
/// could refer to has closed and nothing is left to schedule again.
fn legacy_notice_is_current(
    first_noticed_at_ms: u64,
    now_ms: u64,
    contest_duration: Duration,
) -> bool {
    let window_ms = u64::try_from(contest_duration.as_millis()).unwrap_or(u64::MAX);
    now_ms.saturating_sub(first_noticed_at_ms) < window_ms
}

impl AppContext {
    /// Whether startup should tell the user about old, unexecuted schedules.
    ///
    /// Old schedules are neither imported nor listed, so the user cannot retire
    /// the notice for a contest that has ended. It therefore stops one contest
    /// duration after its first appearance, which this call records.
    pub(crate) fn legacy_scheduled_votes_notice_due(&self) -> Result<bool, TaskError> {
        if !self.has_legacy_scheduled_votes()? {
            return Ok(false);
        }
        let kv = self.det_kv()?;
        let key = format!("{NOTICED_AT_KEY_PREFIX}{}", network_prefix(self.network));
        let storage_err = |source| TaskError::DpnsVoteOperationStorage { source };
        let now = now_ms();
        let first_noticed_at = match kv.get::<u64>(DetScope::Global, &key).map_err(storage_err)? {
            Some(at) => at,
            None => {
                kv.put(DetScope::Global, &key, &now).map_err(storage_err)?;
                now
            }
        };
        let contest_duration = crate::model::dpns::contest_durations(
            self.network,
            super::default_platform_version(&self.network),
        )
        .total;
        Ok(legacy_notice_is_current(
            first_noticed_at,
            now,
            contest_duration,
        ))
    }

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
    use crate::context::AppContext;
    use crate::database::test_helpers::{
        create_legacy_scheduled_votes_table, seed_legacy_scheduled_vote_row,
    };
    use crate::model::dpns_voting::{DpnsVoteTarget, DpnsVoteTargetKey, VoteTiming};
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

        let mut operation = AppContext::new_dpns_vote_operation(vec![DpnsVoteTarget {
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

    /// The user cannot retire the notice for a contest that already ended, so
    /// it must stop by itself once no old schedule can still matter.
    #[test]
    fn startup_notice_stops_one_contest_duration_after_it_first_appeared() {
        use super::{NOTICED_AT_KEY_PREFIX, legacy_notice_is_current};
        use crate::utils::time::now_ms;
        use crate::wallet_backend::{DetScope, network_prefix};
        use std::time::Duration;

        let hour = Duration::from_secs(3600);
        assert!(legacy_notice_is_current(1_000, 1_000, hour));
        assert!(legacy_notice_is_current(1_000, 1_000 + 3_599_999, hour));
        assert!(!legacy_notice_is_current(1_000, 1_000 + 3_600_000, hour));

        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let kv = DetKv::from_store(Arc::new(InMemoryKv::default()));
        context.set_det_kv_override_for_test(kv.clone());
        assert!(!context.legacy_scheduled_votes_notice_due().unwrap());
        create_legacy_scheduled_votes_table(&context.db).unwrap();
        seed_legacy_scheduled_vote_row(&context.db, &[1; 32], "alice", "Lock", Network::Testnet)
            .unwrap();
        assert!(context.legacy_scheduled_votes_notice_due().unwrap());
        assert!(
            context.legacy_scheduled_votes_notice_due().unwrap(),
            "the notice returns on later launches while a contest may still be open"
        );

        let contest_duration = crate::model::dpns::contest_durations(
            Network::Testnet,
            crate::context::default_platform_version(&Network::Testnet),
        )
        .total;
        let long_ago = now_ms() - u64::try_from(contest_duration.as_millis()).unwrap() - 1;
        kv.put(
            DetScope::Global,
            &format!(
                "{NOTICED_AT_KEY_PREFIX}{}",
                network_prefix(Network::Testnet)
            ),
            &long_ago,
        )
        .unwrap();
        assert!(!context.legacy_scheduled_votes_notice_due().unwrap());
        assert!(
            context.has_legacy_scheduled_votes().unwrap(),
            "the old record itself stays untouched"
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
