use crate::model::qualified_identity::PrivateKeyTarget;
use bincode::{Decode, Encode};
use dash_sdk::dpp::identity::{KeyID, TimestampMillis};
use dash_sdk::dpp::prelude::{BlockHeight, CoreBlockHeight, Identifier};
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

/// Maximum number of Unicode scalar values rendered for a pending DPNS label.
pub const MAX_PENDING_USERNAME_DISPLAY_CHARS: usize = 63;

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

    /// The pending DPNS username this contest represents for `identity_id`, if
    /// the identity is a still-undecided contender in it.
    ///
    /// Returns `None` when the contest is already decided (`WonBy` or `Locked`)
    /// or the identity is not among its contenders — an awarded or lost name is
    /// no longer "pending". Used to tell "requested but not yet awarded" apart
    /// from "no username requested".
    pub fn pending_username_for(&self, identity_id: &Identifier) -> Option<PendingUsername> {
        if matches!(self.state, ContestState::WonBy(_) | ContestState::Locked) {
            return None;
        }
        let mine = self
            .contestants
            .as_ref()?
            .iter()
            .find(|c| c.id == *identity_id)?;
        Some(PendingUsername {
            name: mine.name.clone(),
            decided_at: self.end_time,
        })
    }
}

/// A DPNS username an identity has requested but has not yet been awarded — the
/// name contest is still open. Surfaced in the UI as a "Pending" indicator so a
/// requested-but-unawarded name is not mistaken for "no username requested".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingUsername {
    /// The requested name label (without the `.dash` suffix), as submitted.
    pub name: String,
    /// When the request is expected to be decided, in Unix milliseconds.
    /// `None` when the timing is not yet known.
    pub decided_at: Option<TimestampMillis>,
}

/// The highest-priority pending DPNS username `identity_id` has across `contests`.
///
/// Pure and side-effect-free: the caller supplies the contest set (typically the
/// ongoing-contest cache). The earliest known decision time wins; unknown times
/// follow known times, and the name provides a stable tie-breaker.
pub fn pending_username_in<'a, I>(contests: I, identity_id: &Identifier) -> Option<PendingUsername>
where
    I: IntoIterator<Item = &'a ContestedName>,
{
    contests
        .into_iter()
        .filter_map(|contest| contest.pending_username_for(identity_id))
        .min_by(pending_username_priority)
}

/// Build the deterministic pending-username snapshot for every contender.
pub fn pending_usernames_in<'a, I>(contests: I) -> HashMap<Identifier, PendingUsername>
where
    I: IntoIterator<Item = &'a ContestedName>,
{
    let mut pending_by_identity = HashMap::<Identifier, PendingUsername>::new();
    for contest in contests {
        if matches!(contest.state, ContestState::WonBy(_) | ContestState::Locked) {
            continue;
        }
        let Some(contestants) = contest.contestants.as_ref() else {
            continue;
        };
        for contestant in contestants {
            let candidate = PendingUsername {
                name: contestant.name.clone(),
                decided_at: contest.end_time,
            };
            pending_by_identity
                .entry(contestant.id)
                .and_modify(|current| {
                    if pending_username_priority(&candidate, current).is_lt() {
                        current.clone_from(&candidate);
                    }
                })
                .or_insert(candidate);
        }
    }
    pending_by_identity
}

fn pending_username_priority(left: &PendingUsername, right: &PendingUsername) -> Ordering {
    (
        left.decided_at.is_none(),
        left.decided_at.unwrap_or(u64::MAX),
        &left.name,
    )
        .cmp(&(
            right.decided_at.is_none(),
            right.decided_at.unwrap_or(u64::MAX),
            &right.name,
        ))
}

/// Return a bounded pending-name label safe to interpolate into UI text.
pub fn sanitize_pending_username_for_display(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_control() && !is_bidi_control(*character))
        .take(MAX_PENDING_USERNAME_DISPLAY_CHARS)
        .collect()
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

/// Build a complete pending-name tooltip for `decided_at_ms`, measured from
/// `now_ms` (both Unix milliseconds).
///
/// Returns `None` when the deadline is already reached or past — the caller
/// should then use its no-ETA fallback. The estimate is intentionally coarse
/// because the decision time is itself an estimate.
pub fn approximate_time_until(decided_at_ms: TimestampMillis, now_ms: u64) -> Option<String> {
    let remaining_ms = decided_at_ms.checked_sub(now_ms)?;
    if remaining_ms == 0 {
        return None;
    }
    const HOUR: u64 = 3_600;
    const DAY: u64 = 86_400;
    let secs = remaining_ms / 1_000;
    Some(if secs < HOUR {
        "Dash masternodes vote on who receives this username. A decision is expected in less than an hour."
            .to_string()
    } else if secs < 2 * HOUR {
        "Dash masternodes vote on who receives this username. A decision is expected in about 1 hour."
            .to_string()
    } else if secs < DAY {
        let hours = secs / HOUR;
        format!(
            "Dash masternodes vote on who receives this username. A decision is expected in about {hours} hours."
        )
    } else if secs < 2 * DAY {
        "Dash masternodes vote on who receives this username. A decision is expected in about 1 day."
            .to_string()
    } else {
        let days = secs / DAY;
        format!(
            "Dash masternodes vote on who receives this username. A decision is expected in about {days} days."
        )
    })
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

    fn contestant(id: [u8; 32], name: &str) -> Contestant {
        Contestant {
            id: Identifier::from(id),
            name: name.to_string(),
            info: String::new(),
            votes: 0,
            created_at: None,
            created_at_block_height: None,
            created_at_core_block_height: None,
            document_id: Identifier::from([0u8; 32]),
        }
    }

    fn contest_with(state: ContestState, contestants: Vec<Contestant>) -> ContestedName {
        ContestedName {
            contestants: Some(contestants),
            end_time: Some(9_999),
            ..contest(state)
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

    // ----------------------------------------------------------------
    // Pending username detection: requested-but-unawarded vs owned/none.
    // ----------------------------------------------------------------

    #[test]
    fn pending_username_reported_when_contender_and_undecided() {
        let me = [5u8; 32];
        for state in [
            ContestState::Ongoing,
            ContestState::Joinable,
            ContestState::Unknown,
        ] {
            let c = contest_with(state.clone(), vec![contestant(me, "det1")]);
            let pending = c
                .pending_username_for(&Identifier::from(me))
                .expect("an undecided contender has a pending username");
            assert_eq!(pending.name, "det1");
            assert_eq!(pending.decided_at, Some(9_999));
        }
    }

    #[test]
    fn no_pending_username_when_contest_is_decided() {
        let me = [5u8; 32];
        // Won by me, won by another, and locked are all "decided" — the name is
        // no longer pending regardless of who ends up owning it.
        for state in [
            ContestState::WonBy(Identifier::from(me)),
            ContestState::WonBy(Identifier::from([9u8; 32])),
            ContestState::Locked,
        ] {
            let c = contest_with(state, vec![contestant(me, "det1")]);
            assert!(c.pending_username_for(&Identifier::from(me)).is_none());
        }
    }

    #[test]
    fn no_pending_username_when_identity_is_not_a_contender() {
        let c = contest_with(ContestState::Ongoing, vec![contestant([5u8; 32], "det1")]);
        assert!(
            c.pending_username_for(&Identifier::from([6u8; 32]))
                .is_none()
        );
    }

    #[test]
    fn no_pending_username_when_contest_has_no_contenders() {
        let c = contest(ContestState::Ongoing); // contestants: None
        assert!(
            c.pending_username_for(&Identifier::from([5u8; 32]))
                .is_none()
        );
    }

    #[test]
    fn pending_username_in_scans_multiple_contests() {
        let me = Identifier::from([5u8; 32]);
        let contests = vec![
            contest_with(ContestState::Locked, vec![contestant([5u8; 32], "taken")]),
            contest_with(ContestState::Ongoing, vec![contestant([5u8; 32], "det1")]),
        ];
        let pending = pending_username_in(&contests, &me).expect("second contest is pending");
        assert_eq!(pending.name, "det1");
    }

    #[test]
    fn pending_username_in_prioritizes_earliest_known_decision_deterministically() {
        let me = Identifier::from([5u8; 32]);
        let mut later = contest_with(ContestState::Ongoing, vec![contestant([5u8; 32], "later")]);
        later.end_time = Some(20_000);
        let mut earlier = contest_with(
            ContestState::Ongoing,
            vec![contestant([5u8; 32], "earlier")],
        );
        earlier.end_time = Some(10_000);
        let mut unknown = contest_with(
            ContestState::Ongoing,
            vec![contestant([5u8; 32], "unknown")],
        );
        unknown.end_time = None;

        for contests in [
            vec![later.clone(), unknown.clone(), earlier.clone()],
            vec![unknown.clone(), earlier.clone(), later.clone()],
            vec![earlier.clone(), later.clone(), unknown.clone()],
        ] {
            assert_eq!(
                pending_username_in(&contests, &me).map(|pending| pending.name),
                Some("earlier".to_string())
            );
        }
    }

    #[test]
    fn pending_username_in_uses_name_as_a_stable_deadline_tiebreaker() {
        let me = Identifier::from([5u8; 32]);
        let alpha = contest_with(ContestState::Ongoing, vec![contestant([5u8; 32], "alpha")]);
        let zulu = contest_with(ContestState::Ongoing, vec![contestant([5u8; 32], "zulu")]);

        for contests in [vec![zulu.clone(), alpha.clone()], vec![alpha, zulu]] {
            assert_eq!(
                pending_username_in(&contests, &me).map(|pending| pending.name),
                Some("alpha".to_string())
            );
        }
    }

    #[test]
    fn pending_username_in_returns_none_without_matches() {
        let contests = vec![contest_with(
            ContestState::Ongoing,
            vec![contestant([1u8; 32], "other")],
        )];
        assert!(pending_username_in(&contests, &Identifier::from([5u8; 32])).is_none());
    }

    // ----------------------------------------------------------------
    // ETA humanization.
    // ----------------------------------------------------------------

    #[test]
    fn approximate_time_until_buckets_durations() {
        let now = 1_000_000_000_000u64;
        let ms = |secs: u64| now + secs * 1_000;
        assert_eq!(
            approximate_time_until(ms(30 * 60), now).as_deref(),
            Some(
                "Dash masternodes vote on who receives this username. A decision is expected in less than an hour."
            )
        );
        assert_eq!(
            approximate_time_until(ms(90 * 60), now).as_deref(),
            Some(
                "Dash masternodes vote on who receives this username. A decision is expected in about 1 hour."
            )
        );
        assert_eq!(
            approximate_time_until(ms(3 * 3_600), now).as_deref(),
            Some(
                "Dash masternodes vote on who receives this username. A decision is expected in about 3 hours."
            )
        );
        assert_eq!(
            approximate_time_until(ms(36 * 3_600), now).as_deref(),
            Some(
                "Dash masternodes vote on who receives this username. A decision is expected in about 1 day."
            )
        );
        assert_eq!(
            approximate_time_until(ms(3 * 86_400), now).as_deref(),
            Some(
                "Dash masternodes vote on who receives this username. A decision is expected in about 3 days."
            )
        );
    }

    #[test]
    fn approximate_time_until_is_none_when_deadline_passed_or_now() {
        let now = 1_000_000_000_000u64;
        assert!(approximate_time_until(now, now).is_none());
        assert!(approximate_time_until(now - 1, now).is_none());
    }

    #[test]
    fn pending_username_display_sanitizer_strips_controls_and_bidi_markers() {
        assert_eq!(
            sanitize_pending_username_for_display("al\u{0000}i\u{061c}c\u{200f}e\u{202e}\u{2066}"),
            "alice"
        );
    }

    #[test]
    fn pending_username_display_sanitizer_clamps_unicode_scalar_count() {
        let unsafe_name = "é".repeat(MAX_PENDING_USERNAME_DISPLAY_CHARS + 10);
        let sanitized = sanitize_pending_username_for_display(&unsafe_name);
        assert_eq!(
            sanitized.chars().count(),
            MAX_PENDING_USERNAME_DISPLAY_CHARS
        );
        assert!(sanitized.chars().all(|character| character == 'é'));
    }
}
