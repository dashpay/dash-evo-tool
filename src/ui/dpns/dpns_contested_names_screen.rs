mod scheduled_vote_editor;
use scheduled_vote_editor::{EditOutcome, ScheduledVoteEditor};

use crate::wallet_backend::poison::MutexRecover;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, LocalResult, TimeZone, Timelike, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Button, Color32, ComboBox, Label, RichText, Ui};
use egui_extras::{Column, TableBuilder};

use crate::app::{AppAction, DesiredAppAction, scheduled_vote_sweep_is_quiet};
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskContext};
use crate::context::AppContext;
use crate::model::contested_name::{ContestState, ContestedName};
use crate::model::dpns::normalize_dpns_label;
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsScheduledVoteKey, DpnsVoteFailure, DpnsVoteOperation,
    DpnsVoteOperationId, DpnsVoteTarget, DpnsVoteTargetKey, DpnsVoteTargetStatus, VoteTiming,
    dpns_schedule_is_overdue, validate_dpns_schedule_time,
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
use crate::ui::identity::register_dpns_name_screen::RegisterDpnsNameSource;
use crate::ui::state::dpns_contests::{ActiveDpnsContestSnapshot, ActiveDpnsContestView};
use crate::ui::state::dpns_vote_operations::{DpnsVoteOperationSnapshot, ScheduledDpnsVoteRow};
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
const JOURNAL_UNAVAILABLE_MESSAGE: &str = "Saved voting progress could not be read. The displayed history may be incomplete or out of date. Retry loading before managing votes.";
const MISSED_SCHEDULE_GUIDANCE: &str = "The automatic voting time was missed. In Scheduled Votes, use Cast now to vote, Edit to reschedule, or Remove to cancel.";

fn schedule_is_missed(status: DpnsVoteTargetStatus, timing: VoteTiming, now_ms: u64) -> bool {
    status == DpnsVoteTargetStatus::Scheduled
        && matches!(timing, VoteTiming::Scheduled(timestamp) if dpns_schedule_is_overdue(timestamp, now_ms))
}

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

fn target_outcome_label(
    status: DpnsVoteTargetStatus,
    failure: Option<DpnsVoteFailure>,
) -> &'static str {
    if scheduled_failure_guidance(status, failure).is_some() {
        "Not submitted"
    } else {
        target_status_label(status)
    }
}

fn scheduled_failure_guidance(
    status: DpnsVoteTargetStatus,
    failure: Option<DpnsVoteFailure>,
) -> Option<&'static str> {
    if status != DpnsVoteTargetStatus::Scheduled {
        return None;
    }
    match failure? {
        DpnsVoteFailure::CurrentVoteUnavailable => Some(
            "Current vote could not be verified. In Scheduled Votes, use Cast now to check again or Edit to reschedule.",
        ),
        DpnsVoteFailure::SubmissionFailed => Some(
            "The vote could not be submitted. Check your connection and voting key, then use Cast now or Edit in Scheduled Votes.",
        ),
        DpnsVoteFailure::PlatformRejected | DpnsVoteFailure::ResultUnconfirmed => None,
    }
}

fn scheduled_vote_remove_enabled(status: DpnsVoteTargetStatus) -> bool {
    !matches!(
        status,
        DpnsVoteTargetStatus::Queued
            | DpnsVoteTargetStatus::Submitting
            | DpnsVoteTargetStatus::Confirming
            | DpnsVoteTargetStatus::Unconfirmed
    )
}

fn scheduled_vote_cast_enabled(status: DpnsVoteTargetStatus, dispatch_pending: bool) -> bool {
    !dispatch_pending
        && matches!(
            status,
            DpnsVoteTargetStatus::Scheduled
                | DpnsVoteTargetStatus::Rejected
                | DpnsVoteTargetStatus::FailedBeforeSubmission
                | DpnsVoteTargetStatus::Cancelled
                | DpnsVoteTargetStatus::NotApplied
        )
}

fn scheduled_vote_removal_task(row: &ScheduledDpnsVoteRow) -> ContestedResourceTask {
    match &row.journal_target {
        Some((operation_id, key)) => ContestedResourceTask::CancelScheduledDpnsVote {
            operation_id: *operation_id,
            key: key.clone(),
            contested_name: row.vote.contested_name.clone(),
        },
        None => ContestedResourceTask::DeleteScheduledVote(
            row.vote.voter_id,
            row.vote.contested_name.clone(),
        ),
    }
}

/// The loaded nodes that can actually cast a vote.
///
/// A masternode loaded read-only passes the identity-type filter but has no
/// voting key, so the DPNS surfaces must drop it here — otherwise it reaches the
/// composer and only fails once the vote is submitted.
fn loaded_voting_identities(app_context: &AppContext) -> Result<Vec<QualifiedIdentity>, TaskError> {
    Ok(app_context
        .load_local_voting_identities()?
        .into_iter()
        .filter(QualifiedIdentity::can_cast_masternode_vote)
        .collect())
}

/// One reviewed node × contest line, carrying the timing text the operator chose.
struct ReviewEntry {
    target: DpnsVoteTarget,
    timing_label: String,
}

impl ReviewEntry {
    fn node_label(&self) -> String {
        self.target
            .voter_alias
            .clone()
            .unwrap_or_else(|| short_identifier(self.target.key.voter_id))
    }
}

/// Exactly what a submit click would send, resolved before the review is shown.
struct ReviewPlan {
    entries: Vec<ReviewEntry>,
    voters: Vec<QualifiedIdentity>,
}

impl ReviewPlan {
    /// The targets that will really be submitted, no-op targets removed.
    fn effective(&self) -> impl Iterator<Item = &ReviewEntry> {
        self.entries.iter().filter(|entry| !entry.target.is_no_op())
    }

    fn effective_count(&self) -> usize {
        self.effective().count()
    }

    fn no_op_count(&self) -> usize {
        self.entries.len() - self.effective_count()
    }

    fn node_count(&self) -> usize {
        self.effective()
            .map(|entry| entry.target.key.voter_id)
            .collect::<BTreeSet<_>>()
            .len()
    }

    fn has_immediate(&self) -> bool {
        self.effective()
            .any(|entry| matches!(entry.target.timing, VoteTiming::Now))
    }

    fn has_scheduled(&self) -> bool {
        self.effective()
            .any(|entry| matches!(entry.target.timing, VoteTiming::Scheduled(_)))
    }
}

fn review_headline(effective_count: usize, node_count: usize) -> String {
    format!("Votes to submit ({effective_count}) from your nodes ({node_count}).")
}

fn review_target_line(
    node: &str,
    name: &str,
    requested: &str,
    current: &str,
    timing: &str,
) -> String {
    format!("• {node} → {name}.dash: {requested}. Current vote: {current}. {timing}.")
}

fn review_skipped_line(no_op_count: usize) -> String {
    format!("Targets skipped because the node already has that choice ({no_op_count}).")
}

fn review_current_choice_label(
    current: Option<ResourceVoteChoice>,
    candidate_name: Option<&str>,
) -> String {
    match current {
        Some(choice) => review_vote_choice_label(choice, candidate_name),
        None => "Not voted yet".to_owned(),
    }
}

fn dpns_operation_id(
    context: &BackendTaskContext,
    network: dash_sdk::dpp::dashcore::Network,
) -> Option<DpnsVoteOperationId> {
    match context {
        BackendTaskContext::Dispatched { operation, .. } => dpns_operation_id(operation, network),
        BackendTaskContext::DpnsVoteOperation {
            operation_id,
            network: origin,
        } if *origin == network => Some(*operation_id),
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
    Scheduled {
        days: u32,
        hours: u32,
        minutes: u32,
    },
    /// Absolute UTC time, in Unix milliseconds.
    ScheduledAt(u64),
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
    voting_identity_load_error: Option<TaskError>,
    user_identities: Vec<QualifiedIdentity>,
    contested_names: Arc<Mutex<Vec<ContestedName>>>,
    active_contests: ActiveDpnsContestSnapshot,
    local_dpns_names: Arc<Mutex<Vec<(Identifier, DPNSNameInfo)>>>,
    scheduled_votes: Arc<Mutex<Vec<ScheduledDpnsVoteRow>>>,
    pub selected_votes: Vec<SelectedVote>,
    pub app_context: Arc<AppContext>,
    pending_backend_task: Option<BackendTask>,
    vote_operations: DpnsVoteOperationSnapshot,
    vote_state: DpnsVoteStateSnapshot,
    vote_overlay: Option<OverlayHandle>,
    pending_vote_operation: Option<DpnsVoteOperationId>,
    pending_scheduled_actions: BTreeMap<DpnsScheduledVoteKey, BackendTaskContext>,
    clear_vote_overlay_on_error: bool,
    scheduled_clear_dialog: Option<(bool, ConfirmationDialog)>,
    scheduled_vote_editor: Option<ScheduledVoteEditor>,

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
    journal_error_banner: Option<BannerHandle>,

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
        let vote_operations = DpnsVoteOperationSnapshot::load(app_context);
        let legacy_scheduled_votes = app_context.get_scheduled_votes().unwrap_or_default();
        let scheduled_votes = Arc::new(Mutex::new(
            vote_operations.scheduled_vote_rows(&legacy_scheduled_votes),
        ));

        // Load contested names, local dpns, scheduled, etc.:
        let contested_names = Arc::new(Mutex::new(match dpns_subscreen {
            DPNSSubscreen::Active => Vec::new(),
            DPNSSubscreen::Past => app_context.all_contested_names().unwrap_or_default(),
            DPNSSubscreen::Owned => Vec::new(),
            DPNSSubscreen::ScheduledVotes => app_context.all_contested_names().unwrap_or_default(),
        }));
        let active_contests = if dpns_subscreen == DPNSSubscreen::Active {
            ActiveDpnsContestSnapshot::new(
                app_context,
                app_context.ongoing_contested_names().unwrap_or_default(),
            )
        } else {
            ActiveDpnsContestSnapshot::default()
        };

        let local_dpns_names = Arc::new(Mutex::new(match dpns_subscreen {
            DPNSSubscreen::Active => Vec::new(),
            DPNSSubscreen::Past => Vec::new(),
            DPNSSubscreen::Owned => app_context.local_dpns_names().unwrap_or_default(),
            DPNSSubscreen::ScheduledVotes => Vec::new(),
        }));

        let (voting_identities, voting_identity_load_error) =
            match loaded_voting_identities(app_context) {
                Ok(identities) => (identities, None),
                Err(error) => {
                    tracing::warn!(?error, "Could not load local DPNS voting identities");
                    (Vec::new(), Some(error))
                }
            };
        let user_identities = app_context.load_local_user_identities().unwrap_or_default();
        let vote_poll_ids = active_contests.vote_poll_ids();
        let voter_ids = voting_identities
            .iter()
            .map(|identity| identity.identity.id())
            .collect::<Vec<_>>();
        let mut vote_state = DpnsVoteStateSnapshot::default();
        if let Err(error) = vote_state.refresh(app_context, &voter_ids, &vote_poll_ids) {
            tracing::warn!(?error, "Could not cache proved DPNS vote state");
        }

        // Initialize vote handling pop-up state to hidden
        let identity_count = voting_identities.len();
        let bulk_identity_options = vec![VoteOption::CastNow; identity_count];
        let default_schedule_time = Utc::now() + chrono::Duration::days(1);

        Self {
            voting_identities,
            voting_identity_load_error,
            user_identities,
            contested_names,
            active_contests,
            local_dpns_names,
            scheduled_votes,
            selected_votes: Vec::new(),
            app_context: app_context.clone(),
            sort_column: SortColumn::ContestedName,
            sort_order: SortOrder::Ascending,
            active_filter_term: String::new(),
            past_filter_term: String::new(),
            owned_filter_term: String::new(),
            pending_backend_task: None,
            vote_operations,
            vote_state,
            vote_overlay: None,
            pending_vote_operation: None,
            pending_scheduled_actions: BTreeMap::new(),
            clear_vote_overlay_on_error: false,
            scheduled_clear_dialog: None,
            scheduled_vote_editor: None,
            dpns_subscreen,
            refreshing_status: RefreshingStatus::NotRefreshing,
            refresh_banner: None,
            journal_error_banner: None,

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

    fn finish_scheduled_dispatch(&mut self, context: &BackendTaskContext) -> bool {
        let key = self
            .pending_scheduled_actions
            .iter()
            .find(|(_, pending)| *pending == context)
            .map(|(key, _)| key.clone());
        if let Some(key) = key {
            self.pending_scheduled_actions.remove(&key);
            if self.pending_vote_operation.is_none() {
                self.vote_overlay.take_and_clear();
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn reset_for_network_switch(&mut self) {
        self.selected_votes.clear();
        self.show_bulk_schedule_popup = false;
        self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
        self.bulk_identity_options.clear();
        self.set_all_option = VoteOption::CastNow;
        self.pending_backend_task = None;
        self.pending_vote_operation = None;
        self.pending_scheduled_actions.clear();
        self.clear_vote_overlay_on_error = false;
        self.scheduled_clear_dialog = None;
        self.scheduled_vote_editor = None;
        self.vote_overlay.take_and_clear();
        self.refresh_banner.take_and_clear();
        self.journal_error_banner.take_and_clear();
        self.refreshing_status = RefreshingStatus::NotRefreshing;
        self.voting_identities.clear();
        self.voting_identity_load_error = None;
        self.user_identities.clear();
        self.vote_state = DpnsVoteStateSnapshot::default();
        self.vote_operations = DpnsVoteOperationSnapshot::default();
        self.active_contests = ActiveDpnsContestSnapshot::default();
        self.contested_names.lock_recover().clear();
        self.scheduled_votes.lock_recover().clear();
        self.local_dpns_names.lock_recover().clear();
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
        let contests = self.active_contests.contests();
        let mut groups = [Vec::new(), Vec::new(), Vec::new()];
        for contest in contests.iter() {
            let index = match self.contest_group(contest.vote_poll_id) {
                ActiveContestGroup::NeedsVote => 0,
                ActiveContestGroup::Voted => 1,
                ActiveContestGroup::NotVotable => 2,
            };
            groups[index].push(contest);
        }

        let closes_within_day = groups[0]
            .iter()
            .filter(|contest| {
                contest.contest.end_time.is_some_and(|end_time| {
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
                        .contest
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
                    self.voting_identities.is_empty(),
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

    fn contest_group(&self, vote_poll_id: Option<Identifier>) -> ActiveContestGroup {
        let Some(poll_id) = vote_poll_id else {
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
        contests: &[&ActiveDpnsContestView],
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
                        voting_enabled && self.contest_has_available_target(contest.vote_poll_id);
                    ui.add_enabled_ui(contest_enabled, |ui| {
                        self.render_contest_card(
                            ui,
                            contest.contest.as_ref(),
                            contest.vote_poll_id,
                            contest_enabled,
                            show_current_vote,
                        );
                    });
                    ui.add_space(8.0);
                }
            });
    }

    fn contest_has_available_target(&self, vote_poll_id: Option<Identifier>) -> bool {
        let Some(vote_poll_id) = vote_poll_id else {
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

    fn proved_vote_for_contest(&self, vote_poll_id: Option<Identifier>) -> ProvedVoteSummary {
        let Some(poll_id) = vote_poll_id else {
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
        vote_poll_id: Option<Identifier>,
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
            self.proved_vote_for_contest(vote_poll_id)
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

    fn review_failed_target(&mut self, outcome: &crate::model::dpns_voting::DpnsVoteOutcome) {
        self.selected_votes.clear();
        self.set_selected_vote_from_outcome(outcome);
        self.bulk_identity_options = self
            .voting_identities
            .iter()
            .map(|voter| {
                if voter.identity.id() == outcome.target.key.voter_id {
                    VoteOption::CastNow
                } else {
                    VoteOption::NoVote
                }
            })
            .collect();
        self.set_all_option = VoteOption::CastNow;
        self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
        self.show_bulk_schedule_popup = true;
    }

    fn render_voting_activity(&mut self, ui: &mut Ui) {
        let operations = self.vote_operations.recent_operations();
        if operations.is_empty() {
            return;
        }

        let dark_mode = ui.style().visuals.dark_mode;
        ui.add_space(12.0);
        ui.heading("Voting activity");
        for operation in operations.iter() {
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
                    let missed = schedule_is_missed(outcome.status, outcome.target.timing, Utc::now().timestamp_millis().max(0) as u64);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!(
                            "{} — {}.dash — {} — {}",
                            outcome.target.voter_alias.clone().unwrap_or_else(|| short_identifier(outcome.target.key.voter_id)),
                            outcome.target.contested_name,
                            vote_choice_label(
                                outcome.target.requested_choice,
                                self.candidate_name(
                                    &outcome.target.contested_name,
                                    outcome.target.requested_choice,
                                )
                                .as_deref(),
                            ),
                            if missed { "Missed automatic vote" } else { target_outcome_label(outcome.status, outcome.failure) },
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
                            self.review_failed_target(outcome);
                        }
                    });
                    if let Some(guidance) = scheduled_failure_guidance(outcome.status, outcome.failure) {
                        ui.label(guidance);
                    }
                    if missed {
                        ui.label(MISSED_SCHEDULE_GUIDANCE);
                    }
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
        sorted_votes.sort_by(|a, b| {
            let order = a.vote.contested_name.cmp(&b.vote.contested_name);
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
                .column(Column::initial(280.0).resizable(true)) // Status
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
                    for scheduled_row in &sorted_votes {
                        let vote = &scheduled_row.vote;
                        let pending_key = DpnsScheduledVoteKey {
                            network: scheduled_row
                                .journal_target
                                .as_ref()
                                .map_or(self.app_context.network(), |(_, key)| key.network),
                            voter_id: vote.voter_id,
                            contested_name: vote.contested_name.clone(),
                        };
                        let missed = schedule_is_missed(scheduled_row.status, VoteTiming::Scheduled(vote.unix_timestamp), Utc::now().timestamp_millis().max(0) as u64);
                        let failure_guidance = if missed { Some(MISSED_SCHEDULE_GUIDANCE) } else { scheduled_failure_guidance(scheduled_row.status, scheduled_row.failure) };
                        body.row(if failure_guidance.is_some() { 95.0 } else { 25.0 }, |mut row| {
                            row.col(|ui| {
                                ui.add(Label::new(format!("{}.dash", vote.contested_name)));
                            });
                            row.col(|ui| {
                                let voter = self
                                    .voting_identities
                                    .iter()
                                    .find(|identity| identity.identity.id() == vote.voter_id)
                                    .and_then(|identity| identity.alias.clone())
                                    .unwrap_or_else(|| short_identifier(vote.voter_id));
                                ui.add(Label::new(voter));
                            });
                            row.col(|ui| {
                                let candidate_name =
                                    self.candidate_name(&vote.contested_name, vote.choice);
                                let display_text =
                                    vote_choice_label(vote.choice, candidate_name.as_deref());
                                ui.add(Label::new(display_text));
                            });
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                if let LocalResult::Single(dt) =
                                    Utc.timestamp_millis_opt(vote.unix_timestamp as i64)
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
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(if missed { "Missed automatic vote" } else { target_outcome_label(scheduled_row.status, scheduled_row.failure) })
                                        .color(DashColors::text_primary(dark_mode)));
                                    if let Some(guidance) = failure_guidance {
                                        ui.add(Label::new(guidance).wrap());
                                    }
                                });
                            });
                            row.col(|ui| {
                                let remove_enabled =
                                    scheduled_vote_remove_enabled(scheduled_row.status);
                                if ui
                                    .add_enabled(remove_enabled, Button::new("Remove"))
                                    .disabled_tooltip(
                                        "This scheduled vote cannot be removed while its result is being checked.",
                                    )
                                    .clicked()
                                {
                                    action = AppAction::BackendTask(
                                        BackendTask::ContestedResourceTask(
                                            scheduled_vote_removal_task(scheduled_row),
                                        ),
                                    );
                                }
                                if ui.add_enabled(scheduled_row.status == DpnsVoteTargetStatus::Scheduled
                                    && !self.pending_scheduled_actions.contains_key(&pending_key), Button::new("Edit"))
                                    .disabled_tooltip("Only scheduled votes that have not started can be edited.")
                                    .clicked() {
                                    self.open_schedule_editor(scheduled_row);
                                }
                                let cast_button_enabled = scheduled_vote_cast_enabled(
                                    scheduled_row.status,
                                    self.pending_scheduled_actions.contains_key(&pending_key),
                                ) && self.voting_identities.iter().any(|identity| identity.identity.id() == vote.voter_id);
                                if ui
                                    .add_enabled(cast_button_enabled, Button::new("Cast now"))
                                    .disabled_tooltip("Load this node's voting key and wait for any pending submission to finish before casting.")
                                    .clicked()
                                    && let Some(found) = self
                                        .voting_identities
                                        .iter()
                                        .find(|identity| {
                                            identity.identity.id() == vote.voter_id
                                        })
                                        .cloned()
                                {
                                    let task = BackendTask::ContestedResourceTask(
                                        ContestedResourceTask::CastScheduledVote(vote.clone(), Box::new(found)));
                                    let context = BackendTaskContext::for_dispatch_on(&task, self.app_context.network());
                                    self.pending_scheduled_actions.insert(pending_key.clone(), context.clone());
                                    action = AppAction::BackendTaskWithContext { task, context };
                                    show_cast_overlay = true;
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

    fn open_schedule_editor(&mut self, row: &ScheduledDpnsVoteRow) {
        let Ok(poll_id) = self.app_context.dpns_vote_poll_id(&row.vote.contested_name) else {
            return;
        };
        let key = DpnsVoteTargetKey {
            network: self.app_context.network(),
            voter_id: row.vote.voter_id,
            vote_poll_id: poll_id,
        };
        let node_label = self
            .voting_identities
            .iter()
            .find(|identity| identity.identity.id() == row.vote.voter_id)
            .and_then(|identity| identity.alias.clone())
            .unwrap_or_else(|| short_identifier(row.vote.voter_id));
        let mut choices = vec![
            (ResourceVoteChoice::Lock, "Lock".to_owned()),
            (ResourceVoteChoice::Abstain, "Abstain".to_owned()),
        ];
        if let Some(contest) = self
            .contested_names
            .lock_recover()
            .iter()
            .find(|contest| contest.normalized_contested_name == row.vote.contested_name)
            && let Some(candidates) = &contest.contestants
        {
            choices.extend(candidates.iter().map(|candidate| {
                (
                    ResourceVoteChoice::TowardsIdentity(candidate.id),
                    format!("Vote for {}", candidate.name),
                )
            }));
        }
        if !choices.iter().any(|(choice, _)| *choice == row.vote.choice) {
            choices.push((row.vote.choice, vote_choice_label(row.vote.choice, None)));
        }
        self.scheduled_vote_editor =
            ScheduledVoteEditor::new(row.clone(), key, node_label, choices);
    }

    fn simple_schedule_option(&self) -> Option<VoteOption> {
        let timestamp = crate::model::dpns_vote_schedule::parse_utc_schedule(
            &self.simple_schedule_date,
            self.simple_schedule_hour,
            self.simple_schedule_minute,
        )?;
        (timestamp > Utc::now().timestamp_millis() as u64)
            .then_some(VoteOption::ScheduledAt(timestamp))
    }

    fn apply_all_nodes_option(&mut self, option: VoteOption) {
        self.set_all_option = option.clone();
        self.bulk_identity_options.fill(option);
    }

    fn candidate_name(&self, contested_name: &str, choice: ResourceVoteChoice) -> Option<String> {
        let ResourceVoteChoice::TowardsIdentity(candidate_id) = choice else {
            return None;
        };
        if let Some(candidate_name) = self
            .active_contests
            .contest(contested_name)
            .and_then(|contest| contest.contestants.as_ref())
            .and_then(|contestants| {
                contestants
                    .iter()
                    .find(|candidate| candidate.id == candidate_id)
            })
            .map(|candidate| candidate.name.clone())
        {
            return Some(candidate_name);
        }
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

    fn rebuild_scheduled_vote_rows(&mut self) {
        let legacy_votes = self.app_context.get_scheduled_votes().unwrap_or_default();
        *self.scheduled_votes.lock_recover() =
            self.vote_operations.scheduled_vote_rows(&legacy_votes);
    }

    fn render_voting_identity_load_error(&mut self, ui: &mut Ui) {
        ui.colored_label(
            DashColors::warning_color(ui.visuals().dark_mode),
            "Your nodes could not be loaded from this device. Retry loading before voting.",
        );
        if ui.button("Retry loading nodes").clicked() {
            self.refresh_on_arrival();
        }
    }

    fn show_review_and_cast_window(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        let dark_mode = ui.style().visuals.dark_mode;

        if self.bulk_vote_handling_status == VoteHandlingStatus::Completed {
            action |= self.show_bulk_vote_handling_complete(ui);
            return action;
        }

        if self.voting_identity_load_error.is_some() {
            self.render_voting_identity_load_error(ui);
            if ComponentStyles::add_secondary_button(ui, "Close", dark_mode).clicked() {
                self.show_bulk_schedule_popup = false;
            }
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

        self.bulk_identity_options
            .resize(self.voting_identities.len(), VoteOption::CastNow);
        let plan = self.build_review_plan();

        let mut simple_schedule_valid = true;
        egui::ScrollArea::vertical().show(ui, |ui| {
            match &plan {
                Ok(plan) => {
                    ui.heading(review_headline(plan.effective_count(), plan.node_count()));
                    for entry in plan.effective() {
                        let candidate_name = self.candidate_name(
                            &entry.target.contested_name,
                            entry.target.requested_choice,
                        );
                        let requested = review_vote_choice_label(
                            entry.target.requested_choice,
                            candidate_name.as_deref(),
                        );
                        let current_candidate_name = entry
                            .target
                            .current_choice
                            .and_then(|choice| {
                                self.candidate_name(&entry.target.contested_name, choice)
                            });
                        let current = review_current_choice_label(
                            entry.target.current_choice,
                            current_candidate_name.as_deref(),
                        );
                        ui.label(review_target_line(
                            &entry.node_label(),
                            &entry.target.contested_name,
                            &requested,
                            &current,
                            &entry.timing_label,
                        ));
                    }
                    if plan.effective().any(|entry| entry.target.current_choice.is_some()) {
                        ui.colored_label(DashColors::warning_color(dark_mode),
                            "Changing an existing vote uses one of that node's four allowed vote changes.");
                    }
                    if plan.no_op_count() > 0 {
                        ui.label(review_skipped_line(plan.no_op_count()));
                    }
                    if plan.effective_count() == 0 {
                        ui.colored_label(
                            DashColors::warning_color(dark_mode),
                            "Nothing will be submitted. Choose at least one node and a vote it has not already cast.",
                        );
                    }
                }
                Err(message) => {
                    ui.colored_label(DashColors::warning_color(dark_mode), message);
                }
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
                        matches!(self.set_all_option, VoteOption::ScheduledAt(_)),
                        "Schedule for later",
                    )
                    .clicked()
                {
                    let option = self.simple_schedule_option().unwrap_or(VoteOption::ScheduledAt(0));
                    self.apply_all_nodes_option(option);
                }
            });

            if matches!(self.set_all_option, VoteOption::ScheduledAt(_)) {
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
                    self.set_all_option = option.clone();
                    for current in &mut self.bulk_identity_options {
                        if matches!(current, VoteOption::ScheduledAt(_)) {
                            *current = option.clone();
                        }
                    }
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

                                let current_option = &mut self.bulk_identity_options[i];
                                ComboBox::from_id_salt(format!("combo_bulk_identity_{}", i))
                                    .width(120.0)
                                    .selected_text(match current_option {
                                        VoteOption::NoVote => "Do not use this node".to_string(),
                                        VoteOption::CastNow => "Cast now".to_string(),
                                        VoteOption::Scheduled { .. } => "Schedule after a delay".to_string(),
                                        VoteOption::ScheduledAt(_) => "At selected UTC time".to_string(),
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
        if !matches!(self.set_all_option, VoteOption::ScheduledAt(_))
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
        let has_submittable = plan.as_ref().is_ok_and(|plan| plan.effective_count() > 0);
        let submit_label = match plan
            .as_ref()
            .map(|plan| (plan.has_immediate(), plan.has_scheduled()))
        {
            Ok((true, false)) => "Cast votes",
            Ok((false, true)) => "Schedule votes",
            _ => "Submit votes",
        };
        let submit_disabled_reason = if operation_in_progress {
            "The selected votes are already being submitted."
        } else if !simple_schedule_valid {
            "Choose a future date and time before scheduling these votes."
        } else if plan.is_err() {
            "These votes cannot be submitted yet. Fix the problem shown above and try again."
        } else {
            "Nothing can be submitted. Choose at least one node and a vote it has not already cast."
        };
        let (submit_clicked, cancel_clicked) = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let submit_clicked = ComponentStyles::add_primary_button_enabled(
                    ui,
                    !operation_in_progress && simple_schedule_valid && has_submittable,
                    if operation_in_progress {
                        "Submitting votes…"
                    } else {
                        submit_label
                    },
                )
                .disabled_tooltip(submit_disabled_reason)
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

    /// Resolve every node × contest target the current review would submit.
    ///
    /// The review sheet and the submit click share this so the sheet cannot
    /// promise something other than what is sent. `Err` carries the message the
    /// operator sees, and blocks submission.
    fn build_review_plan(&self) -> Result<ReviewPlan, String> {
        self.build_review_plan_at(Utc::now())
    }

    fn build_review_plan_at(&self, now: DateTime<Utc>) -> Result<ReviewPlan, String> {
        let mut entries = Vec::new();
        let mut voters = Vec::new();
        for (identity, option) in self
            .voting_identities
            .iter()
            .zip(&self.bulk_identity_options)
        {
            let (timing, timing_label) = match option {
                VoteOption::NoVote => continue,
                VoteOption::CastNow => (VoteTiming::Now, "Cast now".to_owned()),
                VoteOption::ScheduledAt(timestamp) => {
                    if *timestamp <= now.timestamp_millis() as u64 {
                        return Err(
                            "Choose a future date and time before scheduling these votes."
                                .to_owned(),
                        );
                    }
                    let date = DateTime::from_timestamp_millis(*timestamp as i64)
                        .ok_or_else(|| "Choose a valid UTC date and time.".to_owned())?;
                    (
                        VoteTiming::Scheduled(*timestamp),
                        format!("Scheduled for {} UTC", date.format("%Y-%m-%d %H:%M")),
                    )
                }
                VoteOption::Scheduled {
                    days,
                    hours,
                    minutes,
                } => {
                    let offset = chrono::Duration::days(*days as i64)
                        + chrono::Duration::hours(*hours as i64)
                        + chrono::Duration::minutes(*minutes as i64);
                    (
                        VoteTiming::Scheduled((now + offset).timestamp_millis() as u64),
                        format!("Scheduled in {days} d {hours} h {minutes} min"),
                    )
                }
            };
            voters.push(identity.clone());
            for selected_vote in &self.selected_votes {
                if let VoteTiming::Scheduled(timestamp) = timing {
                    validate_dpns_schedule_time(
                        timestamp,
                        now.timestamp_millis() as u64,
                        selected_vote.end_time,
                    )
                    .map_err(|_| {
                        format!(
                            "Choose a future time before the contest ends for {}.dash.",
                            selected_vote.contested_name
                        )
                    })?;
                }
                let voter_id = identity.identity.id();
                let vote_poll_id = self
                    .app_context
                    .dpns_vote_poll_id(&selected_vote.contested_name)
                    .map_err(|error| {
                        tracing::warn!(
                            ?error,
                            contested_name = selected_vote.contested_name,
                            "Could not build a DPNS vote target"
                        );
                        "This vote could not be prepared. Refresh Active contests and try again."
                            .to_owned()
                    })?;
                let target_key = DpnsVoteTargetKey {
                    network: self.app_context.network(),
                    voter_id,
                    vote_poll_id,
                };
                if self.vote_operations.target_status(&target_key).is_some() {
                    return Err(format!(
                        "This node's vote for {} is already in progress. Check its result before submitting again.",
                        selected_vote.contested_name
                    ));
                }
                let DpnsCurrentVoteState::Available(current_choice) =
                    self.vote_state.state(voter_id, vote_poll_id)
                else {
                    return Err(
                        "Current vote state is unavailable. Refresh voting before applying votes."
                            .to_owned(),
                    );
                };
                entries.push(ReviewEntry {
                    target: DpnsVoteTarget {
                        key: target_key,
                        voter_alias: identity.alias.clone(),
                        contested_name: selected_vote.contested_name.clone(),
                        requested_choice: selected_vote.vote_choice,
                        current_choice,
                        timing,
                    },
                    timing_label: timing_label.clone(),
                });
            }
        }
        Ok(ReviewPlan { entries, voters })
    }

    fn bulk_apply_votes(&mut self) -> AppAction {
        let plan = match self.build_review_plan() {
            Ok(plan) => plan,
            Err(message) => {
                self.bulk_vote_handling_status = VoteHandlingStatus::Failed(message);
                return AppAction::None;
            }
        };
        let has_immediate = plan.has_immediate();
        let ReviewPlan {
            entries,
            voters: selected_voters,
        } = plan;

        if entries.is_empty() {
            self.bulk_vote_handling_status = VoteHandlingStatus::Failed(
                "No votes selected. Choose at least one node and contest.".to_owned(),
            );
            return AppAction::None;
        }
        // No-op targets are handed over unfiltered: the operation counts them,
        // and that count drives the post-submit feedback.
        let operation =
            DpnsVoteOperation::new(entries.into_iter().map(|entry| entry.target).collect());
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
        if let Err(error) = self.vote_operations.refresh(&self.app_context) {
            tracing::warn!(?error, "Could not refresh cached DPNS vote operations");
        }
        self.rebuild_scheduled_vote_rows();

        match self.dpns_subscreen {
            DPNSSubscreen::Active => {
                self.active_contests = ActiveDpnsContestSnapshot::new(
                    &self.app_context,
                    self.app_context
                        .ongoing_contested_names()
                        .unwrap_or_default(),
                );
            }
            DPNSSubscreen::Past => {
                *self.contested_names.lock_recover() =
                    self.app_context.all_contested_names().unwrap_or_default();
            }
            DPNSSubscreen::Owned => {
                *self.local_dpns_names.lock_recover() =
                    self.app_context.local_dpns_names().unwrap_or_default();
            }
            DPNSSubscreen::ScheduledVotes => {
                *self.contested_names.lock_recover() =
                    self.app_context.all_contested_names().unwrap_or_default();
            }
        }

        let voter_ids = self
            .voting_identities
            .iter()
            .map(|identity| identity.identity.id())
            .collect::<Vec<_>>();
        let poll_ids = self.active_contests.vote_poll_ids();
        if let Err(error) = self
            .vote_state
            .refresh(&self.app_context, &voter_ids, &poll_ids)
        {
            tracing::warn!(?error, "Could not refresh cached DPNS vote state");
        }
    }

    fn refresh_on_arrival(&mut self) {
        let previous_options = self
            .voting_identities
            .iter()
            .zip(&self.bulk_identity_options)
            .map(|(identity, option)| (identity.identity.id(), option.clone()))
            .collect::<BTreeMap<_, _>>();
        match loaded_voting_identities(&self.app_context) {
            Ok(identities) => {
                self.voting_identities = identities;
                self.voting_identity_load_error = None;
            }
            Err(error) => {
                tracing::warn!(?error, "Could not reload local DPNS voting identities");
                self.voting_identities.clear();
                self.voting_identity_load_error = Some(error);
            }
        }
        self.bulk_identity_options = self
            .voting_identities
            .iter()
            .map(|identity| {
                previous_options
                    .get(&identity.identity.id())
                    .cloned()
                    .unwrap_or({
                        if self.selected_votes.is_empty() {
                            VoteOption::CastNow
                        } else {
                            VoteOption::NoVote
                        }
                    })
            })
            .collect();
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
            Some(operation_id) => {
                dpns_operation_id(context, self.app_context.network()) == Some(operation_id)
            }
            None => self.finish_scheduled_dispatch(context),
        };
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        if let Err(refresh_error) = self.vote_state.reload(&self.app_context) {
            tracing::warn!(
                ?refresh_error,
                "Could not refresh proved DPNS votes after a task error"
            );
        }
        if self.clear_vote_overlay_on_error {
            self.vote_overlay.take_and_clear();
            self.pending_vote_operation = None;
            self.clear_vote_overlay_on_error = false;
            // The review window disables both Submit and Cancel while a
            // submission is in flight, so a failed submission must leave that
            // state here. Otherwise the window stays on "Submitting votes…"
            // with no way to retry or close it.
            if matches!(
                self.bulk_vote_handling_status,
                VoteHandlingStatus::CastingVotes | VoteHandlingStatus::SchedulingVotes
            ) {
                self.bulk_vote_handling_status = VoteHandlingStatus::Failed(error.to_string());
            }
        }
        if let Err(refresh_error) = self.vote_operations.refresh(&self.app_context) {
            tracing::warn!(
                ?refresh_error,
                "Could not refresh DPNS voting state after a task error"
            );
        }
        self.rebuild_scheduled_vote_rows();
        scheduled_vote_sweep_is_quiet(error)
    }

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        result: BackendTaskSuccessResult,
    ) {
        self.finish_scheduled_dispatch(context);
        self.display_task_result(result);
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::DpnsVoteOperationUpdated {
                network,
                operation_id,
            } if network == self.app_context.network() => {
                if let Err(error) = self.vote_state.reload(&self.app_context) {
                    tracing::warn!(
                        ?error,
                        "Could not reload proved DPNS vote state after an operation update"
                    );
                }
                let owns_result = self.pending_vote_operation == Some(operation_id);
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
                if let Err(error) = self.vote_operations.refresh(&self.app_context) {
                    tracing::warn!(
                        ?error,
                        "Could not refresh scheduled votes after an operation update"
                    );
                }
                self.rebuild_scheduled_vote_rows();
            }
            BackendTaskSuccessResult::ScheduledVoteSweepCompleted { network, .. }
                if network == self.app_context.network() =>
            {
                self.refresh();
            }
            BackendTaskSuccessResult::ScheduledVotesInProgress(_) => {
                if let Err(error) = self.vote_operations.refresh(&self.app_context) {
                    tracing::warn!(?error, "Could not refresh scheduled-vote operation state");
                }
                self.rebuild_scheduled_vote_rows();
            }
            BackendTaskSuccessResult::ScheduledVotesCleared(_) => {
                if let Err(error) = self.vote_operations.refresh(&self.app_context) {
                    tracing::warn!(
                        ?error,
                        "Could not refresh scheduled votes after clearing them"
                    );
                }
                self.rebuild_scheduled_vote_rows();
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
            if let Some(error) = self.vote_operations.read_error() {
                if self.journal_error_banner.is_none() || self.journal_error_banner.was_evicted() {
                    self.journal_error_banner.raise_persistent(ui.ctx(), "Saved voting progress could not be read. Retry loading before managing votes.", MessageType::Warning);
                    if let Some(handle) = &self.journal_error_banner {
                        handle.with_details(error);
                    }
                }
                ui.label(JOURNAL_UNAVAILABLE_MESSAGE);
                if ComponentStyles::add_secondary_button(
                    ui,
                    "Retry loading",
                    ui.visuals().dark_mode,
                )
                .clicked()
                {
                    self.refresh();
                }
                ui.separator();
            } else {
                self.journal_error_banner.take_and_clear();
            }
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
            if let Some(editor) = self.scheduled_vote_editor.as_mut() {
                match editor.show(ui.ctx()) {
                    EditOutcome::KeepOpen => {}
                    EditOutcome::Cancel => self.scheduled_vote_editor = None,
                    EditOutcome::Save(edit) => {
                        let key = editor.scheduled_key();
                        let task = BackendTask::ContestedResourceTask(edit);
                        let context =
                            BackendTaskContext::for_dispatch_on(&task, self.app_context.network());
                        self.pending_scheduled_actions.insert(key, context.clone());
                        inner_action = AppAction::BackendTaskWithContext { task, context };
                        self.scheduled_vote_editor = None;
                        self.raise_vote_overlay(ui.ctx(), "Saving the scheduled vote…");
                    }
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

            if self.voting_identity_load_error.is_some() && !self.show_bulk_schedule_popup {
                self.render_voting_identity_load_error(ui);
            }
            // Render sub-screen
            match self.dpns_subscreen {
                DPNSSubscreen::Active => {
                    let has_any = !self.active_contests.is_empty();
                    if self.voting_identities.is_empty()
                        && self.voting_identity_load_error.is_none()
                    {
                        inner_action |= self.render_no_voting_nodes(ui);
                    }
                    if has_any {
                        self.render_active_contests(ui);
                    } else {
                        inner_action |= self.render_no_active_contests_or_owned_names(ui);
                        egui::ScrollArea::vertical()
                            .id_salt("voting_activity_without_contests")
                            .show(ui, |ui| self.render_voting_activity(ui));
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
    use crate::backend_task::contested_names::ScheduledDPNSVote;
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

    fn kv_ctx() -> (Arc<AppContext>, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let kv = crate::wallet_backend::DetKv::from_store(Arc::new(
            crate::wallet_backend::kv_test_support::InMemoryKv::default(),
        ));
        let ctx = crate::context::test_support::test_app_context_with_kv(
            temp_dir.path(),
            Arc::new(kv.clone()),
        );
        ctx.set_det_kv_override_for_test(kv);
        (ctx, temp_dir)
    }

    async fn wired_ctx() -> (
        Arc<AppContext>,
        tempfile::TempDir,
        tokio::sync::mpsc::Receiver<crate::app::TaskResult>,
    ) {
        let (ctx, temp_dir) = offline_ctx();
        let (tx, events) = tokio::sync::mpsc::channel(32);
        ctx.ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
            tx,
            ctx.egui_ctx().clone(),
        ))
        .await
        .expect("wire the wallet backend offline");
        (ctx, temp_dir, events)
    }

    /// A masternode identity as the DPNS screen sees it: `voting` decides whether
    /// its voter identity — the key the vote submission path needs — is loaded.
    fn masternode_identity(
        id: u8,
        alias: &str,
        voting: bool,
        network: Network,
    ) -> QualifiedIdentity {
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;
        use dash_sdk::platform::IdentityPublicKey;

        let platform_version = PlatformVersion::latest();
        let identity =
            Identity::create_basic_identity(Identifier::from([id; 32]), platform_version)
                .expect("identity");
        let associated_voter_identity = voting.then(|| {
            let voter = Identity::create_basic_identity(
                Identifier::from([id.wrapping_add(100); 32]),
                platform_version,
            )
            .expect("voter identity");
            (
                voter,
                IdentityPublicKey::random_key(0, Some(id as u64), platform_version),
            )
        });
        QualifiedIdentity {
            identity,
            associated_voter_identity,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: crate::model::qualified_identity::IdentityType::Masternode,
            alias: Some(alias.to_owned()),
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: std::collections::BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: Default::default(),
            network,
        }
    }

    fn pending_scheduled_key(context: &AppContext) -> DpnsScheduledVoteKey {
        DpnsScheduledVoteKey {
            network: context.network(),
            voter_id: Identifier::from([1; 32]),
            contested_name: "pending".to_owned(),
        }
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

        screen
            .pending_scheduled_actions
            .insert(pending_scheduled_key(&ctx), BackendTaskContext::Unknown);
        screen.display_task_result(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: ctx.network(),
            operation_id: DpnsVoteOperationId::from_bytes([7; 16]),
        });

        assert!(
            !crate::ui::components::progress_overlay::ProgressOverlay::has_global(ctx.egui_ctx())
        );
        assert!(!screen.pending_scheduled_actions.is_empty());
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
    fn scheduled_vote_sweep_error_preserves_unrelated_manual_cast() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::ScheduledVotes);
        let error = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::NoVotingIdentity {
                identity_id: "voter-id".to_string(),
            }),
        };

        screen
            .pending_scheduled_actions
            .insert(pending_scheduled_key(&ctx), BackendTaskContext::Unknown);
        assert!(screen.display_task_error(&error));
        assert!(!screen.pending_scheduled_actions.is_empty());
    }

    #[test]
    fn direct_scheduled_vote_error_remains_available_to_global_handling() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::ScheduledVotes);

        screen
            .pending_scheduled_actions
            .insert(pending_scheduled_key(&ctx), BackendTaskContext::Unknown);
        assert!(!screen.display_task_error(&TaskError::ScheduledVoteResultUnavailable));
        assert!(!screen.pending_scheduled_actions.is_empty());

        let sweep_error = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::ScheduledVoteResultUnavailable),
        };
        assert!(!screen.display_task_error(&sweep_error));

        screen
            .pending_scheduled_actions
            .insert(pending_scheduled_key(&ctx), BackendTaskContext::Unknown);
        let exhausted_error = TaskError::ScheduledVoteSweepAllAddressesExhausted {
            network: Network::Regtest,
            source: Box::new(TaskError::DapiNoAddresses {
                source_error: Box::new(dash_sdk::Error::DapiClientError(
                    dash_sdk::dapi_client::DapiClientError::NoAvailableAddresses,
                )),
            }),
        };
        assert!(!screen.display_task_error(&exhausted_error));
        assert!(!screen.pending_scheduled_actions.is_empty());
    }

    #[test]
    fn scheduled_vote_actions_follow_the_journal_status() {
        for status in [
            DpnsVoteTargetStatus::Scheduled,
            DpnsVoteTargetStatus::Confirmed,
            DpnsVoteTargetStatus::Rejected,
            DpnsVoteTargetStatus::FailedBeforeSubmission,
            DpnsVoteTargetStatus::Cancelled,
            DpnsVoteTargetStatus::NotApplied,
        ] {
            assert!(scheduled_vote_remove_enabled(status), "{status:?}");
        }
        for status in [
            DpnsVoteTargetStatus::Queued,
            DpnsVoteTargetStatus::Submitting,
            DpnsVoteTargetStatus::Confirming,
            DpnsVoteTargetStatus::Unconfirmed,
        ] {
            assert!(!scheduled_vote_remove_enabled(status), "{status:?}");
        }
        for status in [
            DpnsVoteTargetStatus::Scheduled,
            DpnsVoteTargetStatus::Rejected,
            DpnsVoteTargetStatus::FailedBeforeSubmission,
            DpnsVoteTargetStatus::Cancelled,
            DpnsVoteTargetStatus::NotApplied,
        ] {
            assert!(scheduled_vote_cast_enabled(status, false), "{status:?}");
        }
        for status in [
            DpnsVoteTargetStatus::Queued,
            DpnsVoteTargetStatus::Submitting,
            DpnsVoteTargetStatus::Confirming,
            DpnsVoteTargetStatus::Confirmed,
            DpnsVoteTargetStatus::Unconfirmed,
        ] {
            assert!(!scheduled_vote_cast_enabled(status, false), "{status:?}");
        }
        assert!(!scheduled_vote_cast_enabled(
            DpnsVoteTargetStatus::Scheduled,
            true
        ));
    }

    #[test]
    fn scheduled_vote_removal_dispatches_the_journal_or_legacy_task() {
        let operation_id = DpnsVoteOperationId::from_bytes([31; 16]);
        let key = DpnsVoteTargetKey {
            network: Network::Testnet,
            voter_id: Identifier::from([32; 32]),
            vote_poll_id: Identifier::from([33; 32]),
        };
        let vote = ScheduledDPNSVote {
            contested_name: "dispatch".to_owned(),
            voter_id: key.voter_id,
            choice: ResourceVoteChoice::Lock,
            unix_timestamp: 42,
            executed_successfully: false,
        };
        let journal_row = ScheduledDpnsVoteRow {
            vote: vote.clone(),
            journal_target: Some((operation_id, key.clone())),
            status: DpnsVoteTargetStatus::Scheduled,
            failure: None,
        };
        let legacy_row = ScheduledDpnsVoteRow {
            vote,
            journal_target: None,
            status: DpnsVoteTargetStatus::Scheduled,
            failure: None,
        };

        assert!(matches!(
            scheduled_vote_removal_task(&journal_row),
            ContestedResourceTask::CancelScheduledDpnsVote {
                operation_id: id,
                key: task_key,
                contested_name,
            } if id == operation_id && task_key == key && contested_name == "dispatch"
        ));
        assert!(matches!(
            scheduled_vote_removal_task(&legacy_row),
            ContestedResourceTask::DeleteScheduledVote(voter_id, contested_name)
                if voter_id == Identifier::from([32; 32]) && contested_name == "dispatch"
        ));
    }

    /// A failed submission must release the review window's in-flight state.
    /// While it is held, the window disables Submit *and* Cancel and offers no
    /// close control, so a stuck status leaves the user with no way out.
    #[test]
    fn failed_submission_releases_the_review_window() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::Active);
        let operation_id = DpnsVoteOperationId::from_bytes([11; 16]);
        let context = BackendTaskContext::DpnsVoteOperation {
            network: ctx.network(),
            operation_id,
        };
        let error = TaskError::DpnsCurrentVoteUnavailable;

        for in_progress in [
            VoteHandlingStatus::CastingVotes,
            VoteHandlingStatus::SchedulingVotes,
        ] {
            screen.show_bulk_schedule_popup = true;
            screen.bulk_vote_handling_status = in_progress;
            screen.pending_vote_operation = Some(operation_id);
            screen.raise_vote_overlay(ctx.egui_ctx(), "Submitting the selected votes…");

            screen.display_backend_task_error(&context, &error);
            screen.display_task_error(&error);

            assert!(
                !crate::ui::components::progress_overlay::ProgressOverlay::has_global(
                    ctx.egui_ctx()
                )
            );
            assert_eq!(screen.pending_vote_operation, None);
            assert!(
                screen.bulk_vote_handling_status == VoteHandlingStatus::Failed(error.to_string()),
                "the review window must leave its in-flight state so Submit and Cancel work again"
            );
        }
    }

    /// An error belonging to a different operation must not disturb the review
    /// window of the submission this screen is still waiting on.
    #[test]
    fn unrelated_error_keeps_the_pending_submission_in_progress() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::Active);
        let error = TaskError::DpnsCurrentVoteUnavailable;
        screen.show_bulk_schedule_popup = true;
        screen.bulk_vote_handling_status = VoteHandlingStatus::CastingVotes;
        screen.pending_vote_operation = Some(DpnsVoteOperationId::from_bytes([11; 16]));

        screen.display_backend_task_error(
            &BackendTaskContext::DpnsVoteOperation {
                network: ctx.network(),
                operation_id: DpnsVoteOperationId::from_bytes([12; 16]),
            },
            &error,
        );
        screen.display_task_error(&error);

        assert_eq!(
            screen.pending_vote_operation,
            Some(DpnsVoteOperationId::from_bytes([11; 16]))
        );
        assert!(screen.bulk_vote_handling_status == VoteHandlingStatus::CastingVotes);
    }

    /// VOTE-TC-013: a masternode loaded without its voting key must not reach the
    /// composer — the actionable "no voting key" state has to come first. This is
    /// distinct from having no masternode loaded at all.
    #[tokio::test]
    async fn a_loaded_masternode_without_a_voting_key_cannot_compose_votes() {
        let (ctx, _temp_dir, _events) = wired_ctx().await;
        ctx.insert_local_qualified_identity(
            &masternode_identity(41, "read-only-node", false, ctx.network()),
            &None,
        )
        .expect("insert keyless masternode");

        let screen = DPNSScreen::new(&ctx, DPNSSubscreen::Active);

        assert!(
            !ctx.load_local_voting_identities()
                .expect("load voting identities")
                .is_empty(),
            "the keyless masternode must still be loaded, or this proves nothing"
        );
        assert!(
            screen.voting_identities.is_empty(),
            "a node with no voting key cannot satisfy the submit path"
        );
    }

    #[tokio::test]
    async fn a_loaded_masternode_with_a_voting_key_can_compose_votes() {
        let (ctx, _temp_dir, _events) = wired_ctx().await;
        ctx.insert_local_qualified_identity(
            &masternode_identity(42, "voting-node", true, ctx.network()),
            &None,
        )
        .expect("insert voting masternode");

        let screen = DPNSScreen::new(&ctx, DPNSSubscreen::Active);

        assert_eq!(screen.voting_identities.len(), 1);
    }

    fn voting_ui_review_fixture() -> (DPNSScreen, tempfile::TempDir) {
        let (ctx, temp_dir) = kv_ctx();
        let voter = masternode_identity(1, "node-one", true, ctx.network());
        let poll = ctx.dpns_vote_poll_id("alpha").unwrap();
        ctx.cache_confirmed_dpns_vote(voter.identity.id(), poll, ResourceVoteChoice::Lock)
            .unwrap();
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::Active);
        screen.voting_identities = vec![voter.clone()];
        screen.bulk_identity_options = vec![VoteOption::CastNow];
        screen.selected_votes = vec![SelectedVote {
            contested_name: "alpha".to_owned(),
            vote_choice: ResourceVoteChoice::Abstain,
            end_time: None,
        }];
        screen.vote_state =
            DpnsVoteStateSnapshot::load(&ctx, &[voter.identity.id()], &[poll]).unwrap();
        (screen, temp_dir)
    }

    #[test]
    fn voting_ui_unconfirmed_activity_is_reachable_without_active_contests_after_restart() {
        use egui_kittest::kittest::Queryable;
        let (screen, _dir) = voting_ui_review_fixture();
        let target = screen.build_review_plan().unwrap().entries[0]
            .target
            .clone();
        let mut operation = DpnsVoteOperation::new(vec![target]);
        operation.targets[0].status = DpnsVoteTargetStatus::Unconfirmed;
        screen
            .app_context
            .insert_dpns_vote_operation(&mut operation, None)
            .unwrap();
        let mut reopened = DPNSScreen::new(&screen.app_context, DPNSSubscreen::Active);
        assert!(reopened.active_contests.is_empty());
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 1200.0))
            .build_ui(move |ui| {
                reopened.ui(ui);
            });
        harness.run();
        assert!(harness.query_by_label("Check again").is_some());
    }

    #[test]
    fn additional_scenarios_missed_schedule_is_explained_after_reopening() {
        use egui_kittest::kittest::Queryable;
        for status in [
            DpnsVoteTargetStatus::Scheduled,
            DpnsVoteTargetStatus::Queued,
        ] {
            let (screen, _dir) = voting_ui_review_fixture();
            let mut target = screen.build_review_plan().unwrap().entries[0]
                .target
                .clone();
            target.timing = VoteTiming::Scheduled(1);
            let mut operation = DpnsVoteOperation::new(vec![target]);
            operation.targets[0].status = status;
            screen
                .app_context
                .insert_dpns_vote_operation(&mut operation, None)
                .unwrap();
            let mut reopened = DPNSScreen::new(&screen.app_context, DPNSSubscreen::ScheduledVotes);
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1600.0, 1200.0))
                .build_ui(move |ui| {
                    reopened.ui(ui);
                });
            harness.run();
            assert_eq!(
                harness.query_by_label("Missed automatic vote").is_some(),
                status == DpnsVoteTargetStatus::Scheduled
            );
            if status == DpnsVoteTargetStatus::Scheduled {
                assert!(harness.query_by_label("The automatic voting time was missed. In Scheduled Votes, use Cast now to vote, Edit to reschedule, or Remove to cancel.").is_some());
                assert!(harness.query_by_label("Cast now").is_some());
                assert!(harness.query_by_label("Edit").is_some());
            }
        }
    }

    #[test]
    fn additional_scenarios_journal_failure_stays_visible_until_refresh_succeeds() {
        use egui_kittest::kittest::Queryable;
        for fail_at_construction in [true, false] {
            let (context, _dir) = kv_ctx();
            let store = Arc::new(crate::wallet_backend::kv_test_support::FailingKv::default());
            context.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
                store.clone(),
            ));
            if fail_at_construction {
                store.fail_next_gets_containing("det:dpns_vote_operations:v2:", 1);
            }
            let mut screen = DPNSScreen::new(&context, DPNSSubscreen::ScheduledVotes);
            if !fail_at_construction {
                store.fail_next_gets_containing("det:dpns_vote_operations:v2:", 1);
                screen.refresh();
            }
            let screen = Arc::new(Mutex::new(screen));
            let rendering = screen.clone();
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1600.0, 1200.0))
                .build_ui(move |ui| {
                    rendering.lock_recover().ui(ui);
                });
            harness.run();
            let message = "Saved voting progress could not be read. The displayed history may be incomplete or out of date. Retry loading before managing votes.";
            assert!(harness.query_by_label(message).is_some());
            screen
                .lock_recover()
                .journal_error_banner
                .as_ref()
                .unwrap()
                .clone()
                .clear();
            harness.run();
            assert!(harness.query_by_label(message).is_some());
            harness.get_by_label("Retry loading").click();
            harness.run();
            assert!(harness.query_by_label(message).is_none());
        }
    }

    #[test]
    fn voting_ui_scheduled_failure_reason_survives_reopening() {
        use crate::model::dpns_voting::DpnsVoteFailure;
        use egui_kittest::kittest::Queryable;
        for (failure, reason) in [
            (
                DpnsVoteFailure::CurrentVoteUnavailable,
                "Current vote could not be verified. In Scheduled Votes, use Cast now to check again or Edit to reschedule.",
            ),
            (
                DpnsVoteFailure::SubmissionFailed,
                "The vote could not be submitted. Check your connection and voting key, then use Cast now or Edit in Scheduled Votes.",
            ),
        ] {
            let (screen, _dir) = voting_ui_review_fixture();
            let mut target = screen.build_review_plan().unwrap().entries[0]
                .target
                .clone();
            target.timing = VoteTiming::Scheduled(Utc::now().timestamp_millis() as u64);
            let mut operation = DpnsVoteOperation::new(vec![target]);
            operation.targets[0].failure = Some(failure);
            screen
                .app_context
                .insert_dpns_vote_operation(&mut operation, None)
                .unwrap();
            for subscreen in [DPNSSubscreen::ScheduledVotes, DPNSSubscreen::Active] {
                let scheduled = subscreen == DPNSSubscreen::ScheduledVotes;
                let mut reopened = DPNSScreen::new(&screen.app_context, subscreen);
                let mut harness = egui_kittest::Harness::builder()
                    .with_size(egui::vec2(1600.0, 1200.0))
                    .build_ui(move |ui| {
                        reopened.ui(ui);
                    });
                harness.run();
                if scheduled {
                    assert!(harness.query_by_label("Not submitted").is_some());
                }
                assert!(harness.query_by_label(reason).is_some());
            }
        }
    }

    #[tokio::test]
    async fn voting_ui_constructor_keeps_other_voters_when_one_snapshot_read_fails() {
        let (context, _dir, _events) = wired_ctx().await;
        let store = Arc::new(crate::wallet_backend::kv_test_support::FailingKv::default());
        let kv = crate::wallet_backend::DetKv::from_store(store.clone());
        context.set_det_kv_override_for_test(kv);
        for id in [1, 2] {
            let voter = masternode_identity(id, &format!("node-{id}"), true, context.network());
            context
                .insert_local_qualified_identity(&voter, &None)
                .unwrap();
            let poll = context.dpns_vote_poll_id("alpha").unwrap();
            context
                .cache_confirmed_dpns_vote(voter.identity.id(), poll, ResourceVoteChoice::Lock)
                .unwrap();
        }
        context
            .insert_name_contests_as_normalized_names(vec!["alpha".into()])
            .unwrap();
        store.fail_next_gets_containing("det:dpns_current_votes:v3:", 1);
        let screen = DPNSScreen::new(&context, DPNSSubscreen::Active);
        assert_eq!(screen.voting_identities.len(), 2);
        let poll = context.dpns_vote_poll_id("alpha").unwrap();
        assert_eq!(
            screen
                .vote_state
                .state(screen.voting_identities[0].identity.id(), poll),
            DpnsCurrentVoteState::Unavailable
        );
        assert_eq!(
            screen
                .vote_state
                .state(screen.voting_identities[1].identity.id(), poll),
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
        );
    }

    #[tokio::test]
    async fn voting_ui_task_error_reloads_current_state_and_preserves_valid_confirmations() {
        let (context, _dir) = kv_ctx();
        let voter = Identifier::from([1; 32]);
        let poll = context.dpns_vote_poll_id("alpha").unwrap();
        let other = context.dpns_vote_poll_id("beta").unwrap();
        context
            .seed_proved_dpns_votes_for_test(
                voter,
                BTreeMap::from([(poll.to_buffer(), ResourceVoteChoice::Lock)]),
            )
            .await
            .unwrap();
        context
            .cache_confirmed_dpns_vote(voter, other, ResourceVoteChoice::Lock)
            .unwrap();
        let mut screen = DPNSScreen::new(&context, DPNSSubscreen::Active);
        screen
            .vote_state
            .refresh(&context, &[voter], &[poll, other])
            .unwrap();
        assert_eq!(
            screen.vote_state.state(voter, poll),
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
        );
        let error = TaskError::DpnsVotePreflightFailed {
            source: Arc::new(
                context
                    .fail_proved_dpns_votes_for_test(voter)
                    .await
                    .unwrap_err(),
            ),
        };
        assert_eq!(
            context.dpns_current_vote_state(voter, poll).unwrap(),
            DpnsCurrentVoteState::Unavailable
        );
        screen.display_task_error(&error);
        assert_eq!(
            screen.vote_state.state(voter, poll),
            DpnsCurrentVoteState::Unavailable
        );
        assert_eq!(
            screen.vote_state.state(voter, other),
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock))
        );
    }

    #[test]
    fn voting_ui_review_rejects_absolute_and_relative_schedules_at_contest_deadline() {
        let (mut screen, _dir) = voting_ui_review_fixture();
        let now = Utc::now();
        let scheduled_at = (now + chrono::Duration::hours(1)).timestamp_millis() as u64;
        for option in [
            VoteOption::ScheduledAt(scheduled_at),
            VoteOption::Scheduled {
                days: 0,
                hours: 1,
                minutes: 0,
            },
        ] {
            screen.bulk_identity_options = vec![option];
            for deadline in [scheduled_at - 1, scheduled_at, scheduled_at + 1] {
                screen.selected_votes[0].end_time = Some(deadline);
                assert_eq!(
                    screen.build_review_plan_at(now).is_ok(),
                    deadline > scheduled_at
                );
            }
        }
        screen.selected_votes.push(SelectedVote {
            contested_name: "beta".into(),
            vote_choice: ResourceVoteChoice::Abstain,
            end_time: Some(scheduled_at),
        });
        assert!(
            screen
                .build_review_plan_at(now)
                .err()
                .expect("the second contest has ended before the schedule")
                .contains("beta.dash")
        );
    }

    #[test]
    fn voting_ui_absolute_schedule_preserves_the_requested_utc_minute() {
        let (mut screen, _temp_dir) = voting_ui_review_fixture();
        let requested = (Utc::now() + chrono::Duration::days(1))
            .date_naive()
            .and_hms_opt(18, 30, 0)
            .unwrap()
            .and_utc();
        screen.simple_schedule_date = requested.format("%Y-%m-%d").to_string();
        screen.simple_schedule_hour = 18;
        screen.simple_schedule_minute = 30;
        screen.apply_all_nodes_option(screen.simple_schedule_option().unwrap());
        let plan = screen.build_review_plan().unwrap();
        assert_eq!(
            plan.entries[0].target.timing,
            VoteTiming::Scheduled(requested.timestamp_millis() as u64)
        );
        let delayed = screen
            .build_review_plan_at(Utc::now() + chrono::Duration::minutes(20))
            .unwrap();
        assert_eq!(
            delayed.entries[0].target.timing,
            plan.entries[0].target.timing
        );
    }

    #[test]
    fn voting_ui_manual_cast_completes_only_for_its_exact_dispatch() {
        let (mut screen, _dir) = voting_ui_review_fixture();
        let voter = screen.voting_identities[0].clone();
        let vote = crate::backend_task::contested_names::ScheduledDPNSVote {
            contested_name: "alpha".into(),
            voter_id: voter.identity.id(),
            choice: ResourceVoteChoice::Abstain,
            unix_timestamp: 123,
            executed_successfully: false,
        };
        let task = BackendTask::ContestedResourceTask(ContestedResourceTask::CastScheduledVote(
            vote,
            Box::new(voter),
        ));
        let dispatch = BackendTaskContext::for_dispatch_on(&task, screen.app_context.network());
        let unrelated_dispatch =
            BackendTaskContext::for_dispatch_on(&task, screen.app_context.network());
        let key = DpnsScheduledVoteKey {
            network: screen.app_context.network(),
            voter_id: screen.voting_identities[0].identity.id(),
            contested_name: "alpha".into(),
        };
        screen
            .pending_scheduled_actions
            .insert(key.clone(), dispatch.clone());
        screen.raise_vote_overlay(&screen.app_context.egui_ctx().clone(), "Submitting…");
        let updated = BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: screen.app_context.network(),
            operation_id: DpnsVoteOperationId::from_bytes([9; 16]),
        };
        screen.display_backend_task_result(&unrelated_dispatch, updated.clone());
        assert!(screen.pending_scheduled_actions.contains_key(&key));
        assert!(screen.vote_overlay.is_some());
        let sweep = BackendTaskContext::ScheduledVoteSweep {
            network: screen.app_context.network(),
        };
        screen.display_backend_task_result(
            &sweep,
            BackendTaskSuccessResult::ScheduledVoteSweepCompleted {
                network: screen.app_context.network(),
                preserve_eligibility_since_ms: None,
            },
        );
        let error = TaskError::ScheduledVoteResultUnavailable;
        screen.display_backend_task_error(&sweep, &error);
        screen.display_task_error(&error);
        assert!(screen.pending_scheduled_actions.contains_key(&key));
        assert!(screen.vote_overlay.is_some());
        screen.display_backend_task_result(&dispatch, updated);
        assert!(screen.pending_scheduled_actions.is_empty());
        assert!(screen.vote_overlay.is_none());
        screen
            .pending_scheduled_actions
            .insert(key.clone(), dispatch.clone());
        screen.display_backend_task_error(&dispatch, &error);
        screen.display_task_error(&error);
        assert!(screen.pending_scheduled_actions.is_empty());
        screen
            .pending_scheduled_actions
            .insert(key, dispatch.clone());
        screen.reset_for_network_switch();
        assert!(screen.pending_scheduled_actions.is_empty());
        assert!(!screen.finish_scheduled_dispatch(&dispatch));
    }

    #[test]
    fn voting_ui_keyless_users_can_read_cached_active_contests() {
        use egui_kittest::kittest::Queryable;
        let (context, _dir) = kv_ctx();
        let mut screen = DPNSScreen::new(&context, DPNSSubscreen::Active);
        screen.active_contests = ActiveDpnsContestSnapshot::new(
            &context,
            vec![ContestedName {
                normalized_contested_name: "readable".into(),
                contestants: None,
                locked_votes: Some(2),
                abstain_votes: Some(3),
                awarded_to: None,
                end_time: None,
                state: ContestState::Ongoing,
                last_updated: None,
                my_votes: BTreeMap::new(),
            }],
        );
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 1200.0))
            .build_ui(move |ui| {
                screen.ui(ui);
            });
        harness.run();
        assert!(harness.query_by_label(NO_VOTING_NODES_MESSAGE).is_some());
        assert!(harness.query_by_label("readable.dash").is_some());
    }

    #[test]
    fn voting_ui_identity_storage_error_is_not_reported_as_a_missing_key() {
        use egui_kittest::kittest::Queryable;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::wallet_backend::kv_test_support::FailingKv::default());
        let kv = crate::wallet_backend::DetKv::from_store(store.clone());
        let context = crate::context::test_support::test_app_context_with_kv(
            dir.path(),
            Arc::new(kv.clone()),
        );
        context.set_det_kv_override_for_test(kv);
        store.fail_reads(true);
        assert!(loaded_voting_identities(&context).is_err());
        for reviewing in [false, true] {
            let mut screen = DPNSScreen::new(&context, DPNSSubscreen::Active);
            assert!(screen.voting_identity_load_error.is_some());
            screen.show_bulk_schedule_popup = reviewing;
            let mut harness = egui_kittest::Harness::builder().build_ui(move |ui| {
                screen.ui(ui);
            });
            harness.run();
            assert!(harness.query_by_label("Your nodes could not be loaded from this device. Retry loading before voting.").is_some());
            assert!(harness.query_by_label(NO_VOTING_NODES_MESSAGE).is_none());
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn voting_ui_retry_keeps_its_node_selection_after_returning_to_the_page() {
        let (ctx, _dir, _events) = wired_ctx().await;
        let first = masternode_identity(1, "first-node", true, ctx.network());
        let second = masternode_identity(2, "other-node", true, ctx.network());
        ctx.insert_local_qualified_identity(&first, &None).unwrap();
        ctx.insert_local_qualified_identity(&second, &None).unwrap();
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::Active);
        screen.selected_votes = vec![SelectedVote {
            contested_name: "alpha".into(),
            vote_choice: ResourceVoteChoice::Lock,
            end_time: None,
        }];
        screen.bulk_identity_options = screen
            .voting_identities
            .iter()
            .map(|identity| {
                if identity.identity.id() == first.identity.id() {
                    VoteOption::CastNow
                } else {
                    VoteOption::NoVote
                }
            })
            .collect();
        screen.refresh_on_arrival();
        let selected = screen
            .voting_identities
            .iter()
            .zip(&screen.bulk_identity_options)
            .filter(|(_, option)| !matches!(option, VoteOption::NoVote))
            .map(|(identity, _)| identity.identity.id())
            .collect::<Vec<_>>();
        assert_eq!(selected, vec![first.identity.id()]);
        let newly_loaded = masternode_identity(3, "new-node", true, ctx.network());
        ctx.insert_local_qualified_identity(&newly_loaded, &None)
            .unwrap();
        screen.refresh_on_arrival();
        let new_option = screen
            .voting_identities
            .iter()
            .zip(&screen.bulk_identity_options)
            .find(|(identity, _)| identity.identity.id() == newly_loaded.identity.id())
            .unwrap()
            .1;
        assert!(matches!(new_option, VoteOption::NoVote));
        ctx.wallet_backend().unwrap().shutdown().await;
    }

    #[test]
    fn voting_ui_retry_reviews_only_the_failed_node_and_contest() {
        let (mut screen, _dir) = voting_ui_review_fixture();
        let operation = DpnsVoteOperation::new(vec![
            screen.build_review_plan().unwrap().entries.remove(0).target,
        ]);
        screen.voting_identities.push(masternode_identity(
            2,
            "node-two",
            true,
            screen.app_context.network(),
        ));
        screen.selected_votes.push(SelectedVote {
            contested_name: "unrelated".into(),
            vote_choice: ResourceVoteChoice::Lock,
            end_time: None,
        });
        screen.review_failed_target(&operation.targets[0]);
        let plan = screen.build_review_plan().unwrap();
        assert_eq!(plan.effective_count(), 1);
        assert_eq!(plan.entries[0].target.key, operation.targets[0].target.key);
    }

    #[test]
    fn voting_ui_review_warns_before_using_a_limited_change() {
        use egui_kittest::kittest::Queryable;
        let (mut screen, _temp_dir) = voting_ui_review_fixture();
        let mut harness = egui_kittest::Harness::builder().build_ui(move |ui| {
            screen.show_review_and_cast_window(ui);
        });
        harness.run();
        assert!(
            harness
                .query_by_label(
                    "Changing an existing vote uses one of that node's four allowed vote changes."
                )
                .is_some()
        );
    }

    #[test]
    fn voting_ui_no_op_review_does_not_warn_that_a_change_will_be_used() {
        use egui_kittest::kittest::Queryable;
        let (mut screen, _dir) = voting_ui_review_fixture();
        screen.selected_votes[0].vote_choice = ResourceVoteChoice::Lock;
        assert_eq!(screen.build_review_plan().unwrap().effective_count(), 0);
        let mut harness = egui_kittest::Harness::builder().build_ui(move |ui| {
            screen.show_review_and_cast_window(ui);
        });
        harness.run();
        assert!(
            harness
                .query_by_label(
                    "Changing an existing vote uses one of that node's four allowed vote changes."
                )
                .is_none()
        );
    }

    #[test]
    fn voting_ui_activity_identifies_each_node() {
        use egui_kittest::kittest::Queryable;
        let (mut screen, _temp_dir) = voting_ui_review_fixture();
        let plan = screen.build_review_plan().unwrap();
        let mut first = plan.entries[0].target.clone();
        first.voter_alias = Some("node-one".to_owned());
        let mut second = first.clone();
        second.key.voter_id = Identifier::from([2; 32]);
        second.voter_alias = Some("node-two".to_owned());
        let mut operation = DpnsVoteOperation::new(vec![first, second]);
        operation.targets[0].status = DpnsVoteTargetStatus::Confirmed;
        operation.targets[1].status = DpnsVoteTargetStatus::Rejected;
        screen
            .app_context
            .insert_dpns_vote_operation(&mut operation, None)
            .unwrap();
        screen.vote_operations.refresh(&screen.app_context).unwrap();
        let mut harness = egui_kittest::Harness::builder().build_ui(move |ui| {
            screen.render_voting_activity(ui);
        });
        harness.run();
        assert!(
            harness
                .query_by_label("node-one — alpha.dash — Abstain — Confirmed")
                .is_some()
        );
        assert!(
            harness
                .query_by_label("node-two — alpha.dash — Abstain — Rejected")
                .is_some()
        );
    }

    #[test]
    fn voting_ui_network_switch_discards_staged_choices() {
        let (mut screen, _temp_dir) = voting_ui_review_fixture();
        screen.show_bulk_schedule_popup = true;
        screen.pending_scheduled_actions.insert(
            pending_scheduled_key(&screen.app_context),
            BackendTaskContext::Unknown,
        );
        let new_dir = tempfile::tempdir().unwrap();
        let new_context = crate::context::test_support::test_app_context_for_network(
            new_dir.path(),
            Network::Regtest,
        );
        assert_ne!(screen.app_context.network(), new_context.network());
        let mut wrapper = crate::ui::Screen::DPNSScreen(screen);
        wrapper.change_context(new_context);
        wrapper.refresh_on_arrival();
        let crate::ui::Screen::DPNSScreen(screen) = wrapper else {
            unreachable!()
        };
        assert!(screen.selected_votes.is_empty());
        assert!(!screen.show_bulk_schedule_popup);
        assert!(screen.pending_scheduled_actions.is_empty());
    }

    #[test]
    fn voting_ui_sweep_completion_refreshes_the_scheduled_row() {
        let (mut screen, _temp_dir) = voting_ui_review_fixture();
        screen.dpns_subscreen = DPNSSubscreen::ScheduledVotes;
        let mut target = screen.build_review_plan().unwrap().entries.remove(0).target;
        target.timing = VoteTiming::Scheduled(42);
        let mut operation = DpnsVoteOperation::new(vec![target]);
        operation.targets[0].status = DpnsVoteTargetStatus::Queued;
        screen
            .app_context
            .insert_dpns_vote_operation(&mut operation, None)
            .unwrap();
        screen.refresh();
        assert_eq!(
            screen.scheduled_votes.lock_recover()[0].status,
            DpnsVoteTargetStatus::Queued
        );
        operation.targets[0].status = DpnsVoteTargetStatus::Confirmed;
        screen
            .app_context
            .update_dpns_vote_operation(&operation)
            .unwrap();
        screen.display_task_result(BackendTaskSuccessResult::ScheduledVoteSweepCompleted {
            network: screen.app_context.network(),
            preserve_eligibility_since_ms: None,
        });
        assert_eq!(
            screen.scheduled_votes.lock_recover()[0].status,
            DpnsVoteTargetStatus::Confirmed
        );
    }

    /// The review sheet must expand every node × contest pair, suppress the
    /// targets that already hold the requested choice, and count what it dropped.
    #[tokio::test]
    async fn review_plan_expands_every_node_and_contest_and_suppresses_no_ops() {
        let (ctx, _temp_dir) = kv_ctx();
        let alpha = ctx.dpns_vote_poll_id("alpha").expect("alpha poll id");
        let beta = ctx.dpns_vote_poll_id("beta").expect("beta poll id");
        let first = masternode_identity(1, "node-one", true, ctx.network());
        let second = masternode_identity(2, "node-two", true, ctx.network());
        ctx.seed_proved_dpns_votes_for_test(
            first.identity.id(),
            std::collections::BTreeMap::from([(alpha.to_buffer(), ResourceVoteChoice::Lock)]),
        )
        .await
        .expect("seed first node's complete proved votes");
        ctx.seed_proved_dpns_votes_for_test(
            second.identity.id(),
            std::collections::BTreeMap::from([(beta.to_buffer(), ResourceVoteChoice::Abstain)]),
        )
        .await
        .expect("seed second node's complete proved votes");

        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::Active);
        screen.voting_identities = vec![first.clone(), second.clone()];
        screen.bulk_identity_options = vec![
            VoteOption::CastNow,
            VoteOption::Scheduled {
                days: 1,
                hours: 2,
                minutes: 3,
            },
        ];
        screen.selected_votes = vec![
            SelectedVote {
                contested_name: "alpha".to_owned(),
                vote_choice: ResourceVoteChoice::Lock,
                end_time: None,
            },
            SelectedVote {
                contested_name: "beta".to_owned(),
                vote_choice: ResourceVoteChoice::Abstain,
                end_time: None,
            },
        ];
        screen.vote_state = DpnsVoteStateSnapshot::load(
            &ctx,
            &[first.identity.id(), second.identity.id()],
            &[alpha, beta],
        )
        .expect("load proved vote state");

        let plan = screen.build_review_plan().expect("review plan");

        assert_eq!(plan.entries.len(), 4, "two nodes across two contests");
        assert_eq!(plan.effective_count(), 2);
        assert_eq!(plan.no_op_count(), 2);
        assert_eq!(plan.node_count(), 2);
        assert!(plan.has_immediate());
        assert!(plan.has_scheduled());
        let retained = plan
            .effective()
            .map(|entry| {
                (
                    entry.node_label(),
                    entry.target.contested_name.clone(),
                    entry.timing_label.clone(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            retained,
            vec![
                (
                    "node-one".to_owned(),
                    "beta".to_owned(),
                    "Cast now".to_owned()
                ),
                (
                    "node-two".to_owned(),
                    "alpha".to_owned(),
                    "Scheduled in 1 d 2 h 3 min".to_owned()
                ),
            ]
        );
    }

    /// A review whose every target is a no-op must submit nothing.
    #[test]
    fn review_plan_reports_an_all_no_op_selection_as_nothing_to_submit() {
        let (ctx, _temp_dir) = kv_ctx();
        let alpha = ctx.dpns_vote_poll_id("alpha").expect("alpha poll id");
        let node = masternode_identity(3, "node-three", true, ctx.network());
        ctx.cache_confirmed_dpns_vote(node.identity.id(), alpha, ResourceVoteChoice::Lock)
            .expect("seed proved vote");

        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::Active);
        screen.voting_identities = vec![node.clone()];
        screen.bulk_identity_options = vec![VoteOption::CastNow];
        screen.selected_votes = vec![SelectedVote {
            contested_name: "alpha".to_owned(),
            vote_choice: ResourceVoteChoice::Lock,
            end_time: None,
        }];
        screen.vote_state = DpnsVoteStateSnapshot::load(&ctx, &[node.identity.id()], &[alpha])
            .expect("load proved vote state");

        let plan = screen.build_review_plan().expect("review plan");

        assert_eq!(plan.effective_count(), 0);
        assert_eq!(plan.no_op_count(), 1);
        assert_eq!(plan.node_count(), 0);
    }

    /// A node that already voted differently keeps its proved choice on the
    /// review line, so the operator sees what the vote replaces.
    #[test]
    fn review_plan_carries_the_proved_choice_a_target_replaces() {
        let (ctx, _temp_dir) = kv_ctx();
        let alpha = ctx.dpns_vote_poll_id("alpha").expect("alpha poll id");
        let node = masternode_identity(4, "node-four", true, ctx.network());
        ctx.cache_confirmed_dpns_vote(node.identity.id(), alpha, ResourceVoteChoice::Lock)
            .expect("seed proved vote");

        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::Active);
        screen.voting_identities = vec![node.clone()];
        screen.bulk_identity_options = vec![VoteOption::CastNow];
        screen.selected_votes = vec![SelectedVote {
            contested_name: "alpha".to_owned(),
            vote_choice: ResourceVoteChoice::Abstain,
            end_time: None,
        }];
        screen.vote_state = DpnsVoteStateSnapshot::load(&ctx, &[node.identity.id()], &[alpha])
            .expect("load proved vote state");

        let plan = screen.build_review_plan().expect("review plan");

        assert_eq!(plan.effective_count(), 1);
        assert_eq!(
            plan.effective()
                .next()
                .map(|entry| entry.target.current_choice),
            Some(Some(ResourceVoteChoice::Lock))
        );
        assert_eq!(
            review_current_choice_label(Some(ResourceVoteChoice::Lock), None),
            "Lock"
        );
    }

    #[test]
    fn review_lines_name_the_node_the_contest_the_choices_and_the_timing() {
        let line = review_target_line("node-one", "alice", "Lock", "Not voted yet", "Cast now");

        assert_eq!(
            line,
            "• node-one → alice.dash: Lock. Current vote: Not voted yet. Cast now."
        );
        assert_eq!(
            review_headline(6, 3),
            "Votes to submit (6) from your nodes (3)."
        );
        assert!(review_skipped_line(2).contains("(2)"));
        assert_eq!(
            review_current_choice_label(Some(ResourceVoteChoice::Abstain), None),
            "Abstain"
        );
        assert_eq!(review_current_choice_label(None, None), "Not voted yet");
    }

    /// Removing a scheduled vote must clear its row. The journal cancellation is
    /// durable, so the row cannot come back with a Remove button that does nothing.
    #[test]
    fn removing_a_scheduled_vote_drops_its_row_from_the_table() {
        let (ctx, _temp_dir) = kv_ctx();
        let voter_id = Identifier::from([23; 32]);
        let key = DpnsVoteTargetKey {
            network: ctx.network(),
            voter_id,
            vote_poll_id: Identifier::from([24; 32]),
        };
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: key.clone(),
            voter_alias: None,
            contested_name: "remove-me".to_owned(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Scheduled(42),
        }]);
        ctx.insert_dpns_vote_operation(&mut operation, None)
            .expect("insert scheduled operation");
        ctx.insert_scheduled_votes(&[ScheduledDPNSVote {
            contested_name: "remove-me".to_owned(),
            voter_id,
            choice: ResourceVoteChoice::Lock,
            unix_timestamp: 42,
            executed_successfully: false,
        }])
        .expect("insert compatibility mirror");
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::ScheduledVotes);
        assert_eq!(screen.scheduled_votes.lock_recover().len(), 1);

        ctx.cancel_scheduled_dpns_vote_target(operation.id, &key, "remove-me")
            .expect("remove the scheduled vote");
        screen.display_task_result(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: ctx.network(),
            operation_id: operation.id,
        });

        assert!(
            screen.scheduled_votes.lock_recover().is_empty(),
            "a removed schedule must not return to the table"
        );
    }

    #[test]
    fn clear_all_result_rebuilds_scheduled_rows_immediately() {
        let (ctx, _temp_dir) = kv_ctx();
        let voter_id = Identifier::from([21; 32]);
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: ctx.network(),
                voter_id,
                vote_poll_id: Identifier::from([22; 32]),
            },
            voter_alias: None,
            contested_name: "clear-me".to_owned(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Scheduled(42),
        }]);
        ctx.insert_dpns_vote_operation(&mut operation, None)
            .expect("insert scheduled operation");
        ctx.insert_scheduled_votes(&[ScheduledDPNSVote {
            contested_name: "clear-me".to_owned(),
            voter_id,
            choice: ResourceVoteChoice::Lock,
            unix_timestamp: 42,
            executed_successfully: false,
        }])
        .expect("insert compatibility mirror");
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::ScheduledVotes);
        assert_eq!(screen.scheduled_votes.lock_recover().len(), 1);

        let outcomes = ctx
            .clear_all_scheduled_dpns_votes()
            .expect("clear scheduled votes");
        screen.display_task_result(BackendTaskSuccessResult::ScheduledVotesCleared(outcomes));

        assert!(screen.scheduled_votes.lock_recover().is_empty());
    }
}
