use crate::app::AppAction;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::backend_task::{contested_names::ContestedResourceTask, BackendTask};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::{MessageType, ScreenLike};
use chrono::offset::LocalResult;
use chrono::{Duration, TimeZone, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use eframe::egui::Context;
use eframe::egui::{self, Color32, RichText, Ui};
use std::sync::Arc;

use crate::ui::components::top_panel::add_top_panel;
use crate::ui::RootScreenType;

enum VoteOption {
    None,
    Scheduled { days: u32, hours: u32, minutes: u32 },
}

pub struct ScheduleVoteScreen {
    pub app_context: Arc<AppContext>,
    pub contested_name: String,
    pub ending_time: u64,
    pub identities: Vec<QualifiedIdentity>,
    pub vote_choice: ResourceVoteChoice,
    identity_options: Vec<VoteOption>,
    message: Option<(MessageType, String)>,
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
            .map(|_| VoteOption::Scheduled {
                days: 0,
                hours: 0,
                minutes: 0,
            })
            .collect();

        // Default everything to 0 (i.e., "now")
        Self {
            app_context: app_context.clone(),
            contested_name,
            ending_time,
            identities: potential_voting_identities,
            vote_choice,
            identity_options,
            message: None,
        }
    }

    fn display_identity_options(&mut self, ui: &mut Ui) {
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

                    // Dropdown for None/VoteNow/Scheduled
                    let current_option = &mut self.identity_options[i];
                    egui::ComboBox::from_id_salt(format!("combo_for_identity_{}", i))
                        .selected_text(match current_option {
                            VoteOption::None => "None".to_string(),
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
                                self.message = None;
                            }
                            if ui
                                .selectable_label(
                                    matches!(current_option, VoteOption::Scheduled { .. }),
                                    "Scheduled",
                                )
                                .clicked()
                            {
                                // If we had a previous scheduled option, keep the old values. Otherwise, default to 0.
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
                                self.message = None;
                            }
                        });

                    // If Scheduled is chosen, let the user pick how far in the future
                    if let VoteOption::Scheduled {
                        days,
                        hours,
                        minutes,
                    } = current_option
                    {
                        ui.label("Schedule Vote In:");
                        ui.horizontal(|ui| {
                            ui.add(egui::DragValue::new(days).range(0..=14).prefix("Days: "));
                            ui.add(egui::DragValue::new(hours).range(0..=23).prefix("Hours: "));
                            ui.add(egui::DragValue::new(minutes).range(0..=59).prefix("Min: "));
                        });
                    }
                });
            });

            ui.add_space(10.0);
        }
    }

    fn cast_votes_button(&mut self) -> AppAction {
        let mut scheduled_votes = Vec::new();

        for (identity, option) in self.identities.iter().zip(self.identity_options.iter()) {
            match option {
                VoteOption::None => {}
                VoteOption::Scheduled {
                    days,
                    hours,
                    minutes,
                } => {
                    let now = chrono::Utc::now();
                    let offset = Duration::days((*days).into())
                        + Duration::hours((*hours).into())
                        + Duration::minutes((*minutes).into());

                    let scheduled_time = now + offset;
                    let chosen_time = scheduled_time.timestamp_millis() as u64;

                    if chosen_time > self.ending_time {
                        self.message = Some((
                            MessageType::Error,
                            "Error inserting scheduled votes: Scheduled time is after contest end time.".to_string(),
                        ));
                        return AppAction::None;
                    }

                    let scheduled_vote = ScheduledDPNSVote {
                        contested_name: self.contested_name.clone(),
                        voter_id: identity.identity.id().clone(),
                        choice: self.vote_choice,
                        unix_timestamp: chosen_time,
                        executed_successfully: false,
                    };
                    scheduled_votes.push(scheduled_vote);
                }
            }
        }

        if scheduled_votes.is_empty() {
            self.message = Some((
                MessageType::Error,
                "Error inserting scheduled votes: No votes selected.".to_string(),
            ));
            return AppAction::None;
        }

        let updated_action = ContestedResourceTask::ScheduleDPNSVotes(scheduled_votes);
        AppAction::BackendTask(BackendTask::ContestedResourceTask(updated_action))
    }

    fn show_success(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Center the content vertically and horizontally
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Successfully scheduled votes.");

            ui.add_space(20.0);

            if ui.button("Go to Scheduled Votes Screen").clicked() {
                // Handle navigation back to the identities screen
                action = AppAction::SetMainScreenThenPopScreen(
                    RootScreenType::RootScreenDPNSScheduledVotes,
                );
            }

            ui.add_space(10.0);

            if ui.button("Go back to Active Contests").clicked() {
                // Handle navigation back to the identities screen
                action = AppAction::PopScreenAndRefresh;
            }
        });

        action
    }
}

impl ScreenLike for ScheduleVoteScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message_type, message.to_string()));
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DPNS", AppAction::GoToMainScreen),
                ("Schedule Votes", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Schedule Votes");
            ui.add_space(10.0);

            ui.label("Please note that Dash Evo Tool must be running and connected to Platform in order for scheduled votes to execute at the specified time.");
            ui.add_space(10.0);

            // Convert the timestamp to a DateTime object using timestamp_millis_opt
            if let LocalResult::Single(datetime) = Utc.timestamp_millis_opt(self.ending_time as i64) {
                let iso_date = datetime.format("%Y-%m-%d %H:%M:%S").to_string();
                let relative_time = HumanTime::from(datetime).to_string();
                let display_text = format!(
                    "Contest for name {} ends at {} ({})",
                    self.contested_name, iso_date, relative_time
                );
                ui.label(display_text);
            } else {
                ui.colored_label(
                    Color32::DARK_RED,
                    "Error getting contest ending time".to_string(),
                );
            }
            ui.add_space(10.0);

            self.display_identity_options(ui);
            ui.add_space(10.0);

            let button = egui::Button::new(RichText::new("Schedule Votes").color(Color32::WHITE))
                .fill(Color32::from_rgb(0, 128, 255))
                .rounding(3.0);
            if ui.add(button).clicked() {
                action = self.cast_votes_button();
            }
            ui.add_space(10.0);

            if let Some(message) = &self.message {
                match message.0 {
                    MessageType::Error => {
                        if message.1.contains("Error inserting scheduled votes") {
                            ui.colored_label(Color32::DARK_RED, message.1.clone());
                        }
                    }
                    MessageType::Success => {
                        if message.1.contains("Votes scheduled") {
                            action = self.show_success(ui);
                        }
                    }
                    MessageType::Info => {
                        ui.colored_label(Color32::DARK_BLUE, message.1.clone());
                    }
                }
                ui.add_space(10.0);
            }
        });

        action
    }
}
