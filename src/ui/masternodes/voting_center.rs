//! Full-page Nodes → Votes → Review DPNS voting composer.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{Duration, LocalResult, TimeZone, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, ComboBox, RichText};

use crate::app::AppAction;
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::contested_name::ContestedName;
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteOutcome, DpnsVoteTarget,
    DpnsVoteTargetKey, DpnsVoteTargetStatus, VoteTiming,
};
use crate::model::qualified_identity::PrivateKeyTarget;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::state::dpns_vote_workspace::{
    ComposerKeyAction, DpnsVoteComposerStep, DpnsVoteWorkspace, DraftVoteTiming,
};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};

pub enum VotingCenterOutcome {
    None,
    BackToNodes,
    Action(Box<AppAction>),
}

pub struct DpnsVotingCenter {
    app_context: Arc<AppContext>,
    voters: Vec<QualifiedIdentity>,
    contests: Vec<ContestedName>,
    workspace: DpnsVoteWorkspace,
    submitted_operation: Option<DpnsVoteOperationId>,
    vote_state_refresh_dispatched: bool,
    editing_scheduled_key: Option<DpnsVoteTargetKey>,
    editing_scheduled_original: Option<DpnsVoteTarget>,
    focus_step_heading: bool,
}

struct ReviewDraft {
    operation: DpnsVoteOperation,
    exclusions: Vec<ReviewExclusion>,
}

struct ReviewExclusion {
    voter: String,
    contest: String,
    requested_choice: ResourceVoteChoice,
    reason: &'static str,
    no_op: bool,
}

impl DpnsVotingCenter {
    pub(crate) fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) {
        if matches!(result, BackendTaskSuccessResult::RefreshedDpnsContests) {
            self.vote_state_refresh_dispatched = false;
            match self.app_context.ongoing_contested_names() {
                Ok(contests) => self.contests = contests,
                Err(error) => {
                    tracing::warn!(?error, "Could not reload refreshed DPNS contests");
                }
            }
        }
        self.submitted_operation =
            updated_submitted_operation(self.submitted_operation, context, result);
    }

    pub(crate) fn display_backend_task_error(&mut self, context: &BackendTaskContext) {
        let Some(operation_id) = self.submitted_operation else {
            return;
        };
        if should_return_to_review_after_error(
            operation_id,
            self.app_context.network(),
            context,
            self.app_context
                .dpns_vote_operation(operation_id)
                .ok()
                .flatten()
                .is_some(),
        ) {
            self.submitted_operation = None;
            self.workspace.step = DpnsVoteComposerStep::Review;
            self.focus_step_heading = true;
        }
    }

    pub fn new(
        app_context: &Arc<AppContext>,
        preselected_voter: Option<Identifier>,
        preselected_contests: Vec<String>,
    ) -> Self {
        let voters = app_context
            .load_local_voting_identities()
            .unwrap_or_default();
        let mut workspace = DpnsVoteWorkspace::new(voters.iter().map(|voter| voter.identity.id()));
        if let Some(voter_id) = preselected_voter {
            workspace.prefilter_node(voter_id);
        }
        let contests = app_context.ongoing_contested_names().unwrap_or_default();
        if !preselected_contests.is_empty() {
            for name in preselected_contests {
                if contests
                    .iter()
                    .any(|contest| contest.normalized_contested_name == name)
                {
                    workspace
                        .contest_choices
                        .entry(name)
                        .or_insert(ResourceVoteChoice::Abstain);
                }
            }
        }
        Self {
            app_context: Arc::clone(app_context),
            voters,
            contests,
            workspace,
            submitted_operation: None,
            vote_state_refresh_dispatched: false,
            editing_scheduled_key: None,
            editing_scheduled_original: None,
            focus_step_heading: true,
        }
    }

    pub fn for_scheduled_edit(app_context: &Arc<AppContext>, outcome: &DpnsVoteOutcome) -> Self {
        let vote = &outcome.target;
        let mut center = Self::new(
            app_context,
            Some(vote.key.voter_id),
            vec![vote.contested_name.clone()],
        );
        center
            .workspace
            .contest_choices
            .insert(vote.contested_name.clone(), vote.requested_choice);
        let VoteTiming::Scheduled(timestamp) = vote.timing else {
            return center;
        };
        center
            .workspace
            .node_timing
            .insert(vote.key.voter_id, scheduled_offset_from_now(timestamp));
        center.editing_scheduled_key = Some(vote.key.clone());
        center.editing_scheduled_original = Some(vote.clone());
        center
    }

    pub fn for_quick_votes(
        app_context: &Arc<AppContext>,
        voter_id: Identifier,
        choices: BTreeMap<String, ResourceVoteChoice>,
    ) -> Self {
        let mut center = Self::new(
            app_context,
            Some(voter_id),
            choices.keys().cloned().collect(),
        );
        center.workspace.contest_choices = choices;
        center.workspace.step = DpnsVoteComposerStep::Review;
        center
    }

    pub fn for_bulk_choices(
        app_context: &Arc<AppContext>,
        choices: BTreeMap<String, ResourceVoteChoice>,
    ) -> Self {
        let mut center = Self::new(app_context, None, choices.keys().cloned().collect());
        center.workspace.contest_choices = choices;
        center
    }

    pub fn for_operation(app_context: &Arc<AppContext>, operation_id: DpnsVoteOperationId) -> Self {
        let mut center = Self::new(app_context, None, Vec::new());
        center.submitted_operation = Some(operation_id);
        center
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> VotingCenterOutcome {
        if self.submitted_operation.is_some() {
            return self.render_operation(ui);
        }
        let (enter, escape) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Escape),
            )
        });
        let can_continue = match self.workspace.step {
            DpnsVoteComposerStep::Nodes => self.workspace.selected_node_count() > 0,
            DpnsVoteComposerStep::Votes => !self.workspace.contest_choices.is_empty(),
            DpnsVoteComposerStep::Review => !self.build_review().operation.targets.is_empty(),
        };
        match self.workspace.keyboard_action(enter, escape, can_continue) {
            ComposerKeyAction::CloseDraft => return VotingCenterOutcome::BackToNodes,
            ComposerKeyAction::Advance => {
                self.workspace.step = match self.workspace.step {
                    DpnsVoteComposerStep::Nodes => DpnsVoteComposerStep::Votes,
                    DpnsVoteComposerStep::Votes | DpnsVoteComposerStep::Review => {
                        DpnsVoteComposerStep::Review
                    }
                };
                self.focus_step_heading = true;
            }
            ComposerKeyAction::Submit => {
                return self.submit_operation(self.build_review().operation);
            }
            ComposerKeyAction::None => {}
        }
        let needs_refresh = self
            .selected_current_states()
            .iter()
            .any(|(_, state)| matches!(state, DpnsCurrentVoteState::Checking));
        if needs_refresh && !self.vote_state_refresh_dispatched {
            self.vote_state_refresh_dispatched = true;
            return VotingCenterOutcome::Action(Box::new(AppAction::BackendTask(
                BackendTask::ContestedResourceTask(ContestedResourceTask::QueryDPNSContests),
            )));
        }

        match self.workspace.step {
            DpnsVoteComposerStep::Nodes => self.render_nodes(ui),
            DpnsVoteComposerStep::Votes => self.render_votes(ui),
            DpnsVoteComposerStep::Review => self.render_review(ui),
        }
    }

    fn render_nodes(&mut self, ui: &mut egui::Ui) -> VotingCenterOutcome {
        let dark_mode = ui.style().visuals.dark_mode;
        self.step_heading(ui, "Step 1 of 3: Nodes and timing");
        ui.label("Choose which nodes will vote and when each node should submit.");
        if self.voters.is_empty() {
            ui.separator();
            let go_to_nodes = ui
                .vertical_centered(|ui| {
                    ui.heading("No voting nodes are available");
                    ui.label("Load a masternode on the Nodes tab before creating a vote.");
                    ui.add_space(8.0);
                    ComponentStyles::add_primary_button_enabled(ui, true, "Go to Nodes").clicked()
                })
                .inner;
            return if go_to_nodes {
                VotingCenterOutcome::BackToNodes
            } else {
                VotingCenterOutcome::None
            };
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Set all:");
            timing_combo(
                ui,
                "voting_center_set_all",
                &mut self.workspace.set_all_timing,
            );
            if ComponentStyles::add_secondary_button(ui, "Apply", dark_mode).clicked() {
                self.workspace.apply_timing_to_selected();
            }
        });
        ui.separator();
        for voter in &self.voters {
            let voter_id = voter.identity.id();
            let alias = voter
                .alias
                .clone()
                .unwrap_or_else(|| voter_id.to_string(Encoding::Base58));
            let has_voting_key = has_loaded_voting_key(voter);
            ui.horizontal_wrapped(|ui| {
                let mut selected = self.workspace.is_node_selected(&voter_id);
                let checkbox = ui.add_enabled(
                    has_voting_key,
                    egui::Checkbox::new(&mut selected, RichText::new(alias).strong()),
                );
                if checkbox.changed() {
                    self.workspace.set_node_selected(voter_id, selected);
                }
                checkbox
                    .disabled_tooltip("Load this node's voting private key before selecting it.");
                let timing = self
                    .workspace
                    .node_timing
                    .entry(voter_id)
                    .or_insert(DraftVoteTiming::Now);
                ui.add_enabled_ui(selected && has_voting_key, |ui| {
                    timing_combo(ui, format!("voting_center_node_{voter_id}"), timing);
                    render_schedule_offset(ui, timing);
                });
                if !has_voting_key {
                    ui.label(
                        RichText::new("Voting key missing")
                            .color(DashColors::warning_color(dark_mode)),
                    );
                }
            });
        }
        ui.separator();
        let enabled = self.workspace.selected_node_count() > 0;
        if ComponentStyles::add_primary_button_enabled(ui, enabled, "Next: Choose votes")
            .disabled_tooltip("Choose at least one node before continuing.")
            .clicked()
        {
            self.workspace.step = DpnsVoteComposerStep::Votes;
            self.focus_step_heading = true;
        }
        VotingCenterOutcome::None
    }

    fn render_votes(&mut self, ui: &mut egui::Ui) -> VotingCenterOutcome {
        let dark_mode = ui.style().visuals.dark_mode;
        self.step_heading(ui, "Step 2 of 3: Votes");
        ui.label("Choose one requested vote for each contested name.");
        let mut outcome = VotingCenterOutcome::None;
        if self.contests.is_empty() {
            ui.separator();
            ui.label("No active contests are available. Refresh contests to check again.");
            if ComponentStyles::add_secondary_button(ui, "Refresh contests", dark_mode).clicked() {
                self.vote_state_refresh_dispatched = true;
                outcome = VotingCenterOutcome::Action(Box::new(AppAction::BackendTask(
                    BackendTask::ContestedResourceTask(ContestedResourceTask::QueryDPNSContests),
                )));
            }
        }
        for contest in &self.contests {
            let name = &contest.normalized_contested_name;
            ui.separator();
            ui.label(RichText::new(format!("{name}.dash")).strong());
            let states = self.current_states_for_contest(contest);
            ui.label(
                RichText::new(current_summary(&states))
                    .color(DashColors::text_secondary(dark_mode)),
            );
            let vote_poll_id = self.app_context.dpns_vote_poll_id(name).ok();
            let controls_enabled = states.iter().any(|(voter_id, state, locked)| {
                let lock_is_this_edit = vote_poll_id.is_some_and(|vote_poll_id| {
                    self.editing_scheduled_key.as_ref()
                        == Some(&DpnsVoteTargetKey {
                            network: self.app_context.network(),
                            voter_id: *voter_id,
                            vote_poll_id,
                        })
                });
                (!*locked || lock_is_this_edit)
                    && matches!(state, DpnsCurrentVoteState::Available(_))
            });
            let selected = self.workspace.contest_choices.get(name).copied();
            ui.add_enabled_ui(controls_enabled, |ui| {
                ui.horizontal_wrapped(|ui| {
                    vote_choice(
                        ui,
                        selected,
                        ResourceVoteChoice::Abstain,
                        "Abstain",
                        &mut self.workspace,
                        name,
                    );
                    vote_choice(
                        ui,
                        selected,
                        ResourceVoteChoice::Lock,
                        "Lock",
                        &mut self.workspace,
                        name,
                    );
                    for candidate in contest.contestants.as_deref().unwrap_or_default() {
                        vote_choice(
                            ui,
                            selected,
                            ResourceVoteChoice::TowardsIdentity(candidate.id),
                            &format!("Vote for {}", candidate.name),
                            &mut self.workspace,
                            name,
                        );
                    }
                });
            });
            for (voter_id, state, locked) in &states {
                let voter = self.voter_label(*voter_id);
                let status = match (state, locked) {
                    (_, true) => "This target already has a voting operation in progress.",
                    (DpnsCurrentVoteState::Checking, false) => {
                        "DET is checking this node's current vote."
                    }
                    (DpnsCurrentVoteState::Unavailable, false) => {
                        "This node's current vote is unavailable."
                    }
                    _ => continue,
                };
                ui.label(
                    RichText::new(format!("{voter}: {status}"))
                        .color(DashColors::text_secondary(dark_mode)),
                );
            }
            let can_refresh = states.iter().any(|(_, state, locked)| {
                !locked
                    && matches!(
                        state,
                        DpnsCurrentVoteState::Checking | DpnsCurrentVoteState::Unavailable
                    )
            });
            if can_refresh
                && ComponentStyles::add_secondary_button(ui, "Refresh vote state", dark_mode)
                    .clicked()
            {
                self.vote_state_refresh_dispatched = true;
                outcome = VotingCenterOutcome::Action(Box::new(AppAction::BackendTask(
                    BackendTask::ContestedResourceTask(ContestedResourceTask::QueryDPNSContests),
                )));
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ComponentStyles::add_secondary_button(ui, "Back", dark_mode).clicked() {
                self.workspace.step = DpnsVoteComposerStep::Nodes;
                self.focus_step_heading = true;
            }
            let target_count =
                self.workspace.selected_node_count() * self.workspace.contest_choices.len();
            let enabled = target_count > 0;
            if ComponentStyles::add_primary_button_enabled(
                ui,
                enabled,
                format!("Review {target_count} targets"),
            )
            .disabled_tooltip("Choose at least one contested name before continuing.")
            .clicked()
            {
                self.workspace.step = DpnsVoteComposerStep::Review;
                self.focus_step_heading = true;
            }
            if ComponentStyles::add_secondary_button(ui, "Close Voting Center", dark_mode).clicked()
            {
                outcome = VotingCenterOutcome::BackToNodes;
            }
        });
        outcome
    }

    fn render_review(&mut self, ui: &mut egui::Ui) -> VotingCenterOutcome {
        let dark_mode = ui.style().visuals.dark_mode;
        self.step_heading(ui, "Step 3 of 3: Review");
        let review = self.build_review();
        for outcome in &review.operation.targets {
            let voter = outcome
                .target
                .voter_alias
                .clone()
                .unwrap_or_else(|| shorten_identifier(outcome.target.key.voter_id));
            ui.group(|ui| {
                ui.label(RichText::new(format!(
                    "{voter} / {}.dash",
                    outcome.target.contested_name
                ))
                .strong());
                ui.label(format!(
                    "{} → {}",
                    self.choice_label(
                        &outcome.target.contested_name,
                        outcome.target.current_choice
                    ),
                    self.choice_label(
                        &outcome.target.contested_name,
                        Some(outcome.target.requested_choice)
                    )
                ));
                if self.editing_scheduled_key.as_ref() == Some(&outcome.target.key) {
                    if let Some(original) = &self.editing_scheduled_original {
                        ui.label(scheduled_replacement_summary(
                            original,
                            &outcome.target,
                            |name, choice| self.choice_label(name, choice),
                        ));
                    }
                } else {
                    ui.label(format_timing(outcome.target.timing));
                }
                if outcome.target.current_choice.is_some() {
                    ui.label(
                        RichText::new(
                            "This changes an existing vote. Platform permits only a limited number of vote changes.",
                        )
                        .color(DashColors::warning_color(dark_mode)),
                    );
                }
            });
        }
        if review.operation.no_op_count > 0 {
            ui.label(format!(
                "{} targets already have the requested vote and will not be submitted.",
                review.operation.no_op_count
            ));
        }
        for exclusion in &review.exclusions {
            ui.label(format!(
                "{} / {}.dash → {} was excluded. {}",
                exclusion.voter,
                exclusion.contest,
                self.choice_label(&exclusion.contest, Some(exclusion.requested_choice)),
                exclusion.reason
            ));
        }
        ui.label(format!(
            "{} targets total. Each submitted vote uses Platform credits.",
            review.operation.targets.len()
        ));
        ui.horizontal(|ui| {
            if ComponentStyles::add_secondary_button(ui, "Back", dark_mode).clicked() {
                self.workspace.step = DpnsVoteComposerStep::Votes;
                self.focus_step_heading = true;
            }
        });
        if review.operation.targets.is_empty() {
            if review.operation.no_op_count > 0
                && review.exclusions.iter().all(|exclusion| exclusion.no_op)
            {
                ui.label(
                    "Every selected node already has the requested vote. Nothing will be submitted.",
                );
            } else {
                ui.label(
                    "No targets are ready to submit. Refresh unavailable vote state or wait for in-progress targets.",
                );
            }
            return VotingCenterOutcome::None;
        }
        let target_count = review.operation.targets.len();
        if ComponentStyles::add_primary_button(ui, format!("Submit {target_count} targets"))
            .clicked()
        {
            return self.submit_operation(review.operation);
        }
        VotingCenterOutcome::None
    }

    fn render_operation(&mut self, ui: &mut egui::Ui) -> VotingCenterOutcome {
        let dark_mode = ui.style().visuals.dark_mode;
        let Some(operation_id) = self.submitted_operation else {
            return VotingCenterOutcome::None;
        };
        ui.heading("Voting operation");
        match self.app_context.dpns_vote_operation(operation_id) {
            Ok(Some(operation)) => {
                for outcome in &operation.targets {
                    ui.group(|ui| {
                        let voter = outcome
                            .target
                            .voter_alias
                            .clone()
                            .unwrap_or_else(|| shorten_identifier(outcome.target.key.voter_id));
                        ui.label(
                            RichText::new(format!(
                                "{voter} / {}.dash",
                                outcome.target.contested_name
                            ))
                            .strong(),
                        );
                        ui.label(format!(
                            "{} → {}",
                            self.choice_label(
                                &outcome.target.contested_name,
                                outcome.target.current_choice
                            ),
                            self.choice_label(
                                &outcome.target.contested_name,
                                Some(outcome.target.requested_choice)
                            )
                        ));
                        ui.label(format_timing(outcome.target.timing));
                        ui.label(format!("Status: {}", status_label(outcome.status)));
                        ui.label(status_explanation(outcome.status));
                        if let Some(hash) = outcome.transition_hash {
                            let hash = hex::encode(hash);
                            if ui.small_button("Copy transition hash").clicked() {
                                ui.ctx().copy_text(hash);
                            }
                        }
                    });
                }
                if operation
                    .targets
                    .iter()
                    .any(|outcome| outcome.status == DpnsVoteTargetStatus::Unconfirmed)
                    && ComponentStyles::add_secondary_button(ui, "Check again", dark_mode).clicked()
                {
                    return VotingCenterOutcome::Action(Box::new(AppAction::BackendTask(
                        BackendTask::ContestedResourceTask(
                            ContestedResourceTask::ReconcileDpnsVoteOperation(
                                operation_id,
                                self.app_context.network(),
                            ),
                        ),
                    )));
                }
                if operation.targets.iter().any(|outcome| {
                    matches!(
                        outcome.status,
                        DpnsVoteTargetStatus::Rejected
                            | DpnsVoteTargetStatus::FailedBeforeSubmission
                            | DpnsVoteTargetStatus::NotApplied
                    )
                }) && ComponentStyles::add_secondary_button(ui, "Review again", dark_mode)
                    .clicked()
                {
                    self.workspace.contest_choices = operation
                        .targets
                        .iter()
                        .filter(|outcome| {
                            matches!(
                                outcome.status,
                                DpnsVoteTargetStatus::Rejected
                                    | DpnsVoteTargetStatus::FailedBeforeSubmission
                                    | DpnsVoteTargetStatus::NotApplied
                            )
                        })
                        .map(|outcome| {
                            (
                                outcome.target.contested_name.clone(),
                                outcome.target.requested_choice,
                            )
                        })
                        .collect();
                    for outcome in &operation.targets {
                        if matches!(
                            outcome.status,
                            DpnsVoteTargetStatus::Rejected
                                | DpnsVoteTargetStatus::FailedBeforeSubmission
                                | DpnsVoteTargetStatus::NotApplied
                        ) {
                            self.workspace
                                .set_node_selected(outcome.target.key.voter_id, true);
                            self.workspace.node_timing.insert(
                                outcome.target.key.voter_id,
                                match outcome.target.timing {
                                    VoteTiming::Now => DraftVoteTiming::Now,
                                    VoteTiming::Scheduled(timestamp) => {
                                        scheduled_offset_from_now(timestamp)
                                    }
                                },
                            );
                        }
                    }
                    self.workspace.step = DpnsVoteComposerStep::Review;
                    self.submitted_operation = None;
                    self.focus_step_heading = true;
                }
                if ComponentStyles::add_secondary_button(ui, "Continue in background", dark_mode)
                    .clicked()
                {
                    return VotingCenterOutcome::BackToNodes;
                }
            }
            Ok(None) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Queuing votes…");
                });
            }
            Err(error) => {
                ui.label(error.to_string());
            }
        }
        VotingCenterOutcome::None
    }

    fn selected_voters(&self) -> Vec<QualifiedIdentity> {
        self.voters
            .iter()
            .filter(|voter| self.workspace.is_node_selected(&voter.identity.id()))
            .filter(|voter| has_loaded_voting_key(voter))
            .cloned()
            .collect()
    }

    fn submit_operation(&mut self, operation: DpnsVoteOperation) -> VotingCenterOutcome {
        self.submitted_operation = Some(operation.id);
        VotingCenterOutcome::Action(Box::new(AppAction::BackendTask(
            BackendTask::ContestedResourceTask(ContestedResourceTask::SubmitDpnsVoteOperation(
                operation,
                self.selected_voters(),
                self.editing_scheduled_key.clone(),
                self.app_context.network(),
            )),
        )))
    }

    fn step_heading(&mut self, ui: &mut egui::Ui, text: &str) {
        let response = ui.heading(text);
        if std::mem::take(&mut self.focus_step_heading) {
            response.request_focus();
        }
    }

    fn selected_current_states(&self) -> Vec<(Identifier, DpnsCurrentVoteState)> {
        self.contests
            .iter()
            .flat_map(|contest| {
                let poll_id = self
                    .app_context
                    .dpns_vote_poll_id(&contest.normalized_contested_name)
                    .ok();
                self.selected_voters().into_iter().filter_map(move |voter| {
                    let poll_id = poll_id?;
                    let voter_id = voter.identity.id();
                    Some((
                        voter_id,
                        self.app_context
                            .dpns_current_vote_state(voter_id, poll_id)
                            .unwrap_or(DpnsCurrentVoteState::Unavailable),
                    ))
                })
            })
            .collect()
    }

    fn current_states_for_contest(
        &self,
        contest: &ContestedName,
    ) -> Vec<(Identifier, DpnsCurrentVoteState, bool)> {
        let Ok(vote_poll_id) = self
            .app_context
            .dpns_vote_poll_id(&contest.normalized_contested_name)
        else {
            return self
                .selected_voters()
                .into_iter()
                .map(|voter| {
                    (
                        voter.identity.id(),
                        DpnsCurrentVoteState::Unavailable,
                        false,
                    )
                })
                .collect();
        };
        self.selected_voters()
            .into_iter()
            .map(|voter| {
                let voter_id = voter.identity.id();
                let key = DpnsVoteTargetKey {
                    network: self.app_context.network(),
                    voter_id,
                    vote_poll_id,
                };
                (
                    voter_id,
                    self.app_context
                        .dpns_current_vote_state(voter_id, vote_poll_id)
                        .unwrap_or(DpnsCurrentVoteState::Unavailable),
                    self.app_context
                        .dpns_vote_target_status(&key)
                        .ok()
                        .flatten()
                        .is_some(),
                )
            })
            .collect()
    }

    fn build_review(&self) -> ReviewDraft {
        let mut targets = Vec::new();
        let mut exclusions = Vec::new();
        for voter in self.selected_voters() {
            let voter_id = voter.identity.id();
            let draft_timing = self.workspace.node_timing[&voter_id];
            let timing = match draft_timing {
                DraftVoteTiming::Now => VoteTiming::Now,
                DraftVoteTiming::Scheduled {
                    days,
                    hours,
                    minutes,
                } => VoteTiming::Scheduled(
                    (Utc::now()
                        + Duration::days(i64::from(days))
                        + Duration::hours(i64::from(hours))
                        + Duration::minutes(i64::from(minutes)))
                    .timestamp_millis() as u64,
                ),
            };
            for (name, requested_choice) in &self.workspace.contest_choices {
                let vote_poll_id = match self.app_context.dpns_vote_poll_id(name) {
                    Ok(vote_poll_id) => vote_poll_id,
                    Err(_) => {
                        exclusions.push(ReviewExclusion {
                            voter: self.voter_label(voter_id),
                            contest: name.clone(),
                            requested_choice: *requested_choice,
                            reason: "Refresh this contest before submitting a vote.",
                            no_op: false,
                        });
                        continue;
                    }
                };
                let key = DpnsVoteTargetKey {
                    network: self.app_context.network(),
                    voter_id,
                    vote_poll_id,
                };
                let existing_status = match self.app_context.dpns_vote_target_status(&key) {
                    Ok(status) => status,
                    Err(_) => {
                        exclusions.push(ReviewExclusion {
                            voter: self.voter_label(voter_id),
                            contest: name.clone(),
                            requested_choice: *requested_choice,
                            reason: "DET could not check whether this target is already in use.",
                            no_op: false,
                        });
                        continue;
                    }
                };
                let replacing_schedule = is_explicit_schedule_replacement(
                    self.editing_scheduled_key.as_ref(),
                    &key,
                    existing_status,
                    timing,
                );
                if existing_status.is_some() && !replacing_schedule {
                    exclusions.push(ReviewExclusion {
                        voter: self.voter_label(voter_id),
                        contest: name.clone(),
                        requested_choice: *requested_choice,
                        reason: "Another voting operation is still using this target.",
                        no_op: false,
                    });
                    continue;
                }
                let current_choice = match self
                    .app_context
                    .dpns_current_vote_state(voter_id, vote_poll_id)
                {
                    Ok(DpnsCurrentVoteState::Available(current_choice)) => current_choice,
                    Ok(DpnsCurrentVoteState::Checking) => {
                        exclusions.push(ReviewExclusion {
                            voter: self.voter_label(voter_id),
                            contest: name.clone(),
                            requested_choice: *requested_choice,
                            reason: "DET is still checking this node's current vote.",
                            no_op: false,
                        });
                        continue;
                    }
                    Ok(DpnsCurrentVoteState::Unavailable) | Err(_) => {
                        exclusions.push(ReviewExclusion {
                            voter: self.voter_label(voter_id),
                            contest: name.clone(),
                            requested_choice: *requested_choice,
                            reason: "Refresh this node's vote state before submitting it.",
                            no_op: false,
                        });
                        continue;
                    }
                };
                if current_choice == Some(*requested_choice) {
                    exclusions.push(ReviewExclusion {
                        voter: self.voter_label(voter_id),
                        contest: name.clone(),
                        requested_choice: *requested_choice,
                        reason: "This node already has the requested vote, so nothing will be submitted.",
                        no_op: true,
                    });
                }
                targets.push(DpnsVoteTarget {
                    key,
                    voter_alias: voter.alias.clone(),
                    contested_name: name.clone(),
                    requested_choice: *requested_choice,
                    current_choice,
                    timing,
                });
            }
        }
        ReviewDraft {
            operation: DpnsVoteOperation::new(targets),
            exclusions,
        }
    }

    fn voter_label(&self, voter_id: Identifier) -> String {
        self.voters
            .iter()
            .find(|voter| voter.identity.id() == voter_id)
            .and_then(|voter| voter.alias.clone())
            .unwrap_or_else(|| shorten_identifier(voter_id))
    }

    fn choice_label(&self, contest_name: &str, choice: Option<ResourceVoteChoice>) -> String {
        let contestants = self
            .contests
            .iter()
            .find(|contest| contest.normalized_contested_name == contest_name)
            .and_then(|contest| contest.contestants.as_deref())
            .unwrap_or_default();
        choice_label(choice, contestants)
    }
}

fn timing_combo(ui: &mut egui::Ui, id: impl Into<String>, timing: &mut DraftVoteTiming) {
    ComboBox::from_id_salt(id.into())
        .selected_text(match timing {
            DraftVoteTiming::Now => "Cast now",
            DraftVoteTiming::Scheduled { .. } => "Schedule",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(timing, DraftVoteTiming::Now, "Cast now");
            if ui
                .selectable_label(
                    matches!(timing, DraftVoteTiming::Scheduled { .. }),
                    "Schedule",
                )
                .clicked()
            {
                *timing = DraftVoteTiming::Scheduled {
                    days: 0,
                    hours: 0,
                    minutes: 0,
                };
            }
        });
}

fn render_schedule_offset(ui: &mut egui::Ui, timing: &mut DraftVoteTiming) {
    if let DraftVoteTiming::Scheduled {
        days,
        hours,
        minutes,
    } = timing
    {
        ui.add(egui::DragValue::new(days).prefix("Days: ").range(0..=14));
        ui.add(egui::DragValue::new(hours).prefix("Hours: ").range(0..=23));
        ui.add(
            egui::DragValue::new(minutes)
                .prefix("Minutes: ")
                .range(0..=59),
        );
    }
}

fn vote_choice(
    ui: &mut egui::Ui,
    selected: Option<ResourceVoteChoice>,
    choice: ResourceVoteChoice,
    label: &str,
    workspace: &mut DpnsVoteWorkspace,
    name: &str,
) {
    if ui
        .selectable_label(selected == Some(choice), label)
        .clicked()
    {
        workspace.contest_choices.insert(name.to_owned(), choice);
    }
}

fn current_summary(states: &[(Identifier, DpnsCurrentVoteState, bool)]) -> String {
    if states.is_empty() {
        return "Current vote state is unavailable for selected nodes".to_owned();
    }
    let checking = states
        .iter()
        .filter(|(_, state, locked)| !locked && *state == DpnsCurrentVoteState::Checking)
        .count();
    let unavailable = states
        .iter()
        .filter(|(_, state, locked)| !locked && *state == DpnsCurrentVoteState::Unavailable)
        .count();
    let busy = states.iter().filter(|(_, _, locked)| *locked).count();
    let available = states
        .iter()
        .filter(|(_, state, locked)| !locked && matches!(state, DpnsCurrentVoteState::Available(_)))
        .count();
    let not_voted = states
        .iter()
        .filter(|(_, state, locked)| !locked && *state == DpnsCurrentVoteState::Available(None))
        .count();
    if available == states.len() && not_voted == states.len() {
        return "Current across selected nodes: Not voted".to_owned();
    }
    if available == states.len() && not_voted == 0 {
        return "Current across selected nodes: All already voted".to_owned();
    }
    let mut parts = Vec::new();
    if not_voted > 0 {
        parts.push(format!("{not_voted} not voted"));
    }
    let already_voted = available.saturating_sub(not_voted);
    if already_voted > 0 {
        parts.push(format!("{already_voted} already voted"));
    }
    if checking > 0 {
        parts.push(format!("{checking} checking"));
    }
    if unavailable > 0 {
        parts.push(format!("{unavailable} unavailable"));
    }
    if busy > 0 {
        parts.push(format!("{busy} in progress"));
    }
    if parts.is_empty() {
        "Current across selected nodes: No usable targets".to_owned()
    } else {
        format!("Current across selected nodes: {}", parts.join(", "))
    }
}

fn choice_label(
    choice: Option<ResourceVoteChoice>,
    contestants: &[crate::model::contested_name::Contestant],
) -> String {
    match choice {
        None => "Not voted".to_owned(),
        Some(ResourceVoteChoice::Abstain) => "Abstain".to_owned(),
        Some(ResourceVoteChoice::Lock) => "Lock".to_owned(),
        Some(ResourceVoteChoice::TowardsIdentity(identity)) => {
            let short_id = shorten_identifier(identity);
            contestants
                .iter()
                .find(|contestant| contestant.id == identity)
                .map(|contestant| format!("{} ({short_id})", contestant.name))
                .unwrap_or_else(|| format!("Candidate {short_id}"))
        }
    }
}

fn shorten_identifier(identifier: Identifier) -> String {
    let encoded = identifier.to_string(Encoding::Base58);
    if encoded.chars().count() <= 12 {
        return encoded;
    }
    let first = encoded.chars().take(6).collect::<String>();
    let last = encoded
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{first}…{last}")
}

fn format_timing(timing: VoteTiming) -> String {
    match timing {
        VoteTiming::Now => "When: Cast now".to_owned(),
        VoteTiming::Scheduled(timestamp) => match Utc.timestamp_millis_opt(timestamp as i64) {
            LocalResult::Single(date_time) => format!(
                "When: {} UTC ({})",
                date_time.format("%Y-%m-%d %H:%M"),
                HumanTime::from(date_time)
            ),
            _ => "When: The scheduled time is unavailable.".to_owned(),
        },
    }
}

fn scheduled_replacement_summary(
    original: &DpnsVoteTarget,
    replacement: &DpnsVoteTarget,
    choice_label: impl Fn(&str, Option<ResourceVoteChoice>) -> String,
) -> String {
    format!(
        "Replacing scheduled vote: {} at {} → {} at {}.",
        choice_label(&original.contested_name, Some(original.requested_choice)),
        scheduled_time_label(original.timing),
        choice_label(
            &replacement.contested_name,
            Some(replacement.requested_choice)
        ),
        scheduled_time_label(replacement.timing),
    )
}

fn scheduled_time_label(timing: VoteTiming) -> String {
    match timing {
        VoteTiming::Now => "now".to_owned(),
        VoteTiming::Scheduled(timestamp) => match Utc.timestamp_millis_opt(timestamp as i64) {
            LocalResult::Single(date_time) => {
                format!("{} UTC", date_time.format("%Y-%m-%d %H:%M"))
            }
            _ => "an unavailable time".to_owned(),
        },
    }
}

fn scheduled_offset_from_now(timestamp: u64) -> DraftVoteTiming {
    let remaining_minutes = timestamp.saturating_sub(Utc::now().timestamp_millis() as u64) / 60_000;
    DraftVoteTiming::Scheduled {
        days: (remaining_minutes / (24 * 60)) as u32,
        hours: ((remaining_minutes / 60) % 24) as u32,
        minutes: (remaining_minutes % 60) as u32,
    }
}

fn is_explicit_schedule_replacement(
    editing_key: Option<&DpnsVoteTargetKey>,
    target_key: &DpnsVoteTargetKey,
    existing_status: Option<DpnsVoteTargetStatus>,
    replacement_timing: VoteTiming,
) -> bool {
    editing_key == Some(target_key)
        && existing_status == Some(DpnsVoteTargetStatus::Scheduled)
        && matches!(replacement_timing, VoteTiming::Scheduled(_))
}

fn should_return_to_review_after_error(
    submitted_operation: DpnsVoteOperationId,
    network: dash_sdk::dpp::dashcore::Network,
    context: &BackendTaskContext,
    operation_was_journaled: bool,
) -> bool {
    !operation_was_journaled
        && context
            == &BackendTaskContext::DpnsVoteOperation {
                network,
                operation_id: submitted_operation,
            }
}

fn updated_submitted_operation(
    submitted_operation: Option<DpnsVoteOperationId>,
    context: &BackendTaskContext,
    result: &BackendTaskSuccessResult,
) -> Option<DpnsVoteOperationId> {
    match (submitted_operation, context, result) {
        (
            Some(submitted),
            BackendTaskContext::DpnsVoteOperation {
                network: submitted_network,
                operation_id: submitted_id,
            },
            BackendTaskSuccessResult::DpnsVoteOperationUpdated {
                network: result_network,
                operation_id: result_id,
            },
        ) if submitted == *submitted_id && submitted_network == result_network => Some(*result_id),
        _ => submitted_operation,
    }
}

fn has_loaded_voting_key(voter: &QualifiedIdentity) -> bool {
    voter
        .private_keys
        .keys_set()
        .iter()
        .any(|(target, _)| *target == PrivateKeyTarget::PrivateKeyOnVoterIdentity)
}

fn status_label(status: DpnsVoteTargetStatus) -> &'static str {
    match status {
        DpnsVoteTargetStatus::Scheduled => "Scheduled",
        DpnsVoteTargetStatus::Queued => "Queued",
        DpnsVoteTargetStatus::Submitting => "Submitting",
        DpnsVoteTargetStatus::Confirming => "Confirming",
        DpnsVoteTargetStatus::Confirmed => "Confirmed",
        DpnsVoteTargetStatus::Unconfirmed => "Checking result",
        DpnsVoteTargetStatus::Rejected => "Rejected",
        DpnsVoteTargetStatus::FailedBeforeSubmission => "Failed before submission",
        DpnsVoteTargetStatus::NotApplied => "Not applied",
    }
}

fn status_explanation(status: DpnsVoteTargetStatus) -> &'static str {
    match status {
        DpnsVoteTargetStatus::Scheduled => "DET will submit this vote at the scheduled time.",
        DpnsVoteTargetStatus::Queued => "This vote is waiting to be submitted.",
        DpnsVoteTargetStatus::Submitting => "DET is submitting this vote.",
        DpnsVoteTargetStatus::Confirming => "The vote was submitted and is being confirmed.",
        DpnsVoteTargetStatus::Confirmed => "Platform confirmed this vote.",
        DpnsVoteTargetStatus::Unconfirmed => {
            "The vote was submitted, but DET could not confirm it yet. Do not submit it again."
        }
        DpnsVoteTargetStatus::Rejected => {
            "Platform rejected this vote. Review the choice before trying again."
        }
        DpnsVoteTargetStatus::FailedBeforeSubmission => {
            "This vote was not submitted. Review it before trying again."
        }
        DpnsVoteTargetStatus::NotApplied => {
            "DET proved that this vote was not applied. It is safe to review it again."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TaskResult;
    use crate::app_dir::ensure_env_file;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::model::contested_name::Contestant;
    use crate::model::user_role::UserRoleCell;
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::utils::tasks::TaskManager;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    async fn offline_ctx() -> (Arc<AppContext>, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let context = AppContext::new(
            data_dir,
            dash_sdk::dpp::dashcore::Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            UserRoleCell::default(),
        )
        .expect("offline AppContext");
        let (sender, _receiver) = tokio::sync::mpsc::channel::<TaskResult>(32);
        context
            .ensure_wallet_backend(SenderAsync::new(sender, context.egui_ctx().clone()))
            .await
            .expect("wire wallet backend offline");
        (context, temp_dir)
    }

    fn contestant(id: Identifier, name: &str) -> Contestant {
        Contestant {
            id,
            name: name.to_owned(),
            info: String::new(),
            votes: 0,
            created_at: None,
            created_at_block_height: None,
            created_at_core_block_height: None,
            document_id: Identifier::from([9; 32]),
        }
    }

    #[test]
    fn candidate_choice_uses_name_and_short_identifier() {
        let candidate = Identifier::from([7; 32]);
        let label = choice_label(
            Some(ResourceVoteChoice::TowardsIdentity(candidate)),
            &[contestant(candidate, "dominguez")],
        );

        assert!(label.starts_with("dominguez ("));
        assert!(label.contains('…'));
        assert!(!label.contains(&candidate.to_string(Encoding::Base58)));
    }

    #[test]
    fn current_summary_keeps_healthy_targets_usable() {
        let states = [
            (
                Identifier::from([1; 32]),
                DpnsCurrentVoteState::Available(None),
                false,
            ),
            (
                Identifier::from([2; 32]),
                DpnsCurrentVoteState::Unavailable,
                false,
            ),
            (
                Identifier::from([3; 32]),
                DpnsCurrentVoteState::Available(Some(ResourceVoteChoice::Lock)),
                false,
            ),
        ];

        assert_eq!(
            current_summary(&states),
            "Current across selected nodes: 1 not voted, 1 already voted, 1 unavailable"
        );
    }

    #[test]
    fn scheduled_time_is_absolute_and_relative() {
        let timestamp = (Utc::now() + Duration::hours(2)).timestamp_millis() as u64;
        let label = format_timing(VoteTiming::Scheduled(timestamp));

        assert!(label.starts_with("When: 20"));
        assert!(label.contains(" UTC ("));
    }

    #[test]
    fn only_the_exact_scheduled_edit_target_can_be_replaced() {
        let edited = DpnsVoteTargetKey {
            network: dash_sdk::dpp::dashcore::Network::Testnet,
            voter_id: Identifier::from([1; 32]),
            vote_poll_id: Identifier::from([2; 32]),
        };
        let other = DpnsVoteTargetKey {
            vote_poll_id: Identifier::from([3; 32]),
            ..edited.clone()
        };

        assert!(is_explicit_schedule_replacement(
            Some(&edited),
            &edited,
            Some(DpnsVoteTargetStatus::Scheduled),
            VoteTiming::Scheduled(10),
        ));
        assert!(!is_explicit_schedule_replacement(
            None,
            &edited,
            Some(DpnsVoteTargetStatus::Scheduled),
            VoteTiming::Scheduled(10),
        ));
        assert!(!is_explicit_schedule_replacement(
            Some(&edited),
            &other,
            Some(DpnsVoteTargetStatus::Scheduled),
            VoteTiming::Scheduled(10),
        ));
    }

    #[test]
    fn scheduled_edit_review_names_the_old_and_new_schedule() {
        let original = DpnsVoteTarget {
            key: DpnsVoteTargetKey {
                network: dash_sdk::dpp::dashcore::Network::Testnet,
                voter_id: Identifier::from([1; 32]),
                vote_poll_id: Identifier::from([2; 32]),
            },
            voter_alias: Some("Eve".to_owned()),
            contested_name: "dominguez".to_owned(),
            requested_choice: ResourceVoteChoice::Lock,
            current_choice: None,
            timing: VoteTiming::Scheduled(1_700_000_000_000),
        };
        let mut replacement = original.clone();
        replacement.requested_choice = ResourceVoteChoice::Abstain;
        replacement.timing = VoteTiming::Scheduled(1_700_003_600_000);

        let summary = scheduled_replacement_summary(&original, &replacement, |_, choice| {
            choice_label(choice, &[])
        });

        assert!(summary.contains("Lock at "));
        assert!(summary.contains("→ Abstain at "));
    }

    #[test]
    fn matching_pre_journal_error_returns_the_composer_to_review() {
        let operation_id = DpnsVoteOperationId::from_bytes([4; 16]);
        let context = BackendTaskContext::DpnsVoteOperation {
            network: dash_sdk::dpp::dashcore::Network::Testnet,
            operation_id,
        };

        assert!(should_return_to_review_after_error(
            operation_id,
            dash_sdk::dpp::dashcore::Network::Testnet,
            &context,
            false,
        ));
        assert!(!should_return_to_review_after_error(
            operation_id,
            dash_sdk::dpp::dashcore::Network::Testnet,
            &context,
            true,
        ));
        assert!(!should_return_to_review_after_error(
            operation_id,
            dash_sdk::dpp::dashcore::Network::Mainnet,
            &context,
            false,
        ));
    }

    #[test]
    fn schedule_replacement_tracks_the_authoritative_operation_id() {
        let submitted = DpnsVoteOperationId::from_bytes([4; 16]);
        let stored = DpnsVoteOperationId::from_bytes([5; 16]);
        let context = BackendTaskContext::DpnsVoteOperation {
            network: dash_sdk::dpp::dashcore::Network::Testnet,
            operation_id: submitted,
        };
        let result = crate::backend_task::BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network: dash_sdk::dpp::dashcore::Network::Testnet,
            operation_id: stored,
        };

        assert_eq!(
            updated_submitted_operation(Some(submitted), &context, &result),
            Some(stored)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refreshed_contests_replace_the_voting_center_snapshot() {
        let (context, _temp_dir) = offline_ctx().await;
        let mut center = DpnsVotingCenter::new(&context, None, Vec::new());
        assert!(center.contests.is_empty());
        context
            .insert_name_contests_as_normalized_names(vec!["dominguez".to_owned()])
            .expect("seed refreshed contest");
        center.vote_state_refresh_dispatched = true;

        center.display_backend_task_result(
            &BackendTaskContext::Other,
            &BackendTaskSuccessResult::RefreshedDpnsContests,
        );

        assert_eq!(center.contests.len(), 1);
        assert_eq!(center.contests[0].normalized_contested_name, "dominguez");
        assert!(!center.vote_state_refresh_dispatched);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn empty_votes_step_offers_a_contest_refresh_action() {
        let (context, _temp_dir) = offline_ctx().await;
        let center = Rc::new(RefCell::new(DpnsVotingCenter::new(
            &context,
            None,
            Vec::new(),
        )));
        let dispatched = Rc::new(Cell::new(false));
        let center_for_ui = Rc::clone(&center);
        let dispatched_for_ui = Rc::clone(&dispatched);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 400.0))
            .build_ui(move |ui| {
                if matches!(
                    center_for_ui.borrow_mut().render_votes(ui),
                    VotingCenterOutcome::Action(action)
                        if matches!(
                            *action,
                            AppAction::BackendTask(BackendTask::ContestedResourceTask(
                                ContestedResourceTask::QueryDPNSContests
                            ))
                        )
                ) {
                    dispatched_for_ui.set(true);
                }
            });

        harness.get_by_label("Refresh contests").click();
        harness.run();

        assert!(dispatched.get());
    }
}
