use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::model::contested_name::ContestedName;
use crate::platform::contested_names::ContestedResourceTask;
use crate::platform::BackendTask;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use chrono::{DateTime, LocalResult, TimeZone, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use egui::{Context, Frame, Margin, Ui};
use egui_extras::{Column, TableBuilder};
use itertools::Itertools;
use std::sync::{Arc, Mutex};

use super::ScreenType;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    ContestedName,
    LockedVotes,
    AbstainVotes,
    EndingTime,
    LastUpdated,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortOrder {
    Ascending,
    Descending,
}

pub struct DPNSContestedNamesScreen {
    contested_names: Arc<Mutex<Vec<ContestedName>>>,
    pub app_context: Arc<AppContext>,
    error_message: Option<(String, MessageType, DateTime<Utc>)>,
    sort_column: SortColumn,
    sort_order: SortOrder,
    show_vote_popup_info: Option<(String, ContestedResourceTask)>,
}

impl DPNSContestedNamesScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let contested_names = Arc::new(Mutex::new(
            app_context.load_contested_names().unwrap_or_default(),
        ));
        Self {
            contested_names,
            app_context: app_context.clone(),
            error_message: None,
            sort_column: SortColumn::ContestedName,
            sort_order: SortOrder::Ascending,
            show_vote_popup_info: None,
        }
    }

    fn show_contested_name_details(
        &mut self,
        ui: &mut Ui,
        contested_name: &ContestedName,
        is_locked_votes_bold: bool,
        max_contestant_votes: u32,
    ) {
        if let Some(contestants) = &contested_name.contestants {
            for contestant in contestants {
                let button_text = format!("{} - {} votes", contestant.name, contestant.votes);

                // Determine if this contestant's votes should be bold
                let text = if contestant.votes == max_contestant_votes && !is_locked_votes_bold {
                    egui::RichText::new(button_text)
                        .strong()
                        .color(egui::Color32::from_rgb(0, 100, 0))
                } else {
                    egui::RichText::new(button_text)
                };

                if ui.button(text).clicked() {
                    self.show_vote_popup_info = Some((
                        format!(
                            "Confirm Voting for Contestant {} for name \"{}\".\n\nSelect the identity to vote with:",
                            contestant.id, contestant.name
                        ),
                        ContestedResourceTask::VoteOnDPNSName(
                            contested_name.normalized_contested_name.clone(),
                            ResourceVoteChoice::TowardsIdentity(contestant.id),vec![]
                        ),
                    ));
                }
            }
        }
    }

    fn sort_contested_names(&self, contested_names: &mut Vec<ContestedName>) {
        contested_names.sort_by(|a, b| {
            let order = match self.sort_column {
                SortColumn::ContestedName => a
                    .normalized_contested_name
                    .cmp(&b.normalized_contested_name),
                SortColumn::LockedVotes => a.locked_votes.cmp(&b.locked_votes),
                SortColumn::AbstainVotes => a.abstain_votes.cmp(&b.abstain_votes),
                SortColumn::EndingTime => a.ending_time.cmp(&b.ending_time),
                SortColumn::LastUpdated => a.last_updated.cmp(&b.last_updated),
            };

            if self.sort_order == SortOrder::Descending {
                order.reverse()
            } else {
                order
            }
        });
    }

    fn dismiss_error(&mut self) {
        self.error_message = None;
    }

    fn check_error_expiration(&mut self) {
        if let Some((_, _, timestamp)) = &self.error_message {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(*timestamp);

            // Automatically dismiss the error message after 5 seconds
            if elapsed.num_seconds() > 5 {
                self.dismiss_error();
            }
        }
    }

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

    fn show_vote_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;

        if let Some((message, action)) = self.show_vote_popup_info.clone() {
            ui.label(message);

            let local_qualified_identities = self
                .app_context
                .db
                .get_local_qualified_identities(&self.app_context)
                .unwrap_or_default();

            let voting_identities = local_qualified_identities
                .iter()
                .filter_map(|id| {
                    if id.associated_voter_identity.is_some() {
                        Some(id)
                    } else {
                        None
                    }
                })
                .collect_vec();

            ui.horizontal(|ui| {
                // Only modify `voters` if `action` is `VoteOnDPNSName`
                if let ContestedResourceTask::VoteOnDPNSName(
                    contested_name,
                    vote_choice,
                    mut voters,
                ) = action
                {
                    // Iterate over the voting identities and create a button for each one
                    for identity in voting_identities.iter() {
                        let button_label = format!(
                            "{}",
                            identity
                                .alias
                                .clone()
                                .unwrap_or(identity.identity.id().to_string(Encoding::Base58))
                        );
                        if ui.button(button_label).clicked() {
                            // Add the selected identity to the `voters` field
                            voters.push(identity.clone().clone());

                            // Create a new `VoteOnDPNSName` task with updated voters
                            let updated_action = ContestedResourceTask::VoteOnDPNSName(
                                contested_name.clone(),
                                vote_choice.clone(),
                                voters.clone(), // Updated voters
                            );

                            // Pass updated action to BackendTask
                            app_action = AppAction::BackendTask(
                                BackendTask::ContestedResourceTask(updated_action),
                            );
                            self.show_vote_popup_info = None;
                        }
                    }

                    // Vote with all identities
                    if ui.button("All").clicked() {
                        for identity in voting_identities.iter() {
                            voters.push(identity.clone().clone());
                        }

                        // Create a new `VoteOnDPNSName` task with all voters
                        let updated_action = ContestedResourceTask::VoteOnDPNSName(
                            contested_name.clone(),
                            vote_choice.clone(),
                            voters.clone(), // Updated voters
                        );

                        // Pass updated action to BackendTask
                        app_action = AppAction::BackendTask(BackendTask::ContestedResourceTask(
                            updated_action,
                        ));
                        self.show_vote_popup_info = None;
                    }
                }

                // Add the "Cancel" button
                if ui.button("Cancel").clicked() {
                    self.show_vote_popup_info = None;
                }
            });
        }

        app_action
    }
}

impl ScreenLike for DPNSContestedNamesScreen {
    fn refresh(&mut self) {
        let mut contested_names = self.contested_names.lock().unwrap();
        *contested_names = self.app_context.load_contested_names().unwrap_or_default();
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.error_message = Some((message.to_string(), message_type, Utc::now()));
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_error_expiration();
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Dash Evo Tool", AppAction::None)],
            vec![
                (
                    "Register Name",
                    DesiredAppAction::AddScreenType(ScreenType::RegisterDpnsName),
                ),
                (
                    "Refresh",
                    DesiredAppAction::BackendTask(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::QueryDPNSContestedResources,
                    )),
                ),
            ],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDPNSContestedNames,
        );

        // Clone the contested names vector to avoid holding the lock during UI rendering
        let contested_names = {
            let contested_names_guard = self.contested_names.lock().unwrap();
            let mut contested_names = contested_names_guard.clone();
            self.sort_contested_names(&mut contested_names);
            contested_names
        };

        // Render the UI with the cloned contested_names vector
        egui::CentralPanel::default().show(ctx, |ui| {
            let error_message = self.error_message.clone();
            if let Some((message, message_type, _)) = error_message {
                if message_type != MessageType::Success {
                    let message_color = match message_type {
                        MessageType::Error => egui::Color32::RED,
                        MessageType::Info => egui::Color32::BLACK,
                        MessageType::Success => unreachable!(),
                    };

                    ui.add_space(10.0);
                    ui.allocate_ui(egui::Vec2::new(ui.available_width(), 50.0), |ui| {
                        ui.group(|ui| {
                            ui.set_min_height(50.0);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(message).color(message_color));
                                if ui.button("Dismiss").clicked() {
                                    // Update the state outside the closure
                                    self.dismiss_error();
                                }
                            });
                        });
                    });
                    ui.add_space(10.0);
                }
            }

            // Show vote popup if active
            if self.show_vote_popup_info.is_some() {
                egui::Window::new("Vote Confirmation")
                    .collapsible(false)
                    .show(ui.ctx(), |ui| {
                        action |= self.show_vote_popup(ui);
                    });
            }

            egui::ScrollArea::vertical().show(ui, |ui| {
                Frame::group(ui.style())
                    .fill(ui.visuals().panel_fill)
                    .stroke(egui::Stroke::new(
                        1.0,
                        ui.visuals().widgets.inactive.bg_stroke.color,
                    ))
                    .inner_margin(Margin::same(8.0))
                    .show(ui, |ui| {
                        TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                            .column(Column::initial(200.0).resizable(true)) // Contested Name
                            .column(Column::initial(100.0).resizable(true)) // Locked Votes
                            .column(Column::initial(100.0).resizable(true)) // Abstain Votes
                            .column(Column::initial(200.0).resizable(true)) // Ending Time
                            .column(Column::initial(200.0).resizable(true)) // Last Updated
                            .column(Column::remainder()) // Contestants
                            .header(30.0, |mut header| {
                                header.col(|ui| {
                                    if ui.button("Contested Name").clicked() {
                                        self.toggle_sort(SortColumn::ContestedName);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Locked Votes").clicked() {
                                        self.toggle_sort(SortColumn::LockedVotes);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Abstain Votes").clicked() {
                                        self.toggle_sort(SortColumn::AbstainVotes);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Ending Time").clicked() {
                                        self.toggle_sort(SortColumn::EndingTime);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Last Updated").clicked() {
                                        self.toggle_sort(SortColumn::LastUpdated);
                                    }
                                });
                                header.col(|ui| {
                                    ui.heading("Contestants");
                                });
                            })
                            .body(|mut body| {
                                for contested_name in &contested_names {
                                    body.row(25.0, |mut row| {
                                        let locked_votes = contested_name.locked_votes.unwrap_or(0);

                                        // Find the highest contestant votes, if any
                                        let max_contestant_votes = contested_name
                                            .contestants
                                            .as_ref()
                                            .map(|contestants| {
                                                contestants
                                                    .iter()
                                                    .map(|c| c.votes)
                                                    .max()
                                                    .unwrap_or(0)
                                            })
                                            .unwrap_or(0);

                                        // Determine if locked votes have strict priority
                                        let is_locked_votes_bold =
                                            locked_votes > max_contestant_votes;

                                        row.col(|ui| {
                                            ui.label(&contested_name.normalized_contested_name);
                                        });
                                        row.col(|ui| {
                                            let label_text = if let Some(locked_votes) =
                                                contested_name.locked_votes
                                            {
                                                let label_text = format!("{}", locked_votes);
                                                if is_locked_votes_bold {
                                                    egui::RichText::new(label_text).strong()
                                                } else {
                                                    egui::RichText::new(label_text)
                                                }
                                            } else {
                                                egui::RichText::new("Fetching".to_string())
                                            };
                                            // Vote button logic for locked votes
                                            if ui.button(label_text).clicked() {
                                                self.show_vote_popup_info = Some((format!("Confirm Voting to Lock the name \"{}\".\n\nSelect the identity to vote with:", contested_name.normalized_contested_name.clone()), ContestedResourceTask::VoteOnDPNSName(contested_name.normalized_contested_name.clone(), ResourceVoteChoice::Lock, vec![])));
                                            }
                                        });
                                        row.col(|ui| {
                                            let label_text = if let Some(abstain_votes) =
                                                contested_name.abstain_votes
                                            {
                                                format!("{}", abstain_votes)
                                            } else {
                                                "Fetching".to_string()
                                            };
                                            if ui.button(label_text).clicked() {
                                                self.show_vote_popup_info = Some((format!("Confirm Voting to Abstain on distribution of \"{}\".\n\nSelect the identity to vote with:", contested_name.normalized_contested_name.clone()), ContestedResourceTask::VoteOnDPNSName(contested_name.normalized_contested_name.clone(), ResourceVoteChoice::Abstain, vec![])));
                                            }
                                        });
                                        row.col(|ui| {
                                            if let Some(ending_time) = contested_name.ending_time {
                                                // Convert the timestamp to a DateTime object using timestamp_millis_opt
                                                if let LocalResult::Single(datetime) =
                                                    Utc.timestamp_millis_opt(ending_time as i64)
                                                {
                                                    // Format the ISO date up to seconds
                                                    let iso_date = datetime
                                                        .format("%Y-%m-%d %H:%M:%S")
                                                        .to_string();

                                                    // Use chrono-humanize to get the relative time
                                                    let relative_time =
                                                        HumanTime::from(datetime).to_string();

                                                    // Combine both the ISO date and relative time
                                                    let display_text =
                                                        format!("{} ({})", iso_date, relative_time);

                                                    ui.label(display_text);
                                                } else {
                                                    // Handle case where the timestamp is invalid
                                                    ui.label("Invalid timestamp");
                                                }
                                            } else {
                                                ui.label("Fetching");
                                            }
                                        });
                                        row.col(|ui| {
                                            if let Some(last_updated) = contested_name.last_updated
                                            {
                                                // Convert the timestamp to a DateTime object using timestamp_millis_opt
                                                if let LocalResult::Single(datetime) =
                                                    Utc.timestamp_opt(last_updated as i64, 0)
                                                {
                                                    // Use chrono-humanize to get the relative time
                                                    let relative_time =
                                                        HumanTime::from(datetime).to_string();

                                                    ui.label(relative_time);
                                                } else {
                                                    // Handle case where the timestamp is invalid
                                                    ui.label("Invalid timestamp");
                                                }
                                            } else {
                                                ui.label("Fetching");
                                            }
                                        });
                                        row.col(|ui| {
                                            self.show_contested_name_details(
                                                ui,
                                                contested_name,
                                                is_locked_votes_bold,
                                                max_contestant_votes,
                                            );
                                        });
                                    });
                                }
                            });
                    });
            });
        });

        action
    }
}
