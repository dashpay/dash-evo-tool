//! Read-only detection of schedules that require a new voting decision.

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::utils::time::now_ms;
use crate::wallet_backend::{DetKv, DetScope, KvAdapterError, network_prefix};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::BTreeSet;
use std::time::Duration;

const NOTICED_AT_KEY_PREFIX: &str = "det:legacy_scheduled_votes_noticed_at:v1:";

/// Voter index of the schedule queue earlier pre-release builds kept in the
/// k/v store. Such a build wrote it when it copied the old SQLite schedules
/// into that queue and kept it, emptied, once they were gone again.
const RETIRED_VOTER_INDEX_KEY: &str = "det:scheduled_vote_voters:v1";

/// Key prefix of one schedule in that queue, completed by the contested name
/// and scoped to [`DetScope::Identity`] of the voter.
const RETIRED_SCHEDULE_KEY_PREFIX: &str = "det:scheduled_vote:";

/// One schedule in the retired queue. The stored format is positional, so
/// every field is declared, in stored order, to reach the last one.
#[derive(Deserialize)]
#[expect(dead_code, reason = "decoded only to reach `executed_successfully`")]
struct RetiredSchedule {
    voter_id: [u8; 32],
    contested_name: String,
    choice: RetiredChoice,
    unix_timestamp: u64,
    executed_successfully: bool,
}

#[derive(Deserialize)]
#[expect(dead_code, reason = "decoded only to keep the stored layout")]
enum RetiredChoice {
    TowardsIdentity([u8; 32]),
    Abstain,
    Lock,
}

/// A retired record as this build finds it. This build never writes one.
enum Retired<T> {
    Absent,
    /// Present but undecodable, so it proves nothing about the schedule.
    Damaged,
    Found(T),
}

fn read_retired<T: DeserializeOwned>(
    kv: &DetKv,
    scope: DetScope<'_>,
    key: &str,
) -> Result<Retired<T>, TaskError> {
    match kv.get::<T>(scope, key) {
        Ok(Some(value)) => Ok(Retired::Found(value)),
        Ok(None) => Ok(Retired::Absent),
        Err(
            error @ (KvAdapterError::SchemaVersion { .. }
            | KvAdapterError::Truncated
            | KvAdapterError::Decode(_)),
        ) => {
            tracing::warn!(
                key,
                ?error,
                "A scheduled-vote record of an earlier pre-release build is unreadable; the old schedules it covers are reported as not cast"
            );
            Ok(Retired::Damaged)
        }
        Err(source @ (KvAdapterError::Store(_) | KvAdapterError::Encode(_))) => {
            Err(TaskError::RetiredScheduledVotesRead { source })
        }
    }
}

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
    ///
    /// Earlier pre-release builds copied these schedules into their own queue
    /// and cast them from there, leaving the SQLite rows marked unexecuted. That
    /// queue is read, never imported, to leave out what such a build already
    /// cast or dropped.
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
        let mut undecided = old
            .votes
            .iter()
            .filter(|vote| {
                !vote.executed_successfully
                    && !represented.contains(&(vote.voter_id, vote.contested_name.as_str()))
            })
            .peekable();
        if undecided.peek().is_none() {
            return Ok(false);
        }

        let kv = self.det_kv()?;
        match read_retired::<Vec<[u8; 32]>>(&kv, DetScope::Global, RETIRED_VOTER_INDEX_KEY)? {
            Retired::Found(_) => {}
            // No earlier build took the schedules over, or its index proves nothing.
            Retired::Absent | Retired::Damaged => return Ok(true),
        }
        for vote in undecided {
            let voter = vote.voter_id.to_buffer();
            let key = format!("{RETIRED_SCHEDULE_KEY_PREFIX}{}", vote.contested_name);
            match read_retired::<RetiredSchedule>(&kv, DetScope::Identity(&voter), &key)? {
                // Cast by the earlier build, or dropped from its queue.
                Retired::Found(RetiredSchedule {
                    executed_successfully: true,
                    ..
                })
                | Retired::Absent => {}
                Retired::Found(_) | Retired::Damaged => return Ok(true),
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use crate::context::AppContext;
    use crate::database::test_helpers::{
        create_legacy_scheduled_votes_table, seed_legacy_scheduled_vote_row,
    };
    use crate::model::dpns_voting::{DpnsVoteTarget, DpnsVoteTargetKey, VoteTiming};
    use crate::wallet_backend::{
        DetKv,
        kv_test_support::{FailingKv, InMemoryKv},
    };
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

    const VOTER: [u8; 32] = [1; 32];

    /// One schedule as `v1.0.0-weekly.20261006` stored it: `StoredScheduledVote`
    /// in that tag's `src/context/identity_db.rs`, field for field.
    #[derive(serde::Serialize)]
    struct PreReleaseSchedule {
        voter_id: [u8; 32],
        contested_name: String,
        choice: PreReleaseChoice,
        unix_timestamp: u64,
        executed_successfully: bool,
    }

    #[derive(serde::Serialize)]
    enum PreReleaseChoice {
        TowardsIdentity([u8; 32]),
        Abstain,
        Lock,
    }

    /// A profile that started on 0.9.x: one unexecuted schedule per name in the
    /// old database, on a store that counts writes and can be made to fail.
    fn profile_from_0_9(
        names: &[&str],
    ) -> (tempfile::TempDir, Arc<AppContext>, DetKv, Arc<FailingKv>) {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let store = Arc::new(FailingKv::default());
        let kv = DetKv::from_store(store.clone());
        context.set_det_kv_override_for_test(kv.clone());
        create_legacy_scheduled_votes_table(&context.db).unwrap();
        for name in names {
            seed_legacy_scheduled_vote_row(&context.db, &VOTER, name, "Lock", Network::Testnet)
                .unwrap();
        }
        (dir, context, kv, store)
    }

    /// Leave the pre-release build's voter index behind, as it did from the
    /// moment it copied the old schedules into its own queue.
    fn pre_release_took_over(kv: &DetKv, voters: &[[u8; 32]]) {
        use crate::wallet_backend::DetScope;
        kv.put(
            DetScope::Global,
            "det:scheduled_vote_voters:v1",
            &voters.to_vec(),
        )
        .unwrap();
    }

    /// Leave `name` in the pre-release build's queue, cast or still waiting.
    fn pre_release_holds(kv: &DetKv, name: &str, choice: PreReleaseChoice, cast: bool) {
        use crate::wallet_backend::DetScope;
        kv.put(
            DetScope::Identity(&VOTER),
            &format!("det:scheduled_vote:{name}"),
            &PreReleaseSchedule {
                voter_id: VOTER,
                contested_name: name.into(),
                choice,
                unix_timestamp: 1_700_000_000,
                executed_successfully: cast,
            },
        )
        .unwrap();
        pre_release_took_over(kv, &[VOTER]);
    }

    fn first_noticed_at(kv: &DetKv) -> Option<u64> {
        use crate::wallet_backend::{DetScope, network_prefix};
        kv.get(
            DetScope::Global,
            &format!(
                "{}{}",
                super::NOTICED_AT_KEY_PREFIX,
                network_prefix(Network::Testnet)
            ),
        )
        .unwrap()
    }

    /// 0.9.x, then a pre-release that cast the vote, then this build: the old
    /// row still says "unexecuted", yet nothing is left for the user to do.
    #[test]
    fn startup_notice_skips_schedules_a_pre_release_build_already_cast() {
        let (_dir, context, kv, _store) = profile_from_0_9(&["alice", "bob"]);
        pre_release_holds(&kv, "alice", PreReleaseChoice::Lock, true);
        pre_release_holds(&kv, "bob", PreReleaseChoice::TowardsIdentity([9; 32]), true);

        assert!(!context.has_legacy_scheduled_votes().unwrap());
        assert!(!context.legacy_scheduled_votes_notice_due().unwrap());
        assert_eq!(
            first_noticed_at(&kv),
            None,
            "a notice that was never due must not start its countdown"
        );
    }

    /// The pre-release build keeps its voter index, emptied, after the user
    /// removed a schedule or cleared the cast ones.
    #[test]
    fn startup_notice_skips_schedules_a_pre_release_build_no_longer_holds() {
        let (_dir, context, kv, _store) = profile_from_0_9(&["alice"]);
        pre_release_took_over(&kv, &[]);

        assert!(!context.has_legacy_scheduled_votes().unwrap());
        assert!(!context.legacy_scheduled_votes_notice_due().unwrap());
    }

    /// A schedule the pre-release build had not cast yet is not cast by this
    /// build either, so the notice is true and must stay.
    #[test]
    fn startup_notice_reports_schedules_a_pre_release_build_left_waiting() {
        let (_dir, context, kv, _store) = profile_from_0_9(&["alice", "bob"]);
        pre_release_holds(&kv, "alice", PreReleaseChoice::Lock, true);
        pre_release_holds(&kv, "bob", PreReleaseChoice::Abstain, false);

        assert!(context.has_legacy_scheduled_votes().unwrap());
        assert!(context.legacy_scheduled_votes_notice_due().unwrap());
        assert!(context.dpns_vote_operations().unwrap().is_empty());
    }

    /// Rows the pre-release build could not read were never cast by anyone.
    #[test]
    fn startup_notice_reports_unreadable_schedules_after_a_pre_release_build() {
        let (_dir, context, kv, _store) = profile_from_0_9(&[]);
        seed_legacy_scheduled_vote_row(
            &context.db,
            &VOTER,
            "alice",
            "invalid choice",
            Network::Testnet,
        )
        .unwrap();
        pre_release_took_over(&kv, &[]);

        assert!(context.has_legacy_scheduled_votes().unwrap());
    }

    /// Damaged pre-release records cannot prove a vote was cast, so the check
    /// errs towards telling the user.
    #[test]
    fn startup_notice_reports_schedules_when_pre_release_records_are_damaged() {
        use crate::wallet_backend::DetScope;

        let (_dir, context, kv, _store) = profile_from_0_9(&["alice"]);
        kv.put(
            DetScope::Global,
            "det:scheduled_vote_voters:v1",
            &vec![0xFFu8; 3],
        )
        .unwrap();
        assert!(
            context.has_legacy_scheduled_votes().unwrap(),
            "a damaged voter index proves nothing"
        );

        pre_release_took_over(&kv, &[VOTER]);
        kv.put(
            DetScope::Identity(&VOTER),
            "det:scheduled_vote:alice",
            &vec![0xFFu8; 3],
        )
        .unwrap();
        assert!(
            context.has_legacy_scheduled_votes().unwrap(),
            "a damaged schedule proves nothing"
        );
    }

    /// Schedules that exist only in the pre-release build's queue (a fresh
    /// install of it, never 0.9.x) are deliberately not reported.
    #[test]
    fn startup_notice_ignores_schedules_made_in_a_pre_release_build() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let kv = DetKv::from_store(Arc::new(InMemoryKv::default()));
        context.set_det_kv_override_for_test(kv.clone());
        pre_release_holds(&kv, "alice", PreReleaseChoice::Lock, false);
        assert!(!context.legacy_scheduled_votes_notice_due().unwrap());

        create_legacy_scheduled_votes_table(&context.db).unwrap();
        assert!(
            !context.legacy_scheduled_votes_notice_due().unwrap(),
            "an empty old table changes nothing"
        );
    }

    /// The check only reads: it never rewrites, prunes or removes what the
    /// pre-release build left behind.
    #[test]
    fn startup_notice_check_leaves_pre_release_records_untouched() {
        let (_dir, context, kv, store) = profile_from_0_9(&["alice", "bob"]);
        pre_release_holds(&kv, "alice", PreReleaseChoice::Lock, true);
        pre_release_holds(&kv, "bob", PreReleaseChoice::Abstain, false);
        let writes_before = store.put_count();
        store.fail_all_deletes(true);

        assert!(context.has_legacy_scheduled_votes().unwrap());
        assert_eq!(store.put_count(), writes_before);
    }

    /// A store that cannot be read is neither "cast" nor "waiting": the caller
    /// gets a typed error and shows its "could not be checked" message.
    #[test]
    fn startup_notice_check_reports_a_store_failure_instead_of_guessing() {
        use crate::backend_task::error::TaskError;

        let (_dir, context, kv, store) = profile_from_0_9(&["alice"]);
        pre_release_holds(&kv, "alice", PreReleaseChoice::Lock, true);

        for failing_key in ["det:scheduled_vote_voters:", "det:scheduled_vote:alice"] {
            store.fail_next_gets_containing(failing_key, 1);
            assert!(
                matches!(
                    context.has_legacy_scheduled_votes(),
                    Err(TaskError::RetiredScheduledVotesRead { .. })
                ),
                "{failing_key}"
            );
        }
        assert!(!context.has_legacy_scheduled_votes().unwrap());
    }
}
