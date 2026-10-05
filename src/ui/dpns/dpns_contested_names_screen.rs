mod scheduled_vote_editor;
use scheduled_vote_editor::{EditOutcome, ScheduledVoteEditor};

use crate::wallet_backend::poison::MutexRecover;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, LocalResult, TimeZone, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Button, ComboBox, Label, RichText, Ui};
use egui_extras::{Column, TableBuilder};

use crate::app::{AppAction, scheduled_vote_sweep_is_quiet};
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::contested_names::{DpnsVotingPreference, RelativeScheduleLabels};
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskContext};
use crate::context::AppContext;
use crate::model::contested_name::{ContestState, ContestedName};
use crate::model::dpns::normalize_dpns_label;
use crate::model::dpns::{contest_durations, urgency_window};
use crate::model::dpns_voting::composer::{
    AggregatePlan, BatchTiming, ComposeError, ComposerNode, Decision, NodeStanding, NodeTiming,
    SkipReason, compose,
};
use crate::model::dpns_voting::operator::NodeExclusion;
use crate::model::dpns_voting::operator::{
    ChangesLeft, NodeSet, ResolvedNodeSet, VotingNode, VotingNodeKind, relative_schedule_preset,
    time_left,
};
use crate::model::dpns_voting::progress::needs_attention;
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsScheduledVoteKey, DpnsVoteFailure, DpnsVoteOperation,
    DpnsVoteOperationId, DpnsVoteTarget, DpnsVoteTargetKey, DpnsVoteTargetStatus, VoteTiming,
    dpns_schedule_is_overdue,
};
use crate::model::qualified_identity::IdentityType;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::utc_schedule_input::UtcScheduleInput;
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::dpns::contest_card::node_label;
use crate::ui::dpns::contest_card::{CardEvent, CardView};
use crate::ui::dpns::copy::needs_attention_line;
use crate::ui::dpns::copy::tray_label;
use crate::ui::dpns::copy::{
    LOCKED_FOR_GOOD, before_end_phrase, changes_left_label, confirm_button_label,
    confirm_change_warning, confirm_title, confirm_transactions_line, decision_row, ends_in_label,
    relative_schedule_label, scheduled_nodes_label, skipped_header, skipped_reason_line,
    went_to_label,
};
use crate::ui::dpns::copy::{ends_soon_now_line, excluded_nodes_line};
use crate::ui::dpns::node_set_picker;
use crate::ui::dpns::progress_drawer;
use crate::ui::state::dpns_contests::ActiveDpnsContestSnapshot;
use crate::ui::state::dpns_vote_cards::{
    CantVoteReason, CardPlacement, NodeContestState, NodeContestStatus, SHORTCUT_HELP, Shortcut,
    VoteCard, move_focus, shortcut_for, sort_by_time_left,
};
use crate::ui::state::dpns_vote_operations::{
    DpnsVoteOperationSnapshot, ScheduledDpnsVoteRow, group_scheduled_rows,
};
use crate::ui::state::dpns_vote_state::DpnsVoteStateSnapshot;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::{BackendTaskSuccessResult, MessageType, ScreenLike};
use crate::utils::time::now_ms;

pub use super::VotesView;

const NO_OPEN_CONTESTS_MESSAGE: &str =
    "There are no open name contests right now. New contests appear here automatically.";
const CANT_VOTE_REASON: &str = "None of the nodes you vote with has a voting key that is in the masternode list. Load a voting key or change the nodes you vote with.";
const NO_VOTING_NODES_MESSAGE: &str = "No masternodes are loaded.";
const NO_VOTING_NODES_DETAIL: &str = "Load a masternode with its voting key to cast votes.";
const JOURNAL_UNAVAILABLE_MESSAGE: &str = crate::ui::dpns::copy::JOURNAL_UNAVAILABLE_MESSAGE;
const MISSED_SCHEDULE_GUIDANCE: &str = "The automatic voting time was missed. On the Scheduled tab, use Cast now to vote, Edit to reschedule, or Remove to cancel.";
const SCHEDULE_IN_FUTURE_MESSAGE: &str =
    "Choose a future date and time before scheduling these votes.";
const KEEP_RUNNING_MESSAGE: &str = "Keep Dash Evo Tool running and connected until the scheduled time, or the scheduled votes will not be cast.";

/// An absolute UTC time next to how far away it is, e.g.
/// `2026-01-02 03:04:05 (in 2 days)`.
fn timestamp_with_relative(date_time: DateTime<Utc>) -> String {
    let distance = HumanTime::from(date_time).to_string();
    let relative = if distance.contains("seconds") {
        "now"
    } else {
        &distance
    };
    format!(
        "{absolute} ({relative})",
        absolute = date_time.format("%Y-%m-%d %H:%M:%S"),
    )
}

fn schedule_is_missed(status: DpnsVoteTargetStatus, timing: VoteTiming, now_ms: u64) -> bool {
    status == DpnsVoteTargetStatus::Scheduled
        && matches!(timing, VoteTiming::Scheduled(timestamp) if dpns_schedule_is_overdue(timestamp, now_ms))
}

fn short_identifier(identifier: Identifier) -> String {
    crate::model::identity_name::shorten_id(&identifier.to_string(Encoding::Base58))
}

use super::copy::{target_status_label, vote_choice_label};

/// Candidate display names resolved once per refresh: contest → candidate → name.
///
/// The render path is immediate-mode, so it looks candidates up here instead of
/// rescanning the contested-name cache — which grows without bound — on every
/// frame, for every row.
type CandidateNameIndex = BTreeMap<String, BTreeMap<Identifier, String>>;

fn candidate_name_index(
    active_contests: &ActiveDpnsContestSnapshot,
    contested_names: &[ContestedName],
) -> CandidateNameIndex {
    let active = active_contests.contests();
    let mut index = CandidateNameIndex::new();
    // Active contests are the fresher source, so they are indexed last and win.
    let contests = contested_names
        .iter()
        .chain(active.iter().map(|view| view.contest.as_ref()));
    for contest in contests {
        let Some(candidates) = contest.contestants.as_ref() else {
            continue;
        };
        let entry = index
            .entry(contest.normalized_contested_name.clone())
            .or_default();
        for candidate in candidates {
            entry.insert(candidate.id, candidate.name.clone());
        }
    }
    index
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
            "Current vote could not be verified. On the Scheduled tab, use Cast now to check again or Edit to reschedule.",
        ),
        DpnsVoteFailure::SubmissionFailed => Some(
            "The vote could not be submitted. Check your connection and voting key, then use Cast now or Edit on the Scheduled tab.",
        ),
        DpnsVoteFailure::VotingKeyMissing => Some(
            "This node's voting key is not loaded. Load it on the Nodes tab, then use Cast now on the Scheduled tab.",
        ),
        DpnsVoteFailure::PlatformRejected
        | DpnsVoteFailure::ResultUnconfirmed
        | DpnsVoteFailure::VotingEnded => None,
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
        && (status.is_reviewable_failure()
            || matches!(
                status,
                DpnsVoteTargetStatus::Scheduled | DpnsVoteTargetStatus::Cancelled
            ))
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

/// Exactly what a confirm click would send, resolved before the confirm step
/// is shown.
struct ReviewPlan {
    aggregate: AggregatePlan,
    /// One line per staged decision: name, choice, and the nodes casting it.
    decisions: Vec<(Decision, usize)>,
}

impl ReviewPlan {
    /// The targets that will really be submitted.
    fn targets(&self) -> &[DpnsVoteTarget] {
        &self.aggregate.targets
    }

    fn effective_count(&self) -> usize {
        self.aggregate.targets.len()
    }

    fn node_count(&self) -> usize {
        self.aggregate.node_count()
    }

    fn has_immediate(&self) -> bool {
        self.targets()
            .iter()
            .any(|target| matches!(target.timing, VoteTiming::Now))
    }

    fn has_scheduled(&self) -> bool {
        self.targets()
            .iter()
            .any(|target| matches!(target.timing, VoteTiming::Scheduled(_)))
    }
}

/// The batch timing picked in the confirm step (VOTE-FR-023).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfirmTiming {
    #[default]
    Now,
    BeforeEnd,
    At,
}

impl ConfirmTiming {
    const ALL: [Self; 3] = [Self::Now, Self::BeforeEnd, Self::At];

    fn label(self) -> &'static str {
        match self {
            Self::Now => "Now",
            Self::BeforeEnd => "When voting is about to end",
            Self::At => "At a specific time",
        }
    }
}

/// A per-node choice under `Adjust nodes`.
fn node_timing_label(timing: NodeTiming) -> &'static str {
    match timing {
        NodeTiming::Batch => "Same as above",
        NodeTiming::Override(BatchTiming::Now) => "Now",
        NodeTiming::Override(BatchTiming::BeforeEnd(_)) => "When voting is about to end",
        NodeTiming::Override(BatchTiming::At(_)) => "At a specific time",
        NodeTiming::DontUse => "Don't use this node",
    }
}

fn utc_minute(timestamp: u64) -> String {
    DateTime::from_timestamp_millis(timestamp as i64)
        .map(|date| date.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

fn review_current_choice_label(
    current: Option<ResourceVoteChoice>,
    candidate_name: Option<&str>,
) -> String {
    match current {
        Some(choice) => vote_choice_label(choice, candidate_name),
        None => "Not voted yet".to_owned(),
    }
}

/// Why the reviewed selection cannot be turned into a submittable plan.
///
/// One variant per condition so a caller can match on the outcome instead of
/// comparing sentences; the copy lives in the `#[error(...)]` attributes.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ReviewPlanError {
    #[error("{}", SCHEDULE_IN_FUTURE_MESSAGE)]
    ScheduleIsNotInTheFuture,
    #[error("Choose a valid UTC date and time.")]
    ScheduleIsNotAValidTime,
    #[error("Choose a future time before the contest ends for {contested_name}.dash.")]
    ScheduleOutlastsContest { contested_name: String },
    #[error("This vote could not be prepared. Refresh the contests and try again.")]
    VotePollUnavailable {
        #[source]
        source: Arc<TaskError>,
    },
    #[error("Voting has ended for {contested_name}.dash. Clear it from your decisions.")]
    VotingEnded { contested_name: String },
}

impl From<ComposeError> for ReviewPlanError {
    fn from(error: ComposeError) -> Self {
        match error {
            ComposeError::ScheduleOutlastsContest { contested_name } => {
                Self::ScheduleOutlastsContest { contested_name }
            }
            ComposeError::VotingEnded { contested_name } => Self::VotingEnded { contested_name },
        }
    }
}

/// Why a submit click produced nothing to send.
#[derive(Debug, Clone, thiserror::Error)]
pub enum VoteSubmissionError {
    #[error(transparent)]
    Plan(#[from] ReviewPlanError),
    #[error("No votes selected. Choose at least one node and contest.")]
    NoTargets,
    #[error("Every selected node already has the requested vote. Nothing will be submitted.")]
    AllNoOps,
    /// A dispatched submission came back as a failure.
    ///
    /// Fieldless on purpose: `display_task_error` only borrows the `TaskError`,
    /// and the alternative — stashing its rendered sentence — is the very thing
    /// a typed error exists to avoid. The specific failure still reaches the
    /// user in full, through the global banner AppState raises for every failed
    /// task with the technical chain in its details.
    #[error(
        "The votes could not be submitted. Check your connection and voting key, then review and submit them again."
    )]
    TaskFailed,
}

fn platform_explorer_contest_url(network: Network, normalized_label: &str) -> Option<String> {
    use base64::Engine;

    let host = match network {
        Network::Mainnet => "platform-explorer.com",
        Network::Testnet => "testnet.platform-explorer.com",
        Network::Devnet | Network::Regtest => return None,
    };
    let resource = serde_json::json!(["dash", normalized_label]).to_string();
    let encoded = base64::engine::general_purpose::STANDARD.encode(resource);
    Some(format!(
        "https://{host}/contestedResource/{}",
        urlencoding::encode(&encoded)
    ))
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

/// Minimal object for storing the user’s currently selected vote on a single contested name.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectedVote {
    pub contested_name: String,
    pub vote_choice: ResourceVoteChoice,
    pub end_time: Option<u64>,
}

pub enum VoteHandlingStatus {
    NotStarted,
    CastingVotes,
    SchedulingVotes,
    Failed(VoteSubmissionError),
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

/// The Masternodes ▸ Votes panel: the single voting workspace.
///
/// One instance lives inside [`crate::ui::masternodes::MasternodesScreen`]; it
/// owns the contest cache, the composer and the scheduled-vote views.
pub struct DPNSScreen {
    voting_identities: Vec<QualifiedIdentity>,
    voting_identity_load_error: Option<TaskError>,
    contested_names: Arc<Mutex<Vec<ContestedName>>>,
    active_contests: ActiveDpnsContestSnapshot,
    /// Rebuilt from `active_contests` and `contested_names` whenever either is
    /// replaced. Keep the three in step: the render path reads only this.
    candidate_names: CandidateNameIndex,
    scheduled_votes: Arc<Mutex<Vec<ScheduledDpnsVoteRow>>>,
    pub selected_votes: Vec<SelectedVote>,
    pub app_context: Arc<AppContext>,
    pending_backend_task: Option<BackendTask>,
    /// A preference to persist through the backend once no user action is out.
    pending_preference: Option<DpnsVotingPreference>,
    vote_operations: DpnsVoteOperationSnapshot,
    vote_state: DpnsVoteStateSnapshot,
    pending_vote_operation: Option<DpnsVoteOperationId>,
    pending_scheduled_actions: BTreeMap<DpnsScheduledVoteKey, BackendTaskContext>,
    /// Set by `display_backend_task_error` when the failed task is this
    /// panel's pending submission; `display_task_error` then releases it.
    release_pending_on_error: bool,
    scheduled_clear_dialog: Option<(bool, ConfirmationDialog)>,
    scheduled_vote_editor: Option<ScheduledVoteEditor>,

    /// Sorting
    sort_column: SortColumn,
    sort_order: SortOrder,
    active_filter_term: String,
    past_filter_term: String,

    /// The visible Votes sub-view.
    pub view: VotesView,
    refreshing_status: RefreshingStatus,
    refresh_banner: Option<BannerHandle>,
    journal_error_banner: Option<BannerHandle>,

    /// Selected vote handling
    show_bulk_schedule_popup: bool,
    /// Per-node overrides under `Adjust nodes`; absent means "same as above".
    node_overrides: BTreeMap<Identifier, NodeTiming>,
    /// Set by `Review again`: the confirm step covers only this node.
    retry_voter: Option<Identifier>,
    bulk_vote_handling_status: VoteHandlingStatus,
    submission_error_banner: Option<BannerHandle>,
    confirm_timing: ConfirmTiming,
    /// Lead time for "When voting is about to end" (VOTE-FR-081).
    relative_preset: std::time::Duration,
    /// Eagerly built, unlike most components: its default is the construction
    /// time plus a day, and the read-only accessors below must see it.
    simple_schedule: UtcScheduleInput,
    /// Set when the operator asks to load a node; the hosting Masternodes
    /// screen consumes it and opens its load form.
    load_node_requested: bool,

    /// Node set for this session; starts from the saved default (VOTE-FR-075).
    node_set: NodeSet,
    /// Network `node_set` was loaded for; `None` forces a reload.
    node_set_network: Option<dash_sdk::dpp::dashcore::Network>,
    voting_nodes: Vec<VotingNode>,
    resolved_nodes: ResolvedNodeSet,
    node_labels: BTreeMap<Identifier, String>,
    /// Open contests as cards, sorted by time left. Rebuilt by `rebuild_cards`.
    cards: Vec<VoteCard>,
    /// "Before the end" presets of scheduled rows, by target and time; a row
    /// without one shows its absolute time only (VOTE-FR-081).
    relative_schedule_labels: BTreeMap<(DpnsVoteTargetKey, u64), std::time::Duration>,
    /// Settled failures older than this are not raised in `Needs attention`.
    session_started_ms: u64,
    /// Cards ticked for a bulk decision, by contest name.
    selected_cards: BTreeSet<String>,
    focused_card: Option<String>,
    /// Whether the contest list has keyboard focus for shortcuts.
    list_focused: bool,
}

impl DPNSScreen {
    pub fn new(app_context: &Arc<AppContext>, view: VotesView) -> Self {
        let vote_operations = DpnsVoteOperationSnapshot::load(app_context);
        let legacy_scheduled_votes = app_context.get_scheduled_votes().unwrap_or_default();
        let scheduled_votes = Arc::new(Mutex::new(
            vote_operations.scheduled_vote_rows(&legacy_scheduled_votes),
        ));

        // One instance serves every sub-view, so it loads every cache.
        let contested_names = Arc::new(Mutex::new(
            app_context.all_contested_names().unwrap_or_default(),
        ));
        let active_contests = ActiveDpnsContestSnapshot::new(
            app_context,
            app_context.ongoing_contested_names().unwrap_or_default(),
        );

        let candidate_names =
            candidate_name_index(&active_contests, &contested_names.lock_recover());

        let (voting_identities, voting_identity_load_error) =
            match loaded_voting_identities(app_context) {
                Ok(identities) => (identities, None),
                Err(error) => {
                    tracing::warn!(?error, "Could not load local DPNS voting identities");
                    (Vec::new(), Some(error))
                }
            };
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
        let default_schedule_time = Utc::now() + chrono::Duration::days(1);

        let mut screen = Self {
            voting_identities,
            voting_identity_load_error,
            contested_names,
            active_contests,
            candidate_names,
            scheduled_votes,
            selected_votes: Vec::new(),
            app_context: app_context.clone(),
            sort_column: SortColumn::ContestedName,
            sort_order: SortOrder::Ascending,
            active_filter_term: String::new(),
            past_filter_term: String::new(),
            pending_backend_task: None,
            pending_preference: None,
            vote_operations,
            vote_state,
            pending_vote_operation: None,
            pending_scheduled_actions: BTreeMap::new(),
            release_pending_on_error: false,
            session_started_ms: now_ms(),
            scheduled_clear_dialog: None,
            scheduled_vote_editor: None,
            view,
            refreshing_status: RefreshingStatus::NotRefreshing,
            refresh_banner: None,
            journal_error_banner: None,

            // Vote handling
            show_bulk_schedule_popup: false,
            node_overrides: BTreeMap::new(),
            retry_voter: None,
            bulk_vote_handling_status: VoteHandlingStatus::NotStarted,
            submission_error_banner: None,
            confirm_timing: ConfirmTiming::Now,
            relative_preset: relative_schedule_preset(app_context.network()),
            simple_schedule: UtcScheduleInput::new().with_time(default_schedule_time),
            load_node_requested: false,
            node_set: NodeSet::All,
            node_set_network: None,
            voting_nodes: Vec::new(),
            resolved_nodes: ResolvedNodeSet::default(),
            node_labels: BTreeMap::new(),
            cards: Vec::new(),
            relative_schedule_labels: BTreeMap::new(),
            selected_cards: BTreeSet::new(),
            focused_card: None,
            list_focused: false,
        };
        screen.rebuild_cards();
        screen.rebuild_scheduled_vote_rows();
        screen
    }

    /// Consume a pending "load a masternode" request from this panel.
    pub fn take_load_node_request(&mut self) -> bool {
        std::mem::take(&mut self.load_node_requested)
    }

    fn finish_scheduled_dispatch(&mut self, context: &BackendTaskContext) -> bool {
        let key = self
            .pending_scheduled_actions
            .iter()
            .find(|(_, pending)| *pending == context)
            .map(|(key, _)| key.clone());
        if let Some(key) = key {
            self.pending_scheduled_actions.remove(&key);
            true
        } else {
            false
        }
    }

    pub(crate) fn reset_for_network_switch(&mut self) {
        self.selected_votes.clear();
        self.show_bulk_schedule_popup = false;
        self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
        self.node_overrides.clear();
        self.retry_voter = None;
        self.confirm_timing = ConfirmTiming::Now;
        self.relative_preset = relative_schedule_preset(self.app_context.network());
        self.pending_backend_task = None;
        self.pending_preference = None;
        self.pending_vote_operation = None;
        self.pending_scheduled_actions.clear();
        self.release_pending_on_error = false;
        self.scheduled_clear_dialog = None;
        self.scheduled_vote_editor = None;
        self.refresh_banner.take_and_clear();
        self.journal_error_banner.take_and_clear();
        self.submission_error_banner.take_and_clear();
        self.refreshing_status = RefreshingStatus::NotRefreshing;
        self.voting_identities.clear();
        self.voting_identity_load_error = None;
        self.vote_state = DpnsVoteStateSnapshot::default();
        self.vote_operations = DpnsVoteOperationSnapshot::default();
        self.active_contests = ActiveDpnsContestSnapshot::default();
        self.candidate_names.clear();
        self.contested_names.lock_recover().clear();
        self.scheduled_votes.lock_recover().clear();
        self.node_set_network = None;
        self.voting_nodes.clear();
        self.resolved_nodes = ResolvedNodeSet::default();
        self.cards.clear();
        self.selected_cards.clear();
        self.focused_card = None;
        self.list_focused = false;
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
    fn no_voting_nodes_copy(&self) -> (&'static str, &'static str, &'static str) {
        if self.voting_nodes.is_empty() {
            (
                NO_VOTING_NODES_MESSAGE,
                NO_VOTING_NODES_DETAIL,
                "Load a masternode",
            )
        } else {
            (
                "None of your nodes has a voting key on this device.",
                "Add a voting key to vote. One key can serve several nodes.",
                "Add a voting key",
            )
        }
    }

    fn render_no_voting_nodes(&mut self, ui: &mut Ui) -> AppAction {
        let (heading, detail, button) = self.no_voting_nodes_copy();
        let action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;
        ui.vertical_centered(|ui| {
            ui.add_space(24.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_max_width(520.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(heading)
                            .strong()
                            .color(DashColors::warning_color(dark_mode)),
                    );
                    ui.label(RichText::new(detail).color(DashColors::text_primary(dark_mode)));
                    ui.add_space(12.0);
                    if ComponentStyles::add_primary_button(ui, button).clicked() {
                        self.load_node_requested = true;
                    }
                });
            });
        });
        action
    }

    fn render_empty_view(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;
        let (heading, detail) = match self.view {
            VotesView::ToDecide | VotesView::Voted => (NO_OPEN_CONTESTS_MESSAGE, None),
            VotesView::History => ("There are no finished name contests yet.", None),
            VotesView::Scheduled => (
                "No scheduled votes.",
                Some("Pick a decision in To decide, then choose a later time in the confirm step."),
            ),
        };
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.label(
                RichText::new(heading)
                    .heading()
                    .strong()
                    .color(DashColors::text_secondary(dark_mode)),
            );
            if let Some(detail) = detail {
                ui.add_space(10.0);
                ui.label(RichText::new(detail).color(DashColors::text_primary(dark_mode)));
            }
            if self.view != VotesView::Scheduled {
                ui.add_space(20.0);
                let refreshing = self.refreshing_status == RefreshingStatus::Refreshing;
                if ComponentStyles::add_primary_button_enabled(ui, !refreshing, "Refresh")
                    .disabled_tooltip("Contests are already being refreshed.")
                    .clicked()
                {
                    app_action = AppAction::BackendTask(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::QueryDPNSContests,
                    ));
                }
            }
        });
        app_action
    }

    // ---------------------------
    // Rendering: To decide, Voted, Scheduled, History
    // ---------------------------

    /// The loaded voting nodes plus key-less ones, as the node-set picker sees them.
    fn collect_voting_nodes(&self) -> Vec<VotingNode> {
        let mut nodes: Vec<VotingNode> = self
            .voting_identities
            .iter()
            .map(|identity| {
                let id = identity.identity.id();
                VotingNode {
                    id,
                    kind: if identity.identity_type == IdentityType::Evonode {
                        VotingNodeKind::Evonode
                    } else {
                        VotingNodeKind::Masternode
                    },
                    has_voting_key: true,
                    membership: self.app_context.masternode_list_membership(id),
                    alias: identity.alias.clone(),
                }
            })
            .collect();
        match self.app_context.dpns_voting_nodes() {
            Ok(all) => {
                let known: BTreeSet<Identifier> = nodes.iter().map(|node| node.id).collect();
                nodes.extend(
                    all.into_iter()
                        .filter(|node| !node.has_voting_key && !known.contains(&node.id)),
                );
            }
            Err(error) => tracing::debug!(?error, "Could not list key-less nodes for the node set"),
        }
        nodes
    }

    /// Rebuild the card view-model from contests, proved state, the journal and
    /// the node set. Call after any of them changes; rendering only reads it.
    pub(crate) fn rebuild_cards(&mut self) {
        let network = self.app_context.network();
        if self.node_set_network != Some(network) {
            self.node_set = self
                .app_context
                .saved_dpns_node_set()
                .unwrap_or_else(|error| {
                    tracing::debug!(
                        ?error,
                        "Could not read the saved node set; voting with all nodes"
                    );
                    NodeSet::All
                });
            self.node_set_network = Some(network);
        }
        self.voting_nodes = self.collect_voting_nodes();
        self.resolved_nodes = self.node_set.resolve(&self.voting_nodes);
        self.node_labels = self
            .voting_nodes
            .iter()
            .filter_map(|node| node.alias.clone().map(|alias| (node.id, alias)))
            .collect();
        let mut cards: Vec<VoteCard> = self
            .active_contests
            .contests()
            .iter()
            .map(|view| {
                let nodes = view.vote_poll_id.map_or_else(Vec::new, |poll| {
                    self.resolved_nodes
                        .included
                        .iter()
                        .map(|voter| {
                            let state = self.vote_state.state(*voter, poll);
                            let lock = self.vote_operations.target_status(&DpnsVoteTargetKey {
                                network,
                                voter_id: *voter,
                                vote_poll_id: poll,
                            });
                            let changes = self.app_context.dpns_changes_left(*voter, poll, state);
                            NodeContestState {
                                node: *voter,
                                status: NodeContestStatus::classify(state, lock, changes),
                                current: match state {
                                    DpnsCurrentVoteState::Available(current) => current,
                                    _ => None,
                                },
                                changes,
                            }
                        })
                        .collect()
                });
                VoteCard::new(view.contest.clone(), view.vote_poll_id, nodes)
            })
            .collect();
        sort_by_time_left(&mut cards);
        self.cards = cards;
        let names: BTreeSet<&str> = self.cards.iter().map(VoteCard::name).collect();
        self.selected_cards
            .retain(|name| names.contains(name.as_str()));
    }

    /// Votes a staged decision would send: submittable nodes not already on it.
    fn transaction_count(&self) -> usize {
        self.selected_votes
            .iter()
            .filter_map(|vote| {
                let card = self
                    .cards
                    .iter()
                    .find(|card| card.name() == vote.contested_name)?;
                Some(
                    card.nodes
                        .iter()
                        .filter(|node| node.status.can_submit())
                        .filter(|node| node.current != Some(vote.vote_choice))
                        .count(),
                )
            })
            .sum()
    }

    /// Show one contest by normalized label: the view holding its card, the
    /// filter set to the label and the card focused.
    pub(crate) fn show_contest(&mut self, label: String) {
        self.view = match self
            .cards
            .iter()
            .find(|card| card.name() == label)
            .map(|card| card.placement)
        {
            Some(CardPlacement::Voted) => VotesView::Voted,
            _ => VotesView::ToDecide,
        };
        self.active_filter_term = label.clone();
        self.focused_card = Some(label);
        self.list_focused = true;
    }

    /// Give up list keyboard focus, e.g. when the Votes segment is left.
    pub(crate) fn release_list_focus(&mut self) {
        self.list_focused = false;
    }

    /// Vote with one node for this session (VOTE-FR-076): To decide opens with
    /// that node as the node set and no staged decisions carried over.
    pub fn vote_with_node(&mut self, node: Identifier) {
        self.selected_votes.clear();
        self.selected_cards.clear();
        self.view = VotesView::ToDecide;
        // The node may have been loaded after this panel last read its nodes.
        self.refresh_on_arrival();
        self.apply_node_set(NodeSet::Custom(BTreeSet::from([node])));
    }

    fn apply_node_set(&mut self, node_set: NodeSet) {
        self.node_set = node_set;
        self.rebuild_cards();
    }

    /// The `Vote with:` chip and its popover, when any node is loaded.
    fn render_node_set_chip(&mut self, ui: &mut Ui) {
        if self.voting_nodes.is_empty() {
            return;
        }
        let picked =
            node_set_picker::show(ui, &self.node_set, &self.resolved_nodes, &self.voting_nodes);
        if picked.save_default {
            self.pending_preference = Some(DpnsVotingPreference::NodeSet(self.node_set.clone()));
        }
        if let Some(node_set) = picked.changed {
            self.apply_node_set(node_set);
        }
    }

    /// Render To decide or Voted: node-set chip, filter, cards and the tray.
    fn render_active_contests(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        ui.horizontal(|ui| {
            self.render_node_set_chip(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let shortcuts = ui.button("Shortcuts");
                egui::Popup::from_toggle_button_response(&shortcuts)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| {
                        ui.label(
                            RichText::new("Shortcuts work while the list of names has focus.")
                                .strong(),
                        );
                        egui::Grid::new("dpns_vote_shortcuts").show(ui, |ui| {
                            for (keys, action) in SHORTCUT_HELP {
                                ui.label(RichText::new(keys).monospace());
                                ui.label(action);
                                ui.end_row();
                            }
                        });
                    });
                let filter = ui
                    .add(
                        egui::TextEdit::singleline(&mut self.active_filter_term)
                            .hint_text("Filter by name")
                            .desired_width(180.0),
                    )
                    .on_hover_text(
                        "The letters i and l match the digit 1, and the letter o matches 0.",
                    );
                if filter.has_focus() {
                    self.list_focused = false;
                }
            });
        });
        ui.add_space(8.0);

        let filter = normalize_dpns_label(&self.active_filter_term);
        let view = self.view;
        let now = crate::utils::time::now_ms();
        let network = self.app_context.network();
        let durations = contest_durations(network, self.app_context.platform_version());
        let urgency = urgency_window(network);
        let has_voting_nodes = !self.resolved_nodes.included.is_empty();
        let matches_filter =
            |card: &VoteCard| filter.is_empty() || card.name().to_lowercase().contains(&filter);
        let listed: Vec<usize> = self
            .cards
            .iter()
            .enumerate()
            .filter(|(_, card)| matches_filter(card))
            .filter(|(_, card)| match view {
                VotesView::Voted => card.placement == CardPlacement::Voted,
                // Unproved state is temporary and fixed by a refresh, so those
                // cards stay in view rather than in the collapsed group.
                _ => matches!(
                    card.placement,
                    CardPlacement::ToDecide
                        | CardPlacement::CantVote(CantVoteReason::VoteStateUnavailable)
                ),
            })
            .map(|(index, _)| index)
            .collect();
        let cant_vote: Vec<usize> = if view == VotesView::ToDecide {
            self.cards
                .iter()
                .enumerate()
                .filter(|(_, card)| matches_filter(card))
                .filter(|(_, card)| {
                    card.placement == CardPlacement::CantVote(CantVoteReason::NoVotingNodes)
                })
                .map(|(index, _)| index)
                .collect()
        } else {
            Vec::new()
        };

        let scroll_to = self.handle_shortcuts(ui, &listed);
        self.render_bulk_bar(ui);

        let mut events: Vec<(usize, CardEvent)> = Vec::new();
        let list = egui::ScrollArea::vertical()
            .id_salt("active_contest_cards")
            .max_height(ui.available_height() - 48.0)
            .show(ui, |ui| {
                if listed.is_empty() {
                    let empty = match view {
                        VotesView::Voted => "Your nodes haven't voted on any open contest yet.",
                        _ => "Nothing needs your vote right now.",
                    };
                    ui.label(RichText::new(empty).color(DashColors::text_secondary(dark_mode)));
                }
                let render = |ui: &mut Ui, index: usize, events: &mut Vec<(usize, CardEvent)>| {
                    let card = &self.cards[index];
                    let staged = self
                        .selected_votes
                        .iter()
                        .find(|vote| vote.contested_name == card.name())
                        .map(|vote| vote.vote_choice);
                    let card_events = CardView {
                        card,
                        staged,
                        selected: self.selected_cards.contains(card.name()),
                        focused: self.focused_card.as_deref() == Some(card.name()),
                        scroll_into_view: scroll_to.as_deref() == Some(card.name()),
                        node_labels: &self.node_labels,
                        node_set_weight: self.resolved_nodes.weight,
                        now_ms: now,
                        urgency,
                        durations,
                        has_voting_nodes,
                    }
                    .show(ui);
                    events.extend(card_events.into_iter().map(|event| (index, event)));
                    ui.add_space(8.0);
                };
                for index in &listed {
                    render(ui, *index, &mut events);
                }
                if !cant_vote.is_empty() {
                    egui::CollapsingHeader::new(format!(
                        "Can't vote with your nodes ({count})",
                        count = cant_vote.len()
                    ))
                    .default_open(!has_voting_nodes)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(CANT_VOTE_REASON)
                                .color(DashColors::text_secondary(dark_mode)),
                        );
                        for index in &cant_vote {
                            render(ui, *index, &mut events);
                        }
                    });
                }
            });

        // The list owns keyboard focus after a click inside it (VOTE-NFR-010).
        if let Some(pos) = ui.input(|input| {
            input
                .pointer
                .any_pressed()
                .then(|| input.pointer.interact_pos())
                .flatten()
        }) {
            self.list_focused = list.inner_rect.contains(pos);
        }
        if !events.is_empty() {
            self.list_focused = true;
        }
        for (index, event) in events {
            let name = self.cards[index].name().to_owned();
            match event {
                CardEvent::Choose(choice) => {
                    let contest = Arc::clone(&self.cards[index].contest);
                    self.set_selected_vote(&contest, choice);
                }
                CardEvent::ToggleSelected => {
                    if !self.selected_cards.remove(&name) {
                        self.selected_cards.insert(name.clone());
                    }
                }
                CardEvent::RefreshVoting
                    if self.refreshing_status != RefreshingStatus::Refreshing =>
                {
                    self.pending_backend_task = Some(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::QueryDPNSContests,
                    ));
                }
                CardEvent::RefreshVoting => {}
            }
            self.focused_card = Some(name);
        }

        self.render_tray(ui);
    }

    /// Stage `choice` for a contest (never toggles it off).
    fn stage_choice(&mut self, contest: &ContestedName, choice: ResourceVoteChoice) {
        match self
            .selected_votes
            .iter_mut()
            .find(|vote| vote.contested_name == contest.normalized_contested_name)
        {
            Some(vote) => vote.vote_choice = choice,
            None => self.selected_votes.push(SelectedVote {
                contested_name: contest.normalized_contested_name.clone(),
                vote_choice: choice,
                end_time: contest.end_time,
            }),
        }
    }

    fn clear_choice(&mut self, contested_name: &str) {
        self.selected_votes
            .retain(|vote| vote.contested_name != contested_name);
    }

    /// Apply this frame's list shortcuts (VOTE-FR-082). Active only while the
    /// list has focus and no text field wants the keyboard (VOTE-NFR-010).
    /// Returns the card to scroll into view when focus moved.
    fn handle_shortcuts(&mut self, ui: &Ui, listed: &[usize]) -> Option<String> {
        if !self.list_focused
            || self.show_bulk_schedule_popup
            || ui.ctx().egui_wants_keyboard_input()
        {
            return None;
        }
        let shortcuts: Vec<Shortcut> = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.is_none() => shortcut_for(*key),
                    _ => None,
                })
                .collect()
        });
        let mut scroll_to = None;
        for shortcut in shortcuts {
            let position = listed
                .iter()
                .position(|index| Some(self.cards[*index].name()) == self.focused_card.as_deref());
            let focused = position.map(|position| listed[position]);
            match shortcut {
                Shortcut::Next | Shortcut::Previous => {
                    if let Some(next) = move_focus(position, listed.len(), shortcut) {
                        let name = self.cards[listed[next]].name().to_owned();
                        scroll_to = Some(name.clone());
                        self.focused_card = Some(name);
                    }
                }
                Shortcut::Contender(_) | Shortcut::Lock | Shortcut::Abstain => {
                    let Some(index) = focused else { continue };
                    let card = &self.cards[index];
                    if !card.accepts_decision() {
                        continue;
                    }
                    let choice = match shortcut {
                        Shortcut::Lock => Some(ResourceVoteChoice::Lock),
                        Shortcut::Abstain => Some(ResourceVoteChoice::Abstain),
                        Shortcut::Contender(n) => card
                            .contest
                            .contestants
                            .as_ref()
                            .and_then(|contenders| contenders.get(n))
                            .map(|contender| ResourceVoteChoice::TowardsIdentity(contender.id)),
                        _ => None,
                    };
                    if let Some(choice) = choice {
                        let contest = Arc::clone(&card.contest);
                        self.stage_choice(&contest, choice);
                    }
                }
                Shortcut::Clear => {
                    if let Some(index) = focused {
                        let name = self.cards[index].name().to_owned();
                        self.clear_choice(&name);
                    }
                }
                Shortcut::ToggleSelected => {
                    if let Some(index) = focused {
                        let name = self.cards[index].name().to_owned();
                        if !self.selected_cards.remove(&name) {
                            self.selected_cards.insert(name);
                        }
                    }
                }
                Shortcut::Confirm => {
                    if !self.selected_votes.is_empty() {
                        self.open_review_for_node_set();
                    }
                }
            }
        }
        scroll_to
    }

    /// Bulk bar for two or more selected names (VOTE-FR-082).
    fn render_bulk_bar(&mut self, ui: &mut Ui) {
        let selected = self.selected_cards.len();
        if selected < 2 {
            return;
        }
        let mut apply: Option<Option<ResourceVoteChoice>> = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{selected} names selected")).strong());
            if ui.button("Lock name").clicked() {
                apply = Some(Some(ResourceVoteChoice::Lock));
            }
            if ui.button("Abstain").clicked() {
                apply = Some(Some(ResourceVoteChoice::Abstain));
            }
            if ui.button("Clear").clicked() {
                apply = Some(None);
            }
        });
        let Some(apply) = apply else {
            return;
        };
        let targets: Vec<Arc<ContestedName>> = self
            .cards
            .iter()
            .filter(|card| self.selected_cards.contains(card.name()))
            .filter(|card| apply.is_none() || card.accepts_decision())
            .map(|card| Arc::clone(&card.contest))
            .collect();
        for contest in targets {
            match apply {
                Some(choice) => self.stage_choice(&contest, choice),
                None => self.clear_choice(&contest.normalized_contested_name),
            }
        }
    }

    /// The tray under the list: what is staged and the Cast action (VOTE-FR-086).
    fn render_tray(&mut self, ui: &mut Ui) {
        let decisions = self.selected_votes.len();
        if decisions == 0 {
            return;
        }
        let transactions = self.transaction_count();
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new(tray_label(decisions, transactions)).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ComponentStyles::add_primary_button_enabled(ui, transactions > 0, "Cast")
                    .disabled_tooltip(
                        "Your nodes already hold these choices. Pick a different decision.",
                    )
                    .clicked()
                {
                    self.open_review_for_node_set();
                }
                if ui.button("Clear").clicked() {
                    self.selected_votes.clear();
                }
            });
        });
    }

    /// Open the confirm step for the staged decisions across the node set.
    fn open_review_for_node_set(&mut self) {
        self.retry_voter = None;
        self.node_overrides.clear();
        self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
        self.show_bulk_schedule_popup = true;
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
        self.retry_voter = Some(outcome.target.key.voter_id);
        self.node_overrides.clear();
        self.confirm_timing = ConfirmTiming::Now;
        self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
        self.show_bulk_schedule_popup = true;
    }

    /// `Needs attention` row (VOTE-FR-084): unconfirmed, failed or missed
    /// targets, linking to the drawer or Scheduled.
    fn render_needs_attention(&mut self, ui: &mut Ui) {
        let attention = needs_attention(
            self.vote_operations.operations(),
            self.session_started_ms,
            &self.app_context.dismissed_dpns_vote_operations(),
            now_ms(),
        );
        if attention.is_empty() {
            return;
        }
        let dark_mode = ui.visuals().dark_mode;
        egui::Frame::group(ui.style())
            .stroke(egui::Stroke::new(1.0, DashColors::warning_color(dark_mode)))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(needs_attention_line(attention))
                            .color(DashColors::warning_color(dark_mode)),
                    );
                    if attention.checking + attention.failed > 0
                        && ui.button("Show progress").clicked()
                    {
                        self.show_vote_progress(ui.ctx());
                    }
                    if attention.missed_schedules > 0 && ui.button("Open Scheduled").clicked() {
                        self.view = VotesView::Scheduled;
                    }
                });
            });
        ui.add_space(6.0);
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

    /// Progress shows in the non-blocking drawer, never a window overlay
    /// (VOTE-FR-035/083).
    fn show_vote_progress(&self, ctx: &egui::Context) {
        progress_drawer::expand(ctx);
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
                .column(Column::remainder()) // Outcome
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
                                if let Some(url) = platform_explorer_contest_url(
                                    self.app_context.network(),
                                    &contested_name.normalized_contested_name,
                                ) {
                                    ui.hyperlink_to("View in Platform Explorer", url)
                                        .on_hover_text("Open this contest in your web browser.");
                                }
                            });
                            // Ended Time
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                if let Some(ended_time) = contested_name.end_time {
                                    if let LocalResult::Single(dt) =
                                        Utc.timestamp_millis_opt(ended_time as i64)
                                    {
                                        ui.label(
                                            RichText::new(timestamp_with_relative(dt))
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
                                            .map(|contestant| contestant.name.as_str());
                                        ui.label(went_to_label(
                                            winner,
                                            &short_identifier(identifier),
                                        ));
                                        if ui.small_button("Copy ID").clicked() {
                                            ui.ctx()
                                                .copy_text(identifier.to_string(Encoding::Base58));
                                        }
                                    }
                                    ContestState::Locked => {
                                        ui.label(
                                            RichText::new(LOCKED_FOR_GOOD)
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

    /// Scheduled view grouped by decision (VOTE-FR-088): one row per name ×
    /// choice × time with an expandable node list carrying per-node status
    /// and actions.
    fn render_table_scheduled_votes(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut show_cast_progress = false;
        let rows = self.scheduled_votes.lock_recover().clone();
        let mut groups = group_scheduled_rows(rows);
        if self.sort_order == SortOrder::Descending {
            groups.reverse();
        }
        let now = Utc::now().timestamp_millis().max(0) as u64;
        egui::ScrollArea::both().show(ui, |ui| {
            for (index, group) in groups.iter().enumerate() {
                let dark_mode = ui.visuals().dark_mode;
                let choice = vote_choice_label(
                    group.choice,
                    self.candidate_name(&group.contested_name, group.choice),
                );
                let preset = group.rows.iter().find_map(|row| {
                    let (_, key) = row.journal_target.as_ref()?;
                    self.relative_schedule_labels
                        .get(&(key.clone(), group.unix_timestamp))
                });
                let when = match preset {
                    Some(preset) => {
                        relative_schedule_label(*preset, &utc_minute(group.unix_timestamp))
                    }
                    None => match Utc.timestamp_millis_opt(group.unix_timestamp as i64) {
                        LocalResult::Single(date) => timestamp_with_relative(date),
                        _ => "Invalid timestamp".to_owned(),
                    },
                };
                let needs_attention = group.rows.iter().any(|row| {
                    schedule_is_missed(
                        row.status,
                        VoteTiming::Scheduled(row.vote.unix_timestamp),
                        now,
                    ) || scheduled_failure_guidance(row.status, row.failure).is_some()
                });
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.label(
                        RichText::new(format!(
                            "{name}.dash · {choice}",
                            name = group.contested_name
                        ))
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.label(RichText::new(when).color(DashColors::text_secondary(dark_mode)));
                    egui::CollapsingHeader::new(scheduled_nodes_label(group.rows.len()))
                        .id_salt(("scheduled_group", index))
                        .default_open(group.rows.len() <= 3 || needs_attention)
                        .show(ui, |ui| {
                            self.render_scheduled_group_rows(
                                ui,
                                index,
                                &group.rows,
                                &mut action,
                                &mut show_cast_progress,
                            );
                        });
                });
                ui.add_space(6.0);
            }
        });
        if show_cast_progress {
            self.show_vote_progress(ui.ctx());
        }
        action
    }

    /// The node list inside one scheduled decision: status, guidance and the
    /// per-node Remove / Edit / Cast now actions.
    fn render_scheduled_group_rows(
        &mut self,
        ui: &mut Ui,
        group_index: usize,
        rows: &[ScheduledDpnsVoteRow],
        action: &mut AppAction,
        show_cast_progress: &mut bool,
    ) {
        let now = Utc::now().timestamp_millis().max(0) as u64;
        TableBuilder::new(ui)
            .id_salt(("scheduled_group_rows", group_index))
            .striped(false)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto().resizable(true)) // Voter
            .column(Column::initial(280.0).resizable(true)) // Status
            .column(Column::auto().resizable(true)) // Actions
            .body(|mut body| {
                for scheduled_row in rows {
                    let vote = &scheduled_row.vote;
                    let pending_key = DpnsScheduledVoteKey {
                        network: scheduled_row
                            .journal_target
                            .as_ref()
                            .map_or(self.app_context.network(), |(_, key)| key.network),
                        voter_id: vote.voter_id,
                        contested_name: vote.contested_name.clone(),
                    };
                    let missed = schedule_is_missed(
                        scheduled_row.status,
                        VoteTiming::Scheduled(vote.unix_timestamp),
                        now,
                    );
                    let failure_guidance = if missed {
                        Some(MISSED_SCHEDULE_GUIDANCE)
                    } else {
                        scheduled_failure_guidance(scheduled_row.status, scheduled_row.failure)
                    };
                    let height = if failure_guidance.is_some() { 95.0 } else { 25.0 };
                    body.row(height, |mut row| {
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
                            let dark_mode = ui.style().visuals.dark_mode;
                            ui.vertical(|ui| {
                                let status = if missed {
                                    "Missed automatic vote"
                                } else {
                                    target_outcome_label(scheduled_row.status, scheduled_row.failure)
                                };
                                ui.label(
                                    RichText::new(status)
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                                if let Some(guidance) = failure_guidance {
                                    ui.add(Label::new(guidance).wrap());
                                }
                            });
                        });
                        row.col(|ui| {
                            if ui
                                .add_enabled(
                                    scheduled_vote_remove_enabled(scheduled_row.status),
                                    Button::new("Remove"),
                                )
                                .disabled_tooltip(
                                    "This scheduled vote cannot be removed while its result is being checked.",
                                )
                                .clicked()
                            {
                                *action = AppAction::BackendTask(BackendTask::ContestedResourceTask(
                                    scheduled_vote_removal_task(scheduled_row),
                                ));
                            }
                            let dispatch_pending =
                                self.pending_scheduled_actions.contains_key(&pending_key);
                            if ui
                                .add_enabled(
                                    scheduled_row.status == DpnsVoteTargetStatus::Scheduled
                                        && !dispatch_pending,
                                    Button::new("Edit"),
                                )
                                .disabled_tooltip(
                                    "Only scheduled votes that have not started can be edited.",
                                )
                                .clicked()
                            {
                                self.open_schedule_editor(scheduled_row);
                            }
                            let voter = self
                                .voting_identities
                                .iter()
                                .find(|identity| identity.identity.id() == vote.voter_id)
                                .cloned();
                            let cast_enabled =
                                scheduled_vote_cast_enabled(scheduled_row.status, dispatch_pending)
                                    && voter.is_some();
                            if ui
                                .add_enabled(cast_enabled, Button::new("Cast now"))
                                .disabled_tooltip(
                                    "Load this node's voting key and wait for any pending submission to finish before casting.",
                                )
                                .clicked()
                                && let Some(found) = voter
                            {
                                let task = BackendTask::ContestedResourceTask(
                                    ContestedResourceTask::CastScheduledVote(
                                        vote.clone(),
                                        Box::new(found),
                                    ),
                                );
                                let context = BackendTaskContext::for_dispatch_on(
                                    &task,
                                    self.app_context.network(),
                                );
                                self.pending_scheduled_actions
                                    .insert(pending_key.clone(), context.clone());
                                *action = AppAction::BackendTaskWithContext { task, context };
                                *show_cast_progress = true;
                            }
                        });
                    });
                }
            });
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
            (
                ResourceVoteChoice::Lock,
                vote_choice_label(ResourceVoteChoice::Lock, None),
            ),
            (
                ResourceVoteChoice::Abstain,
                vote_choice_label(ResourceVoteChoice::Abstain, None),
            ),
        ];
        if let Some(candidates) = self.candidate_names.get(&row.vote.contested_name) {
            choices.extend(candidates.iter().map(|(candidate_id, name)| {
                (
                    ResourceVoteChoice::TowardsIdentity(*candidate_id),
                    vote_choice_label(
                        ResourceVoteChoice::TowardsIdentity(*candidate_id),
                        Some(name),
                    ),
                )
            }));
        }
        if !choices.iter().any(|(choice, _)| *choice == row.vote.choice) {
            choices.push((row.vote.choice, vote_choice_label(row.vote.choice, None)));
        }
        self.scheduled_vote_editor =
            ScheduledVoteEditor::new(row.clone(), key, node_label, choices);
    }

    /// The batch timing the confirm step resolves targets with.
    fn batch_timing(&self, now_ms: u64) -> Result<BatchTiming, ReviewPlanError> {
        match self.confirm_timing {
            ConfirmTiming::Now => Ok(BatchTiming::Now),
            ConfirmTiming::BeforeEnd => Ok(BatchTiming::BeforeEnd(self.relative_preset)),
            ConfirmTiming::At => {
                let at = self
                    .simple_schedule
                    .current_value()
                    .ok_or(ReviewPlanError::ScheduleIsNotAValidTime)?;
                if at <= now_ms {
                    return Err(ReviewPlanError::ScheduleIsNotInTheFuture);
                }
                Ok(BatchTiming::At(at))
            }
        }
    }

    /// The cached display name of a `TowardsIdentity` candidate.
    ///
    /// A keyed lookup into the refresh-time index — no lock and no allocation,
    /// because every render path calls this once per row, every frame.
    fn candidate_name(&self, contested_name: &str, choice: ResourceVoteChoice) -> Option<&str> {
        let ResourceVoteChoice::TowardsIdentity(candidate_id) = choice else {
            return None;
        };
        self.candidate_names
            .get(contested_name)?
            .get(&candidate_id)
            .map(String::as_str)
    }

    fn rebuild_candidate_names(&mut self) {
        self.candidate_names =
            candidate_name_index(&self.active_contests, &self.contested_names.lock_recover());
    }

    fn rebuild_scheduled_vote_rows(&mut self) {
        let legacy_votes = self.app_context.get_scheduled_votes().unwrap_or_default();
        let rows = self.vote_operations.scheduled_vote_rows(&legacy_votes);
        self.relative_schedule_labels = rows
            .iter()
            .filter_map(|row| {
                let (operation_id, key) = row.journal_target.as_ref()?;
                let at = row.vote.unix_timestamp;
                let preset = self
                    .vote_operations
                    .operation(*operation_id)?
                    .outcome(key)?
                    .relative_schedule_preset_ms?;
                let preset = std::time::Duration::from_millis(preset);
                Some(((key.clone(), at), preset))
            })
            .collect();
        *self.scheduled_votes.lock_recover() = rows;
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

    /// The confirm step (VOTE-FR-080): an aggregate of the staged decisions
    /// across the node set, the batch timing, and `Adjust nodes`.
    fn show_review_and_cast_window(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;

        if self.voting_identity_load_error.is_some() {
            self.render_voting_identity_load_error(ui);
            if ComponentStyles::add_secondary_button(ui, "Close", dark_mode).clicked() {
                self.show_bulk_schedule_popup = false;
            }
            return action;
        }
        if self.voting_identities.is_empty() {
            let (heading, detail, button) = self.no_voting_nodes_copy();
            ui.colored_label(DashColors::warning_color(dark_mode), heading);
            ui.label(detail);
            ui.horizontal(|ui| {
                if ComponentStyles::add_primary_button(ui, button).clicked() {
                    self.load_node_requested = true;
                    self.show_bulk_schedule_popup = false;
                }
                if ComponentStyles::add_secondary_button(ui, "Close", dark_mode).clicked() {
                    self.show_bulk_schedule_popup = false;
                }
            });
            return action;
        }
        if self.selected_votes.is_empty() {
            ui.colored_label(
                DashColors::warning_color(dark_mode),
                "No decisions are staged. Pick a choice on at least one name, then try again.",
            );
            if ComponentStyles::add_secondary_button(ui, "Back to To decide", dark_mode).clicked() {
                self.show_bulk_schedule_popup = false;
            }
            return action;
        }

        let plan = self.build_review_plan();
        match &plan {
            Ok(plan) => {
                ui.heading(confirm_title(plan.decisions.len(), plan.node_count()));
                for (decision, nodes) in &plan.decisions {
                    let choice = vote_choice_label(
                        decision.choice,
                        self.candidate_name(&decision.contested_name, decision.choice),
                    );
                    let mut row = decision_row(&decision.contested_name, &choice, *nodes);
                    if let Some(end) = decision.end_time {
                        row = format!(
                            "{row} · {ends}",
                            ends = ends_in_label(time_left(end, now_ms()))
                        );
                    }
                    ui.label(row);
                }
                ui.add_space(6.0);
                ui.label(confirm_transactions_line(plan.effective_count()));
                let changes = plan.aggregate.change_count();
                if changes > 0 {
                    ui.colored_label(
                        DashColors::warning_color(dark_mode),
                        confirm_change_warning(changes),
                    );
                }
                if plan.aggregate.ends_soon_now > 0 {
                    ui.label(ends_soon_now_line(plan.aggregate.ends_soon_now));
                }
                let skipped = plan.aggregate.skipped_by_reason();
                if !skipped.is_empty() {
                    ui.label(RichText::new(skipped_header(plan.aggregate.skipped.len())).strong());
                    for (reason, count) in skipped {
                        ui.label(skipped_reason_line(count, reason));
                    }
                }
                for exclusion in [
                    NodeExclusion::NotInMasternodeList,
                    NodeExclusion::NoVotingKey,
                ] {
                    let count = self
                        .resolved_nodes
                        .excluded
                        .iter()
                        .filter(|(_, reason)| *reason == exclusion)
                        .count();
                    if count > 0 {
                        ui.label(excluded_nodes_line(count, exclusion));
                    }
                }
            }
            Err(error) => {
                ui.colored_label(DashColors::warning_color(dark_mode), error.to_string());
            }
        }

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label("When:");
            for timing in ConfirmTiming::ALL {
                if ui
                    .selectable_label(self.confirm_timing == timing, timing.label())
                    .clicked()
                {
                    self.confirm_timing = timing;
                }
            }
        });
        match self.confirm_timing {
            ConfirmTiming::Now => {}
            ConfirmTiming::BeforeEnd => {
                ui.horizontal(|ui| {
                    let mut minutes = self.relative_preset.as_secs() / 60;
                    ui.label("Lead time before the end, in minutes:");
                    if ui
                        .add(egui::DragValue::new(&mut minutes).range(1..=7 * 24 * 60))
                        .changed()
                    {
                        self.relative_preset = std::time::Duration::from_secs(minutes * 60);
                    }
                    ui.label(before_end_phrase(self.relative_preset));
                });
                ui.colored_label(DashColors::warning_color(dark_mode), KEEP_RUNNING_MESSAGE);
            }
            ConfirmTiming::At => {
                self.simple_schedule.show(ui);
                ui.colored_label(DashColors::warning_color(dark_mode), KEEP_RUNNING_MESSAGE);
            }
        }

        egui::CollapsingHeader::new("Adjust nodes")
            .default_open(false)
            .show(ui, |ui| self.render_adjust_nodes(ui, plan.as_ref().ok()));

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
        let transactions = plan.as_ref().map_or(0, ReviewPlan::effective_count);
        let all_now = plan.as_ref().is_ok_and(|plan| !plan.has_scheduled());
        let submit_disabled_reason = if operation_in_progress {
            "The selected votes are already being submitted."
        } else if plan.is_err() {
            "These votes cannot be submitted yet. Fix the problem shown above and try again."
        } else {
            "Nothing can be submitted. Choose at least one node and a vote it has not already cast."
        };
        // Holding Enter after opening the sheet must never confirm a vote,
        // including when the primary button has keyboard focus.
        let repeated_enter = ui.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        repeat: true,
                        ..
                    }
                )
            })
        });
        let can_submit = !operation_in_progress && transactions > 0 && !repeated_enter;
        let (mut submit_clicked, mut cancel_clicked) = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let submit_clicked = ComponentStyles::add_primary_button_enabled(
                    ui,
                    can_submit,
                    if operation_in_progress {
                        "Submitting votes…".to_owned()
                    } else {
                        confirm_button_label(transactions, all_now)
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
        // Enter confirms, Esc cancels (VOTE-FR-080) — unless a text field has focus.
        if !ui.ctx().egui_wants_keyboard_input() {
            ui.input_mut(|input| {
                if can_submit && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                    submit_clicked = true;
                }
                if !operation_in_progress
                    && input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                {
                    cancel_clicked = true;
                }
            });
        }
        if submit_clicked {
            action = self.bulk_apply_votes();
            if matches!(
                self.bulk_vote_handling_status,
                VoteHandlingStatus::CastingVotes | VoteHandlingStatus::SchedulingVotes
            ) {
                // The confirm step closes at once; the drawer tracks progress
                // and the staged decisions clear when the operation reports.
                self.show_bulk_schedule_popup = false;
                self.show_vote_progress(ui.ctx());
            }
        }
        if cancel_clicked {
            // Cancel closes the confirm step; staged decisions stay in the tray.
            self.show_bulk_schedule_popup = false;
            self.retry_voter = None;
            self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
        }
        if let VoteHandlingStatus::Failed(error) = &self.bulk_vote_handling_status {
            ui.colored_label(DashColors::error_color(dark_mode), error.to_string());
        }
        action
    }

    /// `Adjust nodes`: per-node timing and the full node × name list.
    fn render_adjust_nodes(&mut self, ui: &mut Ui, plan: Option<&ReviewPlan>) {
        let nodes = self.confirm_nodes();
        for node in &nodes {
            let current = self
                .node_overrides
                .get(&node.voter_id)
                .copied()
                .unwrap_or(NodeTiming::Batch);
            ui.horizontal(|ui| {
                ui.label(node_label(node.voter_id, &self.node_labels));
                ComboBox::from_id_salt(("adjust_node", node.voter_id))
                    .selected_text(node_timing_label(current))
                    .show_ui(ui, |ui| {
                        for option in [
                            NodeTiming::Batch,
                            NodeTiming::Override(BatchTiming::Now),
                            NodeTiming::Override(BatchTiming::BeforeEnd(self.relative_preset)),
                            NodeTiming::DontUse,
                        ] {
                            if ui
                                .selectable_label(current == option, node_timing_label(option))
                                .clicked()
                            {
                                if option == NodeTiming::Batch {
                                    self.node_overrides.remove(&node.voter_id);
                                } else {
                                    self.node_overrides.insert(node.voter_id, option);
                                }
                            }
                        }
                    });
            });
        }
        let Some(plan) = plan else {
            return;
        };
        ui.separator();
        egui::Grid::new("adjust_nodes_targets")
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                for target in plan.targets() {
                    ui.label(node_label(target.key.voter_id, &self.node_labels));
                    ui.label(format!("{name}.dash", name = target.contested_name));
                    let requested = vote_choice_label(
                        target.requested_choice,
                        self.candidate_name(&target.contested_name, target.requested_choice),
                    );
                    let current = review_current_choice_label(
                        target.current_choice,
                        target
                            .current_choice
                            .and_then(|choice| self.candidate_name(&target.contested_name, choice)),
                    );
                    ui.label(format!(
                        "Current choice: {current}. Requested choice: {requested}."
                    ));
                    let timing = match target.timing {
                        VoteTiming::Now => "Now".to_owned(),
                        VoteTiming::Scheduled(at) => format!("{when} UTC", when = utc_minute(at)),
                    };
                    let changes = self
                        .cards
                        .iter()
                        .find(|card| card.vote_poll_id == Some(target.key.vote_poll_id))
                        .and_then(|card| {
                            card.nodes
                                .iter()
                                .find(|node| node.node == target.key.voter_id)
                        })
                        .map(|node| node.changes);
                    ui.label(match changes {
                        Some(changes) if target.current_choice.is_some() => {
                            format!("{timing} · {left}", left = changes_left_label(changes))
                        }
                        _ => timing,
                    });
                    ui.end_row();
                }
            });
    }

    /// Record a failed submission and surface it the way the rest of the app
    /// surfaces errors: a banner carrying the technical chain in its details.
    ///
    /// Raised here rather than in the render loop, because `set_global` called
    /// every frame would resurrect a banner the user just dismissed.
    fn fail_submission(&mut self, error: impl Into<VoteSubmissionError>) {
        let error = error.into();
        self.submission_error_banner.raise_persistent(
            self.app_context.egui_ctx(),
            error.to_string(),
            MessageType::Error,
        );
        if let Some(handle) = &self.submission_error_banner {
            handle.with_details(&error);
        }
        self.bulk_vote_handling_status = VoteHandlingStatus::Failed(error);
    }

    /// Resolve every node × contest target the current review would submit.
    ///
    /// The review sheet and the submit click share this so the sheet cannot
    /// promise something other than what is sent.
    ///
    /// # Errors
    ///
    /// A [`ReviewPlanError`] naming the condition that blocks submission; its
    /// `Display` is the sentence the operator reads.
    fn build_review_plan(&self) -> Result<ReviewPlan, ReviewPlanError> {
        self.build_review_plan_at(Utc::now())
    }

    /// Nodes the confirm step composes with: the retried node alone, else the
    /// node set's voting nodes.
    fn confirm_nodes(&self) -> Vec<ComposerNode> {
        let included: BTreeSet<Identifier> = self.resolved_nodes.included.iter().copied().collect();
        self.voting_identities
            .iter()
            .map(|identity| identity.identity.id())
            .filter(|id| match self.retry_voter {
                Some(voter) => *id == voter,
                None => included.contains(id),
            })
            .map(|voter_id| ComposerNode {
                voter_id,
                alias: self
                    .voting_identities
                    .iter()
                    .find(|identity| identity.identity.id() == voter_id)
                    .and_then(|identity| identity.alias.clone()),
            })
            .collect()
    }

    fn build_review_plan_at(&self, now: DateTime<Utc>) -> Result<ReviewPlan, ReviewPlanError> {
        let now_ms = now.timestamp_millis() as u64;
        let decisions = self
            .selected_votes
            .iter()
            .map(|vote| {
                let vote_poll_id = self
                    .app_context
                    .dpns_vote_poll_id(&vote.contested_name)
                    .map_err(|error| {
                        tracing::warn!(
                            ?error,
                            contested_name = vote.contested_name,
                            "Could not build a DPNS vote target"
                        );
                        ReviewPlanError::VotePollUnavailable {
                            source: Arc::new(error),
                        }
                    })?;
                Ok(Decision {
                    contested_name: vote.contested_name.clone(),
                    vote_poll_id,
                    choice: vote.vote_choice,
                    end_time: vote.end_time,
                })
            })
            .collect::<Result<Vec<_>, ReviewPlanError>>()?;
        let network = self.app_context.network();
        let standing = |voter_id: Identifier, vote_poll_id: Identifier| {
            let key = DpnsVoteTargetKey {
                network,
                voter_id,
                vote_poll_id,
            };
            // Changes left come from the card cache; an uncached contest never
            // blocks a vote, since Platform enforces the limit itself.
            let changes = self
                .cards
                .iter()
                .find(|card| card.vote_poll_id == Some(vote_poll_id))
                .and_then(|card| card.nodes.iter().find(|node| node.node == voter_id))
                .map_or(ChangesLeft::Unknown, |node| node.changes);
            NodeStanding {
                state: self.vote_state.state(voter_id, vote_poll_id),
                locked: self.vote_operations.target_status(&key).is_some(),
                changes,
            }
        };
        let aggregate = compose(
            network,
            &decisions,
            &self.confirm_nodes(),
            standing,
            self.batch_timing(now_ms)?,
            &self.node_overrides,
            now_ms,
        )?;
        let decisions = decisions
            .into_iter()
            .map(|decision| {
                let nodes = aggregate
                    .targets
                    .iter()
                    .filter(|target| target.key.vote_poll_id == decision.vote_poll_id)
                    .count();
                (decision, nodes)
            })
            .collect();
        Ok(ReviewPlan {
            aggregate,
            decisions,
        })
    }

    /// Clone the signing identities of the nodes casting in `plan`.
    ///
    /// Kept out of [`ReviewPlan`], which the review sheet rebuilds every frame:
    /// only the submit click needs owned identities.
    fn casting_voters(&self, plan: &ReviewPlan) -> Vec<QualifiedIdentity> {
        let casting: BTreeSet<Identifier> = plan
            .targets()
            .iter()
            .map(|target| target.key.voter_id)
            .collect();
        self.voting_identities
            .iter()
            .filter(|identity| casting.contains(&identity.identity.id()))
            .cloned()
            .collect()
    }

    fn bulk_apply_votes(&mut self) -> AppAction {
        let plan = match self.build_review_plan() {
            Ok(plan) => plan,
            Err(error) => {
                self.fail_submission(error);
                return AppAction::None;
            }
        };
        let has_immediate = plan.has_immediate();
        if plan.aggregate.targets.is_empty() {
            let all_no_ops = !plan.aggregate.skipped.is_empty()
                && plan
                    .aggregate
                    .skipped
                    .iter()
                    .all(|skipped| skipped.reason == SkipReason::AlreadyVoted);
            self.fail_submission(if all_no_ops {
                VoteSubmissionError::AllNoOps
            } else {
                VoteSubmissionError::NoTargets
            });
            return AppAction::None;
        }
        let voters = self.casting_voters(&plan);
        let operation = DpnsVoteOperation::new(plan.aggregate.targets);
        let labels =
            (self.confirm_timing == ConfirmTiming::BeforeEnd).then(|| RelativeScheduleLabels {
                preset: self.relative_preset,
                targets: operation
                    .targets
                    .iter()
                    .map(|outcome| &outcome.target.key)
                    .filter(|key| !self.node_overrides.contains_key(&key.voter_id))
                    .cloned()
                    .collect(),
            });
        self.submission_error_banner.take_and_clear();
        self.bulk_vote_handling_status = if has_immediate {
            VoteHandlingStatus::CastingVotes
        } else {
            VoteHandlingStatus::SchedulingVotes
        };
        self.retry_voter = None;
        self.pending_vote_operation = Some(operation.id);
        AppAction::BackendTask(BackendTask::ContestedResourceTask(
            ContestedResourceTask::SubmitDpnsVoteOperation {
                operation,
                voters,
                replacing_scheduled_key: None,
                network: self.app_context.network(),
                relative_labels: labels,
            },
        ))
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

        self.active_contests = ActiveDpnsContestSnapshot::new(
            &self.app_context,
            self.app_context
                .ongoing_contested_names()
                .unwrap_or_default(),
        );
        *self.contested_names.lock_recover() =
            self.app_context.all_contested_names().unwrap_or_default();
        self.rebuild_candidate_names();

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
        self.rebuild_cards();
        self.app_context.recompute_dpns_vote_attention();
    }

    fn refresh_on_arrival(&mut self) {
        let previous_voters: BTreeSet<Identifier> = self
            .voting_identities
            .iter()
            .map(|identity| identity.identity.id())
            .collect();
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
        // A node loaded after decisions were staged never joins them silently.
        if !self.selected_votes.is_empty() {
            for identity in &self.voting_identities {
                let id = identity.identity.id();
                if !previous_voters.contains(&id) {
                    self.node_overrides.entry(id).or_insert(NodeTiming::DontUse);
                }
            }
        }
        self.refresh();
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.refresh_banner.take_and_clear();
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        if context.is_dpns_contest_refresh() {
            self.refreshing_status = RefreshingStatus::NotRefreshing;
            self.refresh_banner.take_and_clear();
        }
        self.finish_scheduled_dispatch(context);
        self.release_pending_on_error = self.pending_vote_operation.is_some_and(|operation_id| {
            dpns_operation_id(context, self.app_context.network()) == Some(operation_id)
        });
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        if let Err(refresh_error) = self.vote_state.reload(&self.app_context) {
            tracing::warn!(
                ?refresh_error,
                "Could not refresh proved DPNS votes after a task error"
            );
        }
        if self.release_pending_on_error {
            self.pending_vote_operation = None;
            self.release_pending_on_error = false;
            // The review window disables both Submit and Cancel while a
            // submission is in flight, so a failed submission must leave that
            // state here. Otherwise the window stays on "Submitting votes…"
            // with no way to retry or close it.
            if matches!(error, TaskError::DpnsVoteReviewRequired) {
                // A first vote became a change during preflight: reopen the
                // confirm on fresh vote state so it shows the change warning.
                self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
                self.show_bulk_schedule_popup = !self.selected_votes.is_empty();
            } else if matches!(
                self.bulk_vote_handling_status,
                VoteHandlingStatus::CastingVotes | VoteHandlingStatus::SchedulingVotes
            ) {
                // AppState banners the borrowed `TaskError` itself, so the
                // window only has to leave its in-flight state.
                self.bulk_vote_handling_status =
                    VoteHandlingStatus::Failed(VoteSubmissionError::TaskFailed);
            }
        }
        if let Err(refresh_error) = self.vote_operations.refresh(&self.app_context) {
            tracing::warn!(
                ?refresh_error,
                "Could not refresh DPNS voting state after a task error"
            );
        }
        self.rebuild_scheduled_vote_rows();
        self.rebuild_cards();
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
                    self.pending_vote_operation = None;
                    if matches!(
                        self.bulk_vote_handling_status,
                        VoteHandlingStatus::CastingVotes | VoteHandlingStatus::SchedulingVotes
                    ) {
                        self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
                        self.selected_votes.clear();
                        self.selected_cards.clear();
                    }
                }
                if let Err(error) = self.vote_operations.refresh(&self.app_context) {
                    tracing::warn!(
                        ?error,
                        "Could not refresh scheduled votes after an operation update"
                    );
                }
                self.rebuild_scheduled_vote_rows();
                self.rebuild_cards();
                self.app_context.recompute_dpns_vote_attention();
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
            BackendTaskSuccessResult::RefreshedDpnsContests => {
                self.refresh();
                self.refresh_banner.take_and_clear();
                self.refreshing_status = RefreshingStatus::NotRefreshing;
            }
            _ => {}
        }
    }

    /// Render the Votes body inside an already laid-out central panel.
    ///
    /// The Masternodes screen owns the window chrome (top bar, left nav,
    /// segment header); this draws the sub-view chips and the active view.
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = AppAction::None;

        // `Review again` from the progress drawer, which lives outside this panel.
        if let Some(outcome) = self.app_context.take_dpns_vote_review_request() {
            self.view = VotesView::ToDecide;
            self.review_failed_target(&outcome);
        }
        if let Some(label) = self.app_context.take_dpns_votes_name_request() {
            self.show_contest(label);
        }

        ui.horizontal(|ui| {
            for view in VotesView::ALL {
                if ui.selectable_label(self.view == view, view.label()).clicked() {
                    self.view = view;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let dark_mode = ui.visuals().dark_mode;
                match self.view {
                    VotesView::Scheduled => {
                        if ComponentStyles::add_secondary_button(
                            ui,
                            "Remove finished votes",
                            dark_mode,
                        )
                        .clicked()
                        {
                            self.scheduled_clear_dialog = Some((
                                false,
                                ConfirmationDialog::new(
                                    "Remove finished votes",
                                    "Remove every finished scheduled vote from this device? Pending votes will stay scheduled.",
                                )
                                .danger_mode(true)
                                .confirm_text(Some("Remove finished votes")),
                            ));
                        }
                        if ComponentStyles::add_secondary_button(ui, "Remove all", dark_mode)
                            .clicked()
                        {
                            self.scheduled_clear_dialog = Some((
                                true,
                                ConfirmationDialog::new(
                                    "Remove all scheduled votes",
                                    "Remove every scheduled vote from this device? Votes already submitted to Platform cannot be undone.",
                                )
                                .danger_mode(true)
                                .confirm_text(Some("Remove all scheduled votes")),
                            ));
                        }
                    }
                    VotesView::ToDecide | VotesView::Voted | VotesView::History => {
                        let refreshing = self.refreshing_status == RefreshingStatus::Refreshing;
                        if ui
                            .add_enabled(
                                !refreshing,
                                ComponentStyles::secondary_button("Refresh", dark_mode),
                            )
                            .disabled_tooltip("Contests are already being refreshed.")
                        .clicked()
                        {
                            action = AppAction::BackendTask(BackendTask::ContestedResourceTask(
                                ContestedResourceTask::QueryDPNSContests,
                            ));
                        }
                    }
                }
            });
        });
        ui.add_space(8.0);

        if let Some(error) = self.vote_operations.read_error() {
            if self.journal_error_banner.is_none() || self.journal_error_banner.was_evicted() {
                self.journal_error_banner.raise_persistent(
                    ui.ctx(),
                    JOURNAL_UNAVAILABLE_MESSAGE,
                    MessageType::Warning,
                );
                if let Some(handle) = &self.journal_error_banner {
                    handle.with_details(error);
                }
            }
            // The banner is dismissible, but the warning has to outlive a
            // dismissal for as long as the history is unreliable — so the same
            // sentence falls back inline once the banner is gone, and never
            // renders twice at once.
            if self
                .journal_error_banner
                .as_ref()
                .and_then(BannerHandle::text)
                .is_none()
            {
                ui.label(JOURNAL_UNAVAILABLE_MESSAGE);
            }
            if ComponentStyles::add_secondary_button(ui, "Retry loading", ui.visuals().dark_mode)
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
                action = AppAction::BackendTask(BackendTask::ContestedResourceTask(if clear_all {
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
                    action = AppAction::BackendTaskWithContext { task, context };
                    self.scheduled_vote_editor = None;
                }
            }
        }
        if self.show_bulk_schedule_popup {
            egui::Window::new("Confirm votes")
                .collapsible(false)
                .resizable(true)
                .vscroll(true)
                .show(ui.ctx(), |ui| {
                    action |= self.show_review_and_cast_window(ui);
                });
        }

        if self.voting_identity_load_error.is_some() && !self.show_bulk_schedule_popup {
            self.render_voting_identity_load_error(ui);
        }
        match self.view {
            VotesView::ToDecide | VotesView::Voted => {
                if self.voting_identities.is_empty() && self.voting_identity_load_error.is_none() {
                    action |= self.render_no_voting_nodes(ui);
                }
                self.render_needs_attention(ui);
                if self.active_contests.is_empty() {
                    self.render_node_set_chip(ui);
                    action |= self.render_empty_view(ui);
                } else {
                    self.render_active_contests(ui);
                }
            }
            VotesView::History => {
                if self.contested_names.lock_recover().is_empty() {
                    action |= self.render_empty_view(ui);
                } else {
                    self.render_table_past_contests(ui);
                }
            }
            VotesView::Scheduled => {
                if self.scheduled_votes.lock_recover().is_empty() {
                    action |= self.render_empty_view(ui);
                } else {
                    action |= self.render_table_scheduled_votes(ui);
                }
            }
        }

        if action == AppAction::None
            && let Some(task) = self.pending_backend_task.take()
        {
            action = AppAction::BackendTask(task);
        }
        if let AppAction::BackendTask(BackendTask::ContestedResourceTask(
            ContestedResourceTask::QueryDPNSContests,
        )) = &action
        {
            self.refresh_banner.take_and_clear();
            let handle =
                MessageBanner::set_global(ctx, "Refreshing name contests…", MessageType::Info);
            handle.with_elapsed();
            self.refresh_banner = Some(handle);
            self.refreshing_status = RefreshingStatus::Refreshing;
        }

        if action == AppAction::None
            && let Some(preference) = self.pending_preference.take()
        {
            action = AppAction::BackendTask(BackendTask::ContestedResourceTask(
                ContestedResourceTask::SaveDpnsVotingPreference(preference),
            ));
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

    #[test]
    fn review_regression_failed_manual_contest_refresh_can_be_retried() {
        let (ctx, _dir) = kv_ctx();
        let mut screen = DPNSScreen::new(&ctx, VotesView::History);
        screen.refreshing_status = RefreshingStatus::Refreshing;
        let context = BackendTaskContext::from(&BackendTask::ContestedResourceTask(
            ContestedResourceTask::QueryDPNSContests,
        ));
        screen.display_backend_task_error(&context, &TaskError::DpnsCurrentVoteUnavailable);
        assert!(screen.refreshing_status == RefreshingStatus::NotRefreshing);
    }

    #[test]
    fn completed_contest_refresh_reloads_visible_contests_and_votes() {
        let (mut screen, _dir) = voting_ui_review_fixture();
        assert!(screen.active_contests.is_empty());
        let voter = screen.voting_identities[0].identity.id();
        let poll = screen.app_context.dpns_vote_poll_id("alpha").unwrap();
        screen
            .app_context
            .insert_name_contests_as_normalized_names(vec!["alpha".into()])
            .unwrap();
        screen
            .app_context
            .cache_confirmed_dpns_vote(voter, poll, ResourceVoteChoice::Abstain)
            .unwrap();
        screen.refreshing_status = RefreshingStatus::Refreshing;
        screen.display_task_result(BackendTaskSuccessResult::RefreshedDpnsContests);
        assert!(!screen.active_contests.is_empty());
        assert_eq!(
            screen.vote_state.state(voter, poll),
            DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Abstain))
        );
        assert!(screen.refreshing_status == RefreshingStatus::NotRefreshing);
    }

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
        use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
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
        let mut private_keys =
            crate::model::qualified_identity::encrypted_key_storage::KeyStorage::default();
        if let Some((_, key)) = &associated_voter_identity {
            private_keys.insert_at((crate::model::qualified_identity::PrivateKeyTarget::PrivateKeyOnVoterIdentity, key.id()), (
                crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey::from(key.clone()),
                crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData::InVault,
            ));
        }
        QualifiedIdentity {
            identity,
            associated_voter_identity,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: crate::model::qualified_identity::IdentityType::Masternode,
            alias: Some(alias.to_owned()),
            private_keys,
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

    /// VOTE-TC-100: no window overlay; staged decisions clear only when this
    /// panel's own operation reports, never on another operation's update.
    #[test]
    fn own_operation_result_clears_staged_decisions_without_an_overlay() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
        screen.selected_votes = vec![SelectedVote {
            contested_name: "alpha".to_owned(),
            vote_choice: ResourceVoteChoice::Lock,
            end_time: None,
        }];
        screen.bulk_vote_handling_status = VoteHandlingStatus::CastingVotes;
        screen.pending_vote_operation = Some(DpnsVoteOperationId::from_bytes([7; 16]));

        screen.display_task_result(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: ctx.network(),
            operation_id: DpnsVoteOperationId::from_bytes([8; 16]),
        });
        assert_eq!(screen.selected_votes.len(), 1);
        assert!(screen.pending_vote_operation.is_some());

        screen
            .pending_scheduled_actions
            .insert(pending_scheduled_key(&ctx), BackendTaskContext::Unknown);
        screen.display_task_result(BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: ctx.network(),
            operation_id: DpnsVoteOperationId::from_bytes([7; 16]),
        });

        assert!(screen.selected_votes.is_empty());
        assert!(screen.pending_vote_operation.is_none());
        assert!(matches!(
            screen.bulk_vote_handling_status,
            VoteHandlingStatus::NotStarted
        ));
        assert!(!screen.pending_scheduled_actions.is_empty());
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

        let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
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
    fn review_regression_review_choice_names_the_candidate_or_falls_back_to_its_identifier() {
        let candidate_id = Identifier::from([42; 32]);
        let encoded = candidate_id.to_string(Encoding::Base58);

        let named = vote_choice_label(
            ResourceVoteChoice::TowardsIdentity(candidate_id),
            Some("alice"),
        );
        assert!(named.starts_with("Vote for alice ("));
        assert!(named.contains(&crate::model::identity_name::shorten_id(&encoded)));
        assert!(!named.contains(&encoded));

        let unresolved = vote_choice_label(ResourceVoteChoice::TowardsIdentity(candidate_id), None);
        assert_eq!(
            unresolved,
            format!("Vote for {}", short_identifier(candidate_id))
        );
    }

    /// Submission is not blocked when a candidate name cannot be resolved, so
    /// the review line has to carry the identifier itself.
    #[test]
    fn review_sheet_identifies_an_uncached_candidate_by_its_identifier() {
        use egui_kittest::kittest::Queryable;
        let (mut screen, _dir) = voting_ui_review_fixture();
        let candidate_id = Identifier::from([3; 32]);
        let choice = ResourceVoteChoice::TowardsIdentity(candidate_id);
        screen.selected_votes[0].vote_choice = choice;
        assert!(
            screen.candidate_name("alpha", choice).is_none(),
            "the fixture must leave the candidate name uncached, or this proves nothing"
        );
        let expected = decision_row(
            "alpha",
            &format!("Vote for {handle}", handle = short_identifier(candidate_id)),
            1,
        );

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 800.0))
            .build_ui(move |ui| {
                screen.show_review_and_cast_window(ui);
            });
        harness.run();

        assert!(harness.query_by_label(&expected).is_some());
        for label in [
            "Cast 1 decision with 1 node",
            "1 transaction, one per node and name. Voting is free for your nodes.",
            "Cast 1 vote",
        ] {
            assert!(
                harness.query_by_label(label).is_some(),
                "Missing review detail: {label}"
            );
        }
    }

    /// Candidate labels are resolved once per refresh, so the render path does a
    /// keyed lookup instead of rescanning every contest this install ever cached.
    #[test]
    fn candidate_labels_come_from_the_refresh_time_index_not_a_per_frame_scan() {
        let (ctx, _temp_dir) = kv_ctx();
        ctx.seed_dpns_contest_for_test("alpha", None, false);
        let mut screen = DPNSScreen::new(&ctx, VotesView::Scheduled);
        let candidate = ResourceVoteChoice::TowardsIdentity(Identifier::from([3; 32]));
        assert_eq!(screen.candidate_name("alpha", candidate), Some("alpha"));

        screen.contested_names.lock_recover().clear();
        assert_eq!(
            screen.candidate_name("alpha", candidate),
            Some("alpha"),
            "rendering a row must not depend on scanning the contested-name cache"
        );

        ctx.seed_dpns_contest_for_test("beta", None, false);
        assert_eq!(
            screen.candidate_name("beta", candidate),
            None,
            "a contest cached after the last refresh resolves only on the next refresh"
        );
        screen.refresh();
        assert_eq!(screen.candidate_name("beta", candidate), Some("beta"));
    }

    #[test]
    fn activity_choice_uses_the_cached_candidate_name() {
        let candidate_id = Identifier::from([42; 32]);
        let label = vote_choice_label(
            ResourceVoteChoice::TowardsIdentity(candidate_id),
            Some("alice"),
        );

        assert_eq!(
            label,
            vote_choice_label(
                ResourceVoteChoice::TowardsIdentity(candidate_id),
                Some("alice")
            )
        );
        assert!(!label.contains(&candidate_id.to_string(Encoding::Base58)));
    }

    #[test]
    fn missing_voting_nodes_copy_is_actionable_and_avoids_the_stale_route() {
        assert!(NO_VOTING_NODES_DETAIL.contains("masternode"));
        assert!(!NO_VOTING_NODES_MESSAGE.contains("Identities screen"));
    }

    #[test]
    fn scheduled_vote_sweep_error_preserves_unrelated_manual_cast() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, VotesView::Scheduled);
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
        let mut screen = DPNSScreen::new(&ctx, VotesView::Scheduled);

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
        let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
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

            screen.display_backend_task_error(&context, &error);
            screen.display_task_error(&error);

            assert_eq!(screen.pending_vote_operation, None);
            let VoteHandlingStatus::Failed(failure) = &screen.bulk_vote_handling_status else {
                panic!(
                    "the review window must leave its in-flight state so Submit and Cancel work again"
                );
            };
            assert!(matches!(failure, VoteSubmissionError::TaskFailed));
            assert!(
                failure.to_string().contains("review and submit them again"),
                "the window must name a recovery step the operator can take"
            );
        }
    }

    /// An error belonging to a different operation must not disturb the review
    /// window of the submission this screen is still waiting on.
    #[test]
    fn unrelated_error_keeps_the_pending_submission_in_progress() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
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
        assert!(matches!(
            screen.bulk_vote_handling_status,
            VoteHandlingStatus::CastingVotes
        ));
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

        let screen = DPNSScreen::new(&ctx, VotesView::ToDecide);

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

        let screen = DPNSScreen::new(&ctx, VotesView::ToDecide);

        assert_eq!(screen.voting_identities.len(), 1);
    }

    /// VOTE-FR-080: with Cancel focused, Enter activates Cancel only and
    /// casts nothing (any focused widget turns the window's Enter shortcut
    /// off); with nothing focused, Enter casts.
    #[test]
    fn enter_in_the_confirm_never_casts_while_cancel_has_focus() {
        use egui_kittest::kittest::Queryable;
        let render = |screen: &Arc<Mutex<DPNSScreen>>| {
            let rendering = screen.clone();
            egui_kittest::Harness::builder()
                .with_size(egui::vec2(1000.0, 800.0))
                .build_ui(move |ui| {
                    rendering.lock_recover().show_review_and_cast_window(ui);
                })
        };
        let (mut screen, _dir) = voting_ui_review_fixture();
        screen.show_bulk_schedule_popup = true;
        let screen = Arc::new(Mutex::new(screen));

        let mut harness = render(&screen);
        harness.run();
        harness.get_by_label("Cancel").focus();
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run();
        {
            let screen = screen.lock_recover();
            assert!(
                screen.pending_vote_operation.is_none(),
                "Enter on Cancel must not cast"
            );
            assert!(!screen.show_bulk_schedule_popup, "Enter on Cancel cancels");
        }

        screen.lock_recover().show_bulk_schedule_popup = true;
        let mut harness = render(&screen);
        harness.run();
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2); // casting shows a spinner, which keeps repainting
        assert!(
            screen.lock_recover().pending_vote_operation.is_some(),
            "Enter with nothing focused casts"
        );
    }

    #[tokio::test]
    async fn followup_repeated_enter_does_not_submit_confirmation() {
        let (screen, _dir) = two_contest_screen().await;
        let rendering = screen.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 1200.0))
            .build_ui(move |ui| {
                rendering.lock_recover().ui(ui);
            });
        harness.run();
        screen.lock_recover().list_focused = true;
        for key in [egui::Key::J, egui::Key::L] {
            harness.key_press(key);
            harness.run();
        }
        let enter = |pressed| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false, // egui derives repeat from the held-key state.
            modifiers: egui::Modifiers::NONE,
        };
        harness.event(enter(true));
        harness.run();
        assert!(screen.lock_recover().show_bulk_schedule_popup);
        assert!(screen.lock_recover().pending_vote_operation.is_none());
        harness.event(enter(true));
        harness.run_steps(2);
        assert!(screen.lock_recover().pending_vote_operation.is_none());
        harness.event(enter(false));
        harness.run_steps(2);
        harness.event(enter(true));
        harness.run_steps(2);
        assert!(
            screen.lock_recover().pending_vote_operation.is_some(),
            "a fresh press after release confirms"
        );
    }

    fn voting_ui_review_fixture() -> (DPNSScreen, tempfile::TempDir) {
        let (ctx, temp_dir) = kv_ctx();
        let voter = masternode_identity(1, "node-one", true, ctx.network());
        let poll = ctx.dpns_vote_poll_id("alpha").unwrap();
        ctx.cache_confirmed_dpns_vote(voter.identity.id(), poll, ResourceVoteChoice::Lock)
            .unwrap();
        let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
        screen.voting_identities = vec![voter.clone()];
        screen.rebuild_cards();
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
        let target = screen.build_review_plan().unwrap().aggregate.targets[0].clone();
        let mut operation = DpnsVoteOperation::new(vec![target]);
        operation.targets[0].status = DpnsVoteTargetStatus::Unconfirmed;
        screen
            .app_context
            .insert_dpns_vote_operation(&mut operation, None)
            .unwrap();
        let mut reopened = DPNSScreen::new(&screen.app_context, VotesView::ToDecide);
        assert!(reopened.active_contests.is_empty());
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 1200.0))
            .build_ui(move |ui| {
                reopened.ui(ui);
            });
        harness.run();
        // VOTE-TC-102: the Needs attention row leads to the drawer's Check again.
        assert!(
            harness
                .query_by_label(
                    "Votes still being checked: 1. Do not submit the pending votes again."
                )
                .is_some()
        );
        assert!(harness.query_by_label("Show progress").is_some());
    }

    #[test]
    fn additional_scenarios_missed_schedule_is_explained_after_reopening() {
        use egui_kittest::kittest::Queryable;
        for status in [
            DpnsVoteTargetStatus::Scheduled,
            DpnsVoteTargetStatus::Queued,
        ] {
            let (screen, _dir) = voting_ui_review_fixture();
            let mut target = screen.build_review_plan().unwrap().aggregate.targets[0].clone();
            target.timing = VoteTiming::Scheduled(1);
            let mut operation = DpnsVoteOperation::new(vec![target]);
            operation.targets[0].status = status;
            screen
                .app_context
                .insert_dpns_vote_operation(&mut operation, None)
                .unwrap();
            let mut reopened = DPNSScreen::new(&screen.app_context, VotesView::Scheduled);
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
                assert!(harness.query_by_label("The automatic voting time was missed. On the Scheduled tab, use Cast now to vote, Edit to reschedule, or Remove to cancel.").is_some());
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
            let mut screen = DPNSScreen::new(&context, VotesView::Scheduled);
            if !fail_at_construction {
                store.fail_next_gets_containing("det:dpns_vote_operations:v2:", 1);
                screen.refresh();
            }
            let screen = Arc::new(Mutex::new(screen));
            let rendering = screen.clone();
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1600.0, 1200.0))
                .build_ui(move |ui| {
                    // The hosting Masternodes screen renders global banners
                    // through its island panel.
                    crate::ui::components::styled::island_central_panel(ui, |ui| {
                        rendering.lock_recover().ui(ui)
                    });
                });
            harness.run();
            let message = "Saved voting progress could not be read. The displayed history may be incomplete or out of date. Do not submit votes again until you have retried loading and checked their status.";
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
                "Current vote could not be verified. On the Scheduled tab, use Cast now to check again or Edit to reschedule.",
            ),
            (
                DpnsVoteFailure::SubmissionFailed,
                "The vote could not be submitted. Check your connection and voting key, then use Cast now or Edit on the Scheduled tab.",
            ),
        ] {
            let (screen, _dir) = voting_ui_review_fixture();
            let mut target = screen.build_review_plan().unwrap().aggregate.targets[0].clone();
            target.timing = VoteTiming::Scheduled(Utc::now().timestamp_millis() as u64);
            let mut operation = DpnsVoteOperation::new(vec![target]);
            operation.targets[0].failure = Some(failure);
            screen
                .app_context
                .insert_dpns_vote_operation(&mut operation, None)
                .unwrap();
            // The reason lives on the Scheduled tab, which its own copy names.
            let mut reopened = DPNSScreen::new(&screen.app_context, VotesView::Scheduled);
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1600.0, 1200.0))
                .build_ui(move |ui| {
                    reopened.ui(ui);
                });
            harness.run();
            assert!(harness.query_by_label("Not submitted").is_some());
            assert!(harness.query_by_label(reason).is_some());
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
        let screen = DPNSScreen::new(&context, VotesView::ToDecide);
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
        let mut screen = DPNSScreen::new(&context, VotesView::ToDecide);
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

    /// VOTE-FR-081: absolute and relative schedules must fall strictly
    /// before each contest's end and after now.
    #[test]
    fn voting_ui_review_rejects_absolute_and_relative_schedules_at_contest_deadline() {
        let (mut screen, _dir) = voting_ui_review_fixture();
        let now = Utc::now();
        screen.confirm_timing = ConfirmTiming::At;
        screen.simple_schedule =
            UtcScheduleInput::new().with_time(now + chrono::Duration::hours(1));
        let scheduled_at = screen.simple_schedule.current_value().unwrap();
        for deadline in [scheduled_at - 1, scheduled_at, scheduled_at + 1] {
            screen.selected_votes[0].end_time = Some(deadline);
            assert_eq!(
                screen.build_review_plan_at(now).is_ok(),
                deadline > scheduled_at
            );
        }

        screen.confirm_timing = ConfirmTiming::BeforeEnd;
        screen.relative_preset = std::time::Duration::from_secs(10 * 60);
        let now_ms = now.timestamp_millis() as u64;
        screen.selected_votes[0].end_time = Some(now_ms + 5 * 60_000);
        let plan = screen.build_review_plan_at(now).unwrap();
        assert_eq!(
            (
                plan.aggregate.targets[0].timing,
                plan.aggregate.ends_soon_now
            ),
            (VoteTiming::Now, 1),
            "ten minutes before an end five minutes away has passed, so it votes now"
        );
        let end = now_ms + 60 * 60_000;
        screen.selected_votes[0].end_time = Some(end);
        assert_eq!(
            screen.build_review_plan_at(now).unwrap().aggregate.targets[0].timing,
            VoteTiming::Scheduled(end - 10 * 60_000)
        );

        screen.confirm_timing = ConfirmTiming::At;
        screen.selected_votes[0].end_time = Some(scheduled_at + 1);
        screen.selected_votes.push(SelectedVote {
            contested_name: "beta".into(),
            vote_choice: ResourceVoteChoice::Abstain,
            end_time: Some(scheduled_at),
        });
        let Err(error) = screen.build_review_plan_at(now) else {
            panic!("the second contest has ended before the schedule");
        };
        assert!(matches!(
            error,
            ReviewPlanError::ScheduleOutlastsContest { contested_name } if contested_name == "beta"
        ));
    }

    #[test]
    fn voting_ui_absolute_schedule_preserves_the_requested_utc_minute() {
        let (mut screen, _temp_dir) = voting_ui_review_fixture();
        let requested = (Utc::now() + chrono::Duration::days(1))
            .date_naive()
            .and_hms_opt(18, 30, 0)
            .unwrap()
            .and_utc();
        screen.simple_schedule = UtcScheduleInput::new().with_time(requested);
        screen.confirm_timing = ConfirmTiming::At;
        let plan = screen.build_review_plan().unwrap();
        assert_eq!(
            plan.aggregate.targets[0].timing,
            VoteTiming::Scheduled(requested.timestamp_millis() as u64)
        );
        let delayed = screen
            .build_review_plan_at(Utc::now() + chrono::Duration::minutes(20))
            .unwrap();
        assert_eq!(
            delayed.aggregate.targets[0].timing,
            plan.aggregate.targets[0].timing
        );
    }

    #[test]
    fn followup_manual_cast_failure_during_bulk_clears_only_its_dispatch() {
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
        let updated = BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: screen.app_context.network(),
            operation_id: DpnsVoteOperationId::from_bytes([9; 16]),
        };
        screen.display_backend_task_result(&unrelated_dispatch, updated.clone());
        assert!(screen.pending_scheduled_actions.contains_key(&key));
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
        screen.display_backend_task_result(&dispatch, updated);
        assert!(screen.pending_scheduled_actions.is_empty());
        screen
            .pending_scheduled_actions
            .insert(key.clone(), dispatch.clone());
        let bulk_id = DpnsVoteOperationId::from_bytes([13; 16]);
        screen.pending_vote_operation = Some(bulk_id);
        screen.bulk_vote_handling_status = VoteHandlingStatus::CastingVotes;
        screen.display_backend_task_error(&dispatch, &error);
        screen.display_task_error(&error);
        assert!(screen.pending_scheduled_actions.is_empty());
        screen
            .pending_scheduled_actions
            .insert(key, dispatch.clone());
        assert_eq!(screen.pending_vote_operation, Some(bulk_id));
        assert!(matches!(
            screen.bulk_vote_handling_status,
            VoteHandlingStatus::CastingVotes
        ));
        screen.reset_for_network_switch();
        assert!(screen.pending_scheduled_actions.is_empty());
        assert!(!screen.finish_scheduled_dispatch(&dispatch));
    }

    #[test]
    fn followup_card_refresh_enters_shared_progress_state() {
        use egui_kittest::kittest::Queryable;
        use std::sync::atomic::{AtomicBool, Ordering};
        let (mut screen, _dir) = voting_ui_review_fixture();
        screen
            .app_context
            .seed_dpns_contest_for_test("alpha", None, false);
        screen.refresh();
        screen.vote_state = DpnsVoteStateSnapshot::default();
        screen.rebuild_cards();
        let refreshed = Arc::new(AtomicBool::new(false));
        let received_refresh = refreshed.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 1200.0))
            .build_ui(move |ui| {
                if matches!(
                    screen.ui(ui),
                    AppAction::BackendTask(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::QueryDPNSContests
                    ))
                ) {
                    received_refresh.store(
                        screen.refreshing_status == RefreshingStatus::Refreshing
                            && screen.refresh_banner.is_some(),
                        Ordering::Relaxed,
                    );
                }
            });
        harness.run();
        assert!(
            harness.query_by_label("alpha.dash").is_some(),
            "unavailable state must not hide the contest"
        );
        assert!(harness.query_by_label("Refresh voting").is_some());
        assert!(
            harness
                .query_by_label("Vote state is unavailable for 1 node; it's left out.")
                .is_some()
        );
        harness.get_by_label("Refresh voting").click();
        harness.run();
        assert!(refreshed.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn review_fixes_changed_vote_error_reloads_the_review_warning() {
        use egui_kittest::kittest::Queryable;
        let (mut screen, _dir) = voting_ui_review_fixture();
        let voter = screen.voting_identities[0].identity.id();
        let poll = screen.app_context.dpns_vote_poll_id("alpha").unwrap();
        screen
            .app_context
            .seed_proved_dpns_votes_for_test(voter, BTreeMap::new())
            .await
            .unwrap();
        screen.vote_state.reload(&screen.app_context).unwrap();
        assert!(matches!(
            screen.bulk_apply_votes(),
            AppAction::BackendTask(_)
        ));
        let operation_id = screen.pending_vote_operation.unwrap();
        screen
            .app_context
            .cache_confirmed_dpns_vote(voter, poll, ResourceVoteChoice::Lock)
            .unwrap();
        let error = TaskError::DpnsVoteReviewRequired;
        screen.display_backend_task_error(
            &BackendTaskContext::DpnsVoteOperation {
                network: screen.app_context.network(),
                operation_id,
            },
            &error,
        );
        screen.display_task_error(&error);
        assert!(screen.pending_vote_operation.is_none());
        assert!(matches!(
            screen.bulk_vote_handling_status,
            VoteHandlingStatus::NotStarted
        ));
        assert!(
            screen.show_bulk_schedule_popup,
            "DPN-005: the confirm reopens with the change warning"
        );
        assert_eq!(
            screen.build_review_plan().unwrap().aggregate.targets[0].current_choice,
            Some(ResourceVoteChoice::Lock)
        );
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 800.0))
            .build_ui(move |ui| {
                screen.show_review_and_cast_window(ui);
            });
        harness.run();
        assert!(
            harness
                .query_by_label(
                    "1 of these changes an earlier vote. Each node can change its vote 4 times per name."
                )
                .is_some()
        );
    }

    /// VOTE-TC-089: a contest only some node-set nodes voted on stays in To
    /// decide and says how many voted; the Voted view does not list it.
    #[tokio::test]
    async fn partly_voted_contest_stays_in_to_decide() {
        use egui_kittest::kittest::Queryable;
        let (ctx, _dir) = kv_ctx();
        ctx.seed_dpns_contest_for_test("alpha", None, false);
        let voted = masternode_identity(1, "node-one", true, ctx.network());
        let pending = masternode_identity(2, "node-two", true, ctx.network());
        let poll = ctx.dpns_vote_poll_id("alpha").unwrap();
        ctx.seed_proved_dpns_votes_for_test(pending.identity.id(), BTreeMap::new())
            .await
            .unwrap();
        ctx.cache_confirmed_dpns_vote(voted.identity.id(), poll, ResourceVoteChoice::Lock)
            .unwrap();
        let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
        screen.voting_identities = vec![voted.clone(), pending.clone()];
        screen.vote_state = DpnsVoteStateSnapshot::load(
            &ctx,
            &[voted.identity.id(), pending.identity.id()],
            &[poll],
        )
        .unwrap();
        screen.rebuild_cards();
        assert_eq!(screen.cards[0].placement, CardPlacement::ToDecide);

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1200.0, 1200.0))
            .build_ui(move |ui| {
                screen.ui(ui);
            });
        harness.run();
        assert!(harness.query_by_label("alpha.dash").is_some());
        assert!(harness.query_by_label("Voted with 1 of 2 nodes").is_some());
        assert!(
            harness
                .query_by_label("1 of your nodes has not voted. 1 of your nodes voted: Lock name.")
                .is_some()
        );
    }

    /// A request from another screen opens the contest's card, filtered and
    /// focused, once.
    #[tokio::test]
    async fn name_request_shows_that_contest() {
        let (screen, _dir) = two_contest_screen().await;
        let ctx = screen.lock_recover().app_context.clone();
        ctx.request_dpns_votes_for_name("beta".to_owned());
        let rendering = screen.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 1200.0))
            .build_ui(move |ui| {
                rendering.lock_recover().ui(ui);
            });
        harness.run();
        let screen = screen.lock_recover();
        assert_eq!(screen.view, VotesView::ToDecide);
        assert_eq!(screen.active_filter_term, "beta");
        assert_eq!(screen.focused_card.as_deref(), Some("beta"));
        assert!(ctx.take_dpns_votes_name_request().is_none());
    }

    /// Two open contests a single voting node has not voted on yet.
    async fn two_contest_screen() -> (Arc<Mutex<DPNSScreen>>, tempfile::TempDir) {
        let (ctx, dir) = kv_ctx();
        ctx.seed_dpns_contest_for_test("alpha", None, false);
        ctx.seed_dpns_contest_for_test("beta", None, false);
        let node = masternode_identity(1, "node-one", true, ctx.network());
        ctx.seed_proved_dpns_votes_for_test(node.identity.id(), BTreeMap::new())
            .await
            .unwrap();
        let polls = [
            ctx.dpns_vote_poll_id("alpha").unwrap(),
            ctx.dpns_vote_poll_id("beta").unwrap(),
        ];
        let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
        screen.voting_identities = vec![node.clone()];
        screen.vote_state =
            DpnsVoteStateSnapshot::load(&ctx, &[node.identity.id()], &polls).unwrap();
        screen.rebuild_cards();
        assert_eq!(screen.cards.len(), 2);
        (Arc::new(Mutex::new(screen)), dir)
    }

    fn staged(screen: &Arc<Mutex<DPNSScreen>>) -> Vec<(String, ResourceVoteChoice)> {
        screen
            .lock_recover()
            .selected_votes
            .iter()
            .map(|vote| (vote.contested_name.clone(), vote.vote_choice))
            .collect()
    }

    /// VOTE-TC-098: J, 1, J, L, Enter stages both decisions and opens the
    /// confirm step; keys do nothing without list focus or in the filter field.
    #[tokio::test]
    async fn keyboard_flow_stages_decisions_and_opens_confirm() {
        use egui_kittest::kittest::Queryable;
        let (screen, _dir) = two_contest_screen().await;
        let rendering = screen.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 1200.0))
            .build_ui(move |ui| {
                rendering.lock_recover().ui(ui);
            });
        harness.run();

        harness.key_press(egui::Key::J);
        harness.run();
        assert!(staged(&screen).is_empty(), "no list focus, no shortcut");

        screen.lock_recover().list_focused = true;
        for key in [egui::Key::J, egui::Key::Num1, egui::Key::J, egui::Key::L] {
            harness.key_press(key);
            harness.run();
        }
        let contender = ResourceVoteChoice::TowardsIdentity(Identifier::from([3; 32]));
        assert_eq!(
            staged(&screen),
            vec![
                ("alpha".to_owned(), contender),
                ("beta".to_owned(), ResourceVoteChoice::Lock),
            ]
        );

        harness
            .get_by_role(egui::accesskit::Role::TextInput)
            .click();
        harness.run();
        harness.key_press(egui::Key::A);
        harness.run();
        assert_eq!(
            staged(&screen)[1],
            ("beta".to_owned(), ResourceVoteChoice::Lock),
            "a focused text field swallows shortcuts"
        );

        screen.lock_recover().list_focused = true;
        harness.key_press(egui::Key::Escape);
        harness.run();
        screen.lock_recover().list_focused = true;
        harness.key_press(egui::Key::Enter);
        harness.run();
        assert!(screen.lock_recover().show_bulk_schedule_popup);
        assert!(
            harness
                .query_by_label("Cast 2 decisions with 1 node")
                .is_some()
        );
    }

    /// VOTE-TC-099: with several names selected, the bulk bar stages one
    /// decision on each.
    #[tokio::test]
    async fn bulk_bar_applies_a_decision_to_every_selected_name() {
        use egui_kittest::kittest::Queryable;
        let (screen, _dir) = two_contest_screen().await;
        screen.lock_recover().selected_cards =
            BTreeSet::from(["alpha".to_owned(), "beta".to_owned()]);
        let rendering = screen.clone();
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 1200.0))
            .build_ui(move |ui| {
                rendering.lock_recover().ui(ui);
            });
        harness.run();
        assert!(harness.query_by_label("2 names selected").is_some());
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Abstain")
            .click();
        harness.run();
        assert_eq!(
            staged(&screen),
            vec![
                ("alpha".to_owned(), ResourceVoteChoice::Abstain),
                ("beta".to_owned(), ResourceVoteChoice::Abstain),
            ]
        );
        harness
            .get_all_by_role_and_label(egui::accesskit::Role::Button, "Clear")
            .next()
            .expect("the bulk bar's Clear")
            .click();
        harness.run();
        assert!(staged(&screen).is_empty());
    }

    #[tokio::test]
    async fn blocker_empty_state_distinguishes_absent_nodes_from_missing_keys() {
        use egui_kittest::kittest::Queryable;
        for has_node in [false, true] {
            let (ctx, _dir, _events) = wired_ctx().await;
            if has_node {
                ctx.insert_local_qualified_identity(
                    &masternode_identity(41, "read-only-node", false, ctx.network()),
                    &None,
                )
                .unwrap();
            }
            let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1000.0, 1200.0))
                .build_ui(move |ui| {
                    screen.ui(ui);
                });
            harness.run();
            let (message, action) = if has_node {
                (
                    "None of your nodes has a voting key on this device.",
                    "Add a voting key",
                )
            } else {
                ("No masternodes are loaded.", "Load a masternode")
            };
            assert!(
                harness.query_by_label(message).is_some(),
                "missing {message}"
            );
            assert!(harness.query_by_label(action).is_some(), "missing {action}");
        }
    }

    #[test]
    fn voting_ui_keyless_users_can_read_cached_active_contests() {
        use egui_kittest::kittest::Queryable;
        let (context, _dir) = kv_ctx();
        let mut screen = DPNSScreen::new(&context, VotesView::ToDecide);
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
        screen.rebuild_cards();
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
        store.fail_all_reads(true);
        assert!(loaded_voting_identities(&context).is_err());
        for reviewing in [false, true] {
            let mut screen = DPNSScreen::new(&context, VotesView::ToDecide);
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
        let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
        screen.selected_votes = vec![SelectedVote {
            contested_name: "alpha".into(),
            vote_choice: ResourceVoteChoice::Lock,
            end_time: None,
        }];
        screen
            .node_overrides
            .insert(second.identity.id(), NodeTiming::DontUse);
        screen.refresh_on_arrival();
        assert_eq!(
            screen.node_overrides.get(&second.identity.id()),
            Some(&NodeTiming::DontUse),
            "an operator's per-node choice survives returning to the page"
        );
        assert!(!screen.node_overrides.contains_key(&first.identity.id()));
        let newly_loaded = masternode_identity(3, "new-node", true, ctx.network());
        ctx.insert_local_qualified_identity(&newly_loaded, &None)
            .unwrap();
        screen.refresh_on_arrival();
        assert_eq!(
            screen.node_overrides.get(&newly_loaded.identity.id()),
            Some(&NodeTiming::DontUse),
            "a node loaded after decisions were staged does not join them"
        );
        ctx.wallet_backend().unwrap().shutdown().await;
    }

    #[test]
    fn voting_ui_retry_reviews_only_the_failed_node_and_contest() {
        let (mut screen, _dir) = voting_ui_review_fixture();
        let operation = DpnsVoteOperation::new(vec![
            screen
                .build_review_plan()
                .unwrap()
                .aggregate
                .targets
                .remove(0),
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
        assert_eq!(
            plan.aggregate.targets[0].key,
            operation.targets[0].target.key
        );
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
                    "1 of these changes an earlier vote. Each node can change its vote 4 times per name."
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
                    "1 of these changes an earlier vote. Each node can change its vote 4 times per name."
                )
                .is_none()
        );
    }

    /// VOTE-TC-101: the drawer names each node and its typed status, and
    /// offers only the valid action.
    #[test]
    fn progress_drawer_identifies_each_node_and_status() {
        use egui_kittest::kittest::Queryable;
        let (screen, _temp_dir) = voting_ui_review_fixture();
        let plan = screen.build_review_plan().unwrap();
        let mut first = plan.aggregate.targets[0].clone();
        first.voter_alias = Some("node-one".to_owned());
        let mut second = first.clone();
        second.key.voter_id = Identifier::from([2; 32]);
        second.voter_alias = None;
        let mut third = first.clone();
        third.key.voter_id = Identifier::from([3; 32]);
        third.voter_alias = Some("node-three".to_owned());
        let mut operation = DpnsVoteOperation::new(vec![first, second, third]);
        operation.targets[0].status = DpnsVoteTargetStatus::Confirmed;
        operation.targets[1].status = DpnsVoteTargetStatus::Rejected;
        operation.targets[2].status = DpnsVoteTargetStatus::Unconfirmed;
        let context = screen.app_context.clone();
        context
            .insert_dpns_vote_operation(&mut operation, None)
            .unwrap();
        context.recompute_dpns_vote_attention();
        let mut state = progress_drawer::DrawerState::new(0);
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1000.0, 800.0))
            .build_ui(move |ui| {
                progress_drawer::expand(ui.ctx());
                progress_drawer::show(ui.ctx(), &context, &mut state);
            });
        harness.run();
        for label in [
            "Casting 3 votes · 2 done · 0 sending · 1 being checked",
            "node-one · alpha.dash · Abstain",
            "Confirmed",
            "02020…202 · alpha.dash · Abstain",
            "Rejected",
            "Review again",
            "Still being checked. Do not submit it again.",
            "Check again",
        ] {
            assert!(harness.query_by_label(label).is_some(), "missing {label}");
        }
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
        // What `MasternodesScreen::reset_for_network_change` does for its panel.
        screen.app_context = new_context;
        screen.reset_for_network_switch();
        screen.refresh_on_arrival();
        assert!(screen.selected_votes.is_empty());
        assert!(!screen.show_bulk_schedule_popup);
        assert!(screen.pending_scheduled_actions.is_empty());
    }

    #[test]
    fn voting_ui_sweep_completion_refreshes_the_scheduled_row() {
        let (mut screen, _temp_dir) = voting_ui_review_fixture();
        screen.view = VotesView::Scheduled;
        let mut target = screen
            .build_review_plan()
            .unwrap()
            .aggregate
            .targets
            .remove(0);
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

        let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
        screen.voting_identities = vec![first.clone(), second.clone()];
        screen.rebuild_cards();
        screen.relative_preset = std::time::Duration::from_secs(10 * 60);
        screen.node_overrides.insert(
            second.identity.id(),
            NodeTiming::Override(BatchTiming::BeforeEnd(screen.relative_preset)),
        );
        let end = now_ms() + 24 * 3_600_000;
        screen.selected_votes = vec![
            SelectedVote {
                contested_name: "alpha".to_owned(),
                vote_choice: ResourceVoteChoice::Lock,
                end_time: Some(end),
            },
            SelectedVote {
                contested_name: "beta".to_owned(),
                vote_choice: ResourceVoteChoice::Abstain,
                end_time: Some(end),
            },
        ];
        screen.vote_state = DpnsVoteStateSnapshot::load(
            &ctx,
            &[first.identity.id(), second.identity.id()],
            &[alpha, beta],
        )
        .expect("load proved vote state");

        let plan = screen.build_review_plan().expect("review plan");

        assert_eq!(plan.effective_count(), 2);
        assert_eq!(
            plan.aggregate.skipped_by_reason(),
            BTreeMap::from([(SkipReason::AlreadyVoted, 2)]),
            "two nodes across two contests, two already as requested"
        );
        assert_eq!(plan.node_count(), 2);
        assert!(plan.has_immediate());
        assert!(plan.has_scheduled());
        let retained = plan
            .targets()
            .iter()
            .map(|target| {
                (
                    target.voter_alias.clone(),
                    target.contested_name.clone(),
                    target.timing,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            retained,
            vec![
                (
                    Some("node-two".to_owned()),
                    "alpha".to_owned(),
                    VoteTiming::Scheduled(end - 10 * 60_000)
                ),
                (
                    Some("node-one".to_owned()),
                    "beta".to_owned(),
                    VoteTiming::Now
                ),
            ]
        );
        assert_eq!(plan.decisions.len(), 2);
        assert!(plan.decisions.iter().all(|(_, nodes)| *nodes == 1));
    }

    /// A review whose every target is a no-op must submit nothing.
    #[test]
    fn review_plan_reports_an_all_no_op_selection_as_nothing_to_submit() {
        let (ctx, _temp_dir) = kv_ctx();
        let alpha = ctx.dpns_vote_poll_id("alpha").expect("alpha poll id");
        let node = masternode_identity(3, "node-three", true, ctx.network());
        ctx.cache_confirmed_dpns_vote(node.identity.id(), alpha, ResourceVoteChoice::Lock)
            .expect("seed proved vote");

        let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
        screen.voting_identities = vec![node.clone()];
        screen.rebuild_cards();
        screen.selected_votes = vec![SelectedVote {
            contested_name: "alpha".to_owned(),
            vote_choice: ResourceVoteChoice::Lock,
            end_time: None,
        }];
        screen.vote_state = DpnsVoteStateSnapshot::load(&ctx, &[node.identity.id()], &[alpha])
            .expect("load proved vote state");

        let plan = screen.build_review_plan().expect("review plan");

        assert_eq!(plan.effective_count(), 0);
        assert_eq!(
            plan.aggregate.skipped_by_reason(),
            BTreeMap::from([(SkipReason::AlreadyVoted, 1)])
        );
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

        let mut screen = DPNSScreen::new(&ctx, VotesView::ToDecide);
        screen.voting_identities = vec![node.clone()];
        screen.rebuild_cards();
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
            plan.targets().first().map(|target| target.current_choice),
            Some(Some(ResourceVoteChoice::Lock))
        );
        assert_eq!(
            review_current_choice_label(Some(ResourceVoteChoice::Lock), None),
            "Lock name"
        );
    }

    #[test]
    fn review_labels_previous_choices() {
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
        let mut screen = DPNSScreen::new(&ctx, VotesView::Scheduled);
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

    /// VOTE-TC-067: a "before the end" schedule shows its preset next to the
    /// absolute UTC time; without a stored label only the absolute time shows.
    #[test]
    fn relative_schedule_shows_its_preset_and_absolute_time() {
        use egui_kittest::kittest::Queryable;
        let (ctx, _temp_dir) = kv_ctx();
        let voter_id = Identifier::from([25; 32]);
        let key = DpnsVoteTargetKey {
            network: ctx.network(),
            voter_id,
            vote_poll_id: Identifier::from([26; 32]),
        };
        let at = 1_900_000_000_000;
        let mut operation = DpnsVoteOperation::new(vec![DpnsVoteTarget {
            key: key.clone(),
            voter_alias: None,
            contested_name: "later".to_owned(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Scheduled(at),
        }]);
        operation.targets[0].relative_schedule_preset_ms = Some(6 * 3600 * 1000);
        ctx.insert_dpns_vote_operation(&mut operation, None)
            .expect("insert scheduled operation");
        let mut screen = DPNSScreen::new(&ctx, VotesView::Scheduled);
        let expected =
            relative_schedule_label(std::time::Duration::from_secs(6 * 3600), &utc_minute(at));
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1400.0, 800.0))
            .build_ui(move |ui| {
                screen.ui(ui);
            });
        harness.run();
        assert!(
            harness.query_by_label(&expected).is_some(),
            "missing {expected}"
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
        let mut screen = DPNSScreen::new(&ctx, VotesView::Scheduled);
        assert_eq!(screen.scheduled_votes.lock_recover().len(), 1);

        let outcomes = ctx
            .clear_all_scheduled_dpns_votes()
            .expect("clear scheduled votes");
        screen.display_task_result(BackendTaskSuccessResult::ScheduledVotesCleared(outcomes));

        assert!(screen.scheduled_votes.lock_recover().is_empty());
    }
}
