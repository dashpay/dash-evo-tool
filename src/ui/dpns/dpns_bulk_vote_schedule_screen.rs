use crate::app::AppAction;
use crate::backend_task::contested_names::{ContestedResourceTask, ScheduledDPNSVote};
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::RootScreenType;
use crate::ui::{BackendTaskSuccessResult, MessageType, ScreenLike};
use chrono::{LocalResult, TimeZone, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use eframe::egui::Context;
use eframe::egui::{self, Color32, RichText, Ui};
use std::sync::Arc;

use super::dpns_contested_names_screen::SelectedVote;
use super::dpns_vote_scheduling_screen::VoteOption;

pub struct BulkScheduleVoteScreen {
    pub app_context: Arc<AppContext>,

    /// All the name+choice pairs the user SHIFT-clicked
    pub selected_votes: Vec<SelectedVote>,

    /// The user picks which identities to use + how (Immediate, Scheduled, or None).
    pub identities: Vec<QualifiedIdentity>,

    /// One VoteOption per identity, describing what that identity should do.
    pub identity_options: Vec<VoteOption>,

    /// If we have scheduled votes to insert after immediate casting finishes,
    /// we store them here until the first backend task completes.
    pub pending_scheduled: Option<Vec<ScheduledDPNSVote>>,

    /// We show any success/error messages here
    message: Option<(MessageType, String)>,

    /// If we need to fire a second backend task after the first completes,
    /// we'll store it here, then dispatch it at the end of `ui()`.
    pub pending_backend_task: Option<BackendTask>,
}

impl BulkScheduleVoteScreen {
    pub fn new(app_context: &Arc<AppContext>, selected_votes: Vec<SelectedVote>) -> Self {
        // Query local voting identities from app_context
        let identities = app_context
            .db
            .get_local_voting_identities(app_context)
            .unwrap_or_default();

        // Default each identity to "Immediate"
        let identity_options = identities.iter().map(|_| VoteOption::Immediate).collect();

        Self {
            app_context: app_context.clone(),
            selected_votes,
            identities,
            identity_options,
            pending_scheduled: None,
            message: None,
            pending_backend_task: None,
        }
    }

    fn display_identity_options(&mut self, ui: &mut Ui) {
        ui.heading("Select cast method for each node (None, Immediate, or Scheduled):");
        ui.add_space(5.0);

        for (i, identity) in self.identities.iter().enumerate() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    // Identity label
                    let identity_label = identity
                        .alias
                        .clone()
                        .unwrap_or_else(|| identity.identity.id().to_string(Encoding::Base58));
                    ui.label(format!("Identity: {}", identity_label));

                    let current_option = &mut self.identity_options[i];

                    egui::ComboBox::from_id_source(format!("combo_for_identity_{}", i))
                        .width(120.0)
                        .selected_text(match current_option {
                            VoteOption::None => "None".to_string(),
                            VoteOption::Immediate => "Immediate".to_string(),
                            VoteOption::Scheduled { .. } => "Scheduled".to_string(),
                        })
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(
                                    matches!(current_option, VoteOption::None),
                                    "None",
                                )
                                .clicked()
                            {
                                *current_option = VoteOption::None;
                            }
                            if ui
                                .selectable_label(
                                    matches!(current_option, VoteOption::Immediate),
                                    "Immediate",
                                )
                                .clicked()
                            {
                                *current_option = VoteOption::Immediate;
                            }
                            if ui
                                .selectable_label(
                                    matches!(current_option, VoteOption::Scheduled { .. }),
                                    "Scheduled",
                                )
                                .clicked()
                            {
                                // If we had a previous scheduled option, keep old values
                                let (days, hours, minutes) = match current_option {
                                    VoteOption::Scheduled {
                                        days,
                                        hours,
                                        minutes,
                                    } => (*days, *hours, *minutes),
                                    _ => (0, 0, 0),
                                };
                                *current_option = VoteOption::Scheduled {
                                    days,
                                    hours,
                                    minutes,
                                };
                            }
                        });

                    // If user picks "Scheduled," let them pick days/hours/minutes
                    if let VoteOption::Scheduled {
                        days,
                        hours,
                        minutes,
                    } = current_option
                    {
                        ui.label("Schedule Vote In:");
                        ui.add(
                            egui::DragValue::new(days)
                                .prefix("Days: ")
                                .clamp_range(0..=14),
                        );
                        ui.add(
                            egui::DragValue::new(hours)
                                .prefix("Hours: ")
                                .clamp_range(0..=23),
                        );
                        ui.add(
                            egui::DragValue::new(minutes)
                                .prefix("Min: ")
                                .clamp_range(0..=59),
                        );
                    }
                });
            });

            ui.add_space(10.0);
        }
    }

    fn schedule_votes(&mut self) -> AppAction {
        // Gather immediate and scheduled sets
        let mut immediate_identities: Vec<QualifiedIdentity> = Vec::new();
        let mut scheduled_votes: Vec<ScheduledDPNSVote> = Vec::new();

        for (identity, option) in self.identities.iter().zip(&self.identity_options) {
            match option {
                VoteOption::None => {
                    // skip
                }
                VoteOption::Immediate => {
                    immediate_identities.push(identity.clone());
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
                    let scheduled_time = (now + offset).timestamp_millis() as u64;

                    for sv in &self.selected_votes {
                        scheduled_votes.push(ScheduledDPNSVote {
                            contested_name: sv.contested_name.clone(),
                            voter_id: identity.identity.id().clone(),
                            choice: sv.vote_choice.clone(),
                            unix_timestamp: scheduled_time,
                            executed_successfully: false,
                        });
                    }
                }
            }
        }

        if immediate_identities.is_empty() && scheduled_votes.is_empty() {
            self.message = Some((
                MessageType::Error,
                "No votes selected (or set to None).".to_string(),
            ));
            return AppAction::None;
        }

        // 1) If there are immediate identities, cast them now
        if !immediate_identities.is_empty() {
            let votes_for_all: Vec<(String, ResourceVoteChoice)> = self
                .selected_votes
                .iter()
                .map(|sv| (sv.contested_name.clone(), sv.vote_choice))
                .collect();

            // dispatch the immediate votes
            let immediate_action = AppAction::BackendTask(BackendTask::ContestedResourceTask(
                ContestedResourceTask::VoteOnMultipleDPNSNames(votes_for_all, immediate_identities),
            ));

            // store the scheduled in self for after immediate finishes
            if !scheduled_votes.is_empty() {
                self.pending_scheduled = Some(scheduled_votes);
            }

            return immediate_action;
        } else {
            // 2) otherwise, if we only have scheduled votes, schedule them right away
            return AppAction::BackendTask(BackendTask::ContestedResourceTask(
                ContestedResourceTask::ScheduleDPNSVotes(scheduled_votes),
            ));
        }
    }

    fn show_success(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Successfully scheduled votes.");

            ui.add_space(20.0);

            if ui.button("Go to Scheduled Votes Screen").clicked() {
                action = AppAction::SetMainScreenThenPopScreen(
                    RootScreenType::RootScreenDPNSScheduledVotes,
                );
            }

            ui.add_space(10.0);

            if ui.button("Go back to Active Contests").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
        });

        action
    }
}

impl ScreenLike for BulkScheduleVoteScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message_type, message.to_string()));
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            // If immediate cast succeeded, we might have more scheduling to do
            BackendTaskSuccessResult::MultipleDPNSVotesCast(_results) => {
                // If we have some pending schedules, dispatch them next
                if let Some(to_schedule) = self.pending_scheduled.take() {
                    // store it in pending_backend_task for the next frame
                    self.pending_backend_task = Some(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::ScheduleDPNSVotes(to_schedule),
                    ));
                    // Show message
                    self.message = Some((
                        MessageType::Info,
                        "Immediate votes cast. Now scheduling the rest...".to_string(),
                    ));
                } else {
                    // No pending schedule => done
                    self.message = Some((
                        MessageType::Success,
                        "All votes cast immediately.".to_string(),
                    ));
                }
            }

            // If scheduled insertion succeeded
            BackendTaskSuccessResult::Message(m) if m.contains("Votes scheduled") => {
                // That likely means the second half completed
                self.message = Some((MessageType::Success, "Votes scheduled".to_string()));
            }

            // If the backend task has a different success message
            _ => {
                // Possibly handle other success results
                // e.g. refresh or show partial success
                // self.message = Some((MessageType::Info, format!("{:?}", other)));
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DPNS", AppAction::GoToMainScreen),
                ("Bulk Schedule", AppAction::None),
            ],
            vec![],
        );

        // main UI
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Bulk-Schedule Votes");
            ui.add_space(10.0);

            if self.selected_votes.is_empty() {
                ui.colored_label(
                    Color32::DARK_RED,
                    "No votes selected. SHIFT-click on vote choices in the Active Contests table first.",
                );
                return;
            }

            ui.label("Dash Evo Tool must remain running/connected so that scheduled votes occur on time.");
            ui.add_space(10.0);

            // Show selected votes
            ui.group(|ui| {
                ui.heading("Selected votes:");
                ui.separator();
                for sv in &self.selected_votes {
                    // Convert the end_time to a readable format
                    let end_time_str = if let Some(end_ts) = sv.end_time {
                        if let LocalResult::Single(datetime) = Utc.timestamp_millis_opt(end_ts as i64) {
                            let iso_date = datetime.format("%Y-%m-%d %H:%M:%S").to_string();
                            let relative_time = HumanTime::from(datetime).to_string();
                            format!("{} ({})", iso_date, relative_time)
                        } else {
                            "Invalid timestamp".to_string()
                        }
                    } else {
                        "N/A".to_string()
                    };

                    ui.label(format!(
                        "• {} => {:?}, ends at {}",
                        sv.contested_name, sv.vote_choice, end_time_str
                    ));
                }
            });

            ui.add_space(10.0);
            self.display_identity_options(ui);

            let button = egui::Button::new(RichText::new("Apply Votes").color(Color32::WHITE))
                .fill(Color32::from_rgb(0, 128, 255))
                .rounding(3.0);
            if ui.add(button).clicked() {
                action = self.schedule_votes();
            }

            if let Some((msg_type, msg_text)) = &self.message {
                ui.add_space(10.0);
                match msg_type {
                    MessageType::Error => {
                        ui.colored_label(Color32::RED, msg_text);
                    }
                    MessageType::Success => {
                        if msg_text.contains("Votes scheduled") {
                            action |= self.show_success(ui);
                        } else {
                            ui.colored_label(Color32::DARK_GREEN, msg_text);
                        }
                    }
                    MessageType::Info => {
                        ui.colored_label(Color32::YELLOW, msg_text);
                    }
                }
            }
        });

        // If we have a second backend task pending, dispatch it now
        if action == AppAction::None {
            if let Some(next_task) = self.pending_backend_task.take() {
                action = AppAction::BackendTask(next_task);
            }
        }

        action
    }
}
