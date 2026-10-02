//! Aggregate vote composition: staged decisions × node set → targets (VOTE-FR-080).
//!
//! Pure: callers resolve each node's proved state, target lock and changes
//! left beforehand. Every node × decision either becomes a target or a
//! [`SkipReason`]; nothing is silently dropped.

use super::operator::{ChangesLeft, relative_schedule};
use super::{
    DpnsCurrentVoteState, DpnsVoteTarget, DpnsVoteTargetKey, VoteTiming,
    validate_dpns_schedule_time,
};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// One staged decision: a contest and the choice for every node-set node.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub contested_name: String,
    pub vote_poll_id: Identifier,
    pub choice: ResourceVoteChoice,
    pub end_time: Option<TimestampMillis>,
}

/// One node-set node's standing on one decision's contest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeStanding {
    pub state: DpnsCurrentVoteState,
    /// An unresolved operation holds this node × contest target.
    pub locked: bool,
    pub changes: ChangesLeft,
}

/// A node of the node set, as the composer sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerNode {
    pub voter_id: Identifier,
    pub alias: Option<String>,
}

/// When the batch is cast (VOTE-FR-023).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchTiming {
    Now,
    /// `preset` before each contest's end (VOTE-FR-081).
    BeforeEnd(Duration),
    At(TimestampMillis),
}

/// A per-node override under `Adjust nodes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeTiming {
    Batch,
    Override(BatchTiming),
    DontUse,
}

/// Why a node × decision produces no target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SkipReason {
    /// The node already holds the requested choice (VOTE-FR-013).
    AlreadyVoted,
    /// Five of five votes used on this contest (VOTE-FR-078).
    NoChangesLeft,
    /// Proved state is unavailable (VOTE-FR-016).
    VoteStateUnavailable,
    /// An earlier vote on this contest is still unresolved (VOTE-FR-034).
    AlreadyInProgress,
    /// The operator chose `Don't use this node`.
    NotUsed,
}

/// One skipped node × decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedTarget {
    pub voter_id: Identifier,
    pub contested_name: String,
    pub reason: SkipReason,
}

/// Why no plan can be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComposeError {
    #[error("Choose a future time before the contest ends for {contested_name}.dash.")]
    ScheduleOutlastsContest { contested_name: String },
    #[error("Voting has ended for {contested_name}.dash. Remove it from your decisions.")]
    VotingEnded { contested_name: String },
}

/// Exactly what a confirm click sends.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AggregatePlan {
    pub targets: Vec<DpnsVoteTarget>,
    pub skipped: Vec<SkippedTarget>,
    /// Targets asked to vote "when voting is about to end" whose contest is
    /// already inside the lead time, so they are sent now.
    pub ends_soon_now: usize,
}

impl AggregatePlan {
    /// Targets that change an earlier vote.
    pub fn change_count(&self) -> usize {
        self.targets
            .iter()
            .filter(|target| target.current_choice.is_some())
            .count()
    }

    /// Distinct nodes casting at least one vote.
    pub fn node_count(&self) -> usize {
        self.targets
            .iter()
            .map(|target| target.key.voter_id)
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// Whether every target is cast immediately.
    pub fn all_now(&self) -> bool {
        self.targets
            .iter()
            .all(|target| target.timing == VoteTiming::Now)
    }

    /// Skips grouped by reason, in a stable order.
    pub fn skipped_by_reason(&self) -> BTreeMap<SkipReason, usize> {
        let mut counts = BTreeMap::new();
        for skipped in &self.skipped {
            *counts.entry(skipped.reason).or_default() += 1;
        }
        counts
    }
}

/// Resolve a timing for one decision. "Before the end" on a contest already
/// inside the lead time votes now; the flag reports that fallback.
fn resolve_timing(
    timing: BatchTiming,
    decision: &Decision,
    now_ms: u64,
) -> Result<(VoteTiming, bool), ComposeError> {
    let outlasts = || ComposeError::ScheduleOutlastsContest {
        contested_name: decision.contested_name.clone(),
    };
    match timing {
        BatchTiming::Now => Ok((VoteTiming::Now, false)),
        BatchTiming::BeforeEnd(preset) => {
            let end = decision.end_time.ok_or_else(outlasts)?;
            Ok(match relative_schedule(end, preset, now_ms) {
                Ok(at) => (VoteTiming::Scheduled(at), false),
                Err(_) => (VoteTiming::Now, true),
            })
        }
        BatchTiming::At(at) => validate_dpns_schedule_time(at, now_ms, decision.end_time)
            .map(|()| (VoteTiming::Scheduled(at), false))
            .map_err(|_| outlasts()),
    }
}

/// Build the plan for `decisions` across `nodes`.
///
/// `standing(voter, poll)` reports each node's state on a contest; `overrides`
/// holds per-node timing from `Adjust nodes`.
///
/// # Errors
///
/// [`ComposeError`] when a contest has ended or a scheduled time falls
/// outside a contest's window.
pub fn compose(
    network: Network,
    decisions: &[Decision],
    nodes: &[ComposerNode],
    standing: impl Fn(Identifier, Identifier) -> NodeStanding,
    batch: BatchTiming,
    overrides: &BTreeMap<Identifier, NodeTiming>,
    now_ms: u64,
) -> Result<AggregatePlan, ComposeError> {
    let mut plan = AggregatePlan::default();
    for decision in decisions {
        if decision.end_time.is_some_and(|end| end <= now_ms) {
            return Err(ComposeError::VotingEnded {
                contested_name: decision.contested_name.clone(),
            });
        }
        // The batch timing must fit every decision, whichever nodes end up voting.
        let batch_timing = resolve_timing(batch, decision, now_ms)?;
        for node in nodes {
            let skip = |reason| SkippedTarget {
                voter_id: node.voter_id,
                contested_name: decision.contested_name.clone(),
                reason,
            };
            let timing = match overrides.get(&node.voter_id).copied() {
                Some(NodeTiming::DontUse) => {
                    plan.skipped.push(skip(SkipReason::NotUsed));
                    continue;
                }
                Some(NodeTiming::Override(timing)) => Some(timing),
                Some(NodeTiming::Batch) | None => None,
            };
            let standing = standing(node.voter_id, decision.vote_poll_id);
            if standing.locked {
                plan.skipped.push(skip(SkipReason::AlreadyInProgress));
                continue;
            }
            let DpnsCurrentVoteState::Available(current) = standing.state else {
                plan.skipped.push(skip(SkipReason::VoteStateUnavailable));
                continue;
            };
            if current == Some(decision.choice) {
                plan.skipped.push(skip(SkipReason::AlreadyVoted));
                continue;
            }
            if current.is_some() && standing.changes.is_exhausted() {
                plan.skipped.push(skip(SkipReason::NoChangesLeft));
                continue;
            }
            plan.targets.push(DpnsVoteTarget {
                key: DpnsVoteTargetKey {
                    network,
                    voter_id: node.voter_id,
                    vote_poll_id: decision.vote_poll_id,
                },
                voter_alias: node.alias.clone(),
                contested_name: decision.contested_name.clone(),
                requested_choice: decision.choice,
                current_choice: current,
                timing: {
                    let (timing, ends_soon) = match timing {
                        Some(timing) => resolve_timing(timing, decision, now_ms)?,
                        None => batch_timing,
                    };
                    plan.ends_soon_now += usize::from(ends_soon);
                    timing
                },
            });
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;

    fn id(byte: u8) -> Identifier {
        Identifier::from([byte; 32])
    }

    fn decision(name: &str, poll: u8, choice: ResourceVoteChoice, end: u64) -> Decision {
        Decision {
            contested_name: name.to_owned(),
            vote_poll_id: id(poll),
            choice,
            end_time: Some(end),
        }
    }

    fn nodes(bytes: &[u8]) -> Vec<ComposerNode> {
        bytes
            .iter()
            .map(|byte| ComposerNode {
                voter_id: id(*byte),
                alias: None,
            })
            .collect()
    }

    fn available(current: Option<ResourceVoteChoice>) -> NodeStanding {
        NodeStanding {
            state: DpnsCurrentVoteState::Available(current),
            locked: false,
            changes: ChangesLeft::Known(4),
        }
    }

    /// VOTE-FR-080: two decisions × three nodes give six targets, one per node and name.
    #[test]
    fn one_target_per_node_and_name() {
        let lock = ResourceVoteChoice::Lock;
        let plan = compose(
            Network::Testnet,
            &[
                decision("alice", 10, lock, 100 * MIN),
                decision("bob", 11, ResourceVoteChoice::Abstain, 100 * MIN),
            ],
            &nodes(&[1, 2, 3]),
            |_, _| available(None),
            BatchTiming::Now,
            &BTreeMap::new(),
            0,
        )
        .unwrap();
        assert_eq!(plan.targets.len(), 6);
        assert_eq!(plan.node_count(), 3);
        assert_eq!(plan.change_count(), 0);
        assert!(plan.all_now());
        assert!(plan.skipped.is_empty());
    }

    /// VOTE-TC-003/007/041/096: every exclusion becomes a typed skip.
    #[test]
    fn exclusions_are_skipped_with_reasons() {
        let lock = ResourceVoteChoice::Lock;
        let abstain = ResourceVoteChoice::Abstain;
        let standing = |voter: Identifier, _| match voter.to_buffer()[0] {
            1 => available(Some(lock)),
            2 => NodeStanding {
                changes: ChangesLeft::Known(0),
                ..available(Some(abstain))
            },
            3 => NodeStanding {
                state: DpnsCurrentVoteState::Unavailable,
                ..available(None)
            },
            4 => NodeStanding {
                locked: true,
                ..available(None)
            },
            6 => available(Some(abstain)),
            _ => available(None),
        };
        let overrides = BTreeMap::from([(id(5), NodeTiming::DontUse)]);
        let plan = compose(
            Network::Testnet,
            &[decision("alice", 10, lock, 100 * MIN)],
            &nodes(&[1, 2, 3, 4, 5, 6, 7]),
            standing,
            BatchTiming::Now,
            &overrides,
            0,
        )
        .unwrap();
        assert_eq!(
            plan.skipped_by_reason(),
            BTreeMap::from([
                (SkipReason::AlreadyVoted, 1),
                (SkipReason::NoChangesLeft, 1),
                (SkipReason::VoteStateUnavailable, 1),
                (SkipReason::AlreadyInProgress, 1),
                (SkipReason::NotUsed, 1),
            ])
        );
        assert_eq!(
            plan.targets
                .iter()
                .map(|target| target.key.voter_id)
                .collect::<Vec<_>>(),
            vec![id(6), id(7)]
        );
        assert_eq!(plan.change_count(), 1, "node 6 changes Abstain → Lock");
    }

    /// Unknown changes never block: Platform enforces the five-vote limit.
    #[test]
    fn unknown_changes_left_still_submits() {
        let plan = compose(
            Network::Testnet,
            &[decision("alice", 10, ResourceVoteChoice::Lock, 100 * MIN)],
            &nodes(&[1]),
            |_, _| NodeStanding {
                changes: ChangesLeft::Unknown,
                ..available(Some(ResourceVoteChoice::Abstain))
            },
            BatchTiming::Now,
            &BTreeMap::new(),
            0,
        )
        .unwrap();
        assert_eq!(plan.targets.len(), 1);
    }

    /// VOTE-TC-067/068: "about to end" resolves per contest; per-node overrides win.
    #[test]
    fn before_end_resolves_per_contest_and_overrides_apply() {
        let lock = ResourceVoteChoice::Lock;
        let overrides = BTreeMap::from([(id(2), NodeTiming::Override(BatchTiming::Now))]);
        let plan = compose(
            Network::Testnet,
            &[
                decision("alice", 10, lock, 60 * MIN),
                decision("bob", 11, lock, 90 * MIN),
            ],
            &nodes(&[1, 2]),
            |_, _| available(None),
            BatchTiming::BeforeEnd(Duration::from_secs(10 * 60)),
            &overrides,
            0,
        )
        .unwrap();
        let timings: Vec<(u8, &str, VoteTiming)> = plan
            .targets
            .iter()
            .map(|target| {
                (
                    target.key.voter_id.to_buffer()[0],
                    target.contested_name.as_str(),
                    target.timing,
                )
            })
            .collect();
        assert_eq!(
            timings,
            vec![
                (1, "alice", VoteTiming::Scheduled(50 * MIN)),
                (2, "alice", VoteTiming::Now),
                (1, "bob", VoteTiming::Scheduled(80 * MIN)),
                (2, "bob", VoteTiming::Now),
            ]
        );
        assert!(!plan.all_now());
    }

    /// VOTE-FR-081: a time at or after the deadline is refused.
    #[test]
    fn schedules_outside_the_window_are_rejected() {
        let lock = ResourceVoteChoice::Lock;
        let compose_at = |timing| {
            compose(
                Network::Testnet,
                &[decision("alice", 10, lock, 5 * MIN)],
                &nodes(&[1]),
                |_, _| available(None),
                timing,
                &BTreeMap::new(),
                0,
            )
        };
        let outlasts = Err(ComposeError::ScheduleOutlastsContest {
            contested_name: "alice".to_owned(),
        });
        assert_eq!(compose_at(BatchTiming::At(5 * MIN)), outlasts);
        assert!(compose_at(BatchTiming::At(4 * MIN)).is_ok());

        let all_skipped = compose(
            Network::Testnet,
            &[decision("alice", 10, lock, 5 * MIN)],
            &nodes(&[1]),
            |_, _| NodeStanding {
                state: DpnsCurrentVoteState::Unavailable,
                ..available(None)
            },
            BatchTiming::At(5 * MIN),
            &BTreeMap::new(),
            0,
        );
        assert_eq!(
            all_skipped, outlasts,
            "the batch time is checked even when every node is skipped"
        );
    }

    /// VOTE-FR-081: "before the end" votes now on contests already inside the
    /// lead time and schedules the rest; the plan counts the early votes.
    #[test]
    fn before_end_votes_now_inside_the_lead_time() {
        let lock = ResourceVoteChoice::Lock;
        let hour = 60 * MIN;
        let plan = compose(
            Network::Mainnet,
            &[
                decision("far", 10, lock, 240 * hour),
                decision("urgent", 11, lock, 3 * hour),
            ],
            &nodes(&[1, 2]),
            |_, _| available(None),
            BatchTiming::BeforeEnd(Duration::from_secs(6 * 3600)),
            &BTreeMap::new(),
            0,
        )
        .expect("the urgent contest no longer blocks the batch");
        let timing = |name: &str| {
            plan.targets
                .iter()
                .filter(|target| target.contested_name == name)
                .map(|target| target.timing)
                .collect::<Vec<_>>()
        };
        assert_eq!(timing("far"), vec![VoteTiming::Scheduled(234 * hour); 2]);
        assert_eq!(timing("urgent"), vec![VoteTiming::Now; 2]);
        assert_eq!(plan.ends_soon_now, 2);
    }

    /// VOTE-TC-103 (compose half): a decision whose voting ended is refused.
    #[test]
    fn ended_contests_are_refused() {
        let result = compose(
            Network::Testnet,
            &[decision("alice", 10, ResourceVoteChoice::Lock, MIN)],
            &nodes(&[1]),
            |_, _| available(None),
            BatchTiming::Now,
            &BTreeMap::new(),
            MIN,
        );
        assert_eq!(
            result,
            Err(ComposeError::VotingEnded {
                contested_name: "alice".to_owned()
            })
        );
    }
}
