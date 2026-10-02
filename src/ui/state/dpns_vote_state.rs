//! Per-screen cache of proved DPNS vote state for immediate-mode rendering.

use std::collections::BTreeMap;

use dash_sdk::platform::Identifier;

use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dpns_voting::DpnsCurrentVoteState;

#[derive(Debug, Clone, Default)]
pub struct DpnsVoteStateSnapshot {
    states: BTreeMap<(Identifier, Identifier), DpnsCurrentVoteState>,
    loaded: bool,
}

impl DpnsVoteStateSnapshot {
    pub fn load(
        app_context: &AppContext,
        voter_ids: &[Identifier],
        vote_poll_ids: &[Identifier],
    ) -> Result<Self, TaskError> {
        let mut snapshot = Self::default();
        snapshot.refresh(app_context, voter_ids, vote_poll_ids)?;
        Ok(snapshot)
    }

    /// Publish successful reads and mark failed voter reads unavailable before returning an error.
    pub fn refresh(
        &mut self,
        app_context: &AppContext,
        voter_ids: &[Identifier],
        vote_poll_ids: &[Identifier],
    ) -> Result<(), TaskError> {
        self.refresh_voters(
            app_context,
            voter_ids
                .iter()
                .map(|voter| (*voter, vote_poll_ids.to_vec())),
        )
    }

    pub(crate) fn reload(&mut self, app_context: &AppContext) -> Result<(), TaskError> {
        let mut polls_by_voter = BTreeMap::<Identifier, Vec<Identifier>>::new();
        for &(voter_id, vote_poll_id) in self.states.keys() {
            polls_by_voter
                .entry(voter_id)
                .or_default()
                .push(vote_poll_id);
        }

        self.refresh_voters(app_context, polls_by_voter)
    }

    fn refresh_voters(
        &mut self,
        app_context: &AppContext,
        polls_by_voter: impl IntoIterator<Item = (Identifier, Vec<Identifier>)>,
    ) -> Result<(), TaskError> {
        let mut states = BTreeMap::new();
        let mut first_error = None;
        for (voter_id, vote_poll_ids) in polls_by_voter {
            let voter_states = match app_context
                .dpns_current_vote_states(voter_id, vote_poll_ids.iter().copied())
            {
                Ok(states) => states,
                Err(error) => {
                    first_error.get_or_insert(error);
                    vote_poll_ids
                        .into_iter()
                        .map(|poll| (poll, DpnsCurrentVoteState::Unavailable))
                        .collect()
                }
            };
            states.extend(
                voter_states
                    .into_iter()
                    .map(|(poll_id, state)| ((voter_id, poll_id), state)),
            );
        }
        self.states = states;
        self.loaded = true;
        first_error.map_or(Ok(()), Err)
    }

    pub fn state(&self, voter_id: Identifier, vote_poll_id: Identifier) -> DpnsCurrentVoteState {
        if !self.loaded {
            return DpnsCurrentVoteState::Unavailable;
        }
        self.states
            .get(&(voter_id, vote_poll_id))
            .copied()
            .unwrap_or(DpnsCurrentVoteState::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voting_ui_failed_snapshot_read_invalidates_only_the_affected_voter() {
        use crate::wallet_backend::{DetKv, kv_test_support::FailingKv};
        use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(FailingKv::default());
        let kv = DetKv::from_store(store.clone());
        let context = crate::context::test_support::test_app_context_with_kv(
            dir.path(),
            Arc::new(kv.clone()),
        );
        context.set_det_kv_override_for_test(kv);
        let voters = [Identifier::from([1; 32]), Identifier::from([2; 32])];
        let poll = Identifier::from([3; 32]);
        for voter in voters {
            context
                .cache_confirmed_dpns_vote(voter, poll, ResourceVoteChoice::Lock)
                .unwrap();
        }
        let mut snapshot = DpnsVoteStateSnapshot::load(&context, &voters, &[poll]).unwrap();
        for reload in [false, true] {
            snapshot.refresh(&context, &voters, &[poll]).unwrap();
            store.fail_next_gets_containing("dpns", 1);
            let result = if reload {
                snapshot.reload(&context)
            } else {
                snapshot.refresh(&context, &voters, &[poll])
            };
            assert!(result.is_err());
            assert_eq!(
                snapshot.state(voters[0], poll),
                DpnsCurrentVoteState::Unavailable
            );
            assert_eq!(
                snapshot.state(voters[1], poll),
                DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
            );
        }
    }

    #[test]
    fn render_lookups_use_only_the_in_memory_snapshot() {
        let voter = Identifier::from([1; 32]);
        let poll = Identifier::from([2; 32]);
        let snapshot = DpnsVoteStateSnapshot {
            states: BTreeMap::from([((voter, poll), DpnsCurrentVoteState::Available(None))]),
            loaded: true,
        };

        for _ in 0..120 {
            assert_eq!(
                snapshot.state(voter, poll),
                DpnsCurrentVoteState::Available(None)
            );
        }
    }
}
