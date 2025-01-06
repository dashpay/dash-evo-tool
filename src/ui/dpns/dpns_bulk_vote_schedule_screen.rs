use crate::app::AppAction;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::backend_task::{contested_names::ContestedResourceTask, BackendTask};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::RootScreenType;
use crate::ui::{MessageType, ScreenLike};
use chrono::offset::LocalResult;
use chrono::{Duration, TimeZone, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::egui::Context;
use eframe::egui::{self, Color32, RichText, Ui};
use std::sync::Arc;

use super::dpns_contested_names_screen::SelectedVote;
use super::dpns_vote_scheduling_screen::VoteOption;

pub struct BulkScheduleVoteScreen {
    pub app_context: Arc<AppContext>,
    /// All the name+choice pairs the user SHIFT-clicked
    pub selected_votes: Vec<SelectedVote>,

    /// The user picks which identities to use,
    /// plus how far in the future each identity’s vote is cast.
    pub identities: Vec<QualifiedIdentity>,
    pub identity_options: Vec<VoteOption>,
    message: Option<(MessageType, String)>,
}

impl BulkScheduleVoteScreen {
    pub fn new(app_context: &Arc<AppContext>, selected_votes: Vec<SelectedVote>) -> Self {
        // Query local voting identities from app_context
        let identities = app_context
            .db
            .get_local_voting_identities(&app_context)
            .unwrap_or_default();

        // Initialize each identity’s “VoteOption” to “Scheduled(0,0,0)”
        let identity_options = identities
            .iter()
            .map(|_| VoteOption::Scheduled {
                days: 0,
                hours: 0,
                minutes: 0,
            })
            .collect();

        Self {
            app_context: app_context.clone(),
            selected_votes,
            identities,
            identity_options,
            message: None,
        }
    }

    fn display_identity_options(&mut self, ui: &mut Ui) {
        ui.heading("Select scheduling offsets (Days/Hours/Minutes) for each node:");
        ui.add_space(5.0);

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

                    // Dropdown for None or Scheduled
                    let current_option = &mut self.identity_options[i];
                    egui::ComboBox::from_id_source(format!("combo_for_identity_{}", i))
                        .width(100.0)
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
                                // If we had a previous scheduled option, keep the old values:
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

    fn schedule_votes(&mut self) -> AppAction {
        // Build up a list of ScheduledDPNSVote for *all* selected votes
        let mut all_scheduled = Vec::new();

        for (identity, option) in self.identities.iter().zip(self.identity_options.iter()) {
            if let VoteOption::Scheduled {
                days,
                hours,
                minutes,
            } = option
            {
                // Convert days/hours/minutes into a single timestamp
                let now = Utc::now();
                let offset = Duration::days(*days as i64)
                    + Duration::hours(*hours as i64)
                    + Duration::minutes(*minutes as i64);
                let scheduled_time = (now + offset).timestamp_millis() as u64;

                // For each selected vote in selected_votes
                for sv in &self.selected_votes {
                    let scheduled_vote = ScheduledDPNSVote {
                        contested_name: sv.contested_name.clone(),
                        voter_id: identity.identity.id().clone(),
                        choice: sv.vote_choice.clone(),
                        unix_timestamp: scheduled_time,
                        executed_successfully: false,
                    };
                    all_scheduled.push(scheduled_vote);
                }
            }
        }

        if all_scheduled.is_empty() {
            self.message = Some((
                MessageType::Error,
                "No votes selected or scheduled.".to_string(),
            ));
            return AppAction::None;
        }

        // Send them off to the backend
        AppAction::BackendTask(BackendTask::ContestedResourceTask(
            ContestedResourceTask::ScheduleDPNSVotes(all_scheduled),
        ))
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

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Bulk-Schedule Votes");
            ui.add_space(10.0);

            if self.selected_votes.is_empty() {
                ui.colored_label(Color32::DARK_RED, "No votes selected. You can SHIFT-click on vote choices in the Active Contests table to select votes.");
                return;
            }

            ui.label("Please note that Dash Evo Tool must be running and connected to Platform in order for scheduled votes to execute at the specified time.");
            ui.add_space(10.0);

            // Show a small table or list of the selected votes
            ui.group(|ui| {
                ui.heading("Selected votes:");
                ui.separator();
                for sv in &self.selected_votes {
                    // Convert the timestamp to a DateTime object using timestamp_millis_opt
                    let end_time = if let Some(end_time) = sv.end_time {
                        if let LocalResult::Single(datetime) = Utc.timestamp_millis_opt(end_time as i64) {
                        let iso_date = datetime.format("%Y-%m-%d %H:%M:%S").to_string();
                        let relative_time = HumanTime::from(datetime).to_string();
                        let display_text = format!(
                            "{} ({})",
                            iso_date, relative_time
                        );
                        display_text
                    } else {
                        "Invalid timestamp".to_string()
                    }} else {
                        "Error getting end time".to_string()
                    };

                    ui.label(format!(
                        "• Name: {} | Choice: {} | Contest End Time: {}",
                        sv.contested_name, sv.vote_choice, end_time
                    ));
                }
            });

            ui.add_space(10.0);
            self.display_identity_options(ui);

            let button = egui::Button::new(RichText::new("Schedule All Votes").color(Color32::WHITE))
                .fill(Color32::from_rgb(0, 128, 255))
                .rounding(3.0);
            if ui.add(button).clicked() {
                action = self.schedule_votes();
            }

            if let Some((msg_type, msg_text)) = &self.message {
                ui.add_space(10.0);
                match msg_type {
                    MessageType::Error => {
                        if msg_text.contains("No votes selected") {
                            ui.colored_label(Color32::RED, msg_text);
                        } else {
                            ui.colored_label(Color32::RED, msg_text);
                        }
                    }
                    MessageType::Success => {
                        if msg_text.contains("Votes scheduled") {
                            action = self.show_success(ui);
                        } else {
                            ui.colored_label(Color32::GREEN, msg_text);
                        }
                    }
                    MessageType::Info => {
                        ui.colored_label(Color32::DARK_BLUE, msg_text);
                    }
                }
            }
        });

        action
    }
}
