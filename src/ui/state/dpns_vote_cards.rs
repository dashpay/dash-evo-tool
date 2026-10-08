//! View-model for the Masternodes ▸ Votes contest cards.
//!
//! Built when contests, proved vote state, the journal or the node set change —
//! never per frame — so the render path only reads it.

use crate::model::contested_name::ContestedName;
use crate::model::dpns_voting::operator::ChangesLeft;
use crate::model::dpns_voting::{DpnsCurrentVoteState, DpnsVoteTargetStatus};
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use std::sync::Arc;

/// Where one node-set node stands on one contest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeContestStatus {
    NotVoted,
    Voted(ResourceVoteChoice),
    /// Proved state could not be read; the node is left out (VOTE-FR-016).
    Unavailable,
    /// An unresolved operation is sending or checking this node × contest target.
    InFlight,
    /// A scheduled vote holds this node × contest target.
    Scheduled,
    /// The node has used all five votes on this contest.
    NoChangesLeft(ResourceVoteChoice),
}

impl NodeContestStatus {
    /// Classify one node from its proved state, the status of the operation
    /// holding its target lock (if any) and its changes left.
    pub fn classify(
        state: DpnsCurrentVoteState,
        lock: Option<DpnsVoteTargetStatus>,
        changes: ChangesLeft,
    ) -> Self {
        match lock {
            Some(DpnsVoteTargetStatus::Scheduled) => return Self::Scheduled,
            Some(_) => return Self::InFlight,
            None => {}
        }
        match state {
            DpnsCurrentVoteState::Available(None) => Self::NotVoted,
            DpnsCurrentVoteState::Available(Some(choice)) if changes.is_exhausted() => {
                Self::NoChangesLeft(choice)
            }
            DpnsCurrentVoteState::Available(Some(choice)) => Self::Voted(choice),
            DpnsCurrentVoteState::Checking | DpnsCurrentVoteState::Unavailable => Self::Unavailable,
        }
    }

    /// Whether a new decision can be sent with this node now.
    pub fn can_submit(self) -> bool {
        matches!(self, Self::NotVoted | Self::Voted(_))
    }
}

/// One node-set node on one contest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeContestState {
    pub node: Identifier,
    pub status: NodeContestStatus,
    pub current: Option<ResourceVoteChoice>,
    pub changes: ChangesLeft,
}

/// Why no node-set node can vote on a contest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CantVoteReason {
    NoVotingNodes,
    VoteStateUnavailable,
}

/// Which list a card appears in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardPlacement {
    ToDecide,
    Voted,
    CantVote(CantVoteReason),
}

/// Place a card from its nodes: To decide while any node has neither voted
/// nor a vote on its way (the same rule as the attention badge), Voted once
/// every reachable node has voted, is sending, or is scheduled.
pub fn place_card(nodes: &[NodeContestState]) -> CardPlacement {
    if nodes.is_empty() {
        return CardPlacement::CantVote(CantVoteReason::NoVotingNodes);
    }
    let any =
        |predicate: fn(NodeContestStatus) -> bool| nodes.iter().any(|node| predicate(node.status));
    if any(|status| status == NodeContestStatus::NotVoted) {
        CardPlacement::ToDecide
    } else if any(|status| status != NodeContestStatus::Unavailable) {
        CardPlacement::Voted
    } else {
        CardPlacement::CantVote(CantVoteReason::VoteStateUnavailable)
    }
}

/// One open contest as the Votes list shows it.
#[derive(Debug, Clone)]
pub struct VoteCard {
    pub contest: Arc<ContestedName>,
    pub vote_poll_id: Option<Identifier>,
    pub nodes: Vec<NodeContestState>,
    pub placement: CardPlacement,
}

impl VoteCard {
    /// Build a card; `placement` follows [`place_card`].
    pub fn new(
        contest: Arc<ContestedName>,
        vote_poll_id: Option<Identifier>,
        nodes: Vec<NodeContestState>,
    ) -> Self {
        let placement = if vote_poll_id.is_some() {
            place_card(&nodes)
        } else {
            CardPlacement::CantVote(CantVoteReason::VoteStateUnavailable)
        };
        Self {
            contest,
            vote_poll_id,
            nodes,
            placement,
        }
    }

    /// The contest label (normalized, without `.dash`).
    pub fn name(&self) -> &str {
        &self.contest.normalized_contested_name
    }

    pub fn count(&self, predicate: impl Fn(NodeContestStatus) -> bool) -> usize {
        self.nodes
            .iter()
            .filter(|node| predicate(node.status))
            .count()
    }

    /// Nodes that currently hold any vote on this contest.
    pub fn voted_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.current.is_some())
            .count()
    }

    /// Nodes a new decision would be sent with.
    pub fn submittable_count(&self) -> usize {
        self.count(NodeContestStatus::can_submit)
    }

    /// Nodes whose vote is being sent or checked.
    pub fn in_flight_count(&self) -> usize {
        self.count(|status| status == NodeContestStatus::InFlight)
    }

    /// Nodes with a scheduled vote on this contest.
    pub fn scheduled_count(&self) -> usize {
        self.count(|status| status == NodeContestStatus::Scheduled)
    }

    /// The choice every voted node shares, if they agree.
    pub fn shared_choice(&self) -> Option<ResourceVoteChoice> {
        let mut choices = self.nodes.iter().filter_map(|node| node.current);
        let first = choices.next()?;
        choices.all(|choice| choice == first).then_some(first)
    }

    /// Whether the operator can pick a decision on this card at all.
    pub fn accepts_decision(&self) -> bool {
        self.submittable_count() > 0
    }
}

/// A contest-list shortcut (VOTE-FR-082). Each has a visible control: card
/// focus by click, decision choices, the card checkbox, and the tray's Cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    Next,
    Previous,
    /// Vote for the n-th contender (0-based).
    Contender(usize),
    Lock,
    Abstain,
    Clear,
    ToggleSelected,
    Confirm,
}

/// The shortcut bound to an unmodified key press, if any.
pub fn shortcut_for(key: eframe::egui::Key) -> Option<Shortcut> {
    use eframe::egui::Key;
    Some(match key {
        Key::J | Key::ArrowDown => Shortcut::Next,
        Key::K | Key::ArrowUp => Shortcut::Previous,
        Key::Num1 => Shortcut::Contender(0),
        Key::Num2 => Shortcut::Contender(1),
        Key::Num3 => Shortcut::Contender(2),
        Key::Num4 => Shortcut::Contender(3),
        Key::Num5 => Shortcut::Contender(4),
        Key::Num6 => Shortcut::Contender(5),
        Key::Num7 => Shortcut::Contender(6),
        Key::Num8 => Shortcut::Contender(7),
        Key::Num9 => Shortcut::Contender(8),
        Key::L => Shortcut::Lock,
        Key::A => Shortcut::Abstain,
        Key::Num0 => Shortcut::Clear,
        Key::Space => Shortcut::ToggleSelected,
        Key::Enter => Shortcut::Confirm,
        _ => return None,
    })
}

/// The `Shortcuts` popover rows: keys and what they do.
pub const SHORTCUT_HELP: [(&str, &str); 7] = [
    ("J / ↓", "Next name"),
    ("K / ↑", "Previous name"),
    ("1–9", "Vote for that contender"),
    ("L / A", "Lock name / Abstain"),
    ("0", "Clear the decision"),
    ("Space", "Select for a bulk decision"),
    ("Enter", "Review and cast"),
];

/// Move focus within `len` listed cards; wraps neither way.
pub fn move_focus(current: Option<usize>, len: usize, shortcut: Shortcut) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match (current, shortcut) {
        (None, _) => 0,
        (Some(index), Shortcut::Next) => (index + 1).min(len - 1),
        (Some(index), Shortcut::Previous) => index.saturating_sub(1),
        (Some(index), _) => index.min(len - 1),
    })
}

/// Order cards by time left; contests without a known deadline go last.
pub fn sort_by_time_left(cards: &mut [VoteCard]) {
    cards.sort_by(|a, b| {
        a.contest
            .end_time
            .unwrap_or(u64::MAX)
            .cmp(&b.contest.end_time.unwrap_or(u64::MAX))
            .then_with(|| a.name().cmp(b.name()))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::contested_name::ContestState;
    use std::collections::BTreeMap;

    fn contest(name: &str, end_time: Option<u64>) -> Arc<ContestedName> {
        Arc::new(ContestedName {
            normalized_contested_name: name.to_owned(),
            contestants: None,
            locked_votes: None,
            abstain_votes: None,
            awarded_to: None,
            end_time,
            state: ContestState::Ongoing,
            last_updated: None,
            my_votes: BTreeMap::new(),
        })
    }

    fn node(byte: u8, status: NodeContestStatus) -> NodeContestState {
        let current = match status {
            NodeContestStatus::Voted(choice) | NodeContestStatus::NoChangesLeft(choice) => {
                Some(choice)
            }
            _ => None,
        };
        NodeContestState {
            node: Identifier::from([byte; 32]),
            status,
            current,
            changes: ChangesLeft::Known(4),
        }
    }

    #[test]
    fn classification_puts_locks_first_and_exhaustion_on_voted_nodes() {
        let lock = ResourceVoteChoice::Lock;
        assert_eq!(
            NodeContestStatus::classify(
                DpnsCurrentVoteState::Available(None),
                Some(DpnsVoteTargetStatus::Unconfirmed),
                ChangesLeft::Known(4)
            ),
            NodeContestStatus::InFlight
        );
        assert_eq!(
            NodeContestStatus::classify(
                DpnsCurrentVoteState::Available(None),
                Some(DpnsVoteTargetStatus::Scheduled),
                ChangesLeft::Known(4)
            ),
            NodeContestStatus::Scheduled
        );
        assert_eq!(
            NodeContestStatus::classify(
                DpnsCurrentVoteState::Available(Some(lock)),
                None,
                ChangesLeft::Known(0)
            ),
            NodeContestStatus::NoChangesLeft(lock)
        );
        assert_eq!(
            NodeContestStatus::classify(
                DpnsCurrentVoteState::Available(Some(lock)),
                None,
                ChangesLeft::Unknown
            ),
            NodeContestStatus::Voted(lock),
            "unknown changes never block a change; Platform enforces the limit"
        );
        assert_eq!(
            NodeContestStatus::classify(
                DpnsCurrentVoteState::Checking,
                None,
                ChangesLeft::Known(4)
            ),
            NodeContestStatus::Unavailable
        );
    }

    /// VOTE-TC-089: a partly voted contest stays in To decide.
    #[test]
    fn partly_voted_contest_stays_to_decide() {
        let lock = ResourceVoteChoice::Lock;
        let mut nodes = vec![node(1, NodeContestStatus::NotVoted)];
        nodes.extend((2..=6).map(|byte| node(byte, NodeContestStatus::Voted(lock))));
        let card = VoteCard::new(
            contest("alice", Some(1)),
            Some(Identifier::from([9; 32])),
            nodes,
        );
        assert_eq!(card.placement, CardPlacement::ToDecide);
        assert_eq!(card.voted_count(), 5);
        assert_eq!(card.nodes.len(), 6);
    }

    /// VOTE-TC-002: once every node voted the card moves to Voted and stays changeable.
    #[test]
    fn fully_voted_contest_moves_to_voted_and_accepts_a_change() {
        let lock = ResourceVoteChoice::Lock;
        let card = VoteCard::new(
            contest("alice", Some(1)),
            Some(Identifier::from([9; 32])),
            vec![
                node(1, NodeContestStatus::Voted(lock)),
                node(2, NodeContestStatus::Unavailable),
            ],
        );
        assert_eq!(card.placement, CardPlacement::Voted);
        assert_eq!(card.shared_choice(), Some(lock));
        assert!(card.accepts_decision());
        assert_eq!(
            card.submittable_count(),
            1,
            "VOTE-TC-007: the unavailable node is left out"
        );
    }

    /// VOTE-TC-041: a card whose only node is sending needs no decision and
    /// locks that node's choices.
    #[test]
    fn in_flight_card_needs_no_decision_and_locks_its_nodes() {
        let card = VoteCard::new(
            contest("alice", Some(1)),
            Some(Identifier::from([9; 32])),
            vec![node(1, NodeContestStatus::InFlight)],
        );
        assert_eq!(card.placement, CardPlacement::Voted);
        assert_eq!(card.in_flight_count(), 1);
        assert!(!card.accepts_decision());
    }

    #[test]
    fn cards_without_usable_nodes_explain_why() {
        let none = VoteCard::new(contest("a", None), Some(Identifier::from([9; 32])), vec![]);
        assert_eq!(
            none.placement,
            CardPlacement::CantVote(CantVoteReason::NoVotingNodes)
        );
        let unavailable = VoteCard::new(
            contest("b", None),
            Some(Identifier::from([9; 32])),
            vec![node(1, NodeContestStatus::Unavailable)],
        );
        assert_eq!(
            unavailable.placement,
            CardPlacement::CantVote(CantVoteReason::VoteStateUnavailable)
        );
        let exhausted = VoteCard::new(
            contest("c", None),
            Some(Identifier::from([9; 32])),
            vec![node(
                1,
                NodeContestStatus::NoChangesLeft(ResourceVoteChoice::Abstain),
            )],
        );
        assert_eq!(exhausted.placement, CardPlacement::Voted);
        assert!(!exhausted.accepts_decision());
    }

    /// VOTE-TC-098 (key map half): keys map to shortcuts; focus stays in bounds.
    #[test]
    fn shortcuts_map_keys_and_bound_focus() {
        use eframe::egui::Key;
        assert_eq!(shortcut_for(Key::J), Some(Shortcut::Next));
        assert_eq!(shortcut_for(Key::ArrowUp), Some(Shortcut::Previous));
        assert_eq!(shortcut_for(Key::Num2), Some(Shortcut::Contender(1)));
        assert_eq!(shortcut_for(Key::L), Some(Shortcut::Lock));
        assert_eq!(shortcut_for(Key::Num0), Some(Shortcut::Clear));
        assert_eq!(shortcut_for(Key::Q), None);
        assert_eq!(move_focus(None, 3, Shortcut::Next), Some(0));
        assert_eq!(move_focus(Some(2), 3, Shortcut::Next), Some(2));
        assert_eq!(move_focus(Some(0), 3, Shortcut::Previous), Some(0));
        assert_eq!(move_focus(Some(1), 3, Shortcut::Previous), Some(0));
        assert_eq!(move_focus(Some(1), 0, Shortcut::Next), None);
    }

    /// VOTE-TC-088: sorted by time left, unknown deadlines last.
    #[test]
    fn cards_sort_by_time_left() {
        let poll = Some(Identifier::from([9; 32]));
        let mut cards = vec![
            VoteCard::new(contest("four-days", Some(4 * 86_400_000)), poll, vec![]),
            VoteCard::new(contest("unknown", None), poll, vec![]),
            VoteCard::new(contest("twelve-min", Some(720_000)), poll, vec![]),
            VoteCard::new(contest("six-hours", Some(21_600_000)), poll, vec![]),
        ];
        sort_by_time_left(&mut cards);
        assert_eq!(
            cards.iter().map(VoteCard::name).collect::<Vec<_>>(),
            vec!["twelve-min", "six-hours", "four-days", "unknown"]
        );
    }
}
