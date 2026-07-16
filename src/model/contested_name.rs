use crate::model::qualified_identity::PrivateKeyTarget;
use bincode::{Decode, Encode};
use dash_sdk::dpp::identity::{KeyID, TimestampMillis};
use dash_sdk::dpp::prelude::{BlockHeight, CoreBlockHeight, Identifier};
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use std::collections::BTreeMap;

#[derive(Debug, Encode, Decode, Clone, PartialEq)]
pub enum ContestState {
    Unknown,
    Joinable,
    Ongoing,
    WonBy(Identifier),
    Locked,
}

impl ContestState {
    /// Whether the contest still accepts votes — `Joinable` or `Ongoing`.
    pub fn state_is_votable(&self) -> bool {
        matches!(self, ContestState::Joinable | ContestState::Ongoing)
    }
}

#[derive(Debug, Encode, Decode, Clone)]
pub struct ContestedName {
    pub normalized_contested_name: String,
    pub contestants: Option<Vec<Contestant>>,
    pub locked_votes: Option<u32>,
    pub abstain_votes: Option<u32>,
    pub awarded_to: Option<Identifier>,
    pub end_time: Option<TimestampMillis>,
    pub state: ContestState,
    pub last_updated: Option<TimestampMillis>,
    pub my_votes: BTreeMap<(Identifier, PrivateKeyTarget, KeyID), ResourceVoteChoice>,
}

impl ContestedName {
    /// Whether the contest still accepts this node's initial vote or a change.
    pub fn is_open_for_voter(&self, _voter_id: &Identifier) -> bool {
        self.state.state_is_votable()
    }
}

/// Per-node DPNS voting summary shown on the Masternodes card grid.
///
/// Composed by a display-layer read of existing contest + scheduled-vote state
/// (no new backend concept). Feeds the count-first status line: open contests
/// take precedence, then a pending scheduled vote, then "no open contests"
/// (requirements §10.1).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MasternodeContestSummary {
    /// Number of active contests, including contests with an existing vote.
    pub open_contest_count: usize,
    /// Number of active contests whose proved state is `Not voted`.
    pub needs_vote_count: usize,
    /// Whether the node has at least one pending (not-yet-executed) scheduled
    /// vote, reusing the DPNS Scheduled Votes screen's existing state.
    pub has_scheduled_vote: bool,
}

#[derive(Debug, Encode, Decode, Clone)]
pub struct Contestant {
    pub id: Identifier,
    pub name: String,
    pub info: String,
    pub votes: u32,
    pub created_at: Option<TimestampMillis>,
    pub created_at_block_height: Option<BlockHeight>,
    pub created_at_core_block_height: Option<CoreBlockHeight>,
    pub document_id: Identifier,
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;

    fn contest(state: ContestState) -> ContestedName {
        ContestedName {
            normalized_contested_name: "alice".to_string(),
            contestants: None,
            locked_votes: None,
            abstain_votes: None,
            awarded_to: None,
            end_time: None,
            state,
            last_updated: None,
            my_votes: BTreeMap::new(),
        }
    }

    #[test]
    fn open_for_voter_when_votable_and_not_yet_voted() {
        let voter = Identifier::from([7u8; 32]);
        assert!(contest(ContestState::Ongoing).is_open_for_voter(&voter));
        assert!(contest(ContestState::Joinable).is_open_for_voter(&voter));
    }

    #[test]
    fn not_open_when_state_not_votable() {
        let voter = Identifier::from([7u8; 32]);
        assert!(!contest(ContestState::Locked).is_open_for_voter(&voter));
        assert!(!contest(ContestState::Unknown).is_open_for_voter(&voter));
        assert!(
            !contest(ContestState::WonBy(Identifier::from([9u8; 32]))).is_open_for_voter(&voter)
        );
    }

    #[test]
    fn existing_vote_remains_actionable_while_contest_is_votable() {
        let voter = Identifier::from([7u8; 32]);
        let mut c = contest(ContestState::Ongoing);
        c.my_votes.insert(
            (voter, PrivateKeyTarget::PrivateKeyOnVoterIdentity, 0),
            ResourceVoteChoice::Abstain,
        );
        assert!(c.is_open_for_voter(&voter));
    }

    #[test]
    fn open_when_a_different_voter_already_voted() {
        let voter = Identifier::from([7u8; 32]);
        let other = Identifier::from([8u8; 32]);
        let mut c = contest(ContestState::Ongoing);
        c.my_votes.insert(
            (other, PrivateKeyTarget::PrivateKeyOnVoterIdentity, 0),
            ResourceVoteChoice::Abstain,
        );
        assert!(c.is_open_for_voter(&voter));
    }
}
