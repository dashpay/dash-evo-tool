//! Cached Active-contest data for the DPNS screen.

use crate::context::AppContext;
use crate::model::contested_name::ContestedName;
use dash_sdk::platform::Identifier;
use std::sync::Arc;

/// One Active contest with its precomputed Platform vote-poll identifier.
#[derive(Debug, Clone)]
pub struct ActiveDpnsContestView {
    pub contest: Arc<ContestedName>,
    pub vote_poll_id: Option<Identifier>,
}

/// Immutable render snapshot for the Active contests screen.
#[derive(Debug, Clone, Default)]
pub struct ActiveDpnsContestSnapshot {
    contests: Arc<[ActiveDpnsContestView]>,
}

impl ActiveDpnsContestSnapshot {
    /// Build a snapshot while resolving each contest's vote-poll identifier once.
    pub(crate) fn new(app_context: &AppContext, contests: Vec<ContestedName>) -> Self {
        Self::build(contests, |name| app_context.dpns_vote_poll_id(name).ok())
    }

    pub(crate) fn contests(&self) -> Arc<[ActiveDpnsContestView]> {
        Arc::clone(&self.contests)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.contests.is_empty()
    }

    pub(crate) fn vote_poll_ids(&self) -> Vec<Identifier> {
        self.contests
            .iter()
            .filter_map(|view| view.vote_poll_id)
            .collect()
    }

    pub(crate) fn contest(&self, contested_name: &str) -> Option<&ContestedName> {
        self.contests
            .iter()
            .find(|view| view.contest.normalized_contested_name == contested_name)
            .map(|view| view.contest.as_ref())
    }

    fn build(
        contests: Vec<ContestedName>,
        mut poll_id: impl FnMut(&str) -> Option<Identifier>,
    ) -> Self {
        let contests = contests
            .into_iter()
            .map(|contest| {
                let vote_poll_id = poll_id(&contest.normalized_contested_name);
                ActiveDpnsContestView {
                    contest: Arc::new(contest),
                    vote_poll_id,
                }
            })
            .collect::<Vec<_>>();
        Self {
            contests: Arc::from(contests),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::contested_name::ContestState;
    use dash_sdk::platform::Identifier;
    use std::collections::BTreeMap;

    fn contest(name: &str) -> ContestedName {
        ContestedName {
            normalized_contested_name: name.to_owned(),
            contestants: None,
            locked_votes: None,
            abstain_votes: None,
            awarded_to: None,
            end_time: None,
            state: ContestState::Ongoing,
            last_updated: None,
            my_votes: BTreeMap::new(),
        }
    }

    #[test]
    fn active_contest_snapshot_computes_each_poll_id_once() {
        let mut calls = Vec::new();

        let snapshot =
            ActiveDpnsContestSnapshot::build(vec![contest("alice"), contest("bob")], |name| {
                calls.push(name.to_owned());
                Some(Identifier::from([calls.len() as u8; 32]))
            });

        assert_eq!(calls, ["alice", "bob"]);
        let contests = snapshot.contests();
        assert_eq!(contests.len(), 2);
        assert_eq!(contests[0].contest.normalized_contested_name, "alice");
        assert_eq!(contests[0].vote_poll_id, Some(Identifier::from([1; 32])));
        assert_eq!(contests[1].vote_poll_id, Some(Identifier::from([2; 32])));
    }
}
