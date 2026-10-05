//! User-facing copy for identity usernames.
//!
//! Every string is a complete sentence with named placeholders, so each one
//! stays a single translation unit. Kept free of egui so the copy is unit-testable.

use std::time::Duration;

use chrono::{Local, TimeZone};
use dash_sdk::dpp::identity::TimestampMillis;

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

/// Format a timestamp as a local calendar date, for example `Oct 5, 2026`.
pub fn format_date(ms: TimestampMillis) -> String {
    local(ms).map_or_else(String::new, |t| t.format("%b %-d, %Y").to_string())
}

/// Format a timestamp as a local date and time, for example `Oct 5, 2026, 14:00`.
pub fn format_date_time(ms: TimestampMillis) -> String {
    local(ms).map_or_else(String::new, |t| t.format("%b %-d, %Y, %H:%M").to_string())
}

fn local(ms: TimestampMillis) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_millis_opt(i64::try_from(ms).ok()?).single()
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
        AvailabilityRow::Checking => (
            Tone::Neutral,
            format!("Checking if @{name} is available…"),
        ),
        AvailabilityRow::CantCheck => (
            Tone::Negative,
            "Availability can't be checked right now. Check your internet connection and try again."
                .to_owned(),
        ),
        AvailabilityRow::Known(availability) => match availability {
            UsernameAvailability::Available => {
                (Tone::Positive, format!("@{name} is available."))
            }
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

/// Complete timeline sentences for the request status page.
pub fn requested_at_line(at: TimestampMillis) -> String {
    format!("Requested on {date}.", date = format_date(at))
}

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
        _ => format!(
            "The community vote ended on {date}.",
            date = format_date(end)
        ),
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
