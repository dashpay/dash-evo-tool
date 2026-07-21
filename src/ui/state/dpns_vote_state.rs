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

    pub fn refresh(
        &mut self,
        app_context: &AppContext,
        voter_ids: &[Identifier],
        vote_poll_ids: &[Identifier],
    ) -> Result<(), TaskError> {
        let mut states = BTreeMap::new();
        for voter_id in voter_ids {
            states.extend(
                app_context
                    .dpns_current_vote_states(*voter_id, vote_poll_ids.iter().copied())?
                    .into_iter()
                    .map(|(poll_id, state)| ((*voter_id, poll_id), state)),
            );
        }
        self.states = states;
        self.loaded = true;
        Ok(())
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
