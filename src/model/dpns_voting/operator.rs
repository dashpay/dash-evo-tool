//! Operator-convenience model for masternode voting: node sets and their vote
//! weight, changes left per node, tally influence, relative schedules, the
//! attention summary, and the Masternodes segment preference.
//!
//! Pure and stateless: callers supply journal counts, proved state and
//! masternode-list membership.

use super::{DpnsCurrentVoteState, DpnsScheduleEditValidationError, validate_dpns_schedule_time};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::time::Duration;

/// Which half of the Masternodes page is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MasternodesSegment {
    Votes,
    #[default]
    Nodes,
}

/// The segment Masternodes opens on: Votes while anything needs the operator,
/// otherwise the segment they used last (Nodes when there is no history).
pub fn opening_masternodes_segment(
    attention_count: usize,
    last_used: Option<MasternodesSegment>,
) -> MasternodesSegment {
    if attention_count > 0 {
        MasternodesSegment::Votes
    } else {
        last_used.unwrap_or_default()
    }
}

/// Vote weight of a regular masternode.
pub const MASTERNODE_VOTE_WEIGHT: u32 = 1;
/// Vote weight of an evonode (Platform counts its vote four times).
pub const EVONODE_VOTE_WEIGHT: u32 = 4;
/// Votes Platform accepts per node per contest: the first vote plus four changes.
pub const MAX_VOTES_PER_NODE_PER_CONTEST: u8 = 5;

/// Node kind as far as vote weight is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VotingNodeKind {
    Masternode,
    Evonode,
}

/// Platform vote weight of one node.
pub fn node_weight(kind: VotingNodeKind) -> u32 {
    match kind {
        VotingNodeKind::Masternode => MASTERNODE_VOTE_WEIGHT,
        VotingNodeKind::Evonode => EVONODE_VOTE_WEIGHT,
    }
}

/// Whether a node is in the current deterministic masternode list.
///
/// `Unknown` (list not synced yet) never excludes a node: Platform itself
/// rejects votes from nodes outside the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListMembership {
    Listed,
    NotListed,
    Unknown,
}

/// The facts about one loaded node a node set is resolved against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VotingNode {
    /// ProTxHash identity of the masternode.
    pub id: Identifier,
    pub kind: VotingNodeKind,
    pub has_voting_key: bool,
    pub membership: ListMembership,
    /// The operator's name for the node, if set.
    pub alias: Option<String>,
}

/// The operator's persistent choice of which nodes vote.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NodeSet {
    #[default]
    All,
    EvonodesOnly,
    MasternodesOnly,
    Custom(BTreeSet<Identifier>),
}

/// Why a node in the set cannot vote at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeExclusion {
    NoVotingKey,
    NotInMasternodeList,
}

/// A node set resolved against the loaded nodes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedNodeSet {
    /// Nodes that vote, in input order.
    pub included: Vec<Identifier>,
    /// Nodes the set selects but that cannot vote, with the reason.
    pub excluded: Vec<(Identifier, NodeExclusion)>,
    /// Summed vote weight of `included`.
    pub weight: u32,
    pub evonodes: usize,
    pub masternodes: usize,
}

impl NodeSet {
    /// Whether the set selects `node`, ignoring whether it can vote.
    pub fn selects(&self, node: &VotingNode) -> bool {
        match self {
            Self::All => true,
            Self::EvonodesOnly => node.kind == VotingNodeKind::Evonode,
            Self::MasternodesOnly => node.kind == VotingNodeKind::Masternode,
            Self::Custom(ids) => ids.contains(&node.id),
        }
    }

    /// Resolve the set: selected nodes that can vote are included, the rest are
    /// excluded with a typed reason.
    pub fn resolve(&self, nodes: &[VotingNode]) -> ResolvedNodeSet {
        let mut resolved = ResolvedNodeSet::default();
        for node in nodes.iter().filter(|node| self.selects(node)) {
            if let Some(reason) = node_exclusion(node) {
                resolved.excluded.push((node.id, reason));
                continue;
            }
            resolved.included.push(node.id);
            resolved.weight += node_weight(node.kind);
            match node.kind {
                VotingNodeKind::Evonode => resolved.evonodes += 1,
                VotingNodeKind::Masternode => resolved.masternodes += 1,
            }
        }
        resolved
    }
}

/// Why `node` cannot vote at all, if it cannot.
pub fn node_exclusion(node: &VotingNode) -> Option<NodeExclusion> {
    if !node.has_voting_key {
        Some(NodeExclusion::NoVotingKey)
    } else if node.membership == ListMembership::NotListed {
        Some(NodeExclusion::NotInMasternodeList)
    } else {
        None
    }
}

/// Your nodes' votes on one contest grouped by choice, heaviest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceTally {
    pub choice: ResourceVoteChoice,
    pub nodes: usize,
    pub weight: u32,
}

/// Group nodes' proved choices by choice with summed vote weight; ties break
/// by node count (History, VOTE-FR-087).
pub fn tally_node_choices(
    votes: impl IntoIterator<Item = (VotingNodeKind, ResourceVoteChoice)>,
) -> Vec<ChoiceTally> {
    let mut tallies: Vec<ChoiceTally> = Vec::new();
    for (kind, choice) in votes {
        match tallies.iter_mut().find(|tally| tally.choice == choice) {
            Some(tally) => {
                tally.nodes += 1;
                tally.weight += node_weight(kind);
            }
            None => tallies.push(ChoiceTally {
                choice,
                nodes: 1,
                weight: node_weight(kind),
            }),
        }
    }
    tallies.sort_by(|a, b| b.weight.cmp(&a.weight).then(b.nodes.cmp(&a.nodes)));
    tallies
}

/// One open contest as a node's detail page lists it (VOTE-FR-076).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeVoteRow {
    pub contested_name: String,
    /// `None` = not voted yet; `Some` = the proved current choice.
    pub choice: Option<ResourceVoteChoice>,
    /// Display name of the contender the choice goes to, when cached.
    pub contender_name: Option<String>,
    /// Whether the proved state could be read at all.
    pub state_known: bool,
    pub changes: ChangesLeft,
    pub end_time: Option<u64>,
}

/// Vote changes a node has left on one contest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangesLeft {
    Known(u8),
    /// Platform shows a vote this device never counted.
    Unknown,
}

impl ChangesLeft {
    /// Whether the node has used all of its votes on the contest.
    pub fn is_exhausted(self) -> bool {
        self == Self::Known(0)
    }
}

/// Changes left for one node × contest.
///
/// `confirmed_votes` is this device's durable count of confirmed votes, `None`
/// when it holds no record; `proved_vote_present` is whether Platform shows a
/// current vote. A proved vote the count does not cover was cast elsewhere, so
/// the remainder is unknown rather than guessed.
pub fn changes_left(confirmed_votes: Option<u8>, proved_vote_present: bool) -> ChangesLeft {
    match confirmed_votes.unwrap_or(0) {
        0 if proved_vote_present => ChangesLeft::Unknown,
        0 => ChangesLeft::Known(MAX_VOTES_PER_NODE_PER_CONTEST - 1),
        used => ChangesLeft::Known(MAX_VOTES_PER_NODE_PER_CONTEST.saturating_sub(used)),
    }
}

/// What the operator's node set can do to a contest's current leader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Influence {
    /// `leader` is ahead by `margin` weighted votes and the set's weight can
    /// change who leads.
    CanChangeLeader {
        leader: ResourceVoteChoice,
        margin: u32,
    },
    /// The top contenders are tied.
    Tied,
    /// Lock name has as many votes as the single top request; the request
    /// still wins, because Lock needs strictly more votes.
    TiedWithLock,
}

/// Influence of a node set of `node_set_weight` on a weighted tally.
///
/// `contenders` are `(identity, weighted votes)`; abstain never wins and is
/// not passed. Lock leads only when strictly ahead of every contender.
/// Returns `None` when there is no lead to change or the set's weight is
/// below the margin.
pub fn influence(
    contenders: &[(Identifier, u32)],
    lock_votes: u32,
    node_set_weight: u32,
) -> Option<Influence> {
    let mut ranked = contenders.to_vec();
    ranked.sort_by_key(|(_, votes)| std::cmp::Reverse(*votes));
    let (top_id, top_votes) = *ranked.first()?;
    let runner_up = ranked.get(1).map_or(0, |(_, votes)| *votes);
    if top_votes.max(lock_votes) == 0 {
        return None;
    }
    if ranked.len() > 1 && top_votes == runner_up && lock_votes <= top_votes {
        return Some(Influence::Tied);
    }
    if lock_votes == top_votes {
        return Some(Influence::TiedWithLock);
    }
    let (leader, margin) = if lock_votes > top_votes {
        (ResourceVoteChoice::Lock, lock_votes - top_votes)
    } else {
        (
            ResourceVoteChoice::TowardsIdentity(top_id),
            top_votes - runner_up.max(lock_votes),
        )
    };
    let leader_votes = top_votes.max(lock_votes);
    (leader_votes > 0 && node_set_weight >= margin)
        .then_some(Influence::CanChangeLeader { leader, margin })
}

/// A coarse remaining time, in the largest whole unit that fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeLeft {
    Minutes(u64),
    Hours(u64),
    Days(u64),
}

/// Remaining time until `deadline_ms`, or `None` once it has passed.
///
/// Under an hour counts minutes (at least 1), under two days counts hours,
/// otherwise days.
pub fn time_left(deadline_ms: u64, now_ms: u64) -> Option<TimeLeft> {
    const MINUTE: u64 = 60_000;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    let remaining = deadline_ms.checked_sub(now_ms).filter(|ms| *ms > 0)?;
    Some(if remaining < HOUR {
        TimeLeft::Minutes((remaining / MINUTE).max(1))
    } else if remaining < 2 * DAY {
        TimeLeft::Hours(remaining / HOUR)
    } else {
        TimeLeft::Days(remaining / DAY)
    })
}

/// Default lead time for "When voting is about to end".
pub fn relative_schedule_preset(network: Network) -> Duration {
    match network {
        Network::Mainnet => Duration::from_secs(6 * 60 * 60),
        _ => Duration::from_secs(10 * 60),
    }
}

/// Absolute UTC time for "`preset` before the end" of a contest ending at `end_time`.
///
/// # Errors
///
/// [`DpnsScheduleEditValidationError::Time`] when that moment is not in the
/// future or not before the deadline.
pub fn relative_schedule(
    end_time: TimestampMillis,
    preset: Duration,
    now_ms: u64,
) -> Result<TimestampMillis, DpnsScheduleEditValidationError> {
    let preset_ms = u64::try_from(preset.as_millis()).unwrap_or(u64::MAX);
    let at = end_time
        .checked_sub(preset_ms)
        .ok_or(DpnsScheduleEditValidationError::Time)?;
    validate_dpns_schedule_time(at, now_ms, Some(end_time))?;
    Ok(at)
}

/// Whether a background contest + vote-state refresh should be dispatched.
///
/// Due once `interval` has passed since both the last completed refresh and
/// the last dispatch (so an in-flight refresh is not re-dispatched). All times
/// are Unix milliseconds; `None` means never.
pub fn background_refresh_due(
    last_completed_ms: Option<u64>,
    last_dispatched_ms: Option<u64>,
    now_ms: u64,
    interval: Duration,
) -> bool {
    let interval = u64::try_from(interval.as_millis()).unwrap_or(u64::MAX);
    let elapsed = |at: Option<u64>| at.is_none_or(|at| now_ms.saturating_sub(at) >= interval);
    elapsed(last_completed_ms) && elapsed(last_dispatched_ms)
}

/// How often contests and node vote state refresh in the background.
pub fn background_refresh_interval(network: Network) -> Duration {
    match network {
        Network::Mainnet => Duration::from_secs(30 * 60),
        _ => Duration::from_secs(3 * 60),
    }
}

/// Whether a contest still needs a decision from at least one node-set node.
///
/// Each item is one node's proved state, whether its target is locked by an
/// unresolved operation, and its changes left. A node needs to decide when it
/// has proved "not voted", holds no lock and still has votes.
pub fn contest_needs_decision(
    nodes: impl IntoIterator<Item = (DpnsCurrentVoteState, bool, ChangesLeft)>,
) -> bool {
    nodes.into_iter().any(|(state, locked, changes)| {
        state == DpnsCurrentVoteState::Available(None) && !locked && !changes.is_exhausted()
    })
}

/// One open contest as the attention summary sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContestAttention {
    pub name: String,
    pub end_time: Option<TimestampMillis>,
    pub needs_decision: bool,
}

/// Names listed in the attention chip tooltip.
pub const ATTENTION_TOOLTIP_NAMES: usize = 3;

/// What needs the operator, shown outside the Votes page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AttentionSummary {
    /// Open contests a node-set node still has to decide on.
    pub needs_decision: usize,
    /// Earliest deadline among those contests.
    pub soonest_end: Option<TimestampMillis>,
    /// Targets still `Unconfirmed`.
    pub unresolved: usize,
    /// Up to [`ATTENTION_TOOLTIP_NAMES`] soonest-ending names needing a decision.
    pub soonest_names: Vec<(String, Option<TimestampMillis>)>,
    /// Node-set nodes able to vote; the chip is hidden when there are none.
    pub voting_nodes: usize,
}

impl AttentionSummary {
    /// Build the summary from open contests and the unresolved-target count.
    pub fn new(contests: &[ContestAttention], unresolved: usize, voting_nodes: usize) -> Self {
        let mut pending: Vec<&ContestAttention> =
            contests.iter().filter(|c| c.needs_decision).collect();
        pending.sort_by_key(|contest| contest.end_time.unwrap_or(u64::MAX));
        Self {
            needs_decision: pending.len(),
            soonest_end: pending.iter().filter_map(|c| c.end_time).min(),
            unresolved,
            soonest_names: pending
                .iter()
                .take(ATTENTION_TOOLTIP_NAMES)
                .map(|contest| (contest.name.clone(), contest.end_time))
                .collect(),
            voting_nodes,
        }
    }

    /// Count shown on the Masternodes nav badge.
    pub fn badge_count(&self) -> usize {
        self.needs_decision + self.unresolved
    }

    /// Whether the soonest deadline falls within `urgency_window` of `now_ms`.
    pub fn is_urgent(&self, now_ms: u64, urgency_window: Duration) -> bool {
        let window = u64::try_from(urgency_window.as_millis()).unwrap_or(u64::MAX);
        self.soonest_end
            .is_some_and(|end| end > now_ms && end - now_ms <= window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(byte: u8, kind: VotingNodeKind, key: bool, membership: ListMembership) -> VotingNode {
        VotingNode {
            id: Identifier::from([byte; 32]),
            kind,
            has_voting_key: key,
            membership,
            alias: None,
        }
    }

    fn fleet() -> Vec<VotingNode> {
        vec![
            node(1, VotingNodeKind::Evonode, true, ListMembership::Listed),
            node(2, VotingNodeKind::Masternode, true, ListMembership::Listed),
            node(3, VotingNodeKind::Masternode, false, ListMembership::Listed),
            node(4, VotingNodeKind::Evonode, true, ListMembership::NotListed),
            node(5, VotingNodeKind::Masternode, true, ListMembership::Unknown),
        ]
    }

    #[test]
    fn segment_opens_on_votes_only_while_something_needs_attention() {
        assert_eq!(
            opening_masternodes_segment(2, Some(MasternodesSegment::Nodes)),
            MasternodesSegment::Votes
        );
        assert_eq!(
            opening_masternodes_segment(0, Some(MasternodesSegment::Nodes)),
            MasternodesSegment::Nodes
        );
        assert_eq!(
            opening_masternodes_segment(0, Some(MasternodesSegment::Votes)),
            MasternodesSegment::Votes
        );
        assert_eq!(
            opening_masternodes_segment(0, None),
            MasternodesSegment::Nodes
        );
    }

    #[test]
    fn evonodes_weigh_four_masternodes_one() {
        assert_eq!(node_weight(VotingNodeKind::Evonode), 4);
        assert_eq!(node_weight(VotingNodeKind::Masternode), 1);
    }

    #[test]
    fn all_nodes_excludes_keyless_and_unlisted_nodes_with_reasons() {
        let resolved = NodeSet::All.resolve(&fleet());
        assert_eq!(
            resolved.included,
            vec![
                Identifier::from([1; 32]),
                Identifier::from([2; 32]),
                Identifier::from([5; 32])
            ],
            "unknown membership fails open"
        );
        assert_eq!(
            resolved.excluded,
            vec![
                (Identifier::from([3; 32]), NodeExclusion::NoVotingKey),
                (
                    Identifier::from([4; 32]),
                    NodeExclusion::NotInMasternodeList
                ),
            ]
        );
        assert_eq!(resolved.weight, 4 + 1 + 1);
        assert_eq!((resolved.evonodes, resolved.masternodes), (1, 2));
    }

    #[test]
    fn typed_node_sets_select_by_kind_and_custom_by_id() {
        let nodes = fleet();
        assert_eq!(
            NodeSet::EvonodesOnly.resolve(&nodes).included,
            vec![Identifier::from([1; 32])]
        );
        assert_eq!(NodeSet::EvonodesOnly.resolve(&nodes).weight, 4);
        assert_eq!(
            NodeSet::MasternodesOnly.resolve(&nodes).included,
            vec![Identifier::from([2; 32]), Identifier::from([5; 32])]
        );
        let custom = NodeSet::Custom(BTreeSet::from([
            Identifier::from([2; 32]),
            Identifier::from([4; 32]),
        ]));
        let resolved = custom.resolve(&nodes);
        assert_eq!(resolved.included, vec![Identifier::from([2; 32])]);
        assert_eq!(
            resolved.excluded,
            vec![(
                Identifier::from([4; 32]),
                NodeExclusion::NotInMasternodeList
            )]
        );
    }

    /// VOTE-TC-090 (unit half): the preference round-trips through the
    /// persisted encoding unchanged.
    #[test]
    fn node_set_preference_round_trips() {
        for set in [
            NodeSet::All,
            NodeSet::EvonodesOnly,
            NodeSet::MasternodesOnly,
            NodeSet::Custom(BTreeSet::from([Identifier::from([9; 32])])),
        ] {
            let bytes =
                bincode::serde::encode_to_vec(&set, bincode::config::standard()).expect("encode");
            let (decoded, _): (NodeSet, usize) =
                bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
                    .expect("decode");
            assert_eq!(decoded, set);
        }
    }

    /// VOTE-TC-094: three confirmed votes leave two changes.
    #[test]
    fn changes_left_counts_down_from_the_journal() {
        assert_eq!(changes_left(Some(3), true), ChangesLeft::Known(2));
        assert_eq!(changes_left(Some(1), true), ChangesLeft::Known(4));
        assert_eq!(changes_left(None, false), ChangesLeft::Known(4));
        assert_eq!(changes_left(Some(0), false), ChangesLeft::Known(4));
    }

    /// VOTE-TC-095: a proved vote the device never counted is unknown.
    #[test]
    fn uncounted_proved_vote_leaves_changes_unknown() {
        assert_eq!(changes_left(None, true), ChangesLeft::Unknown);
        assert_eq!(changes_left(Some(0), true), ChangesLeft::Unknown);
    }

    /// VOTE-TC-096 (model half): five confirmed votes exhaust the node.
    #[test]
    fn five_votes_exhaust_a_node() {
        assert!(changes_left(Some(5), true).is_exhausted());
        assert!(changes_left(Some(9), true).is_exhausted());
        assert!(!changes_left(Some(4), true).is_exhausted());
        assert!(!ChangesLeft::Unknown.is_exhausted());
    }

    /// VOTE-TC-092: influence shows when the set's weight reaches the margin.
    /// History aggregate: choices group with weight, heaviest first.
    #[test]
    fn node_choices_tally_by_weight() {
        let zed = ResourceVoteChoice::TowardsIdentity(Identifier::from([9; 32]));
        let tallies = tally_node_choices([
            (VotingNodeKind::Masternode, ResourceVoteChoice::Lock),
            (VotingNodeKind::Masternode, ResourceVoteChoice::Lock),
            (VotingNodeKind::Evonode, zed),
            (VotingNodeKind::Masternode, zed),
        ]);
        assert_eq!(
            tallies,
            vec![
                ChoiceTally {
                    choice: zed,
                    nodes: 2,
                    weight: 5
                },
                ChoiceTally {
                    choice: ResourceVoteChoice::Lock,
                    nodes: 2,
                    weight: 2
                },
            ]
        );
        assert!(tally_node_choices([]).is_empty());
    }

    #[test]
    fn influence_needs_weight_at_least_the_margin() {
        let zed = Identifier::from([1; 32]);
        let amy = Identifier::from([2; 32]);
        let tally = [(zed, 20), (amy, 14)];
        assert_eq!(
            influence(&tally, 0, 51),
            Some(Influence::CanChangeLeader {
                leader: ResourceVoteChoice::TowardsIdentity(zed),
                margin: 6
            })
        );
        assert_eq!(
            influence(&tally, 0, 6),
            Some(Influence::CanChangeLeader {
                leader: ResourceVoteChoice::TowardsIdentity(zed),
                margin: 6
            })
        );
        assert_eq!(influence(&tally, 0, 5), None);
    }

    #[test]
    fn lock_leads_only_when_strictly_ahead() {
        let zed = Identifier::from([1; 32]);
        assert_eq!(
            influence(&[(zed, 10)], 12, 4),
            Some(Influence::CanChangeLeader {
                leader: ResourceVoteChoice::Lock,
                margin: 2
            })
        );
        assert_eq!(
            influence(&[(zed, 10)], 7, 4),
            Some(Influence::CanChangeLeader {
                leader: ResourceVoteChoice::TowardsIdentity(zed),
                margin: 3
            })
        );
    }

    /// Lock equal to the single top request is a Lock tie, never a 0-vote lead.
    #[test]
    fn lock_equal_to_the_leader_is_a_lock_tie() {
        let zed = Identifier::from([1; 32]);
        let amy = Identifier::from([2; 32]);
        assert_eq!(
            influence(&[(zed, 5), (amy, 3)], 5, 4),
            Some(Influence::TiedWithLock)
        );
        assert_eq!(influence(&[(zed, 5)], 5, 4), Some(Influence::TiedWithLock));
    }

    /// VOTE-TC-093 (model half): equal top contenders are a tie.
    #[test]
    fn equal_top_contenders_tie() {
        let tally = [
            (Identifier::from([1; 32]), 8),
            (Identifier::from([2; 32]), 8),
        ];
        assert_eq!(influence(&tally, 3, 0), Some(Influence::Tied));
        assert_eq!(
            influence(&tally, 8, 0),
            Some(Influence::Tied),
            "a two-request tie stays a request tie when Lock matches it"
        );
        assert_eq!(
            influence(&[(tally[0].0, 0), (tally[1].0, 0)], 0, 4),
            None,
            "a contest with no votes has no tie line"
        );
        assert_eq!(
            influence(&[(Identifier::from([1; 32]), 0)], 0, 10),
            None,
            "an empty tally has no lead"
        );
        assert_eq!(influence(&[], 5, 10), None);
    }

    /// VOTE-TC-067 / VOTE-TC-068.
    #[test]
    fn relative_schedule_uses_the_network_preset() {
        let end = 100 * 60 * 60 * 1000;
        let mainnet =
            relative_schedule(end, relative_schedule_preset(Network::Mainnet), 0).expect("mainnet");
        assert_eq!(mainnet, end - 6 * 60 * 60 * 1000);
        let testnet =
            relative_schedule(end, relative_schedule_preset(Network::Testnet), 0).expect("testnet");
        assert_eq!(testnet, end - 10 * 60 * 1000);
    }

    #[test]
    fn relative_schedule_rejects_a_moment_already_passed() {
        let preset = Duration::from_secs(600);
        assert_eq!(
            relative_schedule(1_000_000, preset, 500_000),
            Err(DpnsScheduleEditValidationError::Time)
        );
        assert_eq!(
            relative_schedule(1_000, preset, 0),
            Err(DpnsScheduleEditValidationError::Time),
            "a preset longer than the whole contest underflows"
        );
    }

    /// VOTE-TC-104 (cadence half).
    #[test]
    fn background_refresh_is_slower_on_mainnet() {
        assert_eq!(
            background_refresh_interval(Network::Mainnet),
            Duration::from_secs(1800)
        );
        assert_eq!(
            background_refresh_interval(Network::Testnet),
            Duration::from_secs(180)
        );
        assert_eq!(
            background_refresh_interval(Network::Devnet),
            Duration::from_secs(180)
        );
    }

    #[test]
    fn time_left_uses_the_largest_whole_unit() {
        let minute = 60_000;
        assert_eq!(time_left(10, 10), None);
        assert_eq!(time_left(5, 10), None);
        assert_eq!(time_left(30_000, 0), Some(TimeLeft::Minutes(1)));
        assert_eq!(time_left(12 * minute, 0), Some(TimeLeft::Minutes(12)));
        assert_eq!(time_left(3 * 60 * minute + 5, 0), Some(TimeLeft::Hours(3)));
        assert_eq!(time_left(47 * 60 * minute, 0), Some(TimeLeft::Hours(47)));
        assert_eq!(time_left(4 * 24 * 60 * minute, 0), Some(TimeLeft::Days(4)));
    }

    /// VOTE-TC-104 (dispatch half): due after the interval since both the last
    /// completion and the last dispatch.
    #[test]
    fn background_refresh_waits_for_completion_and_dispatch() {
        let interval = Duration::from_secs(180);
        let ms = 180_000;
        assert!(background_refresh_due(None, None, 0, interval));
        assert!(!background_refresh_due(
            Some(1_000),
            None,
            1_000 + ms - 1,
            interval
        ));
        assert!(background_refresh_due(
            Some(1_000),
            None,
            1_000 + ms,
            interval
        ));
        assert!(
            !background_refresh_due(Some(0), Some(500_000), 600_000, interval),
            "an in-flight refresh is not dispatched again"
        );
        assert!(background_refresh_due(
            Some(0),
            Some(500_000),
            500_000 + ms,
            interval
        ));
    }

    #[test]
    fn a_contest_needs_a_decision_while_an_unlocked_node_has_not_voted() {
        let open = (
            DpnsCurrentVoteState::Available(None),
            false,
            ChangesLeft::Known(4),
        );
        let voted = (
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock)),
            false,
            ChangesLeft::Known(3),
        );
        assert!(contest_needs_decision([voted, open]));
        assert!(!contest_needs_decision([voted]));
        assert!(!contest_needs_decision([(
            DpnsCurrentVoteState::Available(None),
            true,
            ChangesLeft::Known(4)
        )]));
        assert!(!contest_needs_decision([(
            DpnsCurrentVoteState::Unavailable,
            false,
            ChangesLeft::Known(4)
        )]));
    }

    #[test]
    fn attention_summary_counts_and_orders_by_deadline() {
        let contest = |name: &str, end: Option<u64>, needs: bool| ContestAttention {
            name: name.to_owned(),
            end_time: end,
            needs_decision: needs,
        };
        let summary = AttentionSummary::new(
            &[
                contest("late", Some(900), true),
                contest("done", Some(10), false),
                contest("soon", Some(100), true),
                contest("unknown", None, true),
                contest("mid", Some(500), true),
            ],
            2,
            3,
        );
        assert_eq!(summary.needs_decision, 4);
        assert_eq!(summary.voting_nodes, 3);
        assert_eq!(summary.unresolved, 2);
        assert_eq!(summary.badge_count(), 6);
        assert_eq!(summary.soonest_end, Some(100));
        assert_eq!(
            summary
                .soonest_names
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            vec!["soon", "mid", "late"]
        );
    }

    #[test]
    fn urgency_is_within_the_window_and_before_the_deadline() {
        let summary = AttentionSummary {
            needs_decision: 1,
            soonest_end: Some(10_000),
            ..AttentionSummary::default()
        };
        assert!(summary.is_urgent(9_000, Duration::from_secs(1)));
        assert!(!summary.is_urgent(8_000, Duration::from_secs(1)));
        assert!(!summary.is_urgent(10_000, Duration::from_secs(1)));
        assert!(!AttentionSummary::default().is_urgent(0, Duration::from_secs(1)));
    }
}
