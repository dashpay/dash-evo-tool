use crate::app::AppAction;
use crate::backend_task::contested_names::schedule_dpns_vote::ScheduledDPNSVote;
use crate::backend_task::{contested_names::ContestedResourceTask, BackendTask};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use eframe::egui::Context;
use eframe::egui::{self, Color32, RichText, Ui};
use std::sync::Arc;

/// The voting option a user can choose for each identity.
enum VoteOption {
    None,
    VoteNow,
    Scheduled(String),
}

pub struct ScheduleVoteScreen {
    pub app_context: Arc<AppContext>,
    pub contested_name: String,
    pub ending_time: u64,
    pub identities: Vec<QualifiedIdentity>,
    pub vote_choice: ResourceVoteChoice,
    identity_options: Vec<VoteOption>,
    error_message: Option<String>,
}

impl ScheduleVoteScreen {
    pub fn new(
        app_context: &Arc<AppContext>,
        contested_name: String,
        ending_time: u64,
        potential_voting_identities: Vec<QualifiedIdentity>,
        vote_choice: ResourceVoteChoice,
    ) -> Self {
        let identity_options = potential_voting_identities
            .iter()
            .map(|_| VoteOption::None)
            .collect();
        Self {
            app_context: app_context.clone(),
            contested_name,
            ending_time,
            identities: potential_voting_identities,
            vote_choice,
            identity_options,
            error_message: None,
        }
    }

    fn display_identity_options(&mut self, ui: &mut Ui) {
        ui.heading("Schedule Votes for Identities");
        ui.add_space(10.0);

        ui.label(format!(
            "Contest for name {} ends at {}",
            self.contested_name, self.ending_time
        ));
        ui.add_space(10.0);

        // For each identity, show a row with their alias/ID and voting options
        for (i, identity) in self.identities.iter().enumerate() {
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    // Identity label
                    let identity_label = identity
                        .alias
                        .as_ref()
                        .map(|a| a.clone())
                        .unwrap_or(identity.identity.id().to_string(Encoding::Base58));
                    ui.label(format!("Identity: {}", identity_label));

                    // Dropdown or Radio buttons for None/VoteNow/Scheduled
                    // For simplicity, let's use a ComboBox:
                    let current_option = &mut self.identity_options[i];
                    egui::ComboBox::from_label(identity_label)
                        .selected_text(match current_option {
                            VoteOption::None => "None".to_string(),
                            VoteOption::VoteNow => "Vote Now".to_string(),
                            VoteOption::Scheduled(_) => "Scheduled".to_string(),
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
                                    matches!(current_option, VoteOption::VoteNow),
                                    "Vote Now",
                                )
                                .clicked()
                            {
                                *current_option = VoteOption::VoteNow;
                            }
                            if ui
                                .selectable_label(
                                    matches!(current_option, VoteOption::Scheduled(_)),
                                    "Scheduled",
                                )
                                .clicked()
                            {
                                // If we had a previous schedule time, keep it. Otherwise, empty string.
                                let old_time = match current_option {
                                    VoteOption::Scheduled(s) => s.clone(),
                                    _ => String::new(),
                                };
                                *current_option = VoteOption::Scheduled(old_time);
                            }
                        });

                    // If Scheduled is chosen, display a text field for the schedule time
                    // To Do: This should be a date time selector rather than text input.
                    if let VoteOption::Scheduled(ref mut time_str) = current_option {
                        ui.label("Schedule Time (UNIX timestamp):");
                        ui.text_edit_singleline(time_str);
                    }
                });
            });

            ui.add_space(10.0);
        }
    }

    fn cast_votes_button(&mut self) -> AppAction {
        // Gather the voter identities and their chosen times
        // For simplicity, let's assume the backend can handle a structure where:
        // - If VoteNow, we submit immediately.
        // - If Scheduled(time), we submit the schedule.
        // - If None, we do not include that identity as a voter.

        // Filter only those with VoteNow or Scheduled
        let mut voters = Vec::new();
        let mut scheduled_votes = Vec::new();

        for (identity, option) in self.identities.iter().zip(self.identity_options.iter()) {
            match option {
                VoteOption::None => {
                    // Skip this identity
                }
                VoteOption::VoteNow => {
                    // Immediate vote
                    voters.push(identity.clone());
                }
                VoteOption::Scheduled(time_str) => {
                    // Collect scheduled votes separately
                    // The backend task might need a structure that allows scheduling.
                    // If such a structure doesn’t exist yet, we might need to define one.
                    let scheduled_vote = ScheduledDPNSVote {
                        contested_name: self.contested_name.clone(),
                        voter_id: identity.identity.id().clone(),
                        choice: self.vote_choice,
                        unix_timestamp: time_str.parse().unwrap_or(0),
                    };
                    scheduled_votes.push(scheduled_vote);
                }
            }
        }

        // If no voters and no scheduled, return None to indicate nothing to do.
        if voters.is_empty() && scheduled_votes.is_empty() {
            self.error_message = Some("No votes selected.".to_string());
            return AppAction::None;
        }

        let updated_action = ContestedResourceTask::ScheduleDPNSVote(scheduled_votes);

        AppAction::BackendTask(BackendTask::ContestedResourceTask(updated_action))
    }
}

impl ScreenLike for ScheduleVoteScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                self.error_message = Some(message.to_string());
            }
            MessageType::Info => {
                // Informational messages
            }
            MessageType::Error => {
                self.error_message = Some(message.to_string());
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;

        // A top panel or breadcrumb could be added similarly to the original code.
        // For brevity, let's just add a simple label at the top.
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Back").clicked() {
                    action = AppAction::PopScreen;
                }
                ui.label("Schedule Votes");
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Schedule Votes");
            ui.add_space(10.0);

            if let Some(err) = &self.error_message {
                ui.colored_label(Color32::RED, format!("Error: {}", err));
                ui.add_space(10.0);
            }

            // Display the identity options and scheduling fields
            self.display_identity_options(ui);

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            // Button to cast votes (now or scheduled)
            let button = egui::Button::new(RichText::new("Cast Votes").color(Color32::WHITE))
                .fill(Color32::from_rgb(0, 128, 255))
                .rounding(3.0);

            if ui.add(button).clicked() {
                action = self.cast_votes_button();
            }
        });

        action
    }
}
