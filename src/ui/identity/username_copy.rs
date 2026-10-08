//! User-facing copy for identity usernames.
//!
//! Every string is a complete sentence with named placeholders, so each one
//! stays a single translation unit. Kept free of egui so the copy is unit-testable.

use std::time::Duration;

use dash_sdk::dpp::identity::TimestampMillis;

use crate::backend_task::error::USERNAME_AVAILABILITY_CHECK_FAILED;
use crate::model::datetime;
use crate::model::dpns_usernames::{RequestPhase, TallyStanding, UsernameAvailability};
use crate::model::fee_estimation::format_credits_as_dash;

/// Result of the live availability check for the typed label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AvailabilityRow {
    /// The check is running.
    Checking,
    /// The network answered.
    Known(UsernameAvailability),
    /// The network could not be reached.
    CantCheck,
}

/// Visual tone of a status line, rendered as text plus icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Neutral progress.
    Neutral,
    /// Good news.
    Positive,
    /// Needs attention but allowed.
    Caution,
    /// Not allowed.
    Negative,
}

impl Tone {
    /// Icon shown before the text, so status never depends on color alone.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Neutral => "◌",
            Self::Positive => "✓",
            Self::Caution => "◷",
            Self::Negative => "✗",
        }
    }
}

/// Format a timestamp as a local calendar date, for example `2026-10-05`.
pub fn format_date(ms: TimestampMillis) -> String {
    datetime::instant_from_unix_millis(ms)
        .map(datetime::local_date)
        .unwrap_or_default()
}

/// Format a timestamp as a local date and time, for example `2026-10-05 14:00`.
pub fn format_date_time(ms: TimestampMillis) -> String {
    datetime::local_date_time_from_unix_millis(ms)
}

/// A contest duration in words: `14 days`, `1 day`, `45 minutes`, `2 hours`.
pub fn format_duration(duration: Duration) -> String {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    let secs = duration.as_secs();
    if secs >= DAY && secs.is_multiple_of(DAY) {
        match secs / DAY {
            1 => "1 day".to_owned(),
            days => format!("{days} days"),
        }
    } else if secs >= HOUR && secs.is_multiple_of(HOUR) {
        match secs / HOUR {
            1 => "1 hour".to_owned(),
            hours => format!("{hours} hours"),
        }
    } else {
        match secs / MINUTE {
            1 => "1 minute".to_owned(),
            minutes => format!("{minutes} minutes"),
        }
    }
}

/// The single availability line for `name`, with its tone.
pub fn availability_line(name: &str, row: &AvailabilityRow) -> (Tone, String) {
    match row {
        AvailabilityRow::Checking => (Tone::Neutral, format!("Checking if @{name} is available…")),
        AvailabilityRow::CantCheck => (
            Tone::Negative,
            USERNAME_AVAILABILITY_CHECK_FAILED.to_owned(),
        ),
        AvailabilityRow::Known(availability) => match availability {
            UsernameAvailability::Available => (Tone::Positive, format!("@{name} is available.")),
            UsernameAvailability::NeedsVote => (
                Tone::Caution,
                format!("@{name} is available, but it needs a community vote."),
            ),
            UsernameAvailability::Joinable {
                contenders,
                join_end,
            } => {
                let time = format_date_time(*join_end);
                let text = if *contenders == 1 {
                    format!(
                        "1 other person already asked for @{name}. You can join the vote until {time}."
                    )
                } else {
                    format!(
                        "{contenders} other people already asked for @{name}. You can join the vote until {time}."
                    )
                };
                (Tone::Caution, text)
            }
            UsernameAvailability::Taken => (
                Tone::Negative,
                format!("@{name} is already taken. Try another name."),
            ),
            UsernameAvailability::Locked => (
                Tone::Negative,
                format!("@{name} can't be registered. A community vote locked this name for good."),
            ),
            UsernameAvailability::JoinClosed => (
                Tone::Negative,
                format!("Others can no longer join the vote for @{name}. Try another name."),
            ),
            UsernameAvailability::AlreadyRequested => (
                Tone::Caution,
                format!(
                    "You already asked for @{name}. Its status is on your identity's Usernames list."
                ),
            ),
        },
    }
}

/// Whether suggestions that avoid a vote are offered for this result.
pub fn offers_suggestions(row: &AvailabilityRow) -> bool {
    matches!(
        row,
        AvailabilityRow::Known(
            UsernameAvailability::NeedsVote
                | UsernameAvailability::Joinable { .. }
                | UsernameAvailability::Taken
        )
    )
}

/// The four username rules, in display order.
pub const USERNAME_RULES: [&str; 4] = [
    "Between 3 and 63 characters.",
    "Letters, numbers, and hyphens, with no hyphen at the start or end.",
    "Capital letters are treated as lowercase.",
    "Names shorter than 20 characters that use only letters, hyphens, 0, and 1 need a community vote. Adding a digit from 2 to 9 avoids the vote.",
];

/// Per-keystroke length check under the name field.
pub const LENGTH_CHECK_LABEL: &str = "Between 3 and 63 characters";

/// Per-keystroke character check under the name field.
pub const CHARACTERS_CHECK_LABEL: &str = "Only letters, numbers, and hyphens";

/// Why an identity without a usable key cannot register a username.
pub const NEEDS_KEY_TO_REGISTER: &str = "Add a key to this identity to register usernames.";

/// What a registered name gives its owner: the done page and the won banner.
pub const NAME_REGISTERED_LINE: &str = "People can now find and pay you by this name.";

/// Done-page heading for a name registered right away.
pub fn registered_heading(name: &str) -> String {
    format!("You're @{name}")
}

/// Done-page heading for a request that went to a community vote.
pub fn request_sent_heading(name: &str) -> String {
    format!("Your request for @{name} is in")
}

/// Done-page schedule of a request that went to a community vote.
pub fn request_schedule_line(join_end: TimestampMillis, end: TimestampMillis) -> String {
    format!(
        "Others can still ask for this name until {join}. The community vote ends around {end}.",
        join = format_date_time(join_end),
        end = format_date(end)
    )
}

/// Done-page sentence on how a request nobody contests resolves.
pub fn request_uncontested_line(name: &str) -> String {
    format!("If no one else asks and no one votes to lock it, @{name} becomes yours then.")
}

/// Done-page note that the result arrives without the screen staying open.
pub const REQUEST_RESULT_NOTE: &str =
    "We'll show the result on your identity's page. You don't need to keep this screen open.";

/// Text of the one-time banner for a finished request, if it gets one.
pub fn outcome_banner_line(name: &str, phase: RequestPhase) -> Option<String> {
    match phase {
        RequestPhase::Won => Some(format!("You're @{name}. {NAME_REGISTERED_LINE}")),
        RequestPhase::Lost => Some(format!(
            "@{name} went to someone else. You can choose a different username."
        )),
        RequestPhase::Locked => Some(format!(
            "No one can register @{name} anymore. The community vote locked it."
        )),
        RequestPhase::NoWinner => Some(format!(
            "The vote for @{name} ended without a winner. You can choose a different username."
        )),
        RequestPhase::Joinable | RequestPhase::Voting | RequestPhase::AwaitingOutcome => None,
    }
}

/// First sentence of the Home card for a request that is still open.
pub fn pending_notice_line(
    name: &str,
    phase: RequestPhase,
    end: Option<TimestampMillis>,
) -> String {
    match (phase, end) {
        (RequestPhase::AwaitingOutcome, _) => {
            format!("@{name} is awaiting the voting result. View its status to check the outcome.")
        }
        (_, Some(end)) => format!(
            "@{name} is waiting for a community vote. It ends around {date}.",
            date = format_date(end)
        ),
        (_, None) => format!("@{name} is waiting for a community vote."),
    }
}

/// Usernames-card line under a registered name.
pub fn registered_on_line(at: TimestampMillis) -> String {
    format!("Registered on {date}.", date = format_date(at))
}

/// Usernames-card line under a request that is still open, when there is
/// something to say about it.
pub fn pending_request_detail(
    phase: RequestPhase,
    join_end: Option<TimestampMillis>,
    end: Option<TimestampMillis>,
) -> Option<String> {
    match (phase, join_end, end) {
        (RequestPhase::Joinable, Some(join_end), _) => Some(format!(
            "Others can ask for this name until {date}. Masternodes can already vote.",
            date = format_date(join_end)
        )),
        (RequestPhase::AwaitingOutcome, _, _) => Some(
            "The estimated voting period has ended. View the status to check the outcome."
                .to_owned(),
        ),
        (_, _, Some(end)) => Some(format!(
            "Voting ends around {date}.",
            date = format_date(end)
        )),
        _ => None,
    }
}

/// Usernames-card line under a request that ended without the name.
pub fn finished_request_detail(phase: RequestPhase, decided_at: Option<TimestampMillis>) -> String {
    match phase {
        RequestPhase::Locked => {
            "More votes went to locking this name, so no one can register it.".to_owned()
        }
        RequestPhase::NoWinner => {
            "The community vote ended without giving the name to anyone.".to_owned()
        }
        _ => decided_at.map_or_else(
            || "The community vote ended.".to_owned(),
            vote_ended_on_line,
        ),
    }
}

/// The day a community vote ended.
pub fn vote_ended_on_line(at: TimestampMillis) -> String {
    format!(
        "The community vote ended on {date}.",
        date = format_date(at)
    )
}

/// Title of the community-vote consent dialog.
pub fn consent_title(name: &str) -> String {
    format!("@{name} needs a community vote")
}

/// Body sentences of the community-vote consent dialog.
pub fn consent_lines(name: &str, total: Duration, join: Duration, fee_credits: u64) -> [String; 5] {
    let join_duration = format_duration(join);
    let duration = format_duration(total);
    let contest_fee = format_credits_as_dash(fee_credits);
    [
        format!(
            "Anyone can also ask for @{name} during the first {join_duration}. After that, only Dash masternode votes decide."
        ),
        format!(
            "The vote ends after {duration}, even if no one else asks. You'll see the result on your identity's page."
        ),
        format!(
            "The community vote fee is {contest_fee}. It isn't returned, whether you get the name or not."
        ),
        "If more votes go to locking the name, no one can register it, including you.".to_owned(),
        "Until the vote ends, people can't find you by this name.".to_owned(),
    ]
}

/// Primary pay button label.
pub fn pay_label(name: &str, total_credits: u64, needs_vote: bool) -> String {
    let total = format_credits_as_dash(total_credits);
    if needs_vote {
        format!("Pay {total} and request @{name}")
    } else {
        format!("Pay {total} and get @{name}")
    }
}

/// Low-balance warning on the confirm step.
pub fn low_balance_line(balance_credits: u64, missing_credits: u64) -> String {
    let balance = format_credits_as_dash(balance_credits);
    let missing = format_credits_as_dash(missing_credits);
    format!("This identity has {balance}. Add at least {missing} to continue.")
}

/// Status word for a request phase, as shown on rows and pages.
pub fn phase_label(phase: RequestPhase) -> &'static str {
    match phase {
        RequestPhase::Joinable => "Open for other requests",
        RequestPhase::Voting => "Waiting for vote",
        RequestPhase::AwaitingOutcome => "Awaiting result",
        RequestPhase::Won => "Registered",
        RequestPhase::Lost => "Went to someone else",
        RequestPhase::Locked => "Locked for good",
        RequestPhase::NoWinner => "No one got it",
    }
}

/// Timeline sentence for when the request was made.
pub fn requested_at_line(at: TimestampMillis) -> String {
    format!("Requested on {date}.", date = format_date(at))
}

/// Timeline sentence for the window in which other requests can join.
pub fn joining_timeline_line(end: TimestampMillis, joining: bool) -> String {
    if joining {
        format!(
            "Open for other requests until {date}.",
            date = format_date_time(end)
        )
    } else {
        format!(
            "Other requests could join until {date}.",
            date = format_date(end)
        )
    }
}

/// Timeline sentence for when the community vote ends.
pub fn voting_timeline_line(end: TimestampMillis, phase: RequestPhase) -> String {
    match phase {
        RequestPhase::Joinable => format!(
            "The community vote will end around {date}.",
            date = format_date_time(end)
        ),
        RequestPhase::Voting => format!(
            "The community vote ends around {date}.",
            date = format_date_time(end)
        ),
        RequestPhase::AwaitingOutcome => format!(
            "The estimated voting period ended around {date}.",
            date = format_date(end)
        ),
        _ => vote_ended_on_line(end),
    }
}

/// Result sentence on the request timeline.
pub fn request_result_line(phase: RequestPhase) -> &'static str {
    match phase {
        RequestPhase::Joinable | RequestPhase::Voting => "Result: Not decided yet.",
        RequestPhase::AwaitingOutcome => {
            "Result: Awaiting confirmation. Refresh to check the outcome."
        }
        RequestPhase::Won => "Result: The name is yours.",
        RequestPhase::Lost => "Result: The name went to someone else.",
        RequestPhase::Locked => "Result: The name is locked for good.",
        RequestPhase::NoWinner => "Result: No one got the name.",
    }
}

/// Tally row for votes to lock the name, on the read-only request status page.
pub const TALLY_LOCK_LABEL: &str = "Lock, so no one gets it";

/// Tally row for abstaining votes, on the read-only request status page.
pub const TALLY_ABSTAIN_LABEL: &str = "Abstain";

/// Second sentence of the Home request card.
pub fn standing_line(standing: TallyStanding, others: usize) -> &'static str {
    match standing {
        TallyStanding::LockAhead => {
            "More votes are for locking this name right now. If that holds, no one gets it."
        }
        _ if others == 0 => "No one else has asked so far.",
        TallyStanding::Leading => "You're leading right now.",
        TallyStanding::Tied => {
            "You're tied with another request. Ties go to the most recent request."
        }
        TallyStanding::Trailing => "Another request is leading.",
    }
}

/// The fixed "What happens next" rules on the request status page.
pub fn what_happens_next(name: &str) -> [String; 4] {
    [
        format!(
            "If you still have the most votes when the vote ends, @{name} becomes yours automatically."
        ),
        "If requests are tied at the end, the most recent request wins.".to_owned(),
        format!(
            "If more votes go to locking than to any request, no one can ever register @{name}."
        ),
        "The community vote fee isn't returned in any case.".to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORBIDDEN: [&str; 5] = ["DPNS", "contested", "alias", "deposit", "Contest"];

    fn assert_plain(text: &str) {
        for word in FORBIDDEN {
            assert!(!text.contains(word), "{text:?} uses {word:?}");
        }
    }

    #[test]
    fn dates_are_iso_style_in_the_local_time_zone() {
        use chrono::TimeZone;
        let ms = 1_790_000_000_000;
        let local = chrono::Local
            .timestamp_millis_opt(ms as i64)
            .single()
            .expect("valid test instant");
        assert_eq!(format_date(ms), local.format("%Y-%m-%d").to_string());
        assert_eq!(
            format_date_time(ms),
            local.format("%Y-%m-%d %H:%M").to_string()
        );
    }

    /// The won banner and the done page tell the owner the same thing.
    #[test]
    fn the_won_banner_repeats_the_done_page_sentence() {
        assert_eq!(
            outcome_banner_line("ali", RequestPhase::Won).as_deref(),
            Some("You're @ali. People can now find and pay you by this name.")
        );
        assert_eq!(
            NAME_REGISTERED_LINE,
            "People can now find and pay you by this name."
        );
        assert_eq!(outcome_banner_line("ali", RequestPhase::Voting), None);
    }

    /// The card and the request timeline word the end of a vote identically.
    #[test]
    fn a_finished_vote_reads_the_same_on_the_card_and_the_timeline() {
        let ended = format!("The community vote ended on {}.", format_date(0));
        assert_eq!(finished_request_detail(RequestPhase::Lost, Some(0)), ended);
        assert_eq!(voting_timeline_line(0, RequestPhase::Lost), ended);
        assert_eq!(
            finished_request_detail(RequestPhase::Lost, None),
            "The community vote ended."
        );
        assert_eq!(
            pending_request_detail(RequestPhase::Voting, None, None),
            None
        );
        assert_eq!(
            pending_notice_line("ali", RequestPhase::Voting, None),
            "@ali is waiting for a community vote."
        );
    }

    #[test]
    fn availability_rows_cover_every_state() {
        // USR-TC-032
        let cases = [
            (AvailabilityRow::Checking, "Checking if @nova is available…"),
            (
                AvailabilityRow::Known(UsernameAvailability::Available),
                "@nova is available.",
            ),
            (
                AvailabilityRow::Known(UsernameAvailability::NeedsVote),
                "@nova is available, but it needs a community vote.",
            ),
            (
                AvailabilityRow::Known(UsernameAvailability::Taken),
                "@nova is already taken. Try another name.",
            ),
            (
                AvailabilityRow::Known(UsernameAvailability::Locked),
                "@nova can't be registered. A community vote locked this name for good.",
            ),
            (
                AvailabilityRow::Known(UsernameAvailability::JoinClosed),
                "Others can no longer join the vote for @nova. Try another name.",
            ),
            (
                AvailabilityRow::CantCheck,
                "Availability can't be checked right now. Check your internet connection and try again.",
            ),
        ];
        for (row, expected) in cases {
            let (_, text) = availability_line("nova", &row);
            assert_eq!(text, expected);
            assert_plain(&text);
        }
    }

    #[test]
    fn joinable_row_has_singular_and_plural() {
        // USR-TC-033
        let row = |contenders| {
            availability_line(
                "nova",
                &AvailabilityRow::Known(UsernameAvailability::Joinable {
                    contenders,
                    join_end: 0,
                }),
            )
            .1
        };
        assert!(row(1).starts_with("1 other person already asked for @nova."));
        assert!(row(3).starts_with("3 other people already asked for @nova."));
    }

    #[test]
    fn consent_uses_network_durations_and_fee() {
        // USR-TC-034
        let lines = consent_lines(
            "nova",
            Duration::from_secs(14 * 86_400),
            Duration::from_secs(7 * 86_400),
            20_000_000_000,
        );
        assert!(lines[0].contains("during the first 7 days."));
        assert!(lines[1].contains("after 14 days,"));
        assert!(lines[2].contains("0.2 DASH. It isn't returned"));
        let testnet = consent_lines(
            "nova",
            Duration::from_secs(90 * 60),
            Duration::from_secs(45 * 60),
            10_000_000_000,
        );
        assert!(testnet[0].contains("first 45 minutes."));
        assert!(testnet[1].contains("after 90 minutes,"));
        assert!(testnet[2].contains("0.1 DASH"));
        for line in lines.iter().chain(&testnet) {
            assert_plain(line);
        }
    }

    #[test]
    fn rules_are_correct_and_plain() {
        // USR-TC-039
        assert_eq!(USERNAME_RULES.len(), 4);
        for rule in USERNAME_RULES {
            assert_plain(rule);
            assert!(!rule.contains("case-sensitive"));
        }
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(Duration::from_secs(86_400)), "1 day");
        assert_eq!(format_duration(Duration::from_secs(2 * 3_600)), "2 hours");
        assert_eq!(format_duration(Duration::from_secs(90 * 60)), "90 minutes");
    }

    #[test]
    fn standing_line_matches_home_card_variants() {
        // USR-TC-026
        assert_eq!(
            standing_line(TallyStanding::Leading, 1),
            "You're leading right now."
        );
        assert_eq!(
            standing_line(TallyStanding::Leading, 0),
            "No one else has asked so far."
        );
        assert_eq!(
            standing_line(TallyStanding::Trailing, 2),
            "Another request is leading."
        );
        assert_eq!(
            standing_line(TallyStanding::Tied, 1),
            "You're tied with another request. Ties go to the most recent request."
        );
        assert_eq!(
            standing_line(TallyStanding::LockAhead, 0),
            "More votes are for locking this name right now. If that holds, no one gets it."
        );
    }

    #[test]
    fn suggestions_only_for_vote_or_taken() {
        assert!(offers_suggestions(&AvailabilityRow::Known(
            UsernameAvailability::Taken
        )));
        assert!(!offers_suggestions(&AvailabilityRow::Known(
            UsernameAvailability::Locked
        )));
        assert!(!offers_suggestions(&AvailabilityRow::CantCheck));
    }
}
