use crate::model::qualified_identity::PrivateKeyTarget;
use bincode::{Decode, Encode};
use dash_sdk::dpp::identity::{KeyID, TimestampMillis};
use dash_sdk::dpp::prelude::{BlockHeight, CoreBlockHeight, Identifier};
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use std::collections::BTreeMap;

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

    /// Whether the contest is positively known to be decided — won or locked.
    ///
    /// `Unknown` is not decided: it only means DET lacks the data to tell.
    pub fn is_decided(&self) -> bool {
        matches!(self, ContestState::WonBy(_) | ContestState::Locked)
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
    /// Whether the contest's state still accepts votes.
    pub fn is_votable(&self) -> bool {
        self.state.state_is_votable()
    }

    /// Whether the name was only listed so far: its contenders were never
    /// read and no end time is stored. Nothing says such a name is an open
    /// contest; on a network with a long history it almost never is.
    pub fn is_unread(&self) -> bool {
        self.last_updated.is_none()
            && self.end_time.is_none()
            && self.contestants.as_ref().is_none_or(Vec::is_empty)
    }
}

/// Return a bounded pending-name label safe to interpolate into UI text.
pub fn sanitize_pending_username_for_display(name: &str) -> String {
    name.chars()
        .filter(|character| crate::model::identity_name::is_safe_display_character(*character))
        .take(MAX_PENDING_USERNAME_DISPLAY_CHARS)
        .collect()
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

/// How complete the node's proved current-vote state is across its contests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MasternodeVoteStateSummary {
    /// Every active contest has a proved current-vote state.
    #[default]
    Ready,
    /// At least one contest is still being proved, and none has given up.
    Checking,
    /// At least one contest's proved state could not be obtained.
    Unavailable,
}

/// Per-node DPNS voting summary shown on the Masternodes card grid.
///
/// Composed by a display-layer read of existing contest + scheduled-vote state
/// (no new backend concept). Feeds the count-first status line: open contests
/// take precedence, then failed or pending scheduled votes, then no open
/// contests (requirements §10.1).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MasternodeContestSummary {
    /// Number of active contests, including contests with an existing vote.
    pub open_contest_count: usize,
    /// Number of active contests whose proved state is `Not voted`.
    pub needs_vote_count: usize,
    /// Whether the node's proved current-vote state is complete, still being
    /// checked, or unavailable.
    pub vote_state: MasternodeVoteStateSummary,
    /// Whether the node has at least one pending (not-yet-executed) scheduled
    /// vote in the authoritative operation journal.
    pub has_scheduled_vote: bool,
    /// Whether a scheduled target reached a terminal failure that needs review.
    pub has_failed_scheduled_vote: bool,
}

impl MasternodeContestSummary {
    /// Represent a failed summary read without implying that no contests exist.
    pub fn unavailable() -> Self {
        Self {
            vote_state: MasternodeVoteStateSummary::Unavailable,
            ..Self::default()
        }
    }
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
    fn contest_is_votable_only_in_open_states() {
        for (state, expected) in [
            (ContestState::Unknown, false),
            (ContestState::Joinable, true),
            (ContestState::Ongoing, true),
            (ContestState::WonBy(Identifier::from([9u8; 32])), false),
            (ContestState::Locked, false),
        ] {
            assert_eq!(contest(state).is_votable(), expected);
        }
    }

    #[test]
    fn contest_is_decided_only_when_won_or_locked() {
        for (state, expected) in [
            (ContestState::Unknown, false),
            (ContestState::Joinable, false),
            (ContestState::Ongoing, false),
            (ContestState::WonBy(Identifier::from([9u8; 32])), true),
            (ContestState::Locked, true),
        ] {
            assert_eq!(state.is_decided(), expected, "{state:?}");
        }
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
