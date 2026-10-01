//! User-facing sentences for the voting workspace.
//!
//! Each function returns one complete translation unit with its plural pair,
//! so no caller concatenates fragments.

use crate::model::dpns_voting::composer::SkipReason;
use crate::model::dpns_voting::operator::{ChangesLeft, TimeLeft};

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

/// `Voted with {n} of {total} nodes` (VOTE-FR-071).
pub fn voted_with_label(voted: usize, total: usize) -> String {
    match total {
        1 => format!("Voted with {voted} of 1 node"),
        total => format!("Voted with {voted} of {total} nodes"),
    }
}

/// `{k} requests` for a contest's contender count.
pub fn requests_label(contenders: usize) -> String {
    match contenders {
        1 => "1 request".to_owned(),
        count => format!("{count} requests"),
    }
}

/// Influence line (VOTE-FR-077).
pub fn influence_line(leader: &str, margin: u32, weight: u32) -> String {
    let margin_votes = if margin == 1 {
        "1 vote".to_owned()
    } else {
        format!("{margin} votes")
    };
    match weight {
        1 => format!("{leader} leads by {margin_votes}. Your 1 vote can change who leads."),
        weight => {
            format!("{leader} leads by {margin_votes}. Your {weight} votes can change who leads.")
        }
    }
}

/// Tie line (VOTE-FR-077, PF §3).
pub const TIE_LINE: &str = "If still tied at the end, the most recent request wins.";

/// Weighted-tally hint (VOTE-FR-077).
pub const EVONODE_WEIGHT_HINT: &str = "Evonodes count as 4 votes.";

/// One part of the node line: nodes that have not voted.
pub fn not_voted_part(count: usize) -> String {
    format!("{count} not voted")
}

/// One part of the node line: nodes that voted `choice`.
pub fn voted_part(count: usize, choice_label: &str) -> String {
    format!("{count} voted {choice_label}")
}

/// One part of the node line: nodes out of changes, named when few.
pub fn no_changes_part(names: &[String]) -> String {
    match names {
        [one] => format!("{one} has no changes left"),
        [first, second] => format!("{first} and {second} have no changes left"),
        many => format!("{count} nodes have no changes left", count = many.len()),
    }
}

/// One part of the node line: nodes whose targets are being sent.
pub fn sending_part(count: usize) -> String {
    match count {
        1 => "1 sending".to_owned(),
        count => format!("{count} sending"),
    }
}

/// One part of the node line: nodes with a scheduled vote.
pub fn scheduled_part(count: usize) -> String {
    format!("{count} scheduled")
}

/// The node line under a card's choices, from its parts.
pub fn node_line(parts: &[String]) -> String {
    format!("Your nodes: {parts}", parts = parts.join(" · "))
}

/// Shown when picking a decision changes earlier votes (VOTE-FR-015).
pub fn change_warning_line(changed: usize) -> String {
    match changed {
        1 => "Changing 1 earlier vote uses 1 of that node's 4 changes.".to_owned(),
        changed => format!("Changing {changed} earlier votes uses 1 of each node's 4 changes."),
    }
}

/// Shown when some nodes' proved state is unavailable (VOTE-FR-016).
pub fn unavailable_line(count: usize) -> String {
    match count {
        1 => "Vote state is unavailable for 1 node; it's left out.".to_owned(),
        count => format!("Vote state is unavailable for {count} nodes; they're left out."),
    }
}

/// Card caption while its targets are in flight (VOTE-FR-083).
pub fn sending_with_label(count: usize) -> String {
    match count {
        1 => "Sending with 1 node…".to_owned(),
        count => format!("Sending with {count} nodes…"),
    }
}

/// Tray summary (VOTE-FR-086).
pub fn tray_label(decisions: usize, transactions: usize) -> String {
    let decisions = match decisions {
        1 => "1 decision ready".to_owned(),
        count => format!("{count} decisions ready"),
    };
    match transactions {
        1 => format!("{decisions} · 1 transaction, one per node and name"),
        count => format!("{decisions} · {count} transactions, one per node and name"),
    }
}

/// Node-set chip (VOTE-FR-075).
pub fn node_set_chip_label(set_name: &str, nodes: usize, weight: u32) -> String {
    let nodes = match nodes {
        1 => "1 node".to_owned(),
        count => format!("{count} nodes"),
    };
    let votes = match weight {
        1 => "1 vote".to_owned(),
        weight => format!("{weight} votes"),
    };
    format!("Vote with: {set_name} · {nodes} · {votes}")
}

/// Confirm title (VOTE-FR-080).
pub fn confirm_title(decisions: usize, nodes: usize) -> String {
    match (decisions, nodes) {
        (1, 1) => "Cast 1 decision with 1 node".to_owned(),
        (1, nodes) => format!("Cast 1 decision with {nodes} nodes"),
        (decisions, 1) => format!("Cast {decisions} decisions with 1 node"),
        (decisions, nodes) => format!("Cast {decisions} decisions with {nodes} nodes"),
    }
}

/// Confirm transaction line (VOTE-FR-080).
pub fn confirm_transactions_line(transactions: usize) -> String {
    match transactions {
        1 => "1 transaction, one per node and name. Voting is free for your nodes.".to_owned(),
        count => {
            format!("{count} transactions, one per node and name. Voting is free for your nodes.")
        }
    }
}

/// Confirm change warning, shown once per batch (VOTE-FR-015/080).
pub fn confirm_change_warning(changes: usize) -> String {
    match changes {
        1 => "1 of these changes an earlier vote. Each node can change its vote 4 times per name."
            .to_owned(),
        count => format!(
            "{count} of these change an earlier vote. Each node can change its vote 4 times per name."
        ),
    }
}

/// Confirm skipped header (VOTE-FR-080).
pub fn skipped_header(count: usize) -> String {
    match count {
        1 => "Skipped: 1 vote".to_owned(),
        count => format!("Skipped: {count} votes"),
    }
}

/// One skipped-reason line, e.g. `2 votes: no changes left (5 of 5 votes used)`.
pub fn skipped_reason_line(count: usize, reason: SkipReason) -> String {
    let votes = match count {
        1 => "1 vote".to_owned(),
        count => format!("{count} votes"),
    };
    let reason = match reason {
        SkipReason::AlreadyVoted => "the node already voted this way",
        SkipReason::NoChangesLeft => "no changes left (5 of 5 votes used)",
        SkipReason::VoteStateUnavailable => {
            "vote state is unavailable; refresh voting to include it"
        }
        SkipReason::AlreadyInProgress => "an earlier vote on this name is still being sent",
        SkipReason::NotUsed => "you chose not to use the node",
    };
    format!("{votes}: {reason}")
}

/// Confirm primary button (VOTE-FR-080).
pub fn confirm_button_label(transactions: usize, all_now: bool) -> String {
    match (transactions, all_now) {
        (1, true) => "Cast 1 vote".to_owned(),
        (count, true) => format!("Cast {count} votes"),
        (1, false) => "Schedule 1 vote".to_owned(),
        (count, false) => format!("Schedule {count} votes"),
    }
}

/// `6 hours before the end` for a relative preset (VOTE-FR-081).
pub fn before_end_phrase(preset: std::time::Duration) -> String {
    let minutes = preset.as_secs() / 60;
    match (minutes / 60, minutes % 60) {
        (0, 1) => "1 minute before the end".to_owned(),
        (0, minutes) => format!("{minutes} minutes before the end"),
        (1, 0) => "1 hour before the end".to_owned(),
        (hours, 0) => format!("{hours} hours before the end"),
        (hours, minutes) => format!("{hours} h {minutes} min before the end"),
    }
}

/// A relative schedule with its absolute time: `6 hours before the end · 2026-10-03 12:00 UTC`.
pub fn relative_schedule_label(preset: std::time::Duration, absolute_utc: &str) -> String {
    format!(
        "{relative} · {absolute_utc} UTC",
        relative = before_end_phrase(preset)
    )
}

/// One decision row in the confirm list.
pub fn decision_row(name: &str, choice: &str, nodes: usize) -> String {
    match nodes {
        1 => format!("{name}.dash · {choice} · 1 node"),
        nodes => format!("{name}.dash · {choice} · {nodes} nodes"),
    }
}

/// Changes-left cell under `Adjust nodes` (VOTE-FR-078).
pub fn changes_left_label(changes: ChangesLeft) -> String {
    match changes {
        ChangesLeft::Known(1) => "1 change left, counted on this device".to_owned(),
        ChangesLeft::Known(left) => format!("{left} changes left, counted on this device"),
        ChangesLeft::Unknown => {
            "Changes left unknown. This node voted outside Dash Evo Tool.".to_owned()
        }
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
    fn card_lines_read_as_complete_units() {
        assert_eq!(voted_with_label(5, 24), "Voted with 5 of 24 nodes");
        assert_eq!(
            influence_line("Zed", 6, 51),
            "Zed leads by 6 votes. Your 51 votes can change who leads."
        );
        assert_eq!(
            node_line(&[
                not_voted_part(19),
                voted_part(4, "Abstain"),
                no_changes_part(&["mn-07".to_owned()]),
            ]),
            "Your nodes: 19 not voted · 4 voted Abstain · mn-07 has no changes left"
        );
        assert_eq!(
            change_warning_line(4),
            "Changing 4 earlier votes uses 1 of each node's 4 changes."
        );
        assert_eq!(
            tray_label(1, 3),
            "1 decision ready · 3 transactions, one per node and name"
        );
        assert_eq!(
            node_set_chip_label("All my nodes", 24, 51),
            "Vote with: All my nodes · 24 nodes · 51 votes"
        );
        assert_eq!(
            unavailable_line(3),
            "Vote state is unavailable for 3 nodes; they're left out."
        );
    }

    /// VOTE-TC-090/091 (copy half): the confirm lines follow VOTE-FR-080.
    #[test]
    fn confirm_copy_follows_the_spec() {
        assert_eq!(confirm_title(3, 24), "Cast 3 decisions with 24 nodes");
        assert_eq!(
            confirm_transactions_line(72),
            "72 transactions, one per node and name. Voting is free for your nodes."
        );
        assert_eq!(
            confirm_change_warning(4),
            "4 of these change an earlier vote. Each node can change its vote 4 times per name."
        );
        assert_eq!(skipped_header(2), "Skipped: 2 votes");
        assert_eq!(
            skipped_reason_line(1, SkipReason::NoChangesLeft),
            "1 vote: no changes left (5 of 5 votes used)"
        );
        assert_eq!(confirm_button_label(72, true), "Cast 72 votes");
        assert_eq!(confirm_button_label(1, false), "Schedule 1 vote");
        assert_eq!(
            relative_schedule_label(std::time::Duration::from_secs(6 * 3600), "2026-10-03 12:00"),
            "6 hours before the end · 2026-10-03 12:00 UTC"
        );
        assert_eq!(
            before_end_phrase(std::time::Duration::from_secs(600)),
            "10 minutes before the end"
        );
        assert_eq!(
            changes_left_label(ChangesLeft::Known(3)),
            "3 changes left, counted on this device"
        );
        assert_eq!(
            changes_left_label(ChangesLeft::Unknown),
            "Changes left unknown. This node voted outside Dash Evo Tool."
        );
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
