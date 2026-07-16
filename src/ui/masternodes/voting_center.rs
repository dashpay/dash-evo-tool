//! Full-page Nodes → Votes → Review DPNS voting composer.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{Duration, Utc};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, ComboBox, RichText};

use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::contested_names::{ContestedResourceTask, ScheduledDPNSVote};
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::contested_name::ContestedName;
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsVoteOperation, DpnsVoteOperationId, DpnsVoteTarget,
    DpnsVoteTargetKey, DpnsVoteTargetStatus, VoteTiming,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
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
    focus_step_heading: bool,
}

impl DpnsVotingCenter {
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
            focus_step_heading: true,
        }
    }

    pub fn for_scheduled_edit(app_context: &Arc<AppContext>, vote: &ScheduledDPNSVote) -> Self {
        let mut center = Self::new(
            app_context,
            Some(vote.voter_id),
            vec![vote.contested_name.clone()],
        );
        center
            .workspace
            .contest_choices
            .insert(vote.contested_name.clone(), vote.choice);
        let remaining_minutes = vote
            .unix_timestamp
            .saturating_sub(Utc::now().timestamp_millis() as u64)
            / 60_000;
        center.workspace.node_timing.insert(
            vote.voter_id,
            DraftVoteTiming::Scheduled {
                days: (remaining_minutes / (24 * 60)) as u32,
                hours: ((remaining_minutes / 60) % 24) as u32,
                minutes: (remaining_minutes % 60) as u32,
            },
        );
        center.editing_scheduled_key = app_context
            .dpns_vote_poll_id(&vote.contested_name)
            .ok()
            .map(|vote_poll_id| DpnsVoteTargetKey {
                network: app_context.network(),
                voter_id: vote.voter_id,
                vote_poll_id,
            });
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
            DpnsVoteComposerStep::Review => self
                .build_operation()
                .is_ok_and(|operation| !operation.targets.is_empty()),
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
                if let Ok(operation) = self.build_operation() {
                    return self.submit_operation(operation);
                }
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
        ui.horizontal_wrapped(|ui| {
            ui.label("Set all:");
            timing_combo(
                ui,
                "voting_center_set_all",
                &mut self.workspace.set_all_timing,
            );
            if ComponentStyles::add_secondary_button(ui, "Apply", dark_mode).clicked() {
                self.workspace.apply_timing_to_all();
            }
        });
        ui.separator();
        for voter in &self.voters {
            let voter_id = voter.identity.id();
            let alias = voter
                .alias
                .clone()
                .unwrap_or_else(|| voter_id.to_string(Encoding::Base58));
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(alias).strong());
                let timing = self
                    .workspace
                    .node_timing
                    .entry(voter_id)
                    .or_insert(DraftVoteTiming::Excluded);
                timing_combo(ui, format!("voting_center_node_{voter_id}"), timing);
                render_schedule_offset(ui, timing);
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
            let controls_enabled = !states.iter().any(|(voter_id, state, locked)| {
                let lock_is_this_edit = vote_poll_id.is_some_and(|vote_poll_id| {
                    self.editing_scheduled_key.as_ref()
                        == Some(&DpnsVoteTargetKey {
                            network: self.app_context.network(),
                            voter_id: *voter_id,
                            vote_poll_id,
                        })
                });
                (*locked && !lock_is_this_edit)
                    || !matches!(state, DpnsCurrentVoteState::Available(_))
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
            if !controls_enabled {
                ui.label(
                    RichText::new(
                        "This contest is unavailable for at least one selected node. Refresh or wait for the active vote to finish.",
                    )
                    .color(DashColors::text_secondary(dark_mode)),
                );
            }
        }
        ui.separator();
        let mut outcome = VotingCenterOutcome::None;
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
        let operation = match self.build_operation() {
            Ok(operation) => operation,
            Err(error) => {
                MessageBanner::set_global(ui.ctx(), error.to_string(), MessageType::Error)
                    .with_details(&error);
                self.workspace.step = DpnsVoteComposerStep::Votes;
                self.focus_step_heading = true;
                return VotingCenterOutcome::None;
            }
        };
        for outcome in &operation.targets {
            let voter = outcome
                .target
                .voter_alias
                .as_deref()
                .unwrap_or("Unnamed node");
            ui.group(|ui| {
                ui.label(RichText::new(format!(
                    "{voter} / {}.dash",
                    outcome.target.contested_name
                ))
                .strong());
                ui.label(format!(
                    "Current: {}",
                    choice_label(outcome.target.current_choice)
                ));
                ui.label(format!(
                    "Requested: {}",
                    choice_label(Some(outcome.target.requested_choice))
                ));
                ui.label(match outcome.target.timing {
                    VoteTiming::Now => "When: Cast now".to_owned(),
                    VoteTiming::Scheduled(timestamp) => {
                        format!("When: Scheduled for {timestamp} UTC milliseconds")
                    }
                });
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
        if operation.no_op_count > 0 {
            ui.label(format!(
                "{} targets already have the requested vote and will not be submitted.",
                operation.no_op_count
            ));
        }
        ui.label(format!(
            "{} targets total. Each submitted vote uses Platform credits.",
            operation.targets.len()
        ));
        ui.horizontal(|ui| {
            if ComponentStyles::add_secondary_button(ui, "Back", dark_mode).clicked() {
                self.workspace.step = DpnsVoteComposerStep::Votes;
                self.focus_step_heading = true;
            }
        });
        if operation.targets.is_empty() {
            ui.label(
                "Every selected node already has the requested vote. Nothing will be submitted.",
            );
            return VotingCenterOutcome::None;
        }
        let target_count = operation.targets.len();
        if ComponentStyles::add_primary_button(ui, format!("Submit {target_count} targets"))
            .clicked()
        {
            return self.submit_operation(operation);
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
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!(
                            "{} / {}.dash — {}",
                            outcome
                                .target
                                .voter_alias
                                .as_deref()
                                .unwrap_or("Unnamed node"),
                            outcome.target.contested_name,
                            status_label(outcome.status)
                        ));
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
                            ContestedResourceTask::ReconcileDpnsVoteOperation(operation_id),
                        ),
                    )));
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
            .filter(|voter| {
                self.workspace
                    .node_timing
                    .get(&voter.identity.id())
                    .is_some_and(|timing| *timing != DraftVoteTiming::Excluded)
            })
            .cloned()
            .collect()
    }

    fn submit_operation(&mut self, operation: DpnsVoteOperation) -> VotingCenterOutcome {
        self.submitted_operation = Some(operation.id);
        VotingCenterOutcome::Action(Box::new(AppAction::BackendTask(
            BackendTask::ContestedResourceTask(ContestedResourceTask::SubmitDpnsVoteOperation(
                operation,
                self.selected_voters(),
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
            return Vec::new();
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

    fn build_operation(&self) -> Result<DpnsVoteOperation, TaskError> {
        let mut targets = Vec::new();
        for voter in self.selected_voters() {
            let voter_id = voter.identity.id();
            let draft_timing = self.workspace.node_timing[&voter_id];
            let timing = match draft_timing {
                DraftVoteTiming::Excluded => continue,
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
                let vote_poll_id = self.app_context.dpns_vote_poll_id(name)?;
                let key = DpnsVoteTargetKey {
                    network: self.app_context.network(),
                    voter_id,
                    vote_poll_id,
                };
                let existing_status = self.app_context.dpns_vote_target_status(&key)?;
                let replacing_schedule = existing_status == Some(DpnsVoteTargetStatus::Scheduled)
                    && matches!(timing, VoteTiming::Scheduled(_));
                if existing_status.is_some() && !replacing_schedule {
                    return Err(TaskError::DpnsVoteTargetBusy);
                }
                let DpnsCurrentVoteState::Available(current_choice) = self
                    .app_context
                    .dpns_current_vote_state(voter_id, vote_poll_id)?
                else {
                    return Err(TaskError::DpnsCurrentVoteUnavailable);
                };
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
        Ok(DpnsVoteOperation::new(targets))
    }
}

fn timing_combo(ui: &mut egui::Ui, id: impl Into<String>, timing: &mut DraftVoteTiming) {
    ComboBox::from_id_salt(id.into())
        .selected_text(match timing {
            DraftVoteTiming::Excluded => "Do not use this node",
            DraftVoteTiming::Now => "Cast now",
            DraftVoteTiming::Scheduled { .. } => "Schedule",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(timing, DraftVoteTiming::Excluded, "Do not use this node");
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
    if states.iter().any(|(_, state, _)| {
        matches!(
            state,
            DpnsCurrentVoteState::Checking | DpnsCurrentVoteState::Unavailable
        )
    }) {
        return "Current vote unavailable for at least one selected node".to_owned();
    }
    let not_voted = states
        .iter()
        .filter(|(_, state, _)| *state == DpnsCurrentVoteState::Available(None))
        .count();
    if not_voted == states.len() {
        "Current across selected nodes: Not voted".to_owned()
    } else if not_voted > 0 {
        format!("Current across selected nodes: {not_voted} not voted, others already voted")
    } else {
        "Current across selected nodes: All already voted".to_owned()
    }
}

fn choice_label(choice: Option<ResourceVoteChoice>) -> String {
    match choice {
        None => "Not voted".to_owned(),
        Some(ResourceVoteChoice::Abstain) => "Abstain".to_owned(),
        Some(ResourceVoteChoice::Lock) => "Lock".to_owned(),
        Some(ResourceVoteChoice::TowardsIdentity(identity)) => {
            format!("Vote for {identity}")
        }
    }
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
