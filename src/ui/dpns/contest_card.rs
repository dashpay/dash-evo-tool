//! One contest card in Masternodes ▸ Votes (frame V1): a read-only weighted
//! tally on the left, the decision choices and node line on the right.

use crate::model::datetime;
use crate::model::dpns::ContestDurations;
use crate::model::dpns_voting::operator::{Influence, influence, time_left};
use crate::ui::dpns::copy;
use crate::ui::state::dpns_vote_cards::{NodeContestStatus, VoteCard};
use crate::ui::theme::{DashColors, ResponseExt};
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, RichText, Ui};
use std::collections::BTreeMap;
use std::time::Duration;

/// Shortcut keys shown next to contender choices, in contender order.
pub const CONTENDER_KEYS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];

const TALLY_BAR_WIDTH: f32 = 140.0;

/// A narrower card stacks the tally above the decision: half of it cannot
/// hold a contender's full identifier on one line beside its tally bar.
const TWO_COLUMN_MIN_WIDTH: f32 = 960.0;

/// What the operator did to a card this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardEvent {
    Choose(ResourceVoteChoice),
    ToggleSelected,
    RefreshVoting,
}

/// The decision choices of a card, in display and shortcut order.
pub fn card_choices(card: &VoteCard) -> Vec<(ResourceVoteChoice, String)> {
    let mut choices: Vec<(ResourceVoteChoice, String)> = card
        .contest
        .contestants
        .iter()
        .flatten()
        .map(|contender| {
            (
                ResourceVoteChoice::TowardsIdentity(contender.id),
                copy::vote_choice_label(
                    ResourceVoteChoice::TowardsIdentity(contender.id),
                    Some(&contender.name),
                ),
            )
        })
        .collect();
    choices.push((ResourceVoteChoice::Lock, "Lock name".to_owned()));
    choices.push((ResourceVoteChoice::Abstain, "Abstain".to_owned()));
    choices
}

/// Short label of a choice for node lines and summaries.
pub fn choice_short_label(card: &VoteCard, choice: ResourceVoteChoice) -> String {
    let name = card
        .contest
        .contestants
        .iter()
        .flatten()
        .find(|contender| choice == ResourceVoteChoice::TowardsIdentity(contender.id))
        .map(|contender| contender.name.as_str());
    copy::vote_choice_label(choice, name)
}

/// The node line for a card, or `None` when no node is loaded.
pub fn card_node_line(
    card: &VoteCard,
    node_labels: &BTreeMap<Identifier, String>,
) -> Option<String> {
    let mut parts = Vec::new();
    let not_voted = card.count(|status| status == NodeContestStatus::NotVoted);
    if not_voted > 0 {
        parts.push(copy::not_voted_part(not_voted));
    }
    let mut voted: BTreeMap<String, usize> = BTreeMap::new();
    for node in &card.nodes {
        if let NodeContestStatus::Voted(choice) = node.status {
            *voted.entry(choice_short_label(card, choice)).or_default() += 1;
        }
    }
    parts.extend(
        voted
            .into_iter()
            .map(|(label, count)| copy::voted_part(count, &label)),
    );
    let exhausted: Vec<String> = card
        .nodes
        .iter()
        .filter(|node| matches!(node.status, NodeContestStatus::NoChangesLeft(_)))
        .map(|node| node_label(node.node, node_labels))
        .collect();
    if !exhausted.is_empty() {
        parts.push(copy::no_changes_part(&exhausted));
    }
    let sending = card.in_flight_count();
    if sending > 0 {
        parts.push(copy::sending_part(sending));
    }
    let scheduled = card.scheduled_count();
    if scheduled > 0 {
        parts.push(copy::scheduled_part(scheduled));
    }
    (!parts.is_empty()).then(|| copy::node_line(&parts))
}

/// How many submittable nodes a staged `choice` would change from an earlier vote.
pub fn changed_vote_count(card: &VoteCard, choice: ResourceVoteChoice) -> usize {
    card.nodes
        .iter()
        .filter(|node| node.status.can_submit())
        .filter(|node| node.current.is_some_and(|current| current != choice))
        .count()
}

/// Display label for a node: its alias, else a short identifier.
pub fn node_label(node: Identifier, labels: &BTreeMap<Identifier, String>) -> String {
    crate::model::identity_name::masternode_label(node, labels.get(&node).map(String::as_str))
}

#[test]
fn node_labels_match_the_hex_pro_tx_hash_and_preserve_aliases() {
    let node = Identifier::from([0xab; 32]);
    assert_eq!(node_label(node, &BTreeMap::new()), "ababa…bab");
    assert_eq!(
        node_label(node, &BTreeMap::from([(node, "My node".into())])),
        "My node"
    );
}

/// A stored time to the minute, in the user's time zone.
fn local_minute(ms: u64) -> String {
    datetime::instant_from_unix_millis(ms)
        .map(datetime::local_date_time)
        .unwrap_or_default()
}

/// The schedule caption under a card title.
pub fn schedule_caption(
    card: &VoteCard,
    durations: ContestDurations,
    now_ms: u64,
) -> Option<String> {
    let end = card.contest.end_time?;
    let requests = copy::requests_label(card.contest.contestants.as_ref().map_or(0, Vec::len));
    let total = u64::try_from(durations.total.as_millis()).unwrap_or(u64::MAX);
    let join = u64::try_from(durations.join.as_millis()).unwrap_or(u64::MAX);
    let join_until = end.saturating_sub(total).saturating_add(join);
    Some(if now_ms < join_until {
        format!(
            "{requests} · Others can join until {when}.",
            when = local_minute(join_until)
        )
    } else {
        format!("{requests} · Voting ends {when}.", when = local_minute(end))
    })
}

/// Everything a card needs besides the card itself.
pub struct CardView<'a> {
    pub card: &'a VoteCard,
    pub staged: Option<ResourceVoteChoice>,
    pub selected: bool,
    pub focused: bool,
    /// Scroll the card into view this frame (keyboard focus moved to it).
    pub scroll_into_view: bool,
    pub node_labels: &'a BTreeMap<Identifier, String>,
    pub node_set_weight: u32,
    pub now_ms: u64,
    pub urgency: Duration,
    pub durations: ContestDurations,
    pub has_voting_nodes: bool,
}

impl CardView<'_> {
    /// Render the card; returns the operator's interactions in order.
    pub fn show(self, ui: &mut Ui) -> Vec<CardEvent> {
        let mut events = Vec::new();
        let dark_mode = ui.visuals().dark_mode;
        let stroke = if self.focused {
            egui::Stroke::new(2.0, DashColors::DASH_BLUE)
        } else {
            egui::Stroke::new(1.0, DashColors::border_light(dark_mode))
        };
        let frame = egui::Frame::group(ui.style())
            .stroke(stroke)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if ui.available_width() < TWO_COLUMN_MIN_WIDTH {
                    self.show_tally(ui, &mut events);
                    ui.add_space(8.0);
                    self.show_decision(ui, &mut events);
                } else {
                    ui.columns(2, |columns| {
                        self.show_tally(&mut columns[0], &mut events);
                        self.show_decision(&mut columns[1], &mut events);
                    });
                }
            });
        if self.scroll_into_view {
            ui.scroll_to_rect(frame.response.rect, None);
        }
        events
    }

    fn show_tally(&self, ui: &mut Ui, events: &mut Vec<CardEvent>) {
        let dark_mode = ui.visuals().dark_mode;
        let card = self.card;
        ui.horizontal(|ui| {
            let mut selected = self.selected;
            if ui
                .checkbox(&mut selected, "")
                .on_hover_text("Select this name for a bulk decision.")
                .changed()
            {
                events.push(CardEvent::ToggleSelected);
            }
            ui.label(
                RichText::new(format!("{name}.dash", name = card.name()))
                    .heading()
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
        });
        let remaining = card
            .contest
            .end_time
            .and_then(|end| time_left(end, self.now_ms));
        let urgent = card.contest.end_time.is_some_and(|end| {
            end > self.now_ms && u128::from(end - self.now_ms) <= self.urgency.as_millis()
        });
        if card.contest.end_time.is_some() {
            let color = if urgent {
                DashColors::warning_color(dark_mode)
            } else {
                DashColors::text_secondary(dark_mode)
            };
            ui.label(RichText::new(copy::ends_in_label(remaining)).color(color));
        }
        if let Some(caption) = schedule_caption(card, self.durations, self.now_ms) {
            ui.label(RichText::new(caption).color(DashColors::text_secondary(dark_mode)));
        }
        ui.add_space(4.0);

        let mut rows: Vec<(String, u32)> = card
            .contest
            .contestants
            .iter()
            .flatten()
            .map(|contender| {
                (
                    copy::vote_choice_label(
                        ResourceVoteChoice::TowardsIdentity(contender.id),
                        Some(&contender.name),
                    ),
                    contender.votes,
                )
            })
            .collect();
        rows.push((
            "Lock name".to_owned(),
            card.contest.locked_votes.unwrap_or_default(),
        ));
        rows.push((
            "Abstain".to_owned(),
            card.contest.abstain_votes.unwrap_or_default(),
        ));
        let max = rows
            .iter()
            .map(|(_, votes)| *votes)
            .max()
            .unwrap_or(0)
            .max(1);
        // A grid column grows with its text unless bounded, and `ui.columns`
        // reserves twice the wider half: an unbounded label widens the card.
        let label_width = (ui.available_width() - TALLY_BAR_WIDTH - ui.spacing().item_spacing.x)
            .max(ui.spacing().interact_size.x);
        egui::Grid::new(("tally", card.name()))
            .num_columns(2)
            .max_col_width(label_width)
            .show(ui, |ui| {
                for (label, votes) in rows {
                    ui.label(label);
                    ui.add(
                        egui::ProgressBar::new(votes as f32 / max as f32)
                            .desired_width(TALLY_BAR_WIDTH)
                            .text(votes.to_string()),
                    );
                    ui.end_row();
                }
            });
        ui.label(
            RichText::new(copy::EVONODE_WEIGHT_HINT)
                .small()
                .color(DashColors::text_secondary(dark_mode)),
        );
        let contenders: Vec<(Identifier, u32)> = card
            .contest
            .contestants
            .iter()
            .flatten()
            .map(|contender| (contender.id, contender.votes))
            .collect();
        match influence(
            &contenders,
            card.contest.locked_votes.unwrap_or_default(),
            self.node_set_weight,
        ) {
            Some(Influence::Tied) => {
                ui.label(copy::TIE_LINE);
            }
            Some(Influence::TiedWithLock) => {
                ui.label(copy::LOCK_TIE_LINE);
            }
            Some(Influence::CanChangeLeader { leader, margin }) if self.has_voting_nodes => {
                let leader = match leader {
                    ResourceVoteChoice::TowardsIdentity(id) => card
                        .contest
                        .contestants
                        .iter()
                        .flatten()
                        .find(|contender| contender.id == id)
                        .map_or_else(|| "A contender".to_owned(), |c| c.name.clone()),
                    _ => "Lock name".to_owned(),
                };
                ui.label(copy::influence_line(&leader, margin, self.node_set_weight));
            }
            _ => {}
        }
    }

    fn show_decision(&self, ui: &mut Ui, events: &mut Vec<CardEvent>) {
        let dark_mode = ui.visuals().dark_mode;
        let card = self.card;
        ui.label(RichText::new("Your decision").strong());
        let enabled = card.accepts_decision();
        let disabled_reason = if !self.has_voting_nodes {
            "Load a masternode with a voting key to vote."
        } else if card.in_flight_count() > 0 {
            "These nodes are still sending a vote on this name. Wait for the result."
        } else if card.scheduled_count() > 0 {
            "These nodes already have a scheduled vote on this name. Edit it on the Scheduled tab."
        } else {
            "None of your nodes can vote on this name right now."
        };
        let shared = card.shared_choice();
        let highlighted = self.staged.or(shared);
        let mut contender_index = 0usize;
        // One radio row per choice; a long label wraps inside the column.
        for (choice, label) in card_choices(card) {
            let key = match choice {
                ResourceVoteChoice::TowardsIdentity(_) => {
                    let key = CONTENDER_KEYS.get(contender_index).copied();
                    contender_index += 1;
                    key
                }
                ResourceVoteChoice::Lock => Some("L"),
                ResourceVoteChoice::Abstain => Some("A"),
            };
            let text = match key {
                Some(key) => format!("{label}  [{key}]"),
                None => label.clone(),
            };
            let response = ui
                .add_enabled(
                    enabled,
                    egui::RadioButton::new(highlighted == Some(choice), text),
                )
                .disabled_tooltip(disabled_reason);
            if let ResourceVoteChoice::TowardsIdentity(id) = choice {
                response.clone().on_hover_text(
                    id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58),
                );
            }
            if response.clicked() {
                events.push(CardEvent::Choose(choice));
            }
        }
        let in_flight = card.in_flight_count();
        if in_flight > 0 {
            ui.label(
                RichText::new(copy::sending_with_label(in_flight))
                    .color(DashColors::warning_color(dark_mode)),
            );
        }
        if let Some(line) = card_node_line(card, self.node_labels) {
            ui.label(RichText::new(line).color(DashColors::text_secondary(dark_mode)));
        }
        if card.nodes.len() > 1 && card.voted_count() > 0 && card.voted_count() < card.nodes.len() {
            ui.label(copy::voted_with_label(card.voted_count(), card.nodes.len()));
        }
        if let Some(staged) = self.staged {
            let changed = changed_vote_count(card, staged);
            if changed > 0 {
                ui.label(
                    RichText::new(copy::change_warning_line(changed))
                        .color(DashColors::warning_color(dark_mode)),
                );
            }
        }
        let unavailable = card.count(|status| status == NodeContestStatus::Unavailable);
        if unavailable > 0 {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(copy::unavailable_line(unavailable))
                        .color(DashColors::warning_color(dark_mode)),
                );
                if ui.button("Refresh voting").clicked() {
                    events.push(CardEvent::RefreshVoting);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::contested_name::{ContestState, Contestant, ContestedName};
    use crate::model::dpns_voting::operator::ChangesLeft;
    use crate::ui::state::dpns_vote_cards::NodeContestState;
    use std::sync::Arc;

    fn card(statuses: &[NodeContestStatus]) -> VoteCard {
        let contest = ContestedName {
            normalized_contested_name: "alice".to_owned(),
            contestants: Some(vec![Contestant {
                name: "alice".to_owned(),
                id: Identifier::from([7; 32]),
                info: String::new(),
                votes: 6,
                created_at: None,
                created_at_block_height: None,
                created_at_core_block_height: None,
                document_id: Identifier::from([8; 32]),
            }]),
            locked_votes: Some(2),
            abstain_votes: Some(0),
            awarded_to: None,
            end_time: Some(10_000_000),
            state: ContestState::Ongoing,
            last_updated: None,
            my_votes: Default::default(),
        };
        let nodes = statuses
            .iter()
            .enumerate()
            .map(|(index, status)| {
                let current = match status {
                    NodeContestStatus::Voted(choice) | NodeContestStatus::NoChangesLeft(choice) => {
                        Some(*choice)
                    }
                    _ => None,
                };
                NodeContestState {
                    node: Identifier::from([index as u8 + 1; 32]),
                    status: *status,
                    current,
                    changes: ChangesLeft::Known(4),
                }
            })
            .collect();
        VoteCard::new(Arc::new(contest), Some(Identifier::from([9; 32])), nodes)
    }

    #[test]
    fn choices_list_contenders_then_lock_and_abstain() {
        let card = card(&[]);
        let labels: Vec<String> = card_choices(&card).into_iter().map(|(_, l)| l).collect();
        assert_eq!(
            labels,
            vec![
                copy::vote_choice_label(
                    ResourceVoteChoice::TowardsIdentity(Identifier::from([7; 32])),
                    Some("alice")
                ),
                "Lock name".to_owned(),
                "Abstain".to_owned()
            ]
        );
    }

    #[test]
    fn equal_contender_names_keep_distinct_vote_handles() {
        let mut card = card(&[]);
        let contest = Arc::make_mut(&mut card.contest);
        let mut other = contest.contestants.as_ref().unwrap()[0].clone();
        other.id = Identifier::from([9; 32]);
        contest.contestants.as_mut().unwrap().push(other);
        let choices = card_choices(&card);
        assert_ne!(choices[0].1, choices[1].1);
        assert!(choices[0].1.starts_with("Vote for alice ("));
        assert!(choices[1].1.starts_with("Vote for alice ("));
    }

    #[test]
    fn node_line_groups_by_status() {
        let abstain = ResourceVoteChoice::Abstain;
        let card = card(&[
            NodeContestStatus::NotVoted,
            NodeContestStatus::NotVoted,
            NodeContestStatus::Voted(abstain),
            NodeContestStatus::NoChangesLeft(abstain),
        ]);
        let labels = BTreeMap::from([(Identifier::from([4; 32]), "mn-07".to_owned())]);
        assert_eq!(
            card_node_line(&card, &labels).as_deref(),
            Some(
                "2 of your nodes have not voted. 1 of your nodes voted: Abstain. mn-07 has no changes left."
            )
        );
    }

    /// VOTE-TC-006 (card half): the change count covers only nodes that would
    /// really change.
    #[test]
    fn change_count_excludes_matching_and_unsubmittable_nodes() {
        let lock = ResourceVoteChoice::Lock;
        let abstain = ResourceVoteChoice::Abstain;
        let card = card(&[
            NodeContestStatus::Voted(abstain),
            NodeContestStatus::Voted(abstain),
            NodeContestStatus::Voted(lock),
            NodeContestStatus::NoChangesLeft(abstain),
            NodeContestStatus::NotVoted,
        ]);
        assert_eq!(changed_vote_count(&card, lock), 2);
        assert_eq!(changed_vote_count(&card, abstain), 1);
    }

    #[test]
    fn caption_mentions_the_join_window_only_while_it_is_open() {
        let card = card(&[]);
        let durations = ContestDurations {
            total: Duration::from_millis(9_000_000),
            join: Duration::from_millis(4_000_000),
        };
        let local = |ms| datetime::local_date_time(datetime::instant_from_unix_millis(ms).unwrap());
        assert_eq!(
            schedule_caption(&card, durations, 2_000_000).unwrap(),
            format!(
                "1 request · Others can join until {when}.",
                when = local(5_000_000)
            )
        );
        assert_eq!(
            schedule_caption(&card, durations, 6_000_000).unwrap(),
            format!("1 request · Voting ends {when}.", when = local(10_000_000))
        );
    }

    /// A card view with `Lock name` staged.
    fn lock_staged_view<'a>(
        card: &'a VoteCard,
        node_labels: &'a BTreeMap<Identifier, String>,
    ) -> CardView<'a> {
        CardView {
            card,
            staged: Some(ResourceVoteChoice::Lock),
            selected: false,
            focused: false,
            scroll_into_view: false,
            node_labels,
            node_set_weight: 1,
            now_ms: 6_000_000,
            urgency: Duration::from_secs(60),
            durations: ContestDurations {
                total: Duration::from_millis(9_000_000),
                join: Duration::from_millis(4_000_000),
            },
            has_voting_nodes: true,
        }
    }

    /// The choices read as a pick-one list: one radio row each, in shortcut
    /// order, with the staged choice marked.
    #[test]
    fn decision_choices_are_stacked_radio_rows() {
        use egui::accesskit::{Role, Toggled};
        use egui_kittest::kittest::{NodeT, Queryable};
        let card = card(&[NodeContestStatus::NotVoted]);
        let labels = BTreeMap::new();
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 600.0))
            .build_ui(move |ui| {
                lock_staged_view(&card, &labels).show(ui);
            });

        let rows: Vec<_> = harness.query_all_by_role(Role::RadioButton).collect();
        let texts: Vec<String> = rows
            .iter()
            .map(|row| row.accesskit_node().label().unwrap_or_default())
            .collect();
        assert_eq!(texts.len(), 3, "{texts:?}");
        assert!(texts[0].ends_with("  [1]"), "{texts:?}");
        assert_eq!(texts[1..], ["Lock name  [L]", "Abstain  [A]"]);
        let marked: Vec<bool> = rows
            .iter()
            .map(|row| row.accesskit_node().toggled() == Some(Toggled::True))
            .collect();
        assert_eq!(marked, [false, true, false]);
        for pair in rows.windows(2) {
            let (above, below) = (pair[0].rect(), pair[1].rect());
            assert_eq!(above.left(), below.left(), "the rows share a left edge");
            assert!(
                below.top() >= above.bottom(),
                "{below:?} is not under {above:?}"
            );
        }
    }

    #[test]
    fn clicking_a_decision_row_reports_that_choice() {
        use egui_kittest::kittest::Queryable;
        let card = card(&[NodeContestStatus::NotVoted]);
        let labels = BTreeMap::new();
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let reported = Arc::clone(&events);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 600.0))
            .build_ui(move |ui| {
                let events = lock_staged_view(&card, &labels).show(ui);
                reported.lock().unwrap().extend(events);
            });

        harness.get_by_label("Abstain  [A]").click();
        harness.run();
        harness.get_by_label("Lock name  [L]").click();
        harness.run();

        assert_eq!(
            *events.lock().unwrap(),
            [
                CardEvent::Choose(ResourceVoteChoice::Abstain),
                CardEvent::Choose(ResourceVoteChoice::Lock),
            ],
            "a click on the marked row is reported too, so the screen can clear it"
        );
    }
}
