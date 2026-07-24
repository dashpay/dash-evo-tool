use crate::wallet_backend::poison::MutexRecover;
use std::sync::{Arc, Mutex};
use tracing::error;

use chrono::{DateTime, LocalResult, NaiveDate, NaiveTime, TimeZone, Timelike, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Button, Color32, ComboBox, Label, RichText, Ui};
use egui_extras::{Column, TableBuilder};

use crate::app::{AppAction, DesiredAppAction, scheduled_vote_sweep_is_quiet};
use crate::backend_task::contested_names::{ContestedResourceTask, ScheduledDPNSVote};
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskContext};
use crate::context::AppContext;
use crate::model::contested_name::{ContestState, ContestedName};
use crate::model::dpns::normalize_dpns_label;
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteTarget,
    DpnsVoteTargetKey, DpnsVoteTargetStatus, VoteTiming,
};
use crate::model::qualified_identity::{DPNSNameInfo, QualifiedIdentity};
use crate::ui::components::component_trait::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::dpns_subscreen_chooser_panel::add_dpns_subscreen_chooser_panel;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::progress_overlay::{OptionOverlayExt, OverlayConfig, OverlayHandle};
use crate::ui::components::styled::{StyledButton, island_central_panel};
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::{add_top_panel_with_global_nav, subdued_everyday_spec};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::identities::register_dpns_name_screen::RegisterDpnsNameSource;
use crate::ui::state::dpns_vote_operations::DpnsVoteOperationSnapshot;
use crate::ui::state::dpns_vote_state::DpnsVoteStateSnapshot;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, ScreenLike, ScreenType};

/// Which DPNS sub-screen is currently showing.
#[derive(PartialEq)]
pub enum DPNSSubscreen {
    Active,
    Past,
    Owned,
    ScheduledVotes,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveContestGroup {
    NeedsVote,
    Voted,
    NotVotable,
}

const NO_VOTING_NODES_MESSAGE: &str = "None of your loaded nodes has a voting key.";
const NO_VOTING_NODES_DETAIL: &str = "Load a masternode with its voting key to cast votes.";

fn candidate_choice_label(candidate_name: &str) -> String {
    format!("Vote for {candidate_name}")
}

fn review_vote_choice_label(choice: ResourceVoteChoice, candidate_name: Option<&str>) -> String {
    match choice {
        ResourceVoteChoice::Lock => "Lock".to_owned(),
        ResourceVoteChoice::Abstain => "Abstain".to_owned(),
        ResourceVoteChoice::TowardsIdentity(_) => candidate_name
            .map(candidate_choice_label)
            .unwrap_or_else(|| "Vote for a candidate that is no longer listed.".to_owned()),
    }
}

fn classify_vote_states(
    states: impl IntoIterator<Item = DpnsCurrentVoteState>,
) -> ActiveContestGroup {
    let mut has_vote = false;
    for state in states {
        match state {
            DpnsCurrentVoteState::Available(None) => return ActiveContestGroup::NeedsVote,
            DpnsCurrentVoteState::Available(Some(_)) => has_vote = true,
            DpnsCurrentVoteState::Checking | DpnsCurrentVoteState::Unavailable => {}
        }
    }
    if has_vote {
        ActiveContestGroup::Voted
    } else {
        ActiveContestGroup::NotVotable
    }
}

fn tally_chip(ui: &mut Ui, votes: u32, dark_mode: bool) {
    egui::Frame::new()
        .fill(DashColors::surface(dark_mode))
        .corner_radius(egui::CornerRadius::same(255))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(format!("{votes} votes"));
        });
}

fn short_identifier(identifier: Identifier) -> String {
    let encoded = identifier.to_string(Encoding::Base58);
    format!("{}…{}", &encoded[..6], &encoded[encoded.len() - 4..])
}

fn vote_choice_label(choice: ResourceVoteChoice, candidate_name: Option<&str>) -> String {
    match choice {
        ResourceVoteChoice::Lock => "Lock".to_owned(),
        ResourceVoteChoice::Abstain => "Abstain".to_owned(),
        ResourceVoteChoice::TowardsIdentity(identifier) => candidate_name
            .map(candidate_choice_label)
            .unwrap_or_else(|| format!("Vote for {}", short_identifier(identifier))),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProvedVoteSummary {
    None,
    Choice(ResourceVoteChoice),
    Mixed,
}

fn proved_vote_summary(
    states: impl IntoIterator<Item = DpnsCurrentVoteState>,
) -> ProvedVoteSummary {
    let mut proved_choice = None;
    for state in states {
        let DpnsCurrentVoteState::Available(Some(choice)) = state else {
            continue;
        };
        if proved_choice.is_some_and(|current| current != choice) {
            return ProvedVoteSummary::Mixed;
        }
        proved_choice = Some(choice);
    }
    proved_choice.map_or(ProvedVoteSummary::None, ProvedVoteSummary::Choice)
}

fn target_status_label(status: DpnsVoteTargetStatus) -> &'static str {
    match status {
        DpnsVoteTargetStatus::Scheduled => "Scheduled",
        DpnsVoteTargetStatus::Queued => "Queued",
        DpnsVoteTargetStatus::Submitting => "Submitting",
        DpnsVoteTargetStatus::Confirming => "Confirming",
        DpnsVoteTargetStatus::Confirmed => "Confirmed",
        DpnsVoteTargetStatus::Unconfirmed => "Confirmation is still being checked",
        DpnsVoteTargetStatus::Rejected => "Rejected",
        DpnsVoteTargetStatus::FailedBeforeSubmission => "Not submitted",
        DpnsVoteTargetStatus::NotApplied => "Not applied",
        DpnsVoteTargetStatus::Cancelled => "Cancelled",
    }
}

fn dpns_operation_id(context: &BackendTaskContext) -> Option<DpnsVoteOperationId> {
    match context {
        BackendTaskContext::Dispatched { operation, .. } => dpns_operation_id(operation),
        BackendTaskContext::DpnsVoteOperation { operation_id, .. } => Some(*operation_id),
        _ => None,
    }
}

impl DPNSSubscreen {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Active => "Active contests",
            Self::Past => "Past contests",
            Self::Owned => "My usernames",
            Self::ScheduledVotes => "Scheduled votes",
        }
    }
}

/// Minimal object for storing the user’s currently selected vote on a single contested name.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectedVote {
    pub contested_name: String,
    pub vote_choice: ResourceVoteChoice,
    pub end_time: Option<u64>,
}

#[derive(Clone)]
pub enum VoteOption {
    NoVote,
    CastNow,
    Scheduled { days: u32, hours: u32, minutes: u32 },
}

/// Tracks the casting status for each scheduled vote item.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScheduledVoteCastingStatus {
    NotStarted,
    InProgress,
    Failed,
    Completed,
}

#[derive(PartialEq)]
pub enum VoteHandlingStatus {
    NotStarted,
    CastingVotes,
    SchedulingVotes,
    Completed,
    Failed(String),
}

#[derive(PartialEq)]
pub enum RefreshingStatus {
    Refreshing,
    NotRefreshing,
}

/// Sorting columns
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    ContestedName,
    EndingTime,
    LastUpdated,
    AwardedTo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortOrder {
    Ascending,
    Descending,
}

/// The main, combined DPNSScreen:
/// - Displays active/past/owned DPNS contests
/// - Allows clicking selection of votes (bulk scheduling)
/// - Allows single immediate vote or single schedule
/// - Shows scheduled votes listing
pub struct DPNSScreen {
    voting_identities: Vec<QualifiedIdentity>,
    user_identities: Vec<QualifiedIdentity>,
    contested_names: Arc<Mutex<Vec<ContestedName>>>,
    local_dpns_names: Arc<Mutex<Vec<(Identifier, DPNSNameInfo)>>>,
    pub scheduled_votes: Arc<Mutex<Vec<(ScheduledDPNSVote, ScheduledVoteCastingStatus)>>>,
    pub scheduled_vote_cast_in_progress: bool,
    pub selected_votes: Vec<SelectedVote>,
    pub app_context: Arc<AppContext>,
    pending_backend_task: Option<BackendTask>,
    vote_operations: DpnsVoteOperationSnapshot,
    vote_state: DpnsVoteStateSnapshot,
    vote_overlay: Option<OverlayHandle>,
    pending_vote_operation: Option<DpnsVoteOperationId>,
    clear_vote_overlay_on_error: bool,
    scheduled_clear_dialog: Option<(bool, ConfirmationDialog)>,

    /// Sorting
    sort_column: SortColumn,
    sort_order: SortOrder,
    active_filter_term: String,
    past_filter_term: String,
    owned_filter_term: String,

    /// Which sub-screen is active: Active contests, Past, Owned, or Scheduled
    pub dpns_subscreen: DPNSSubscreen,
    refreshing_status: RefreshingStatus,
    refresh_banner: Option<BannerHandle>,

    /// Selected vote handling
    show_bulk_schedule_popup: bool,
    bulk_identity_options: Vec<VoteOption>,
    bulk_vote_handling_status: VoteHandlingStatus,
    set_all_option: VoteOption,
    simple_schedule_date: String,
    simple_schedule_hour: u32,
    simple_schedule_minute: u32,
}

impl DPNSScreen {
    pub fn new(app_context: &Arc<AppContext>, dpns_subscreen: DPNSSubscreen) -> Self {
        // Load contested names, local dpns, scheduled, etc.:
        let contested_names = Arc::new(Mutex::new(match dpns_subscreen {
            DPNSSubscreen::Active => app_context.ongoing_contested_names().unwrap_or_default(),
            DPNSSubscreen::Past => app_context.all_contested_names().unwrap_or_default(),
            DPNSSubscreen::Owned => Vec::new(),
            DPNSSubscreen::ScheduledVotes => app_context.all_contested_names().unwrap_or_default(),
        }));

        let local_dpns_names = Arc::new(Mutex::new(match dpns_subscreen {
            DPNSSubscreen::Active => Vec::new(),
            DPNSSubscreen::Past => Vec::new(),
            DPNSSubscreen::Owned => app_context.local_dpns_names().unwrap_or_default(),
            DPNSSubscreen::ScheduledVotes => Vec::new(),
        }));

        let scheduled_votes = app_context.get_scheduled_votes().unwrap_or_default();
        let scheduled_votes_with_status = Arc::new(Mutex::new(
            scheduled_votes
                .iter()
                .map(|vote| {
                    if vote.executed_successfully {
                        (vote.clone(), ScheduledVoteCastingStatus::Completed)
                    } else {
                        (vote.clone(), ScheduledVoteCastingStatus::NotStarted)
                    }
                })
                .collect::<Vec<_>>(),
        ));

        let voting_identities = app_context
            .load_local_voting_identities()
            .unwrap_or_default();
        let user_identities = app_context.load_local_user_identities().unwrap_or_default();
        let vote_operations =
            DpnsVoteOperationSnapshot::load(app_context).unwrap_or_else(|error| {
                tracing::warn!(
                    ?error,
                    "Could not cache DPNS vote operations for the DPNS screen"
                );
                DpnsVoteOperationSnapshot::default()
            });
        let vote_poll_ids = contested_names
            .lock_recover()
            .iter()
            .filter_map(|contest| {
                app_context
                    .dpns_vote_poll_id(&contest.normalized_contested_name)
                    .ok()
            })
            .collect::<Vec<_>>();
        let voter_ids = voting_identities
            .iter()
            .map(|identity| identity.identity.id())
            .collect::<Vec<_>>();
        let vote_state = DpnsVoteStateSnapshot::load(app_context, &voter_ids, &vote_poll_ids)
            .unwrap_or_else(|error| {
                tracing::warn!(?error, "Could not cache proved DPNS vote state");
                DpnsVoteStateSnapshot::default()
            });

        // Initialize vote handling pop-up state to hidden
        let identity_count = voting_identities.len();
        let bulk_identity_options = vec![VoteOption::CastNow; identity_count];
        let default_schedule_time = Utc::now() + chrono::Duration::days(1);

        Self {
            voting_identities,
            user_identities,
            contested_names,
            local_dpns_names,
            scheduled_votes: scheduled_votes_with_status,
            selected_votes: Vec::new(),
            app_context: app_context.clone(),
            sort_column: SortColumn::ContestedName,
            sort_order: SortOrder::Ascending,
            active_filter_term: String::new(),
            past_filter_term: String::new(),
            owned_filter_term: String::new(),
            scheduled_vote_cast_in_progress: false,
            pending_backend_task: None,
            vote_operations,
            vote_state,
            vote_overlay: None,
            pending_vote_operation: None,
            clear_vote_overlay_on_error: false,
            scheduled_clear_dialog: None,
            dpns_subscreen,
            refreshing_status: RefreshingStatus::NotRefreshing,
            refresh_banner: None,

            // Vote handling
            show_bulk_schedule_popup: false,
            bulk_identity_options,
            bulk_vote_handling_status: VoteHandlingStatus::NotStarted,
            set_all_option: VoteOption::CastNow,
            simple_schedule_date: default_schedule_time.format("%Y-%m-%d").to_string(),
            simple_schedule_hour: default_schedule_time.hour(),
            simple_schedule_minute: default_schedule_time.minute(),
        }
    }

    // ---------------------------
    // Sorting
    // ---------------------------
    fn toggle_sort(&mut self, column: SortColumn) {
        if self.sort_column == column {
            self.sort_order = match self.sort_order {
                SortOrder::Ascending => SortOrder::Descending,
                SortOrder::Descending => SortOrder::Ascending,
            };
        } else {
            self.sort_column = column;
            self.sort_order = SortOrder::Ascending;
        }
    }

    fn sort_contested_names(&self, contested_names: &mut [ContestedName]) {
        contested_names.sort_by(|a, b| {
            let order = match self.sort_column {
                SortColumn::ContestedName => a
                    .normalized_contested_name
                    .cmp(&b.normalized_contested_name),
                SortColumn::EndingTime => a.end_time.cmp(&b.end_time),
                SortColumn::LastUpdated => a.last_updated.cmp(&b.last_updated),
                SortColumn::AwardedTo => a.awarded_to.cmp(&b.awarded_to),
            };
            if self.sort_order == SortOrder::Descending {
                order.reverse()
            } else {
                order
            }
        });
    }

    // ---------------------------
    // Rendering: Empty states
    // ---------------------------
    fn render_no_voting_nodes(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_max_width(520.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(NO_VOTING_NODES_MESSAGE)
                            .strong()
                            .color(DashColors::warning_color(dark_mode)),
                    );
                    ui.label(
                        RichText::new(NO_VOTING_NODES_DETAIL)
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(12.0);
                    if ComponentStyles::add_primary_button(ui, "Load a masternode").clicked() {
                        action = AppAction::SetMainScreen(RootScreenType::RootScreenMasternodes);
                    }
                });
            });
        });
        action
    }

    fn render_no_active_contests_or_owned_names(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            match self.dpns_subscreen {
                DPNSSubscreen::Active => {
                    ui.label(
                        egui::RichText::new("No active contests at the moment.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                DPNSSubscreen::Past => {
                    ui.label(
                        egui::RichText::new("No active or past contests at the moment.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                DPNSSubscreen::Owned => {
                    ui.label(
                        egui::RichText::new("No owned usernames.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                DPNSSubscreen::ScheduledVotes => {
                    ui.label(
                        egui::RichText::new("No scheduled votes.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
            }
            ui.add_space(10.0);

            if self.dpns_subscreen != DPNSSubscreen::ScheduledVotes {
                let dark_mode = ui.style().visuals.dark_mode;
                ui.label(RichText::new("Please check back later or try refreshing the list.").color(DashColors::text_primary(dark_mode)));
                ui.add_space(20.0);
                if StyledButton::primary("Refresh").show(ui).clicked() {
                    if let RefreshingStatus::Refreshing = self.refreshing_status {
                        app_action = AppAction::None;
                    } else {
                        self.refreshing_status = RefreshingStatus::Refreshing;
                        match self.dpns_subscreen {
                            DPNSSubscreen::Active | DPNSSubscreen::Past => {
                                app_action = AppAction::BackendTask(BackendTask::ContestedResourceTask(
                                    ContestedResourceTask::QueryDPNSContests,
                                ));
                            }
                            DPNSSubscreen::Owned => {
                                app_action = AppAction::BackendTask(BackendTask::IdentityTask(
                                    IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
                                ));
                            }
                            _ => {
                                app_action = AppAction::Refresh;
                            }
                        }
                    }
                }
            } else {
                let dark_mode = ui.style().visuals.dark_mode;
                let text_color = DashColors::text_primary(dark_mode);
                ui.label(
                    RichText::new("Choose votes on the Active contests screen, then use Review and cast to schedule them.").color(text_color)
                );
            }
        });

        app_action
    }

    // ---------------------------
    // Rendering: Active, Past, Owned, Scheduled
    // ---------------------------

    /// Render active contests as decision cards grouped by what the loaded nodes can do.
    fn render_active_contests(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Filter by name:").color(DashColors::text_primary(dark_mode)));
            ui.text_edit_singleline(&mut self.active_filter_term)
                .on_hover_text(
                    "The letters i and l match the digit 1, and the letter o matches 0.",
                );
        });
        ui.label(
            RichText::new("Each node can change its vote up to four times after its initial vote.")
                .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(8.0);

        let filter = normalize_dpns_label(&self.active_filter_term);
        let contests = self
            .contested_names
            .lock_recover()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let mut groups = [Vec::new(), Vec::new(), Vec::new()];
        for contest in contests {
            let index = match self.contest_group(&contest) {
                ActiveContestGroup::NeedsVote => 0,
                ActiveContestGroup::Voted => 1,
                ActiveContestGroup::NotVotable => 2,
            };
            groups[index].push(contest);
        }

        let closes_within_day = groups[0]
            .iter()
            .filter(|contest| {
                contest.end_time.is_some_and(|end_time| {
                    let remaining = end_time as i64 - Utc::now().timestamp_millis();
                    remaining > 0 && remaining <= chrono::Duration::days(1).num_milliseconds()
                })
            })
            .count();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(format!(
                "Names still available to your nodes: {available}. Names closing within 24 hours: {closing}.",
                available = groups[0].len(),
                closing = closes_within_day,
            ));
        });
        ui.add_space(8.0);

        if !filter.is_empty() {
            for group in &mut groups {
                group.retain(|contest| {
                    contest
                        .normalized_contested_name
                        .to_lowercase()
                        .contains(&filter)
                });
            }
        }

        egui::ScrollArea::vertical()
            .id_salt("active_contest_cards")
            .show(ui, |ui| {
                self.render_contest_group(ui, "Needs your vote", &groups[0], true, true, false);
                self.render_contest_group(ui, "Voted", &groups[1], false, true, true);
                self.render_contest_group(
                    ui,
                    "Not votable by your nodes",
                    &groups[2],
                    false,
                    false,
                    false,
                );
                self.render_voting_activity(ui);
            });

        ui.separator();
        ui.horizontal(|ui| {
            let count = self.selected_votes.len();
            ui.label(
                RichText::new(format!("Votes ready to cast: {count}"))
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ComponentStyles::add_primary_button_enabled(ui, count > 0, "Review and cast")
                    .disabled_tooltip("Choose a vote on at least one contest before reviewing it.")
                    .clicked()
                {
                    self.show_bulk_schedule_popup = true;
                }
            });
        });
    }

    fn contest_group(&self, contest: &ContestedName) -> ActiveContestGroup {
        let Ok(poll_id) = self
            .app_context
            .dpns_vote_poll_id(&contest.normalized_contested_name)
        else {
            return ActiveContestGroup::NotVotable;
        };
        classify_vote_states(
            self.voting_identities
                .iter()
                .map(|identity| self.vote_state.state(identity.identity.id(), poll_id)),
        )
    }

    fn render_contest_group(
        &mut self,
        ui: &mut Ui,
        title: &str,
        contests: &[ContestedName],
        default_open: bool,
        voting_enabled: bool,
        show_current_vote: bool,
    ) {
        egui::CollapsingHeader::new(format!("{title} ({})", contests.len()))
            .default_open(default_open)
            .show(ui, |ui| {
                if contests.is_empty() {
                    ui.label("There are no contests in this group.");
                }
                for contest in contests {
                    let contest_enabled =
                        voting_enabled && self.contest_has_available_target(contest);
                    ui.add_enabled_ui(contest_enabled, |ui| {
                        self.render_contest_card(ui, contest, contest_enabled, show_current_vote);
                    });
                    ui.add_space(8.0);
                }
            });
    }

    fn contest_has_available_target(&self, contest: &ContestedName) -> bool {
        let Ok(vote_poll_id) = self
            .app_context
            .dpns_vote_poll_id(&contest.normalized_contested_name)
        else {
            return false;
        };
        self.voting_identities.iter().any(|identity| {
            let voter_id = identity.identity.id();
            matches!(
                self.vote_state.state(voter_id, vote_poll_id),
                DpnsCurrentVoteState::Available(_)
            ) && self
                .vote_operations
                .target_status(&DpnsVoteTargetKey {
                    network: self.app_context.network(),
                    voter_id,
                    vote_poll_id,
                })
                .is_none()
        })
    }

    fn proved_vote_for_contest(&self, contest: &ContestedName) -> ProvedVoteSummary {
        let Ok(poll_id) = self
            .app_context
            .dpns_vote_poll_id(&contest.normalized_contested_name)
        else {
            return ProvedVoteSummary::None;
        };
        proved_vote_summary(
            self.voting_identities
                .iter()
                .map(|identity| self.vote_state.state(identity.identity.id(), poll_id)),
        )
    }

    fn candidate_name_in_contest(
        contest: &ContestedName,
        choice: ResourceVoteChoice,
    ) -> Option<&str> {
        let ResourceVoteChoice::TowardsIdentity(candidate_id) = choice else {
            return None;
        };
        contest
            .contestants
            .as_ref()?
            .iter()
            .find(|candidate| candidate.id == candidate_id)
            .map(|candidate| candidate.name.as_str())
    }

    fn render_contest_card(
        &mut self,
        ui: &mut Ui,
        contest: &ContestedName,
        voting_enabled: bool,
        show_current_vote: bool,
    ) {
        let dark_mode = ui.style().visuals.dark_mode;
        let staged = self
            .selected_votes
            .iter()
            .find(|vote| vote.contested_name == contest.normalized_contested_name)
            .map(|vote| vote.vote_choice);
        let proved = if show_current_vote {
            self.proved_vote_for_contest(contest)
        } else {
            ProvedVoteSummary::None
        };
        let selected = staged.or(match proved {
            ProvedVoteSummary::Choice(choice) => Some(choice),
            ProvedVoteSummary::None | ProvedVoteSummary::Mixed => None,
        });
        let locked_votes = contest.locked_votes.unwrap_or_default();
        let abstain_votes = contest.abstain_votes.unwrap_or_default();

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}.dash", contest.normalized_contested_name))
                        .heading()
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                if let Some(end_time) = contest.end_time
                    && let LocalResult::Single(date_time) =
                        Utc.timestamp_millis_opt(end_time as i64)
                {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("Voting ends {}.", HumanTime::from(date_time)))
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                    });
                }
            });
            ui.add_space(6.0);

            match proved {
                ProvedVoteSummary::Choice(choice) => {
                    let candidate_name = Self::candidate_name_in_contest(contest, choice);
                    ui.label(
                        RichText::new(format!(
                            "You voted: {}.",
                            vote_choice_label(choice, candidate_name)
                        ))
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(4.0);
                }
                ProvedVoteSummary::Mixed => {
                    ui.label(
                        RichText::new("Your loaded nodes have different current votes.")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(4.0);
                }
                ProvedVoteSummary::None => {}
            }

            ui.horizontal_wrapped(|ui| {
                let clicked = ui
                    .selectable_label(selected == Some(ResourceVoteChoice::Lock), "Lock name")
                    .on_disabled_hover_text(
                        "None of your loaded nodes can vote on this contest right now.",
                    )
                    .clicked();
                tally_chip(ui, locked_votes, dark_mode);
                if clicked && voting_enabled {
                    self.set_selected_vote(contest, ResourceVoteChoice::Lock);
                }

                let clicked = ui
                    .selectable_label(selected == Some(ResourceVoteChoice::Abstain), "Abstain")
                    .clicked();
                tally_chip(ui, abstain_votes, dark_mode);
                if clicked && voting_enabled {
                    self.set_selected_vote(contest, ResourceVoteChoice::Abstain);
                }
            });

            if let Some(contestants) = &contest.contestants {
                for contestant in contestants {
                    ui.horizontal_wrapped(|ui| {
                        let choice = ResourceVoteChoice::TowardsIdentity(contestant.id);
                        let clicked = ui
                            .selectable_label(
                                selected == Some(choice),
                                candidate_choice_label(&contestant.name),
                            )
                            .clicked();
                        tally_chip(ui, contestant.votes, dark_mode);
                        ui.label(
                            RichText::new(short_identifier(contestant.id))
                                .monospace()
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        if clicked && voting_enabled {
                            self.set_selected_vote(contest, choice);
                        }
                    });
                }
            }
        });
    }

    fn set_selected_vote(&mut self, contest: &ContestedName, choice: ResourceVoteChoice) {
        if let Some(index) = self
            .selected_votes
            .iter()
            .position(|vote| vote.contested_name == contest.normalized_contested_name)
        {
            if self.selected_votes[index].vote_choice == choice {
                self.selected_votes.remove(index);
            } else {
                self.selected_votes[index].vote_choice = choice;
            }
        } else {
            self.selected_votes.push(SelectedVote {
                contested_name: contest.normalized_contested_name.clone(),
                vote_choice: choice,
                end_time: contest.end_time,
            });
        }
    }

    fn render_voting_activity(&mut self, ui: &mut Ui) {
        let mut operations = self.vote_operations.operations().to_vec();
        operations.sort_by_key(|operation| operation.created_at);
        let operations = operations
            .into_iter()
            .rev()
            .filter(|operation| !operation.targets.is_empty())
            .take(5)
            .collect::<Vec<_>>();
        if operations.is_empty() {
            return;
        }

        let dark_mode = ui.style().visuals.dark_mode;
        ui.add_space(12.0);
        ui.heading("Recent voting activity");
        for operation in operations {
            let settled = operation
                .targets
                .iter()
                .filter(|outcome| !outcome.status.holds_lock())
                .count();
            ui.group(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{settled} of {} votes settled.",
                        operation.targets.len()
                    ))
                    .strong(),
                );
                for outcome in &operation.targets {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!(
                            "{}.dash — {} — {}",
                            outcome.target.contested_name,
                            vote_choice_label(
                                outcome.target.requested_choice,
                                self.candidate_name(
                                    &outcome.target.contested_name,
                                    outcome.target.requested_choice,
                                )
                                .as_deref(),
                            ),
                            target_status_label(outcome.status),
                        ));
                        if outcome.status == DpnsVoteTargetStatus::Unconfirmed
                            && ComponentStyles::add_secondary_button(
                                ui,
                                "Check again",
                                dark_mode,
                            )
                            .clicked()
                        {
                            self.pending_backend_task = Some(BackendTask::ContestedResourceTask(
                                ContestedResourceTask::ReconcileDpnsVoteOperation(
                                    operation.id,
                                    self.app_context.network(),
                                ),
                            ));
                            self.pending_vote_operation = Some(operation.id);
                            self.raise_vote_overlay(
                                ui.ctx(),
                                "Checking the submitted votes with Platform…",
                            );
                        }
                        if matches!(
                            outcome.status,
                            DpnsVoteTargetStatus::Rejected
                                | DpnsVoteTargetStatus::FailedBeforeSubmission
                                | DpnsVoteTargetStatus::NotApplied
                        ) && ComponentStyles::add_secondary_button(
                            ui,
                            "Review again",
                            dark_mode,
                        )
                        .clicked()
                        {
                            self.set_selected_vote_from_outcome(outcome);
                            self.bulk_identity_options =
                                vec![VoteOption::CastNow; self.voting_identities.len()];
                            self.set_all_option = VoteOption::CastNow;
                            self.show_bulk_schedule_popup = true;
                        }
                    });
                    if matches!(
                        outcome.status,
                        DpnsVoteTargetStatus::Confirming
                            | DpnsVoteTargetStatus::Unconfirmed
                    ) {
                        ui.label(
                            RichText::new(
                                "This vote may already have been submitted. Do not submit it again.",
                            )
                            .color(DashColors::warning_color(dark_mode)),
                        );
                    }
                }
            });
        }
    }

    fn set_selected_vote_from_outcome(
        &mut self,
        outcome: &crate::model::dpns_voting::DpnsVoteOutcome,
    ) {
        let name = outcome.target.contested_name.clone();
        if let Some(vote) = self
            .selected_votes
            .iter_mut()
            .find(|vote| vote.contested_name == name)
        {
            vote.vote_choice = outcome.target.requested_choice;
        } else {
            self.selected_votes.push(SelectedVote {
                contested_name: name,
                vote_choice: outcome.target.requested_choice,
                end_time: None,
            });
        }
    }

    fn raise_vote_overlay(&mut self, ctx: &egui::Context, message: &str) {
        self.vote_overlay
            .raise(ctx, message, OverlayConfig::default());
    }
    fn render_table_past_contests(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let dark_mode = ui.style().visuals.dark_mode;
            ui.label(RichText::new("Filter by name:").color(DashColors::text_primary(dark_mode)));
            ui.text_edit_singleline(&mut self.past_filter_term)
                .on_hover_text(
                    "The letters i and l match the digit 1, and the letter o matches 0.",
                );
        });

        let contested_names = {
            let guard = self.contested_names.lock_recover();
            let mut cn = guard.clone();
            cn.retain(|c| c.awarded_to.is_some() || c.state == ContestState::Locked);
            if !self.past_filter_term.is_empty() {
                let filter = normalize_dpns_label(&self.past_filter_term);

                cn.retain(|c| c.normalized_contested_name.to_lowercase().contains(&filter));
            }
            self.sort_contested_names(&mut cn);
            cn
        };

        // Allocate space for refreshing indicator
        // Space allocation for UI elements is handled by the layout system

        egui::ScrollArea::both().show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(false)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().resizable(true)) // Name
                .column(Column::auto().resizable(true)) // Ended Time
                .column(Column::auto().resizable(true)) // Last Updated
                .column(Column::auto().resizable(true)) // Awarded To
                .header(30.0, |mut header| {
                    header.col(|ui| {
                        if ui.button("Name").clicked() {
                            self.toggle_sort(SortColumn::ContestedName);
                        }
                    });
                    header.col(|ui| {
                        if ui.button("Ended Time").clicked() {
                            self.toggle_sort(SortColumn::EndingTime);
                        }
                    });
                    header.col(|ui| {
                        if ui.button("Last Updated").clicked() {
                            self.toggle_sort(SortColumn::LastUpdated);
                        }
                    });
                    header.col(|ui| {
                        if ui.button("Outcome").clicked() {
                            self.toggle_sort(SortColumn::AwardedTo);
                        }
                    });
                })
                .body(|mut body| {
                    for contested_name in &contested_names {
                        body.row(25.0, |mut row| {
                            // Name
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.label(
                                    RichText::new(&contested_name.normalized_contested_name)
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            });
                            // Ended Time
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                if let Some(ended_time) = contested_name.end_time {
                                    if let LocalResult::Single(dt) =
                                        Utc.timestamp_millis_opt(ended_time as i64)
                                    {
                                        let iso = dt.format("%Y-%m-%d %H:%M:%S").to_string();
                                        let relative = HumanTime::from(dt).to_string();
                                        ui.label(
                                            RichText::new(format!("{} ({})", iso, relative))
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new("Invalid timestamp")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                } else {
                                    ui.label(
                                        RichText::new("Fetching")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                }
                            });
                            // Last Updated
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                if let Some(last_updated) = contested_name.last_updated {
                                    if let LocalResult::Single(dt) =
                                        Utc.timestamp_opt(last_updated as i64, 0)
                                    {
                                        let rel = HumanTime::from(dt).to_string();
                                        if rel.contains("seconds") {
                                            ui.label(
                                                RichText::new("now")
                                                    .color(DashColors::text_primary(dark_mode)),
                                            );
                                        } else {
                                            ui.label(
                                                RichText::new(rel)
                                                    .color(DashColors::text_primary(dark_mode)),
                                            );
                                        }
                                    } else {
                                        ui.label(
                                            RichText::new("Invalid timestamp")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                } else {
                                    ui.label(
                                        RichText::new("Fetching")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                }
                            });
                            // Awarded To
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                match contested_name.state {
                                    ContestState::Unknown => {
                                        ui.label(
                                            RichText::new("Fetching")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                    ContestState::Joinable | ContestState::Ongoing => {
                                        ui.label(
                                            RichText::new("Active")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                    ContestState::WonBy(identifier) => {
                                        let winner = contested_name
                                            .contestants
                                            .as_ref()
                                            .and_then(|contestants| {
                                                contestants
                                                    .iter()
                                                    .find(|contestant| contestant.id == identifier)
                                            })
                                            .map(|contestant| contestant.name.clone())
                                            .unwrap_or_else(|| short_identifier(identifier));
                                        ui.label(format!("Won by {winner}."));
                                    }
                                    ContestState::Locked => {
                                        ui.label(
                                            RichText::new("Locked; no name was awarded.")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                }
                            });
                        });
                    }
                });
        });
    }

    /// Show the Owned DPNS names table
    fn render_table_local_dpns_names(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let dark_mode = ui.style().visuals.dark_mode;
            ui.label(RichText::new("Filter by name:").color(DashColors::text_primary(dark_mode)));
            ui.text_edit_singleline(&mut self.owned_filter_term);
        });

        let mut filtered_names = {
            let guard = self.local_dpns_names.lock_recover();
            let mut name_infos = guard.clone();
            if !self.owned_filter_term.is_empty() {
                let filter_lc = self.owned_filter_term.to_lowercase();
                name_infos.retain(|c| c.1.name.to_lowercase().contains(&filter_lc));
            }
            name_infos
        };
        // Sort
        filtered_names.sort_by(|a, b| match self.sort_column {
            SortColumn::ContestedName => {
                let order = a.1.name.cmp(&b.1.name);
                if self.sort_order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                }
            }
            SortColumn::AwardedTo => {
                let order = a.0.cmp(&b.0);
                if self.sort_order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                }
            }
            SortColumn::EndingTime => {
                let order = a.1.acquired_at.cmp(&b.1.acquired_at);
                if self.sort_order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                }
            }
            _ => std::cmp::Ordering::Equal,
        });

        // Space allocation for UI elements is handled by the layout system

        egui::ScrollArea::both().show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(false)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().resizable(true)) // DPNS Name
                .column(Column::auto().resizable(true)) // Owner ID
                .column(Column::auto().resizable(true)) // Acquired At
                .column(Column::auto().resizable(true)) // Actions
                .header(30.0, |mut header| {
                    header.col(|ui| {
                        if ui.button("Name").clicked() {
                            self.toggle_sort(SortColumn::ContestedName);
                        }
                    });
                    header.col(|ui| {
                        if ui.button("Owner ID").clicked() {
                            self.toggle_sort(SortColumn::AwardedTo);
                        }
                    });
                    header.col(|ui| {
                        if ui.button("Acquired At").clicked() {
                            self.toggle_sort(SortColumn::EndingTime);
                        }
                    });
                    header.col(|ui| {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.label(
                            RichText::new("Actions").color(DashColors::text_primary(dark_mode)),
                        );
                    });
                })
                .body(|mut body| {
                    for (identifier, dpns_info) in filtered_names {
                        let name_for_alias = dpns_info.name.clone();
                        // Display name with .dash suffix
                        let display_name = if name_for_alias.ends_with(".dash") {
                            name_for_alias.clone()
                        } else {
                            format!("{}.dash", name_for_alias)
                        };
                        body.row(25.0, |mut row| {
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.label(
                                    RichText::new(&display_name)
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            });
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.label(
                                    RichText::new(identifier.to_string(Encoding::Base58))
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            });
                            let dt = DateTime::from_timestamp(
                                dpns_info.acquired_at as i64 / 1000,
                                ((dpns_info.acquired_at % 1000) * 1_000_000) as u32,
                            )
                            .map(|dt| dt.to_string())
                            .unwrap_or_else(|| "Invalid timestamp".to_string());
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.label(
                                    RichText::new(dt).color(DashColors::text_primary(dark_mode)),
                                );
                            });
                            row.col(|ui| {
                                if ui.small_button("Set Alias").clicked() {
                                    // Append .dash suffix for DPNS names
                                    let alias_with_suffix = if name_for_alias.ends_with(".dash") {
                                        name_for_alias.clone()
                                    } else {
                                        format!("{}.dash", name_for_alias)
                                    };
                                    if let Err(e) = self
                                        .app_context
                                        .set_identity_alias(&identifier, Some(&alias_with_suffix))
                                    {
                                        MessageBanner::set_global(
                                            ui.ctx(),
                                        "The alias could not be saved. Check available disk space and try again.",
                                        MessageType::Error,
                                        )
                                        .with_details(e);
                                    } else {
                                        MessageBanner::set_global(
                                            ui.ctx(),
                                            format!(
                                            "Alias set to '{alias}' for identity {identity_id}",
                                            alias = alias_with_suffix,
                                            identity_id = identifier.to_string(Encoding::Base58)
                                            ),
                                            MessageType::Success,
                                        );
                                    }
                                }
                            });
                        });
                    }
                });
        });
    }

    /// Show the Scheduled Votes table
    fn render_table_scheduled_votes(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut show_cast_overlay = false;
        let mut sorted_votes = {
            let guard = self.scheduled_votes.lock_recover();
            guard.clone()
        };
        // Sort by contested_name or time
        sorted_votes.sort_by(|a, b| {
            let order = a.0.contested_name.cmp(&b.0.contested_name);
            if self.sort_order == SortOrder::Descending {
                order.reverse()
            } else {
                order
            }
        });

        egui::ScrollArea::both().show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(false)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().resizable(true)) // ContestedName
                .column(Column::auto().resizable(true)) // Voter
                .column(Column::auto().resizable(true)) // Choice
                .column(Column::auto().resizable(true)) // Time
                .column(Column::auto().resizable(true)) // Status
                .column(Column::auto().resizable(true)) // Actions
                .header(30.0, |mut header| {
                    header.col(|ui| {
                        if ui.button("Name").clicked() {
                            self.toggle_sort(SortColumn::ContestedName);
                        }
                    });
                    header.col(|ui| {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.heading(
                            RichText::new("Voter").color(DashColors::text_primary(dark_mode)),
                        );
                    });
                    header.col(|ui| {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.heading(
                            RichText::new("Vote Choice").color(DashColors::text_primary(dark_mode)),
                        );
                    });
                    header.col(|ui| {
                        if ui.button("Scheduled Time").clicked() {
                            self.toggle_sort(SortColumn::EndingTime);
                        }
                    });
                    header.col(|ui| {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.heading(
                            RichText::new("Status").color(DashColors::text_primary(dark_mode)),
                        );
                    });
                    header.col(|ui| {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.heading(
                            RichText::new("Actions").color(DashColors::text_primary(dark_mode)),
                        );
                    });
                })
                .body(|mut body| {
                    for vote in sorted_votes.iter_mut() {
                        let operation_status = self
                            .app_context
                            .dpns_vote_poll_id(&vote.0.contested_name)
                            .ok()
                            .and_then(|vote_poll_id| {
                                self.vote_operations
                                    .target_status(&DpnsVoteTargetKey {
                                        network: self.app_context.network(),
                                        voter_id: vote.0.voter_id,
                                        vote_poll_id,
                                    })
                            });
                        body.row(25.0, |mut row| {
                            // Contested name
                            row.col(|ui| {
                                ui.add(Label::new(format!("{}.dash", vote.0.contested_name)));
                            });
                            // Voter
                            row.col(|ui| {
                                let voter = self
                                    .voting_identities
                                    .iter()
                                    .find(|identity| identity.identity.id() == vote.0.voter_id)
                                    .and_then(|identity| identity.alias.clone())
                                    .unwrap_or_else(|| short_identifier(vote.0.voter_id));
                                ui.add(Label::new(voter));
                            });
                            // Choice
                            row.col(|ui| {
                                let candidate_name =
                                    self.candidate_name(&vote.0.contested_name, vote.0.choice);
                                let display_text =
                                    vote_choice_label(vote.0.choice, candidate_name.as_deref());
                                ui.add(Label::new(display_text));
                            });
                            // Time
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                if let LocalResult::Single(dt) =
                                    Utc.timestamp_millis_opt(vote.0.unix_timestamp as i64)
                                {
                                    let iso = dt.format("%Y-%m-%d %H:%M:%S").to_string();
                                    let rel_time = HumanTime::from(dt).to_string();
                                    let relative = if rel_time.contains("seconds") {
                                        "now".to_string()
                                    } else {
                                        rel_time
                                    };
                                    let text = format!("{} ({})", iso, relative);
                                    ui.label(
                                        RichText::new(text)
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                } else {
                                    ui.label(
                                        RichText::new("Invalid timestamp")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                }
                            });
                            // Status
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                if matches!(
                                    operation_status,
                                    Some(DpnsVoteTargetStatus::Queued)
                                        | Some(DpnsVoteTargetStatus::Submitting)
                                        | Some(DpnsVoteTargetStatus::Confirming)
                                ) {
                                    ui.label(
                                        RichText::new("Submitting…")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                    return;
                                }
                                if operation_status == Some(DpnsVoteTargetStatus::Unconfirmed) {
                                    ui.colored_label(
                                        DashColors::warning_color(dark_mode),
                                        "Checking result",
                                    );
                                    return;
                                }
                                match vote.1 {
                                    ScheduledVoteCastingStatus::NotStarted => {
                                        ui.label(
                                            RichText::new("Pending")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                    ScheduledVoteCastingStatus::InProgress => {
                                        ui.label(
                                            RichText::new("Casting...")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                    ScheduledVoteCastingStatus::Failed => {
                                        ui.colored_label(
                                            DashColors::error_color(dark_mode),
                                            "Failed",
                                        );
                                    }
                                    ScheduledVoteCastingStatus::Completed => {
                                        ui.colored_label(
                                            DashColors::success_color(dark_mode),
                                            "Cast",
                                        );
                                    }
                                }
                            });
                            // Actions
                            row.col(|ui| {
                                let target_is_busy = matches!(
                                    operation_status,
                                    Some(DpnsVoteTargetStatus::Queued)
                                        | Some(DpnsVoteTargetStatus::Submitting)
                                        | Some(DpnsVoteTargetStatus::Confirming)
                                        | Some(DpnsVoteTargetStatus::Unconfirmed)
                                );
                                if ui
                                    .add_enabled(!target_is_busy, Button::new("Remove"))
                                    .disabled_tooltip(
                                        "This scheduled vote cannot be removed while its result is being checked.",
                                    )
                                    .clicked()
                                {
                                    action =
                                        AppAction::BackendTask(BackendTask::ContestedResourceTask(
                                            ContestedResourceTask::DeleteScheduledVote(
                                                vote.0.voter_id,
                                                vote.0.contested_name.clone(),
                                            ),
                                        ));
                                }
                                // If the user wants to do "Cast Now" from here, they can
                                // if NotStarted or Failed. If in progress or done, disabled.
                                let cast_button_enabled = matches!(
                                    vote.1,
                                    ScheduledVoteCastingStatus::NotStarted
                                        | ScheduledVoteCastingStatus::Failed
                                ) && !target_is_busy;

                                let cast_button = if cast_button_enabled {
                                    Button::new("Cast now")
                                } else {
                                    Button::new("Cast now").sense(egui::Sense::hover())
                                };

                                if ui.add(cast_button).clicked() && cast_button_enabled {
                                    self.scheduled_vote_cast_in_progress = true;
                                    vote.1 = ScheduledVoteCastingStatus::InProgress;

                                    // Mark in our Arc as well
                                    if let Ok(mut sched_guard) = self.scheduled_votes.lock()
                                        && let Some(t) = sched_guard.iter_mut().find(|(sv, _)| {
                                            sv.voter_id == vote.0.voter_id
                                                && sv.contested_name == vote.0.contested_name
                                        })
                                    {
                                        t.1 = ScheduledVoteCastingStatus::InProgress;
                                    }
                                    // dispatch the actual cast
                                    let local_ids =
                                        match self.app_context.load_local_voting_identities() {
                                            Ok(ids) => ids,
                                            Err(e) => {
                                                error!("{}", e);
                                                return;
                                            }
                                        };
                                    if let Some(found) = local_ids
                                        .iter()
                                        .find(|i| i.identity.id() == vote.0.voter_id)
                                    {
                                    action = AppAction::BackendTask(
                                            BackendTask::ContestedResourceTask(
                                                ContestedResourceTask::CastScheduledVote(
                                                    vote.0.clone(),
                                                    Box::new(found.clone()),
                                                ),
                                            ),
                                        );
                                        show_cast_overlay = true;
                                    }
                                }
                            });
                        });
                    }
                });
        });

        if show_cast_overlay {
            self.raise_vote_overlay(ui.ctx(), "Submitting the scheduled vote to Dash Platform…");
        }

        action
    }

    fn simple_schedule_option(&self) -> Option<VoteOption> {
        let date = NaiveDate::parse_from_str(&self.simple_schedule_date, "%Y-%m-%d").ok()?;
        let time =
            NaiveTime::from_hms_opt(self.simple_schedule_hour, self.simple_schedule_minute, 0)?;
        let scheduled_at = DateTime::<Utc>::from_naive_utc_and_offset(date.and_time(time), Utc);
        let seconds = scheduled_at.signed_duration_since(Utc::now()).num_seconds();
        if seconds <= 0 {
            return None;
        }
        let total_minutes = u32::try_from((seconds + 59) / 60).ok()?;
        Some(VoteOption::Scheduled {
            days: total_minutes / (24 * 60),
            hours: total_minutes / 60 % 24,
            minutes: total_minutes % 60,
        })
    }

    fn apply_all_nodes_option(&mut self, option: VoteOption) {
        self.set_all_option = option.clone();
        self.bulk_identity_options.fill(option);
    }

    fn candidate_name(&self, contested_name: &str, choice: ResourceVoteChoice) -> Option<String> {
        let ResourceVoteChoice::TowardsIdentity(candidate_id) = choice else {
            return None;
        };
        self.contested_names
            .lock_recover()
            .iter()
            .find(|contest| contest.normalized_contested_name == contested_name)
            .and_then(|contest| contest.contestants.as_ref())
            .and_then(|contestants| {
                contestants
                    .iter()
                    .find(|candidate| candidate.id == candidate_id)
            })
            .map(|candidate| candidate.name.clone())
    }

    fn show_review_and_cast_window(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        let dark_mode = ui.style().visuals.dark_mode;

        if self.bulk_vote_handling_status == VoteHandlingStatus::Completed {
            action |= self.show_bulk_vote_handling_complete(ui);
            return action;
        }

        if self.voting_identities.is_empty() {
            ui.add_space(5.0);
            ui.colored_label(
                DashColors::warning_color(dark_mode),
                NO_VOTING_NODES_MESSAGE,
            );
            ui.label(NO_VOTING_NODES_DETAIL);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ComponentStyles::add_primary_button(ui, "Load a masternode").clicked() {
                    action = AppAction::SetMainScreen(RootScreenType::RootScreenMasternodes);
                    self.show_bulk_schedule_popup = false;
                }
                if ComponentStyles::add_secondary_button(ui, "Close", dark_mode).clicked() {
                    self.show_bulk_schedule_popup = false;
                }
            });
            return action;
        }

        if self.selected_votes.is_empty() {
            ui.add_space(5.0);
            ui.colored_label(
                DashColors::warning_color(dark_mode),
                "No votes are ready to review. Choose at least one vote on Active contests, then try again.",
            );
            ui.add_space(10.0);
            if ComponentStyles::add_secondary_button(ui, "Back to Active contests", dark_mode)
                .clicked()
            {
                self.show_bulk_schedule_popup = false;
            }
            return action;
        }

        let mut simple_schedule_valid = true;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(format!(
                "Casting on behalf of all my nodes ({count}).",
                count = self.voting_identities.len()
            ));
            ui.separator();
            ui.heading(format!("Votes to cast ({}):", self.selected_votes.len()));
            for vote in &self.selected_votes {
                let candidate_name = self.candidate_name(&vote.contested_name, vote.vote_choice);
                let choice = review_vote_choice_label(vote.vote_choice, candidate_name.as_deref());
                ui.label(format!("• {name}.dash → {choice}", name = vote.contested_name));
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("When:");
                if ui
                    .selectable_label(
                        matches!(self.set_all_option, VoteOption::CastNow),
                        "Cast now",
                    )
                    .clicked()
                {
                    self.apply_all_nodes_option(VoteOption::CastNow);
                }
                if ui
                    .selectable_label(
                        matches!(self.set_all_option, VoteOption::Scheduled { .. }),
                        "Schedule for later",
                    )
                    .clicked()
                    && let Some(option) = self.simple_schedule_option()
                {
                    self.apply_all_nodes_option(option);
                }
            });

            if matches!(self.set_all_option, VoteOption::Scheduled { .. }) {
                let mut schedule_changed = false;
                ui.horizontal(|ui| {
                    ui.label("Cast on (UTC):");
                    schedule_changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut self.simple_schedule_date)
                                .desired_width(100.0)
                                .hint_text("YYYY-MM-DD"),
                        )
                        .changed();
                    ui.label("at");
                    schedule_changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.simple_schedule_hour)
                                .prefix("Hour: ")
                                .range(0..=23),
                        )
                        .changed();
                    schedule_changed |= ui
                        .add(
                            egui::DragValue::new(&mut self.simple_schedule_minute)
                                .prefix("Minute: ")
                                .range(0..=59),
                        )
                        .changed();
                });
                simple_schedule_valid = self.simple_schedule_option().is_some();
                if schedule_changed
                    && let Some(option) = self.simple_schedule_option()
                {
                    self.apply_all_nodes_option(option);
                }
                if !simple_schedule_valid {
                    ui.colored_label(
                        DashColors::warning_color(dark_mode),
                        "Choose a future date and time before scheduling these votes.",
                    );
                }
                ui.colored_label(
                    DashColors::warning_color(dark_mode),
                    "Keep Dash Evo Tool running and connected until then, or the scheduled votes will not be cast.",
                );
            }

            ui.separator();
            ui.add_space(10.0);
            egui::CollapsingHeader::new("Choose per node (advanced)")
                .default_open(false)
                .show(ui, |ui| {
                    for (i, identity) in self.voting_identities.iter().enumerate() {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let label = identity.alias.clone().unwrap_or_else(|| {
                                    identity.identity.id().to_string(Encoding::Base58)
                                });
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.label(
                                    RichText::new(format!("Identity: {}", label))
                                        .color(DashColors::text_primary(dark_mode)),
                                );

                                if self.bulk_identity_options.len() <= i {
                                    self.bulk_identity_options =
                                        vec![VoteOption::CastNow; self.voting_identities.len()];
                                }

                                let current_option = &mut self.bulk_identity_options[i];
                                ComboBox::from_id_salt(format!("combo_bulk_identity_{}", i))
                                    .width(120.0)
                                    .selected_text(match current_option {
                                        VoteOption::NoVote => "Do not use this node".to_string(),
                                        VoteOption::CastNow => "Cast now".to_string(),
                                        VoteOption::Scheduled { .. } => "Schedule".to_string(),
                                    })
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(
                                                matches!(current_option, VoteOption::NoVote),
                                                "Do not use this node",
                                            )
                                            .clicked()
                                        {
                                            *current_option = VoteOption::NoVote;
                                        }
                                        if ui
                                            .selectable_label(
                                                matches!(current_option, VoteOption::CastNow),
                                                "Cast now",
                                            )
                                            .clicked()
                                        {
                                            *current_option = VoteOption::CastNow;
                                        }
                                        if ui
                                            .selectable_label(
                                                matches!(
                                                    current_option,
                                                    VoteOption::Scheduled { .. }
                                                ),
                                                "Schedule",
                                            )
                                            .clicked()
                                        {
                                            let (d, h, m) = match current_option {
                                                VoteOption::Scheduled {
                                                    days,
                                                    hours,
                                                    minutes,
                                                } => (*days, *hours, *minutes),
                                                _ => (0, 0, 0),
                                            };
                                            *current_option = VoteOption::Scheduled {
                                                days: d,
                                                hours: h,
                                                minutes: m,
                                            };
                                        }
                                    });

                                if let VoteOption::Scheduled {
                                    days,
                                    hours,
                                    minutes,
                                } = current_option
                                {
                                    let dark_mode = ui.style().visuals.dark_mode;
                                    ui.label(
                                        RichText::new("Schedule after:")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                    ui.add(
                                        egui::DragValue::new(days).prefix("Days: ").range(0..=14),
                                    );
                                    ui.add(
                                        egui::DragValue::new(hours).prefix("Hours: ").range(0..=23),
                                    );
                                    ui.add(
                                        egui::DragValue::new(minutes).prefix("Min: ").range(0..=59),
                                    );
                                }
                            });
                        });
                        ui.add_space(10.0);
                    }
                });
        });
        // If any selected votes are scheduled, show a warning
        if !matches!(self.set_all_option, VoteOption::Scheduled { .. })
            && self
                .bulk_identity_options
                .iter()
                .any(|option| matches!(option, VoteOption::Scheduled { .. }))
        {
            ui.colored_label(
                DashColors::warning_color(ui.style().visuals.dark_mode),
                "Keep Dash Evo Tool running and connected until then, or the scheduled votes will not be cast.",
            );
            ui.add_space(10.0);
        }

        let operation_in_progress = matches!(
            self.bulk_vote_handling_status,
            VoteHandlingStatus::CastingVotes | VoteHandlingStatus::SchedulingVotes
        );
        if operation_in_progress {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Submitting votes…");
            });
        }
        let has_immediate = self
            .bulk_identity_options
            .iter()
            .any(|option| matches!(option, VoteOption::CastNow));
        let has_scheduled = self
            .bulk_identity_options
            .iter()
            .any(|option| matches!(option, VoteOption::Scheduled { .. }));
        let submit_label = match (has_immediate, has_scheduled) {
            (true, false) => "Cast votes",
            (false, true) => "Schedule votes",
            _ => "Submit votes",
        };
        let (submit_clicked, cancel_clicked) = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let submit_clicked = ComponentStyles::add_primary_button_enabled(
                    ui,
                    !operation_in_progress && simple_schedule_valid,
                    if operation_in_progress {
                        "Submitting votes…"
                    } else {
                        submit_label
                    },
                )
                .disabled_tooltip(if operation_in_progress {
                    "The selected votes are already being submitted."
                } else {
                    "Choose a future date and time before scheduling these votes."
                })
                .clicked();
                let cancel_clicked = ui
                    .add_enabled(
                        !operation_in_progress,
                        ComponentStyles::secondary_button("Cancel", dark_mode),
                    )
                    .disabled_tooltip("Submitted votes cannot be cancelled.")
                    .clicked();
                (submit_clicked, cancel_clicked)
            })
            .inner;
        if submit_clicked {
            action = self.bulk_apply_votes();
            if matches!(
                self.bulk_vote_handling_status,
                VoteHandlingStatus::CastingVotes | VoteHandlingStatus::SchedulingVotes
            ) {
                self.raise_vote_overlay(
                    ui.ctx(),
                    "Submitting the selected votes to Dash Platform…",
                );
            }
        }

        if cancel_clicked {
            self.selected_votes.clear();
            self.show_bulk_schedule_popup = false;
            self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
            self.vote_overlay.take_and_clear();
            self.pending_vote_operation = None;
        }

        // Handle status
        ui.add_space(10.0);
        match &self.bulk_vote_handling_status {
            VoteHandlingStatus::NotStarted => {}
            VoteHandlingStatus::CastingVotes => {
                // Elapsed time is shown in the global banner
            }
            VoteHandlingStatus::SchedulingVotes => {
                let dark_mode = ui.style().visuals.dark_mode;
                ui.label(
                    RichText::new("Scheduling votes...").color(DashColors::text_primary(dark_mode)),
                );
            }
            VoteHandlingStatus::Completed => {
                // handled above
            }
            VoteHandlingStatus::Failed(message) => {
                ui.colored_label(Color32::RED, message);
            }
        }

        action
    }

    fn bulk_apply_votes(&mut self) -> AppAction {
        let mut targets = Vec::new();
        let mut selected_voters = Vec::new();
        let mut has_immediate = false;
        for (identity, option) in self
            .voting_identities
            .iter()
            .zip(&self.bulk_identity_options)
        {
            let timing = match option {
                VoteOption::NoVote => continue,
                VoteOption::CastNow => {
                    has_immediate = true;
                    VoteTiming::Now
                }
                VoteOption::Scheduled {
                    days,
                    hours,
                    minutes,
                } => {
                    let now = Utc::now();
                    let offset = chrono::Duration::days(*days as i64)
                        + chrono::Duration::hours(*hours as i64)
                        + chrono::Duration::minutes(*minutes as i64);
                    VoteTiming::Scheduled((now + offset).timestamp_millis() as u64)
                }
            };
            selected_voters.push(identity.clone());
            for selected_vote in &self.selected_votes {
                let voter_id = identity.identity.id();
                let vote_poll_id = match self
                    .app_context
                    .dpns_vote_poll_id(&selected_vote.contested_name)
                {
                    Ok(vote_poll_id) => vote_poll_id,
                    Err(error) => {
                        tracing::warn!(
                            ?error,
                            contested_name = selected_vote.contested_name,
                            "Could not build a DPNS vote target"
                        );
                        self.bulk_vote_handling_status = VoteHandlingStatus::Failed(
                            "This vote could not be prepared. Refresh Active contests and try again."
                                .to_owned(),
                        );
                        return AppAction::None;
                    }
                };
                let target_key = DpnsVoteTargetKey {
                    network: self.app_context.network(),
                    voter_id,
                    vote_poll_id,
                };
                if self.vote_operations.target_status(&target_key).is_some() {
                    self.bulk_vote_handling_status = VoteHandlingStatus::Failed(format!(
                        "This node's vote for {} is already in progress. Check its result before submitting again.",
                        selected_vote.contested_name
                    ));
                    return AppAction::None;
                }
                let current_choice = match self.vote_state.state(voter_id, vote_poll_id) {
                    DpnsCurrentVoteState::Available(choice) => choice,
                    DpnsCurrentVoteState::Checking | DpnsCurrentVoteState::Unavailable => {
                        self.bulk_vote_handling_status = VoteHandlingStatus::Failed(
                            "Current vote state is unavailable. Refresh voting before applying votes."
                                .to_owned(),
                        );
                        return AppAction::None;
                    }
                };
                targets.push(DpnsVoteTarget {
                    key: target_key,
                    voter_alias: identity.alias.clone(),
                    contested_name: selected_vote.contested_name.clone(),
                    requested_choice: selected_vote.vote_choice,
                    current_choice,
                    timing,
                });
            }
        }

        if targets.is_empty() {
            self.bulk_vote_handling_status = VoteHandlingStatus::Failed(
                "No votes selected. Choose at least one node and contest.".to_owned(),
            );
            return AppAction::None;
        }
        let operation = DpnsVoteOperation::new(targets);
        if operation.targets.is_empty() {
            self.bulk_vote_handling_status = VoteHandlingStatus::Failed(
                "Every selected node already has the requested vote. Nothing will be submitted."
                    .to_owned(),
            );
            return AppAction::None;
        }
        self.bulk_vote_handling_status = if has_immediate {
            VoteHandlingStatus::CastingVotes
        } else {
            VoteHandlingStatus::SchedulingVotes
        };
        self.pending_vote_operation = Some(operation.id);
        AppAction::BackendTask(BackendTask::ContestedResourceTask(
            ContestedResourceTask::SubmitDpnsVoteOperation(
                operation,
                selected_voters,
                None,
                self.app_context.network(),
            ),
        ))
    }

    /// If voting/scheduling is successful, show success message
    fn show_bulk_vote_handling_complete(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        self.selected_votes.clear();

        ui.vertical_centered(|ui| {
            let dark_mode = ui.style().visuals.dark_mode;
            ui.add_space(20.0);
            match &self.bulk_vote_handling_status {
                VoteHandlingStatus::Completed => {
                    ui.heading(
                        RichText::new("The voting operation has been updated.")
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.label(
                        "Review the recent voting activity for each confirmed, pending, or failed target.",
                    );
                }
                VoteHandlingStatus::Failed(message) => {
                    // This means there was a DET-side error, not Platform-side
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.heading(
                        RichText::new("The votes could not be submitted.")
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(10.0);
                    ui.label(RichText::new(message).color(DashColors::text_primary(dark_mode)));
                }
                _ => {
                    // this should not occur
                }
            }

            ui.add_space(20.0);
            let dark_mode = ui.style().visuals.dark_mode;
            if ComponentStyles::add_primary_button(ui, "Go back to Active Contests").clicked() {
                self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
                self.show_bulk_schedule_popup = false;
                action = AppAction::BackendTask(BackendTask::ContestedResourceTask(
                    ContestedResourceTask::QueryDPNSContests,
                ))
            }
            ui.add_space(5.0);
            if ComponentStyles::add_secondary_button(ui, "Go to Scheduled Votes Screen", dark_mode)
                .clicked()
            {
                self.show_bulk_schedule_popup = false;
                self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
                action = AppAction::SetMainScreenThenPopScreen(
                    RootScreenType::RootScreenDPNSScheduledVotes,
                );
            }
        });

        action
    }
}

// ---------------------------
// ScreenLike implementation
// ---------------------------
impl ScreenLike for DPNSScreen {
    fn refresh(&mut self) {
        self.scheduled_vote_cast_in_progress = false;
        if let Err(error) = self.vote_operations.refresh(&self.app_context) {
            tracing::warn!(?error, "Could not refresh cached DPNS vote operations");
        }
        let mut contested_names = self.contested_names.lock_recover();
        let mut dpns_names = self.local_dpns_names.lock_recover();
        let mut scheduled_votes = self.scheduled_votes.lock_recover();

        match self.dpns_subscreen {
            DPNSSubscreen::Active => {
                *contested_names = self
                    .app_context
                    .ongoing_contested_names()
                    .unwrap_or_default();
            }
            DPNSSubscreen::Past => {
                *contested_names = self.app_context.all_contested_names().unwrap_or_default();
            }
            DPNSSubscreen::Owned => {
                *dpns_names = self.app_context.local_dpns_names().unwrap_or_default();
            }
            DPNSSubscreen::ScheduledVotes => {
                *contested_names = self.app_context.all_contested_names().unwrap_or_default();
                let new_scheduled = self.app_context.get_scheduled_votes().unwrap_or_default();
                *scheduled_votes = new_scheduled
                    .iter()
                    .map(|newv| {
                        if newv.executed_successfully {
                            (newv.clone(), ScheduledVoteCastingStatus::Completed)
                        } else if let Some(existing) = scheduled_votes.iter().find(|(old, _)| {
                            old.contested_name == newv.contested_name
                                && old.voter_id == newv.voter_id
                        }) {
                            // preserve old status if InProgress/Failed
                            match existing.1 {
                                ScheduledVoteCastingStatus::InProgress => {
                                    (newv.clone(), ScheduledVoteCastingStatus::InProgress)
                                }
                                ScheduledVoteCastingStatus::Failed => {
                                    (newv.clone(), ScheduledVoteCastingStatus::Failed)
                                }
                                _ => (newv.clone(), ScheduledVoteCastingStatus::NotStarted),
                            }
                        } else {
                            (newv.clone(), ScheduledVoteCastingStatus::NotStarted)
                        }
                    })
                    .collect();
            }
        }
        drop(contested_names);
        drop(dpns_names);
        drop(scheduled_votes);

        let voter_ids = self
            .voting_identities
            .iter()
            .map(|identity| identity.identity.id())
            .collect::<Vec<_>>();
        let poll_ids = self
            .contested_names
            .lock_recover()
            .iter()
            .filter_map(|contest| {
                self.app_context
                    .dpns_vote_poll_id(&contest.normalized_contested_name)
                    .ok()
            })
            .collect::<Vec<_>>();
        if let Err(error) = self
            .vote_state
            .refresh(&self.app_context, &voter_ids, &poll_ids)
        {
            tracing::warn!(?error, "Could not refresh cached DPNS vote state");
        }
    }

    fn refresh_on_arrival(&mut self) {
        self.voting_identities = self
            .app_context
            .load_local_voting_identities()
            .unwrap_or_default();
        self.bulk_identity_options = vec![VoteOption::CastNow; self.voting_identities.len()];
        self.set_all_option = VoteOption::CastNow;
        self.user_identities = self
            .app_context
            .load_local_user_identities()
            .unwrap_or_default();
        self.refresh();
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.refresh_banner.take_and_clear();
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        self.clear_vote_overlay_on_error = match self.pending_vote_operation {
            Some(operation_id) => dpns_operation_id(context) == Some(operation_id),
            None => self.vote_overlay.is_some() && dpns_operation_id(context).is_none(),
        };
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        if self.clear_vote_overlay_on_error {
            self.vote_overlay.take_and_clear();
            self.pending_vote_operation = None;
            self.clear_vote_overlay_on_error = false;
        }
        if let Err(refresh_error) = self.vote_operations.refresh(&self.app_context) {
            tracing::warn!(
                ?refresh_error,
                "Could not refresh DPNS voting state after a task error"
            );
        }
        let handled = scheduled_vote_sweep_is_quiet(error);
        if matches!(
            error,
            TaskError::ScheduledVoteRejected { .. }
                | TaskError::ScheduledVoteAllAddressesExhausted { .. }
                | TaskError::ScheduledVoteResultUnavailable
                | TaskError::ScheduledVoteSweepFailed { .. }
                | TaskError::ScheduledVoteSweepAllAddressesExhausted { .. }
        ) {
            self.scheduled_vote_cast_in_progress = false;
            if let Ok(mut guard) = self.scheduled_votes.lock() {
                for vote in guard.iter_mut() {
                    if vote.1 == ScheduledVoteCastingStatus::InProgress {
                        vote.1 = ScheduledVoteCastingStatus::Failed;
                    }
                }
            }
        }
        handled
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::DpnsVoteOperationUpdated { operation_id, .. } => {
                if let Err(error) = self.vote_state.reload(&self.app_context) {
                    tracing::warn!(
                        ?error,
                        "Could not reload proved DPNS vote state after an operation update"
                    );
                }
                let owns_result = self.pending_vote_operation == Some(operation_id)
                    || (self.pending_vote_operation.is_none() && self.vote_overlay.is_some());
                if owns_result {
                    self.vote_overlay.take_and_clear();
                    self.pending_vote_operation = None;
                    if matches!(
                        self.bulk_vote_handling_status,
                        VoteHandlingStatus::CastingVotes | VoteHandlingStatus::SchedulingVotes
                    ) {
                        self.bulk_vote_handling_status = VoteHandlingStatus::Completed;
                    }
                }
            }
            BackendTaskSuccessResult::ScheduledVotesInProgress(votes) => {
                if let Err(error) = self.vote_operations.refresh(&self.app_context) {
                    tracing::warn!(?error, "Could not refresh scheduled-vote operation state");
                }
                // The periodic sweep is about to cast these votes; reflect that
                // in the list so the user sees them move before results land.
                self.scheduled_vote_cast_in_progress = true;
                if let Ok(mut guard) = self.scheduled_votes.lock() {
                    for vote in &votes {
                        if let Some((_, status)) = guard.iter_mut().find(|(v, _)| {
                            v.contested_name == vote.contested_name && v.voter_id == vote.voter_id
                        }) {
                            *status = ScheduledVoteCastingStatus::InProgress;
                        }
                    }
                }
            }
            BackendTaskSuccessResult::RefreshedDpnsContests
            | BackendTaskSuccessResult::RefreshedOwnedDpnsNames => {
                self.refresh_banner.take_and_clear();
                self.refreshing_status = RefreshingStatus::NotRefreshing;
            }
            _ => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let has_identity_that_can_register = !self.user_identities.is_empty();
        // Build top-right buttons
        let mut right_buttons = match self.dpns_subscreen {
            DPNSSubscreen::Active => {
                let refresh_button = (
                    "Refresh",
                    DesiredAppAction::BackendTask(Box::new(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::QueryDPNSContests,
                    ))),
                );
                vec![refresh_button]
            }
            DPNSSubscreen::Past => {
                let refresh_button = (
                    "Refresh",
                    DesiredAppAction::BackendTask(Box::new(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::QueryDPNSContests,
                    ))),
                );
                vec![refresh_button]
            }
            DPNSSubscreen::Owned => {
                let refresh_button = (
                    "Refresh",
                    DesiredAppAction::BackendTask(Box::new(BackendTask::IdentityTask(
                        IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
                    ))),
                );
                vec![refresh_button]
            }
            DPNSSubscreen::ScheduledVotes => {
                vec![
                    (
                        "Clear All",
                        DesiredAppAction::Custom("Clear all scheduled votes".to_owned()),
                    ),
                    (
                        "Clear Completed",
                        DesiredAppAction::Custom("Clear completed scheduled votes".to_owned()),
                    ),
                ]
            }
        };

        if has_identity_that_can_register && self.dpns_subscreen != DPNSSubscreen::ScheduledVotes {
            // "Register Name" button on the left
            right_buttons.insert(
                0,
                (
                    "Register Name",
                    DesiredAppAction::AddScreenType(Box::new(ScreenType::RegisterDpnsName(
                        RegisterDpnsNameSource::Dpns,
                    ))),
                ),
            );
        }

        // TODO: wire wallet/identity selection consumption for the DPNS page.
        let mut action = add_top_panel_with_global_nav(
            ui,
            &self.app_context,
            subdued_everyday_spec("DPNS", RootScreenType::RootScreenDPNSActiveContests),
            right_buttons,
        );
        if action == AppAction::Custom("Clear all scheduled votes".to_owned()) {
            self.scheduled_clear_dialog = Some((
                true,
                ConfirmationDialog::new(
                    "Clear all scheduled votes",
                    "Remove every scheduled vote from this device? Votes already submitted to Platform cannot be undone.",
                )
                .danger_mode(true)
                .confirm_text(Some("Clear all scheduled votes")),
            ));
            action = AppAction::None;
        } else if action == AppAction::Custom("Clear completed scheduled votes".to_owned()) {
            self.scheduled_clear_dialog = Some((
                false,
                ConfirmationDialog::new(
                    "Clear completed scheduled votes",
                    "Remove every completed scheduled vote from this device? Pending votes will stay scheduled.",
                )
                .danger_mode(true)
                .confirm_text(Some("Clear completed scheduled votes")),
            ));
            action = AppAction::None;
        }

        // Left panel
        action |= add_left_panel(
            ui,
            &self.app_context,
            RootScreenType::RootScreenToolsPlatformInfoScreen,
        );

        // Tools area chooser
        action |= add_tools_subscreen_chooser_panel(ui, self.app_context.as_ref());

        // DPNS subscreen chooser
        action |= add_dpns_subscreen_chooser_panel(ui, self.app_context.as_ref());

        // Main panel
        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;
            if let Some((clear_all, dialog)) = self.scheduled_clear_dialog.as_mut()
                && let Some(status) = dialog.show(ui).inner.dialog_response
            {
                let clear_all = *clear_all;
                self.scheduled_clear_dialog = None;
                if status == ConfirmationStatus::Confirmed {
                    inner_action =
                        AppAction::BackendTask(BackendTask::ContestedResourceTask(if clear_all {
                            ContestedResourceTask::ClearAllScheduledVotes
                        } else {
                            ContestedResourceTask::ClearExecutedScheduledVotes
                        }));
                }
            }
            // Bulk-schedule ephemeral popup
            if self.show_bulk_schedule_popup {
                egui::Window::new("Review and cast")
                    .collapsible(false)
                    .resizable(true)
                    .vscroll(true)
                    .show(ui.ctx(), |ui| {
                        inner_action |= self.show_review_and_cast_window(ui);
                    });
            }

            // Render sub-screen
            match self.dpns_subscreen {
                DPNSSubscreen::Active => {
                    let has_any = !self.contested_names.lock_recover().is_empty();
                    if self.voting_identities.is_empty() {
                        inner_action |= self.render_no_voting_nodes(ui);
                    } else if has_any {
                        self.render_active_contests(ui);
                    } else {
                        inner_action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::Past => {
                    let has_any = {
                        let guard = self.contested_names.lock_recover();
                        !guard.is_empty()
                    };
                    if has_any {
                        self.render_table_past_contests(ui);
                    } else {
                        inner_action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::Owned => {
                    let has_any = {
                        let guard = self.local_dpns_names.lock_recover();
                        !guard.is_empty()
                    };
                    if has_any {
                        self.render_table_local_dpns_names(ui);
                    } else {
                        inner_action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::ScheduledVotes => {
                    let has_any = {
                        let guard = self.scheduled_votes.lock_recover();
                        !guard.is_empty()
                    };
                    if has_any {
                        inner_action |= self.render_table_scheduled_votes(ui);
                    } else {
                        inner_action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
            }

            // Refreshing indicator is shown via the global banner
            // (no inline elapsed rendering needed)
            inner_action
        });

        // Extra handling for actions
        match action {
            // If refreshing contested names, set self.refreshing = true
            AppAction::BackendTask(BackendTask::ContestedResourceTask(
                ContestedResourceTask::QueryDPNSContests,
            )) => {
                self.refresh_banner.take_and_clear();
                let handle = MessageBanner::set_global(
                    ctx,
                    "Refreshing contested names...",
                    MessageType::Info,
                );
                handle.with_elapsed();
                self.refresh_banner = Some(handle);
                self.refreshing_status = RefreshingStatus::Refreshing;
            }
            // If refreshing owned names, set self.refreshing = true
            AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
            )) => {
                self.refresh_banner.take_and_clear();
                let handle = MessageBanner::set_global(
                    ctx,
                    "Refreshing contested names...",
                    MessageType::Info,
                );
                handle.with_elapsed();
                self.refresh_banner = Some(handle);
                self.refreshing_status = RefreshingStatus::Refreshing;
            }
            AppAction::SetMainScreen(_) => {
                self.refresh_banner.take_and_clear();
                self.refreshing_status = RefreshingStatus::NotRefreshing;
            }
            _ => {}
        }

        // If we have a pending backend task from scheduling (e.g. after immediate votes)
        if action == AppAction::None
            && let Some(bt) = self.pending_backend_task.take()
        {
            action = AppAction::BackendTask(bt);
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::model::user_role::UserRoleCell;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::Network;

    fn offline_ctx() -> (Arc<AppContext>, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        crate::app_dir::ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Regtest,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            UserRoleCell::default(),
        )
        .expect("offline regtest AppContext::new");
        (ctx, temp_dir)
    }

    #[test]
    fn vote_submission_overlay_clears_when_the_operation_finishes() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::Active);

        screen.raise_vote_overlay(
            ctx.egui_ctx(),
            "Submitting the selected votes to Dash Platform…",
        );
        screen.pending_vote_operation = Some(DpnsVoteOperationId::from_bytes([7; 16]));
        assert!(
            crate::ui::components::progress_overlay::ProgressOverlay::has_global(ctx.egui_ctx())
        );

        screen.display_task_result(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: ctx.network(),
            operation_id: DpnsVoteOperationId::from_bytes([8; 16]),
        });
        assert!(
            crate::ui::components::progress_overlay::ProgressOverlay::has_global(ctx.egui_ctx())
        );

        screen.display_task_result(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: ctx.network(),
            operation_id: DpnsVoteOperationId::from_bytes([7; 16]),
        });

        assert!(
            !crate::ui::components::progress_overlay::ProgressOverlay::has_global(ctx.egui_ctx())
        );
    }

    #[test]
    fn successful_vote_change_reloads_the_new_proved_choice() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let kv = crate::wallet_backend::DetKv::from_store(Arc::new(
            crate::wallet_backend::kv_test_support::InMemoryKv::default(),
        ));
        let ctx = crate::context::test_support::test_app_context_with_kv(
            temp_dir.path(),
            Arc::new(kv.clone()),
        );
        ctx.set_det_kv_override_for_test(kv);
        let voter = Identifier::from([1; 32]);
        let poll = Identifier::from([2; 32]);
        let old_choice = ResourceVoteChoice::TowardsIdentity(Identifier::from([3; 32]));
        ctx.cache_confirmed_dpns_vote(voter, poll, old_choice)
            .expect("seed old proved choice");

        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::Active);
        screen.vote_state = DpnsVoteStateSnapshot::load(&ctx, &[voter], &[poll])
            .expect("load initial proved choice");
        assert_eq!(
            screen.vote_state.state(voter, poll),
            DpnsCurrentVoteState::Available(Some(old_choice))
        );

        ctx.cache_confirmed_dpns_vote(voter, poll, ResourceVoteChoice::Lock)
            .expect("cache confirmed vote change");
        screen.display_task_result(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: ctx.network(),
            operation_id: DpnsVoteOperationId::from_bytes([7; 16]),
        });

        assert_eq!(
            screen.vote_state.state(voter, poll),
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
        );
    }

    #[test]
    fn active_contest_groups_prioritize_nodes_that_still_need_a_vote() {
        assert!(matches!(
            classify_vote_states([
                DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock)),
                DpnsCurrentVoteState::Available(None),
            ]),
            ActiveContestGroup::NeedsVote
        ));
        assert!(matches!(
            classify_vote_states([DpnsCurrentVoteState::Available(Some(
                ResourceVoteChoice::Abstain,
            ))]),
            ActiveContestGroup::Voted
        ));
        assert!(matches!(
            classify_vote_states([
                DpnsCurrentVoteState::Checking,
                DpnsCurrentVoteState::Unavailable,
            ]),
            ActiveContestGroup::NotVotable
        ));
    }

    #[test]
    fn review_choice_uses_the_candidate_name_instead_of_its_identifier() {
        let candidate_id = Identifier::from([42; 32]);
        let label = review_vote_choice_label(
            ResourceVoteChoice::TowardsIdentity(candidate_id),
            Some("alice"),
        );

        assert_eq!(label, "Vote for alice");
        assert!(!label.contains(&candidate_id.to_string(Encoding::Base58)));
    }

    #[test]
    fn activity_choice_uses_the_cached_candidate_name() {
        let candidate_id = Identifier::from([42; 32]);
        let label = vote_choice_label(
            ResourceVoteChoice::TowardsIdentity(candidate_id),
            Some("alice"),
        );

        assert_eq!(label, "Vote for alice");
        assert!(!label.contains(&candidate_id.to_string(Encoding::Base58)));
    }

    #[test]
    fn proved_choice_is_highlighted_only_when_loaded_nodes_agree() {
        assert_eq!(
            proved_vote_summary([
                DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Abstain)),
                DpnsCurrentVoteState::Checking,
            ]),
            ProvedVoteSummary::Choice(ResourceVoteChoice::Abstain)
        );
        assert_eq!(
            proved_vote_summary([
                DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Abstain)),
                DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock)),
            ]),
            ProvedVoteSummary::Mixed
        );
    }

    #[test]
    fn missing_voting_nodes_copy_is_actionable_and_avoids_the_stale_route() {
        assert!(NO_VOTING_NODES_DETAIL.contains("masternode"));
        assert!(!NO_VOTING_NODES_MESSAGE.contains("Identities screen"));
    }

    #[test]
    fn scheduled_vote_sweep_error_is_handled_after_cleanup() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::ScheduledVotes);
        let error = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::NoVotingIdentity {
                identity_id: "voter-id".to_string(),
            }),
        };

        screen.scheduled_vote_cast_in_progress = true;
        assert!(screen.display_task_error(&error));
        assert!(!screen.scheduled_vote_cast_in_progress);
    }

    #[test]
    fn direct_scheduled_vote_error_remains_available_to_global_handling() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::ScheduledVotes);

        screen.scheduled_vote_cast_in_progress = true;
        assert!(!screen.display_task_error(&TaskError::ScheduledVoteResultUnavailable));
        assert!(!screen.scheduled_vote_cast_in_progress);

        let sweep_error = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::ScheduledVoteResultUnavailable),
        };
        assert!(!screen.display_task_error(&sweep_error));

        screen.scheduled_vote_cast_in_progress = true;
        let exhausted_error = TaskError::ScheduledVoteSweepAllAddressesExhausted {
            network: Network::Regtest,
            source: Box::new(TaskError::DapiNoAddresses {
                source_error: Box::new(dash_sdk::Error::DapiClientError(
                    dash_sdk::dapi_client::DapiClientError::NoAvailableAddresses,
                )),
            }),
        };
        assert!(!screen.display_task_error(&exhausted_error));
        assert!(!screen.scheduled_vote_cast_in_progress);
    }
}
