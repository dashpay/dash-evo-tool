//! The voting attention signal: which open contests still need a decision
//! from the operator's node set, and how many targets are unresolved.
//!
//! Recomputed on refresh and on coordinator updates, cached for the per-frame
//! readers (top-bar chip, Masternodes badge).

use super::AppContext;
use crate::backend_task::error::TaskError;
use crate::model::dpns_voting::operator::{
    AttentionSummary, ChangesLeft, ContestAttention, ListMembership, VotingNode, VotingNodeKind,
    changes_left, contest_needs_decision,
};
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsVoteOperation, DpnsVotePollAvailability, DpnsVoteTargetKey,
    DpnsVoteTargetStatus, authoritative_dpns_vote_outcome, dpns_vote_lock_holders,
    dpns_vote_poll_availability,
};
use crate::model::qualified_identity::IdentityType;
use crate::utils::time::now_ms;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Distinct targets whose authoritative outcome is still `Unconfirmed`.
pub(crate) fn unresolved_target_count(operations: &[DpnsVoteOperation]) -> usize {
    let keys: BTreeSet<&DpnsVoteTargetKey> = operations
        .iter()
        .flat_map(|operation| operation.targets.iter().map(|outcome| &outcome.target.key))
        .collect();
    keys.into_iter()
        .filter(|key| {
            authoritative_dpns_vote_outcome(operations, key)
                .is_some_and(|outcome| outcome.status == DpnsVoteTargetStatus::Unconfirmed)
        })
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
        let now = now_ms();
        let closed: BTreeSet<Identifier> = self
            .all_contested_names()
            .unwrap_or_default()
            .iter()
            .filter(|contest| {
                dpns_vote_poll_availability(Some(contest), now)
                    == DpnsVotePollAvailability::ProvedClosed
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
                }
            })
            .collect())
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
        match self.compute_dpns_vote_attention() {
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

    fn compute_dpns_vote_attention(&self) -> Result<AttentionSummary, TaskError> {
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
        let operations = self.dpns_vote_operations()?;
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
                        let locked = dpns_vote_lock_holders(&operations, &key).next().is_some();
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
            unresolved_target_count(&operations),
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
