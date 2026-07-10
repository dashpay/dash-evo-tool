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
    /// Whether `voter_id` still has an actionable vote to cast on this contest:
    /// the contest is in a votable state and the voter has not already recorded
    /// a vote on it. Drives the Masternodes card DPNS status line (§10.1).
    pub fn is_open_for_voter(&self, voter_id: &Identifier) -> bool {
        self.state.state_is_votable() && !self.my_votes.keys().any(|(id, _, _)| id == voter_id)
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
    /// Number of open contests this node can still vote on.
    pub open_contest_count: usize,
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
    fn not_open_when_voter_already_voted() {
        let voter = Identifier::from([7u8; 32]);
        let mut c = contest(ContestState::Ongoing);
        c.my_votes.insert(
            (voter, PrivateKeyTarget::PrivateKeyOnVoterIdentity, 0),
            ResourceVoteChoice::Abstain,
        );
        assert!(!c.is_open_for_voter(&voter));
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
