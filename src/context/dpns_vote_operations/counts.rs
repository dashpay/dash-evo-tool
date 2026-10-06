//! Durable count of votes this device saw Platform apply, per node × poll.
//!
//! Journal history is pruned, so "changes left" cannot be counted from it. The
//! count lives in its own namespace, survives pruning, and is dropped only for
//! polls proven closed.

use super::keys::{operation_err, unreadable_operation_err};
use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::{
    DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteTargetKey, DpnsVoteTargetStatus,
};
use crate::wallet_backend::{DetKv, DetScope, network_prefix};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::platform::Identifier;

const VOTE_COUNT_KEY_PREFIX: &str = "det:dpns_vote_counts:v1:";
/// The operation whose confirmed vote was last added to a node × poll count.
const COUNTED_OPERATION_KEY_PREFIX: &str = "det:dpns_vote_counted:v1:";

fn count_key_prefix(network: Network) -> String {
    format!("{VOTE_COUNT_KEY_PREFIX}{}:", network_prefix(network))
}

fn count_key(network: Network, voter_id: Identifier, vote_poll_id: Identifier) -> String {
    format!("{}{voter_id}:{vote_poll_id}", count_key_prefix(network))
}

fn counted_operation_key_prefix(network: Network) -> String {
    format!("{COUNTED_OPERATION_KEY_PREFIX}{}:", network_prefix(network))
}

fn counted_operation_key(key: &DpnsVoteTargetKey) -> String {
    format!(
        "{}{}:{}",
        counted_operation_key_prefix(key.network),
        key.voter_id,
        key.vote_poll_id
    )
}

/// Whether moving `previous` → `next` records a vote Platform applied.
///
/// Only a broadcast target (Confirming or Unconfirmed) that becomes Confirmed
/// spent a vote; a queued target confirmed as an exact no-op did not.
fn spent_a_vote(previous: Option<DpnsVoteTargetStatus>, next: DpnsVoteTargetStatus) -> bool {
    next == DpnsVoteTargetStatus::Confirmed
        && matches!(
            previous,
            Some(DpnsVoteTargetStatus::Confirming | DpnsVoteTargetStatus::Unconfirmed)
        )
}

/// Targets of `next` that spent a vote relative to the stored `previous` record.
pub(super) fn newly_spent_targets<'a>(
    previous: Option<&DpnsVoteOperation>,
    next: &'a DpnsVoteOperation,
) -> Vec<&'a DpnsVoteTargetKey> {
    next.targets
        .iter()
        .filter(|outcome| {
            let before = previous
                .and_then(|operation| operation.outcome(&outcome.target.key))
                .map(|previous| previous.status);
            spent_a_vote(before, outcome.status)
        })
        .map(|outcome| &outcome.target.key)
        .collect()
}

/// Add the vote `operation_id` spent on `key`, unless it is already counted.
///
/// Callers hold the journal guard and bump before persisting the transition.
/// When that write fails the target is confirmed, and reaches here, a second
/// time; the recorded operation keeps the same vote from counting twice. One
/// record per node × poll is enough: an unresolved target holds its lock, so
/// no other operation can confirm on it in between.
///
/// The count is written before the record of who was counted, so a failure
/// between the two over-counts, which understates changes left instead of
/// overstating them.
pub(super) fn bump_vote_count(
    kv: &DetKv,
    key: &DpnsVoteTargetKey,
    operation_id: DpnsVoteOperationId,
) -> Result<(), TaskError> {
    let counted_key = counted_operation_key(key);
    let counted: Option<[u8; 16]> = kv
        .get(DetScope::Global, &counted_key)
        .map_err(unreadable_operation_err)?;
    if counted == Some(operation_id.to_bytes()) {
        return Ok(());
    }
    let storage_key = count_key(key.network, key.voter_id, key.vote_poll_id);
    let count: u8 = kv
        .get(DetScope::Global, &storage_key)
        .map_err(unreadable_operation_err)?
        .unwrap_or(0);
    kv.put(DetScope::Global, &storage_key, &count.saturating_add(1))
        .map_err(operation_err)?;
    kv.put(DetScope::Global, &counted_key, &operation_id.to_bytes())
        .map_err(operation_err)
}

/// Votes this device saw applied for one node × poll; `None` without a record.
pub(super) fn vote_count(
    kv: &DetKv,
    network: Network,
    voter_id: Identifier,
    vote_poll_id: Identifier,
) -> Result<Option<u8>, TaskError> {
    kv.get(
        DetScope::Global,
        &count_key(network, voter_id, vote_poll_id),
    )
    .map_err(unreadable_operation_err)
}

/// Forget the counts of every poll in `closed_polls`.
pub(super) fn forget_vote_counts(
    kv: &DetKv,
    network: Network,
    closed_polls: &std::collections::BTreeSet<Identifier>,
) -> Result<usize, TaskError> {
    if closed_polls.is_empty() {
        return Ok(0);
    }
    let suffixes: Vec<String> = closed_polls.iter().map(|poll| format!(":{poll}")).collect();
    let mut removed = 0;
    for (prefix, is_count) in [
        (count_key_prefix(network), true),
        (counted_operation_key_prefix(network), false),
    ] {
        for key in kv
            .list(DetScope::Global, Some(&prefix))
            .map_err(unreadable_operation_err)?
        {
            if suffixes.iter().any(|suffix| key.ends_with(suffix.as_str())) {
                kv.delete(DetScope::Global, &key).map_err(operation_err)?;
                removed += usize::from(is_count);
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;
    use crate::model::dpns_voting::{DpnsVoteTarget, VoteTiming};
    use crate::wallet_backend::kv_test_support::InMemoryKv;
    use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
    use std::collections::BTreeSet;
    use std::sync::Arc;

    fn key(poll: u8) -> DpnsVoteTargetKey {
        DpnsVoteTargetKey {
            network: Network::Testnet,
            voter_id: Identifier::from([1; 32]),
            vote_poll_id: Identifier::from([poll; 32]),
        }
    }

    fn operation(status: DpnsVoteTargetStatus) -> DpnsVoteOperation {
        let mut operation = AppContext::new_dpns_vote_operation(vec![DpnsVoteTarget {
            key: key(2),
            voter_alias: None,
            contested_name: "alice".to_owned(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Now,
        }]);
        operation.targets[0].status = status;
        operation
    }

    #[test]
    fn only_a_broadcast_target_that_confirms_spends_a_vote() {
        use DpnsVoteTargetStatus::*;
        assert!(spent_a_vote(Some(Confirming), Confirmed));
        assert!(spent_a_vote(Some(Unconfirmed), Confirmed));
        assert!(!spent_a_vote(Some(Queued), Confirmed), "exact no-op");
        assert!(!spent_a_vote(Some(Confirmed), Confirmed), "already counted");
        assert!(!spent_a_vote(None, Confirmed), "migrated legacy record");
        assert!(!spent_a_vote(Some(Unconfirmed), NotApplied));
    }

    #[test]
    fn newly_spent_targets_compare_against_the_stored_record() {
        let before = operation(DpnsVoteTargetStatus::Confirming);
        let mut after = before.clone();
        after.targets[0].status = DpnsVoteTargetStatus::Confirmed;
        assert_eq!(newly_spent_targets(Some(&before), &after), vec![&key(2)]);
        assert!(newly_spent_targets(Some(&after), &after).is_empty());
        assert!(newly_spent_targets(None, &after).is_empty());
    }

    #[test]
    fn counts_accumulate_saturate_and_are_forgotten_for_closed_polls() {
        let kv = DetKv::from_store(Arc::new(InMemoryKv::default()));
        let network = Network::Testnet;
        let voter = Identifier::from([1; 32]);
        assert_eq!(
            vote_count(&kv, network, voter, Identifier::from([2; 32])).unwrap(),
            None
        );
        for operation in 1..=3 {
            let id = DpnsVoteOperationId::from_bytes([operation; 16]);
            bump_vote_count(&kv, &key(2), id).unwrap();
            bump_vote_count(&kv, &key(2), id).unwrap();
        }
        bump_vote_count(&kv, &key(3), DpnsVoteOperationId::from_bytes([3; 16])).unwrap();
        assert_eq!(
            vote_count(&kv, network, voter, Identifier::from([2; 32])).unwrap(),
            Some(3)
        );
        assert_eq!(
            vote_count(&kv, Network::Mainnet, voter, Identifier::from([2; 32])).unwrap(),
            None,
            "counts are network-scoped"
        );

        let removed =
            forget_vote_counts(&kv, network, &BTreeSet::from([Identifier::from([2; 32])])).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(
            vote_count(&kv, network, voter, Identifier::from([2; 32])).unwrap(),
            None
        );
        assert_eq!(
            kv.list(DetScope::Global, Some(COUNTED_OPERATION_KEY_PREFIX))
                .unwrap()
                .len(),
            1,
            "a closed poll keeps no record of who was counted"
        );
        assert_eq!(
            vote_count(&kv, network, voter, Identifier::from([3; 32])).unwrap(),
            Some(1),
            "open polls keep their count"
        );
    }
}
