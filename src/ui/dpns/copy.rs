//! User-facing sentences for the voting workspace.
//!
//! Each function returns one complete translation unit with its plural pair,
//! so no caller concatenates fragments.

use crate::model::dpns_voting::operator::TimeLeft;

/// A remaining time as a phrase, e.g. `3 hours`.
pub fn time_left_phrase(time_left: TimeLeft) -> String {
    match time_left {
        TimeLeft::Minutes(1) => "1 minute".to_owned(),
        TimeLeft::Minutes(minutes) => format!("{minutes} minutes"),
        TimeLeft::Hours(1) => "1 hour".to_owned(),
        TimeLeft::Hours(hours) => format!("{hours} hours"),
        TimeLeft::Days(1) => "1 day".to_owned(),
        TimeLeft::Days(days) => format!("{days} days"),
    }
}

/// `Ends in {time}`, or `Voting has ended` once the deadline has passed.
pub fn ends_in_label(time_left: Option<TimeLeft>) -> String {
    match time_left {
        Some(time_left) => format!("Ends in {time}", time = time_left_phrase(time_left)),
        None => "Voting has ended".to_owned(),
    }
}

/// Top-bar chip: contests needing the operator's vote (VOTE-FR-072).
///
/// `urgent` carries the soonest deadline when it falls within the urgency window.
pub fn needs_vote_chip_label(count: usize, urgent: Option<TimeLeft>) -> String {
    match (count, urgent) {
        (1, Some(time_left)) => format!(
            "1 name needs your vote · first ends in {time}",
            time = time_left_phrase(time_left)
        ),
        (count, Some(time_left)) => format!(
            "{count} names need your vote · first ends in {time}",
            time = time_left_phrase(time_left)
        ),
        (1, None) => "1 name needs your vote".to_owned(),
        (count, None) => format!("{count} names need your vote"),
    }
}

/// Top-bar chip: targets whose result is still being checked (VOTE-FR-072).
pub fn unresolved_chip_label(count: usize) -> String {
    match count {
        1 => "1 vote is still being checked".to_owned(),
        count => format!("{count} votes are still being checked"),
    }
}

/// One tooltip line naming a contest and its deadline.
pub fn deadline_line(name: &str, time_left: Option<TimeLeft>) -> String {
    match time_left {
        Some(time_left) => format!(
            "{name}.dash ends in {time}.",
            time = time_left_phrase(time_left)
        ),
        None => format!("{name}.dash has no known deadline yet."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VOTE-TC-085 (copy half).
    #[test]
    fn needs_vote_chip_reads_as_a_complete_sentence() {
        assert_eq!(
            needs_vote_chip_label(1, Some(TimeLeft::Hours(3))),
            "1 name needs your vote · first ends in 3 hours"
        );
        assert_eq!(
            needs_vote_chip_label(4, Some(TimeLeft::Minutes(1))),
            "4 names need your vote · first ends in 1 minute"
        );
        assert_eq!(needs_vote_chip_label(1, None), "1 name needs your vote");
        assert_eq!(needs_vote_chip_label(6, None), "6 names need your vote");
    }

    /// VOTE-TC-087 (copy half).
    #[test]
    fn unresolved_chip_has_a_plural_pair() {
        assert_eq!(unresolved_chip_label(1), "1 vote is still being checked");
        assert_eq!(unresolved_chip_label(3), "3 votes are still being checked");
    }

    #[test]
    fn deadlines_read_naturally() {
        assert_eq!(ends_in_label(Some(TimeLeft::Days(1))), "Ends in 1 day");
        assert_eq!(ends_in_label(None), "Voting has ended");
        assert_eq!(
            deadline_line("alice", Some(TimeLeft::Hours(6))),
            "alice.dash ends in 6 hours."
        );
    }
}
