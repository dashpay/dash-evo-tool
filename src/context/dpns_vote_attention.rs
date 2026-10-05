//! The voting attention signal: which open contests still need a decision
//! from the operator's node set, and how many targets are unresolved.
//!
//! Recomputed on refresh and on coordinator updates, cached for the per-frame
//! readers (top-bar chip, Masternodes badge).

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::operator::{
    AttentionSummary, ChangesLeft, ContestAttention, ListMembership, NodeVoteRow, VotingNode,
    VotingNodeKind, changes_left, contest_needs_decision,
};
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsVoteOperation, DpnsVoteOutcome, DpnsVotePollAvailability,
    DpnsVoteTargetKey, DpnsVoteTargetStatus, dpns_vote_authority_rank, dpns_vote_lock_holders,
    dpns_vote_poll_availability,
};
use crate::model::qualified_identity::IdentityType;
use crate::utils::time::now_ms;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Distinct targets whose authoritative outcome is still `Unconfirmed`.
pub(crate) fn unresolved_target_count(operations: &[DpnsVoteOperation]) -> usize {
    let mut authoritative = BTreeMap::new();
    for operation in operations {
        for outcome in &operation.targets {
            let rank = dpns_vote_authority_rank(
                operation.created_at,
                outcome.operation_id,
                outcome.status,
            );
            let entry = authoritative
                .entry(&outcome.target.key)
                .or_insert((rank, outcome.status));
            if rank >= entry.0 {
                *entry = (rank, outcome.status);
            }
        }
    }
    authoritative
        .values()
        .filter(|(_, status)| *status == DpnsVoteTargetStatus::Unconfirmed)
        .count()
}

impl AppContext {
    /// The latest attention summary (cheap; safe to call every frame).
    pub fn dpns_vote_attention(&self) -> Arc<AttentionSummary> {
        Arc::clone(
            &self
                .dpns_vote_attention
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Journal operations as of the last recompute, for the progress drawer
    /// (cheap; safe to call every frame).
    pub fn dpns_vote_progress(&self) -> Arc<[DpnsVoteOperation]> {
        Arc::clone(
            &self
                .dpns_vote_progress
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Ask the voting panel to reopen the confirm step for one failed target
    /// (`Review again` in the progress drawer).
    pub fn request_dpns_vote_review(&self, outcome: DpnsVoteOutcome) {
        *self
            .dpns_vote_review_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(outcome);
    }

    /// Take a pending `Review again` request, if any.
    pub fn take_dpns_vote_review_request(&self) -> Option<DpnsVoteOutcome> {
        self.dpns_vote_review_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Ask the voting panel to show one contest, filtered to its normalized
    /// label. Pair with
    /// `AppAction::SetMainScreenThenGoToMainScreen(RootScreenDPNSActiveContests)`.
    pub fn request_dpns_votes_for_name(&self, normalized_label: String) {
        *self
            .dpns_votes_name_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(normalized_label);
    }

    /// Dismiss settled operations from the progress drawer and the
    /// `Needs attention` row for this session.
    pub fn dismiss_dpns_vote_operations(
        &self,
        ids: impl IntoIterator<Item = crate::model::dpns_voting::DpnsVoteOperationId>,
    ) {
        self.dpns_dismissed_vote_operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(ids);
    }

    /// Operations dismissed in the progress drawer this session.
    pub fn dismissed_dpns_vote_operations(
        &self,
    ) -> std::collections::BTreeSet<crate::model::dpns_voting::DpnsVoteOperationId> {
        self.dpns_dismissed_vote_operations
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Take a pending show-one-contest request, if any.
    pub fn take_dpns_votes_name_request(&self) -> Option<String> {
        self.dpns_votes_name_request
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Record that a contest + vote-state refresh just completed.
    pub(crate) fn mark_dpns_contests_refreshed(&self) {
        self.dpns_contests_refreshed_at_ms
            .store(now_ms(), std::sync::atomic::Ordering::Relaxed);
    }

    /// When the last contest + vote-state refresh completed (Unix ms).
    pub fn dpns_contests_refreshed_at_ms(&self) -> Option<u64> {
        match self
            .dpns_contests_refreshed_at_ms
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            0 => None,
            at => Some(at),
        }
    }

    /// Replace the masternode-list membership snapshot (`None`: list unavailable).
    pub(crate) fn set_masternode_list_membership(
        &self,
        membership: Option<BTreeMap<Identifier, bool>>,
    ) {
        *self
            .masternode_list_membership
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = membership.map(Arc::new);
    }

    /// Re-read masternode-list membership from the SPV list. A list that is
    /// temporarily unavailable keeps the last known snapshot.
    pub(crate) async fn refresh_masternode_list_membership(&self) {
        let Ok(backend) = self.wallet_backend() else {
            return;
        };
        match backend.masternode_list_membership().await {
            Some(membership) => self.set_masternode_list_membership(Some(membership)),
            None => tracing::debug!("Masternode list unavailable; keeping the last membership"),
        }
    }

    /// Drop vote counts for polls the contest cache proves closed.
    pub(crate) fn forget_closed_dpns_vote_counts(&self) {
        let closed: BTreeSet<Identifier> = self
            .all_contested_names()
            .unwrap_or_default()
            .iter()
            .filter(|contest| {
                dpns_vote_poll_availability(Some(contest)) == DpnsVotePollAvailability::ProvedClosed
            })
            .filter_map(|contest| {
                self.dpns_vote_poll_id(&contest.normalized_contested_name)
                    .ok()
            })
            .collect();
        if let Err(error) = self.forget_dpns_vote_counts(&closed) {
            tracing::debug!(
                ?error,
                "Closed-contest vote counts will be pruned on the next refresh"
            );
        }
    }

    /// Whether `node_id` is in the current masternode list, as far as DET knows.
    pub fn masternode_list_membership(&self, node_id: Identifier) -> ListMembership {
        match self
            .masternode_list_membership
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_deref()
        {
            None => ListMembership::Unknown,
            Some(list) if list.contains_key(&node_id) => ListMembership::Listed,
            Some(_) => ListMembership::NotListed,
        }
    }

    /// Every loaded masternode/evonode as the node-set resolver sees it.
    pub fn dpns_voting_nodes(&self) -> Result<Vec<VotingNode>, TaskError> {
        Ok(self
            .load_local_masternode_identities()?
            .into_iter()
            .map(|identity| {
                let id = identity.identity.id();
                VotingNode {
                    id,
                    kind: if identity.identity_type == IdentityType::Evonode {
                        VotingNodeKind::Evonode
                    } else {
                        VotingNodeKind::Masternode
                    },
                    has_voting_key: identity.can_cast_masternode_vote(),
                    membership: self.masternode_list_membership(id),
                    alias: identity.alias.clone(),
                }
            })
            .collect())
    }

    /// Every open contest with this node's proved choice and changes left,
    /// soonest deadline first (VOTE-FR-076).
    pub fn dpns_node_votes(&self, node_id: Identifier) -> Result<Vec<NodeVoteRow>, TaskError> {
        let now = now_ms();
        let contests: Vec<(crate::model::contested_name::ContestedName, Identifier)> = self
            .ongoing_contested_names()?
            .into_iter()
            .filter(|contest| contest.end_time.is_none_or(|end| end > now))
            .filter_map(|contest| {
                let poll = self
                    .dpns_vote_poll_id(&contest.normalized_contested_name)
                    .ok()?;
                Some((contest, poll))
            })
            .collect();
        let states = self
            .dpns_current_vote_states(node_id, contests.iter().map(|(_, poll)| *poll))
            .unwrap_or_default();
        let mut rows: Vec<NodeVoteRow> = contests
            .into_iter()
            .map(|(contest, poll)| {
                let state = states
                    .get(&poll)
                    .copied()
                    .unwrap_or(DpnsCurrentVoteState::Unavailable);
                let choice = match state {
                    DpnsCurrentVoteState::Available(choice) => choice,
                    _ => None,
                };
                let contender_name = match choice {
                    Some(ResourceVoteChoice::TowardsIdentity(id)) => contest
                        .contestants
                        .iter()
                        .flatten()
                        .find(|contender| contender.id == id)
                        .map(|contender| contender.name.clone()),
                    _ => None,
                };
                NodeVoteRow {
                    contested_name: contest.normalized_contested_name,
                    choice,
                    contender_name,
                    state_known: matches!(state, DpnsCurrentVoteState::Available(_)),
                    changes: self.dpns_changes_left(node_id, poll, state),
                    end_time: contest.end_time,
                }
            })
            .collect();
        rows.sort_by_key(|row| (row.end_time.unwrap_or(u64::MAX), row.contested_name.clone()));
        Ok(rows)
    }

    /// Changes left for one node × poll given its proved state.
    pub fn dpns_changes_left(
        &self,
        voter_id: Identifier,
        vote_poll_id: Identifier,
        state: DpnsCurrentVoteState,
    ) -> ChangesLeft {
        let count = self
            .dpns_vote_count(voter_id, vote_poll_id)
            .inspect_err(|error| {
                tracing::debug!(?error, %voter_id, "Vote count unavailable; changes left unknown");
            })
            .ok();
        match count {
            Some(count) => changes_left(
                count,
                matches!(state, DpnsCurrentVoteState::Available(Some(_))),
            ),
            None => ChangesLeft::Unknown,
        }
    }

    /// Recompute and cache the attention summary from the contest cache, the
    /// proved vote state, the journal and the saved node set.
    ///
    /// Storage failures keep the previous summary rather than signalling
    /// nothing.
    pub fn recompute_dpns_vote_attention(&self) -> Arc<AttentionSummary> {
        let operations = match self.dpns_vote_operations() {
            Ok(operations) => operations,
            Err(error) => {
                tracing::debug!(?error, "Keeping the previous voting attention and progress");
                return self.dpns_vote_attention();
            }
        };
        *self
            .dpns_vote_progress
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::from(operations.as_slice());
        match self.compute_dpns_vote_attention(&operations) {
            Ok(summary) => {
                let summary = Arc::new(summary);
                *self
                    .dpns_vote_attention
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::clone(&summary);
                summary
            }
            Err(error) => {
                tracing::debug!(?error, "Keeping the previous voting attention summary");
                self.dpns_vote_attention()
            }
        }
    }

    fn compute_dpns_vote_attention(
        &self,
        operations: &[DpnsVoteOperation],
    ) -> Result<AttentionSummary, TaskError> {
        let resolved = self
            .saved_dpns_node_set()
            .unwrap_or_default()
            .resolve(&self.dpns_voting_nodes()?);
        let now = now_ms();
        let contests: Vec<(String, Option<u64>, Identifier)> = self
            .ongoing_contested_names()?
            .into_iter()
            .filter(|contest| contest.end_time.is_none_or(|end| end > now))
            .filter_map(|contest| {
                let poll = self
                    .dpns_vote_poll_id(&contest.normalized_contested_name)
                    .ok()?;
                Some((contest.normalized_contested_name, contest.end_time, poll))
            })
            .collect();
        let polls: Vec<Identifier> = contests.iter().map(|(_, _, poll)| *poll).collect();
        let mut states = BTreeMap::new();
        for voter in &resolved.included {
            let voter_states = self
                .dpns_current_vote_states(*voter, polls.iter().copied())
                .unwrap_or_default();
            states.insert(*voter, voter_states);
        }

        let network = self.network();
        let attention: Vec<ContestAttention> = contests
            .into_iter()
            .map(|(name, end_time, poll)| {
                let needs_decision =
                    contest_needs_decision(resolved.included.iter().map(|voter| {
                        let state = states
                            .get(voter)
                            .and_then(|voter_states| voter_states.get(&poll))
                            .copied()
                            .unwrap_or(DpnsCurrentVoteState::Unavailable);
                        let key = DpnsVoteTargetKey {
                            network,
                            voter_id: *voter,
                            vote_poll_id: poll,
                        };
                        let locked = dpns_vote_lock_holders(operations, &key).next().is_some();
                        (state, locked, self.dpns_changes_left(*voter, poll, state))
                    }));
                ContestAttention {
                    name,
                    end_time,
                    needs_decision,
                }
            })
            .collect();
        Ok(AttentionSummary::new(
            &attention,
            unresolved_target_count(operations),
            resolved.included.len(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::dpns_voting::{DpnsVoteTarget, VoteTiming};
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;

    fn operation(poll: u8, status: DpnsVoteTargetStatus, created_at: u64) -> DpnsVoteOperation {
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: Network::Testnet,
                voter_id: Identifier::from([1; 32]),
                vote_poll_id: Identifier::from([poll; 32]),
            },
            voter_alias: None,
            contested_name: format!("name-{poll}"),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Now,
        }]);
        operation.targets[0].status = status;
        operation.created_at = created_at;
        operation
    }

    /// VOTE-TC-087 (count half): only targets whose authoritative outcome is
    /// still unconfirmed count, once each.
    #[test]
    fn unresolved_count_follows_the_authoritative_outcome() {
        let operations = vec![
            operation(2, DpnsVoteTargetStatus::Unconfirmed, 1),
            operation(3, DpnsVoteTargetStatus::Unconfirmed, 1),
            operation(3, DpnsVoteTargetStatus::Unconfirmed, 2),
            operation(4, DpnsVoteTargetStatus::Confirmed, 1),
        ];
        assert_eq!(unresolved_target_count(&operations), 2);
        assert_eq!(unresolved_target_count(&[]), 0);
    }

    #[test]
    fn unresolved_count_preserves_authority_order_for_competing_outcomes() {
        use crate::model::dpns_voting::DpnsVoteOperationId;
        use DpnsVoteTargetStatus::{Confirmed, Scheduled, Unconfirmed};

        for (first_status, first_time, second_status, second_time, expected) in [
            (Unconfirmed, 1, Confirmed, 2, 1),
            (Scheduled, 1, Unconfirmed, 2, 1),
            (Unconfirmed, 2, Scheduled, 1, 1),
            (Unconfirmed, 1, Scheduled, 1, 0),
            (Scheduled, 1, Unconfirmed, 1, 1),
        ] {
            let mut first = operation(2, first_status, first_time);
            first.id = DpnsVoteOperationId::from_bytes([1; 16]);
            first.targets[0].operation_id = first.id;
            let mut second = operation(2, second_status, second_time);
            second.id = DpnsVoteOperationId::from_bytes([2; 16]);
            second.targets[0].operation_id = second.id;
            let mut operations = vec![first, second];
            assert_eq!(unresolved_target_count(&operations), expected);
            operations.reverse();
            assert_eq!(unresolved_target_count(&operations), expected);
        }
    }

    #[test]
    fn unresolved_count_handles_bulk_operations_and_distinct_nodes() {
        let mut bulk = operation(2, DpnsVoteTargetStatus::Confirmed, 1);
        for poll in 3..=100 {
            let mut outcome = bulk.targets[0].clone();
            outcome.target.key.vote_poll_id = Identifier::from([poll; 32]);
            bulk.targets.push(outcome);
        }
        let first_node = operation(2, DpnsVoteTargetStatus::Unconfirmed, 2);
        let mut second_node = first_node.clone();
        second_node.targets[0].target.key.voter_id = Identifier::from([2; 32]);
        assert_eq!(unresolved_target_count(&[bulk.clone()]), 0);
        assert_eq!(unresolved_target_count(&[bulk, first_node, second_node]), 2);
    }

    /// VOTE-TC-094 (node detail half): the node's votes list every open
    /// contest with its proved choice, contender and changes left.
    #[test]
    fn node_votes_list_open_contests_with_choice_and_changes() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let kv = crate::wallet_backend::DetKv::from_store(Arc::new(
            crate::wallet_backend::kv_test_support::InMemoryKv::default(),
        ));
        let context = crate::context::test_support::test_app_context_with_kv(
            temp_dir.path(),
            Arc::new(kv.clone()),
        );
        context.set_det_kv_override_for_test(kv);
        context.seed_dpns_contest_for_test("beta", Some(now_ms() + 600_000), false);
        context.seed_dpns_contest_for_test("alpha", Some(now_ms() + 60_000), false);
        let node = Identifier::from([1; 32]);
        let alpha = context.dpns_vote_poll_id("alpha").unwrap();
        let contender = ResourceVoteChoice::TowardsIdentity(Identifier::from([3; 32]));
        context
            .cache_confirmed_dpns_vote(node, alpha, contender)
            .unwrap();

        let rows = context.dpns_node_votes(node).unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| row.contested_name.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"],
            "soonest deadline first"
        );
        assert_eq!(rows[0].choice, Some(contender));
        assert_eq!(rows[0].contender_name.as_deref(), Some("alpha"));
        assert!(rows[0].state_known);
        assert_eq!(
            rows[0].changes,
            ChangesLeft::Unknown,
            "a proved vote this device never counted"
        );
    }

    #[test]
    fn membership_is_unknown_until_the_list_is_available() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(temp_dir.path());
        let listed = Identifier::from([1; 32]);
        assert_eq!(
            context.masternode_list_membership(listed),
            ListMembership::Unknown
        );
        context.set_masternode_list_membership(Some(BTreeMap::from([(listed, true)])));
        assert_eq!(
            context.masternode_list_membership(listed),
            ListMembership::Listed
        );
        assert_eq!(
            context.masternode_list_membership(Identifier::from([2; 32])),
            ListMembership::NotListed
        );
    }
}
