//! User-facing sentences for the voting workspace.
//!
//! Shared labels and sentences; count-sensitive sentences keep their plural forms here.

use crate::model::dpns_voting::composer::SkipReason;
use crate::model::dpns_voting::operator::{ChangesLeft, NodeExclusion, TimeLeft};
use crate::model::dpns_voting::progress::{NeedsAttention, ProgressCounts};
use crate::model::dpns_voting::{
    DpnsScheduledVoteClearDisposition, DpnsScheduledVoteClearOutcome, DpnsVoteFailure,
    DpnsVoteOperation, DpnsVoteTargetStatus,
};
use crate::ui::MessageType;
use std::collections::BTreeSet;

/// Recovery guidance while saved voting progress is unreadable.
pub const JOURNAL_UNAVAILABLE_MESSAGE: &str = "Saved voting progress could not be read. The displayed history may be incomplete or out of date. Do not submit votes again until you have retried loading and checked their status.";

/// Feedback for removing scheduled votes, including votes already in progress.
pub fn scheduled_vote_clear_feedback(
    outcomes: &[DpnsScheduledVoteClearOutcome],
) -> (String, MessageType) {
    let cleared = outcomes
        .iter()
        .filter(|outcome| outcome.disposition == DpnsScheduledVoteClearDisposition::Cleared)
        .count();
    let in_flight = outcomes.len().saturating_sub(cleared);
    match (cleared, in_flight) {
        (0, 0) => (
            "No scheduled votes were removed.".to_owned(),
            MessageType::Info,
        ),
        (cleared, 0) => (
            format!("Scheduled votes removed: {cleared}."),
            MessageType::Success,
        ),
        (0, in_flight) => (
            format!(
                "No scheduled votes were removed. Votes already in progress: {in_flight}. Wait for voting to finish before trying again."
            ),
            MessageType::Info,
        ),
        (cleared, in_flight) => (
            format!(
                "Scheduled votes removed: {cleared}. Votes already in progress: {in_flight}. Wait for voting to finish before trying again."
            ),
            MessageType::Info,
        ),
    }
}

/// Candidate choice with an identity handle, including when its name is unavailable.
pub fn vote_choice_label(
    choice: dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice,
    candidate_name: Option<&str>,
) -> String {
    use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
    match choice {
        ResourceVoteChoice::Lock => "Lock name".to_owned(),
        ResourceVoteChoice::Abstain => "Abstain".to_owned(),
        ResourceVoteChoice::TowardsIdentity(id) => {
            let handle = crate::model::identity_name::shorten_id(
                &id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
            );
            match candidate_name {
                Some(name) => format!("Vote for {name} ({handle})"),
                None => format!("Vote for {handle}"),
            }
        }
    }
}

/// Shared status label for a vote in progress or history.
pub fn target_status_label(status: DpnsVoteTargetStatus) -> &'static str {
    match status {
        DpnsVoteTargetStatus::Scheduled => "Scheduled",
        DpnsVoteTargetStatus::Queued => "Queued",
        DpnsVoteTargetStatus::Submitting => "Submitting",
        DpnsVoteTargetStatus::Confirming => "Confirming",
        DpnsVoteTargetStatus::Confirmed => "Confirmed",
        DpnsVoteTargetStatus::Unconfirmed => "Still being checked",
        DpnsVoteTargetStatus::Rejected => "Rejected",
        DpnsVoteTargetStatus::FailedBeforeSubmission => "Not submitted",
        DpnsVoteTargetStatus::NotApplied => "Not applied",
        DpnsVoteTargetStatus::Cancelled => "Cancelled",
    }
}

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
    match (margin, weight) {
        (1, 1) => format!("{leader} leads by 1 vote. Your 1 vote can change who leads."),
        (1, weight) => {
            format!("{leader} leads by 1 vote. Your {weight} votes can change who leads.")
        }
        (margin, 1) => {
            format!("{leader} leads by {margin} votes. Your 1 vote can change who leads.")
        }
        (margin, weight) => {
            format!("{leader} leads by {margin} votes. Your {weight} votes can change who leads.")
        }
    }
}

/// Tie line (VOTE-FR-077, PF §3).
pub const TIE_LINE: &str = "If still tied at the end, the most recent request wins.";

/// Lock tied with the single leading request (Lock needs strictly more votes).
pub const LOCK_TIE_LINE: &str =
    "Lock name is tied with the leading request. If still tied at the end, the request wins.";

/// Weighted-tally hint (VOTE-FR-077).
pub const EVONODE_WEIGHT_HINT: &str = "Evonodes count as 4 votes.";

/// One part of the node line: nodes that have not voted.
pub fn not_voted_part(count: usize) -> String {
    match count {
        1 => "1 of your nodes has not voted.".to_owned(),
        count => format!("{count} of your nodes have not voted."),
    }
}

/// One part of the node line: nodes that voted `choice`.
pub fn voted_part(count: usize, choice_label: &str) -> String {
    match count {
        1 => format!("1 of your nodes voted: {choice_label}."),
        count => format!("{count} of your nodes voted: {choice_label}."),
    }
}

/// One part of the node line: nodes out of changes, named when few.
pub fn no_changes_part(names: &[String]) -> String {
    match names {
        [one] => format!("{one} has no changes left."),
        [first, second] => format!("{first} and {second} have no changes left."),
        many => format!("{count} nodes have no changes left.", count = many.len()),
    }
}

/// One part of the node line: nodes whose targets are being sent.
pub fn sending_part(count: usize) -> String {
    match count {
        1 => "1 of your nodes is sending a vote.".to_owned(),
        count => format!("{count} of your nodes are sending votes."),
    }
}

/// One part of the node line: nodes with a scheduled vote.
pub fn scheduled_part(count: usize) -> String {
    match count {
        1 => "1 of your nodes has a scheduled vote.".to_owned(),
        count => format!("{count} of your nodes have scheduled votes."),
    }
}

/// The node line under a card's choices, from its parts.
pub fn node_line(parts: &[String]) -> String {
    parts.join(" ")
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
    match (decisions, transactions) {
        (1, 1) => "1 decision ready · 1 transaction, one per node and name".to_owned(),
        (1, transactions) => {
            format!("1 decision ready · {transactions} transactions, one per node and name")
        }
        (decisions, 1) => {
            format!("{decisions} decisions ready · 1 transaction, one per node and name")
        }
        (decisions, transactions) => format!(
            "{decisions} decisions ready · {transactions} transactions, one per node and name"
        ),
    }
}

/// Node-set chip (VOTE-FR-075).
pub fn node_set_chip_label(set_name: &str, nodes: usize, weight: u32) -> String {
    match (nodes, weight) {
        (1, 1) => format!("Vote with: {set_name} · 1 node · 1 vote"),
        (1, weight) => format!("Vote with: {set_name} · 1 node · {weight} votes"),
        (nodes, 1) => format!("Vote with: {set_name} · {nodes} nodes · 1 vote"),
        (nodes, weight) => format!("Vote with: {set_name} · {nodes} nodes · {weight} votes"),
    }
}

/// Final batch banner (VOTE-FR-061): `{n} nodes voted on {d} names.`, with
/// the still-checking count and VOTE-FR-064 guidance when `checking > 0`.
pub fn batch_voted_line(nodes: usize, names: usize, checking: usize) -> String {
    if checking > 0 {
        return format!(
            "Participating nodes: {nodes}. Names with confirmed votes: {names}. Votes still being checked: {checking}. Dash Evo Tool will keep checking. Do not submit the pending votes again."
        );
    }
    match (nodes, names) {
        (1, 1) => "1 node voted on 1 name.".to_owned(),
        (1, names) => format!("1 node voted on {names} names."),
        (nodes, 1) => format!("{nodes} nodes voted on 1 name."),
        (nodes, names) => format!("{nodes} nodes voted on {names} names."),
    }
}

/// Drawer header (VOTE-FR-083).
pub fn drawer_header(counts: ProgressCounts) -> String {
    let done = counts.done();
    let ProgressCounts {
        total,
        sending,
        checking,
        ..
    } = counts;
    match total {
        1 => format!("Casting 1 vote · {done} done · {sending} sending · {checking} being checked"),
        total => format!(
            "Casting {total} votes · {done} done · {sending} sending · {checking} being checked"
        ),
    }
}

/// Drawer row status (VOTE-FR-083/087).
pub fn progress_row_status(
    status: DpnsVoteTargetStatus,
    failure: Option<DpnsVoteFailure>,
) -> &'static str {
    match (status, failure) {
        (DpnsVoteTargetStatus::FailedBeforeSubmission, Some(DpnsVoteFailure::VotingEnded)) => {
            "Not submitted. Voting ended."
        }
        (DpnsVoteTargetStatus::FailedBeforeSubmission, Some(DpnsVoteFailure::VotingKeyMissing)) => {
            "Not submitted. The voting key is not loaded."
        }
        (DpnsVoteTargetStatus::Unconfirmed, _) => "Still being checked. Do not submit it again.",
        (status, _) => target_status_label(status),
    }
}

/// `Needs attention` summary (VOTE-FR-084).
pub fn needs_attention_line(attention: NeedsAttention) -> String {
    let NeedsAttention {
        checking,
        failed,
        missed_schedules,
    } = attention;
    match (checking > 0, failed > 0, missed_schedules > 0) {
        (true, true, true) => format!(
            "Votes still being checked: {checking}. Failed votes: {failed}. Missed scheduled votes: {missed_schedules}. Do not submit the pending votes again."
        ),
        (true, true, false) => format!(
            "Votes still being checked: {checking}. Failed votes: {failed}. Do not submit the pending votes again."
        ),
        (true, false, true) => format!(
            "Votes still being checked: {checking}. Missed scheduled votes: {missed_schedules}. Do not submit the pending votes again."
        ),
        (true, false, false) => {
            format!("Votes still being checked: {checking}. Do not submit the pending votes again.")
        }
        (false, true, true) => {
            format!("Failed votes: {failed}. Missed scheduled votes: {missed_schedules}.")
        }
        (false, true, false) => format!("Failed votes: {failed}."),
        (false, false, true) => format!("Missed scheduled votes: {missed_schedules}."),
        (false, false, false) => String::new(),
    }
}

/// A node's voting weight on its detail page (MN-003).
pub fn voting_weight_label(weight: u32) -> String {
    match weight {
        1 => "Voting weight: 1 vote".to_owned(),
        weight => format!("Voting weight: {weight} votes"),
    }
}

/// History outcome of an awarded contest (VOTE-FR-087).
pub fn went_to_label(name: Option<&str>, short_id: &str) -> String {
    match name {
        Some(name) => format!("Went to {name} ({short_id})"),
        None => format!("Went to {short_id}"),
    }
}

/// History outcome of a locked contest.
pub const LOCKED_FOR_GOOD: &str = "Locked for good, no one can register it";

/// Expander label of a scheduled decision's node list (VOTE-FR-088).
pub fn scheduled_nodes_label(nodes: usize) -> String {
    match nodes {
        1 => "1 node".to_owned(),
        nodes => format!("{nodes} nodes"),
    }
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

/// Confirm line for "when voting is about to end" votes whose contest is
/// already inside the lead time (VOTE-FR-081).
pub fn ends_soon_now_line(count: usize) -> String {
    format!("{count} of these votes will be sent now because voting ends soon.")
}

/// Confirm line for nodes in the set that cannot vote at all (VOTE-FR-080).
pub fn excluded_nodes_line(count: usize, exclusion: NodeExclusion) -> String {
    match (count, exclusion) {
        (1, NodeExclusion::NotInMasternodeList) => {
            "1 node not used: it is not in the masternode list, so its votes don't count."
                .to_owned()
        }
        (count, NodeExclusion::NotInMasternodeList) => format!(
            "{count} nodes not used: they are not in the masternode list, so their votes don't count."
        ),
        (1, NodeExclusion::NoVotingKey) => {
            "1 node not used: no voting key is loaded for it.".to_owned()
        }
        (count, NodeExclusion::NoVotingKey) => {
            format!("{count} nodes not used: no voting key is loaded for them.")
        }
    }
}

/// One skipped-reason line, e.g. `2 votes: no changes left (4 of 4 changes used)`.
pub fn skipped_reason_line(count: usize, reason: SkipReason) -> String {
    match reason {
        SkipReason::AlreadyVoted => {
            format!("Votes skipped: {count}. The selected nodes already voted this way.")
        }
        SkipReason::NoChangesLeft => format!(
            "Votes skipped: {count}. The selected nodes have no changes left (4 of 4 changes used)."
        ),
        SkipReason::VoteStateUnavailable => format!(
            "Votes skipped: {count}. Vote state is unavailable. Refresh voting to include these nodes."
        ),
        SkipReason::AlreadyInProgress => {
            format!("Votes skipped: {count}. Earlier votes on these names are still being sent.")
        }
        SkipReason::NotUsed => format!("Votes skipped: {count}. You chose not to use these nodes."),
    }
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
        (0, 0) => "Less than a minute before the end".to_owned(),
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
        ChangesLeft::Known(left) => format!("{left} of 4 changes left, counted on this device"),
        ChangesLeft::Unknown => {
            "Changes left unknown. This device has no record of this node's earlier votes."
                .to_owned()
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DpnsVoteFeedbackCounts {
    confirmed: usize,
    scheduled: usize,
    unconfirmed: usize,
    rejected: usize,
    failed_before_submission: usize,
    cancelled: usize,
    not_applied: usize,
    in_progress: usize,
}

/// Feedback for a submit whose single requested vote was already in place.
const DPNS_ONE_VOTE_ALREADY_CAST: &str =
    "The selected node already has the requested vote. Nothing was submitted.";

/// Feedback for a submit in which every requested vote was already in place.
/// Also used where the skipped count is unavailable, so it has to read true for
/// a single selected node too.
pub(crate) const DPNS_ALL_VOTES_ALREADY_CAST: &str =
    "Every selected node already has the requested vote. Nothing was submitted.";

/// Guidance for votes that reached Platform without a confirmed outcome. Shared
/// by the all-unconfirmed and mixed branches so the "do not resubmit" warning
/// never differs between them.
fn dpns_unconfirmed_guidance(unconfirmed: usize) -> String {
    match unconfirmed {
        1 => "The vote was submitted, but the result could not be confirmed yet. Dash Evo Tool will keep checking. Do not submit it again.".to_owned(),
        count => format!(
            "{count} votes were submitted, but their results could not be confirmed yet. Dash Evo Tool will keep checking. Do not submit them again."
        ),
    }
}

pub(crate) fn dpns_vote_feedback(operation: &DpnsVoteOperation) -> (String, MessageType, bool) {
    let mut counts = DpnsVoteFeedbackCounts::default();
    for outcome in &operation.targets {
        match outcome.status {
            DpnsVoteTargetStatus::Confirmed => counts.confirmed += 1,
            DpnsVoteTargetStatus::Scheduled => counts.scheduled += 1,
            DpnsVoteTargetStatus::Unconfirmed => counts.unconfirmed += 1,
            DpnsVoteTargetStatus::Rejected => counts.rejected += 1,
            DpnsVoteTargetStatus::FailedBeforeSubmission => {
                counts.failed_before_submission += 1;
            }
            DpnsVoteTargetStatus::Cancelled => counts.cancelled += 1,
            DpnsVoteTargetStatus::NotApplied => counts.not_applied += 1,
            DpnsVoteTargetStatus::Queued
            | DpnsVoteTargetStatus::Submitting
            | DpnsVoteTargetStatus::Confirming => counts.in_progress += 1,
        }
    }
    if operation.targets.is_empty() {
        let message = match operation.no_op_count {
            1 => DPNS_ONE_VOTE_ALREADY_CAST,
            _ => DPNS_ALL_VOTES_ALREADY_CAST,
        };
        return (message.to_owned(), MessageType::Info, false);
    }
    let target_count = operation.targets.len();
    if target_count == 1 && counts.confirmed == 1 {
        return (
            "Vote cast successfully.".to_owned(),
            MessageType::Success,
            false,
        );
    }
    // VOTE-FR-061: a batch that only confirmed, or confirmed with some still
    // being checked, reads as nodes × names.
    if counts.confirmed > 0 && counts.confirmed + counts.unconfirmed == target_count {
        let confirmed = operation
            .targets
            .iter()
            .filter(|outcome| outcome.status == DpnsVoteTargetStatus::Confirmed);
        let nodes = confirmed
            .clone()
            .map(|outcome| outcome.target.key.voter_id)
            .collect::<BTreeSet<_>>()
            .len();
        let names = confirmed
            .map(|outcome| outcome.target.key.vote_poll_id)
            .collect::<BTreeSet<_>>()
            .len();
        let checking = counts.unconfirmed;
        let message_type = if checking == 0 {
            MessageType::Success
        } else {
            MessageType::Warning
        };
        return (
            batch_voted_line(nodes, names, checking),
            message_type,
            checking > 0,
        );
    }
    let single = target_count == 1;
    if counts.scheduled == target_count {
        let message = if single {
            "Vote scheduled successfully.".to_owned()
        } else {
            format!("{target_count} votes were scheduled.")
        };
        return (message, MessageType::Success, false);
    }
    if counts.confirmed + counts.scheduled == target_count {
        return (
            format!(
                "Votes cast successfully: {confirmed}. Votes scheduled: {scheduled}.",
                confirmed = counts.confirmed,
                scheduled = counts.scheduled,
            ),
            MessageType::Success,
            false,
        );
    }
    if counts.unconfirmed == target_count {
        return (
            dpns_unconfirmed_guidance(counts.unconfirmed),
            MessageType::Warning,
            true,
        );
    }
    if counts.rejected == target_count {
        let message = if single {
            "The vote was rejected. Review the vote and try again."
        } else {
            "The votes were rejected. Review the votes and try again."
        };
        return (message.to_owned(), MessageType::Error, false);
    }
    if counts.failed_before_submission == target_count {
        let message = if single {
            "This vote was not submitted. Check your connection and try again."
        } else {
            "These votes were not submitted. Check your connection and try again."
        };
        return (message.to_owned(), MessageType::Error, false);
    }
    if counts.not_applied == target_count {
        let message = if single {
            "The submitted vote was not applied. Review the vote and try again."
        } else {
            "The submitted votes were not applied. Review the votes and try again."
        };
        return (message.to_owned(), MessageType::Error, false);
    }
    if counts.cancelled == target_count {
        let message = if single {
            "The scheduled vote was cancelled. Nothing was submitted."
        } else {
            "The scheduled votes were cancelled. Nothing was submitted."
        };
        return (message.to_owned(), MessageType::Info, false);
    }
    if counts.in_progress == target_count {
        return (
            "Voting is still in progress. Wait for the result before submitting again.".to_owned(),
            MessageType::Info,
            false,
        );
    }

    let reviewable = counts.rejected + counts.failed_before_submission + counts.not_applied;
    let DpnsVoteFeedbackCounts {
        confirmed,
        scheduled,
        cancelled,
        in_progress,
        unconfirmed,
        ..
    } = counts;
    let message = format!(
        "Vote results: {confirmed} confirmed, {scheduled} scheduled, {reviewable} needing review, {cancelled} cancelled, {in_progress} in progress, and {unconfirmed} still being checked. Review failed votes before retrying. Wait for votes in progress or still being checked; do not submit those votes again."
    );
    (message, MessageType::Warning, unconfirmed > 0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn blocker_exhausted_vote_limit_uses_changes_consistently() {
        let copy = super::skipped_reason_line(1, super::SkipReason::NoChangesLeft);
        assert!(copy.contains("4 of 4 changes used"), "{copy}");
        assert!(!copy.contains("5 of 5 votes"));
    }

    /// History outcome and node-vote wording (frame V4).
    #[test]
    fn history_copy_names_the_outcome_and_votes() {
        use super::*;
        assert_eq!(went_to_label(Some("zed"), "AbC1…"), "Went to zed (AbC1…)");
        assert_eq!(went_to_label(None, "AbC1…"), "Went to AbC1…");
        assert_eq!(voting_weight_label(4), "Voting weight: 4 votes");
        assert_eq!(voting_weight_label(1), "Voting weight: 1 vote");
        assert_eq!(
            before_end_phrase(std::time::Duration::from_secs(30)),
            "Less than a minute before the end"
        );
        assert_eq!(
            excluded_nodes_line(2, NodeExclusion::NotInMasternodeList),
            "2 nodes not used: they are not in the masternode list, so their votes don't count."
        );
    }

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
            "19 of your nodes have not voted. 4 of your nodes voted: Abstain. mn-07 has no changes left."
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

    /// VOTE-TC-006/094/095/096 (copy half): the confirm lines follow VOTE-FR-080.
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
            "Votes skipped: 1. The selected nodes have no changes left (4 of 4 changes used)."
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
            "3 of 4 changes left, counted on this device"
        );
        assert_eq!(
            changes_left_label(ChangesLeft::Unknown),
            "Changes left unknown. This device has no record of this node's earlier votes."
        );
    }

    /// VOTE-FR-061: the batch banner counts nodes and names.
    #[test]
    fn batch_banner_counts_nodes_and_names() {
        assert_eq!(batch_voted_line(24, 3, 0), "24 nodes voted on 3 names.");
        assert_eq!(
            batch_voted_line(2, 1, 1),
            "Participating nodes: 2. Names with confirmed votes: 1. Votes still being checked: 1. Dash Evo Tool will keep checking. Do not submit the pending votes again."
        );
    }

    /// VOTE-TC-101/102/103 (copy half).
    #[test]
    fn drawer_and_attention_copy() {
        assert_eq!(
            drawer_header(ProgressCounts {
                total: 72,
                confirmed: 40,
                failed: 2,
                sending: 25,
                checking: 5,
            }),
            "Casting 72 votes · 42 done · 25 sending · 5 being checked"
        );
        assert_eq!(
            progress_row_status(
                DpnsVoteTargetStatus::FailedBeforeSubmission,
                Some(DpnsVoteFailure::VotingEnded)
            ),
            "Not submitted. Voting ended."
        );
        assert_eq!(
            needs_attention_line(NeedsAttention {
                checking: 1,
                failed: 0,
                missed_schedules: 2,
            }),
            "Votes still being checked: 1. Missed scheduled votes: 2. Do not submit the pending votes again."
        );
    }

    #[test]
    fn attention_copy_omits_zero_counts_and_warns_about_pending_votes() {
        for checking in 0..=1 {
            for failed in 0..=1 {
                for missed_schedules in 0..=1 {
                    let message = needs_attention_line(NeedsAttention {
                        checking,
                        failed,
                        missed_schedules,
                    });
                    assert!(!message.contains(": 0."));
                    assert_eq!(message.contains("Do not submit"), checking > 0);
                    assert_eq!(
                        message.is_empty(),
                        checking + failed + missed_schedules == 0
                    );
                }
            }
        }
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
