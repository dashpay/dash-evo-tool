use super::components::dpns_subscreen_chooser_panel::add_dpns_subscreen_chooser_panel;
use super::{Screen, ScreenType};
use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::model::contested_name::{ContestState, ContestedName};
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::platform::contested_names::ContestedResourceTask;
use crate::platform::BackendTask;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::add_existing_identity_screen::AddExistingIdentityScreen;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use chrono::{DateTime, LocalResult, TimeZone, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use egui::{Context, Frame, Margin, Ui};
use egui_extras::{Column, TableBuilder};
use std::sync::{Arc, Mutex};
use tracing::error;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    ContestedName,
    LockedVotes,
    AbstainVotes,
    EndingTime,
    LastUpdated,
    AwardedTo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortOrder {
    Ascending,
    Descending,
}

#[derive(PartialEq)]
pub enum DPNSSubscreen {
    Active,
    Past,
    Owned,
}

impl DPNSSubscreen {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Active => "Active contests",
            Self::Past => "Past contests",
            Self::Owned => "My usernames",
        }
    }
}

pub struct DPNSContestedNamesScreen {
    // No need for Mutex as this can only refresh when entering screen
    voting_identities: Arc<Vec<QualifiedIdentity>>,
    user_identities: Arc<Vec<QualifiedIdentity>>,
    contested_names: Arc<Mutex<Vec<ContestedName>>>,
    local_dpns_names: Arc<Vec<(Identifier, String)>>,
    pub app_context: Arc<AppContext>,
    error_message: Option<(String, MessageType, DateTime<Utc>)>,
    sort_column: SortColumn,
    sort_order: SortOrder,
    show_vote_popup_info: Option<(String, ContestedResourceTask)>,
    pub dpns_subscreen: DPNSSubscreen,
}

impl DPNSContestedNamesScreen {
    pub fn new(app_context: &Arc<AppContext>, dpns_subscreen: DPNSSubscreen) -> Self {
        let contested_names = Arc::new(Mutex::new(match dpns_subscreen {
            DPNSSubscreen::Active => app_context.ongoing_contested_names().unwrap_or_else(|e| {
                error!("Failed to load contested names: {:?}", e);
                Vec::new()
            }),
            DPNSSubscreen::Past => app_context.all_contested_names().unwrap_or_else(|e| {
                error!("Failed to load contested names: {:?}", e);
                Vec::new()
            }),
            DPNSSubscreen::Owned => Vec::new(),
        }));
        let local_dpns_names = app_context.local_dpns_names().unwrap_or_default();
        let voting_identities = app_context
            .db
            .get_local_voting_identities(&app_context)
            .unwrap_or_default();
        let user_identities = app_context
            .db
            .get_local_user_identities(&app_context)
            .unwrap_or_default();
        Self {
            voting_identities: Arc::new(voting_identities),
            user_identities: Arc::new(user_identities),
            contested_names,
            local_dpns_names: Arc::new(local_dpns_names),
            app_context: app_context.clone(),
            error_message: None,
            sort_column: SortColumn::ContestedName,
            sort_order: SortOrder::Ascending,
            show_vote_popup_info: None,
            dpns_subscreen,
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
                let first_6_chars_of_id: String = contestant
                    .id
                    .to_string(Encoding::Base58)
                    .chars()
                    .take(6)
                    .collect();
                let button_text =
                    format!("{}... - {} votes", first_6_chars_of_id, contestant.votes);

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

    fn render_no_active_contests(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(20.0); // Add some space to separate from the top
            match self.dpns_subscreen {
                DPNSSubscreen::Active => {
                    ui.label(
                        egui::RichText::new("No active contests at the moment.")
                            .heading()
                            .strong()
                            .color(egui::Color32::GRAY),
                    );
                }
                DPNSSubscreen::Past => {
                    ui.label(
                        egui::RichText::new("No active or past contests at the moment.")
                            .heading()
                            .strong()
                            .color(egui::Color32::GRAY),
                    );
                }
                DPNSSubscreen::Owned => {
                    ui.label(
                        egui::RichText::new("No owned usernames.")
                            .heading()
                            .strong()
                            .color(egui::Color32::GRAY),
                    );
                }
            }
            ui.add_space(10.0);
            ui.label("Please check back later or try refreshing the list.");
            ui.add_space(20.0);
            if ui.button("Refresh").clicked() {
                self.refresh(); // Call refresh logic when the user clicks "Refresh"
            }
        });
    }

    fn render_table_active_contests(&mut self, ui: &mut Ui) {
        // Clone the contested names vector to avoid holding the lock during UI rendering
        let contested_names = {
            let contested_names_guard = self.contested_names.lock().unwrap();
            let mut contested_names = contested_names_guard.clone();
            self.sort_contested_names(&mut contested_names);
            contested_names
        };

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
                                        if let Some(ending_time) = contested_name.end_time {
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
    }

    fn render_table_past_contests(&mut self, ui: &mut Ui) {
        // Clone the contested names vector to avoid holding the lock during UI rendering
        let contested_names = {
            let contested_names_guard = self.contested_names.lock().unwrap();
            let mut contested_names = contested_names_guard.clone();
            self.sort_contested_names(&mut contested_names);
            contested_names
        };

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
                        .column(Column::initial(200.0).resizable(true)) // Ended Time
                        .column(Column::initial(200.0).resizable(true)) // Last Updated
                        .column(Column::initial(200.0).resizable(true)) // Awarded To
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                if ui.button("Contested Name").clicked() {
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
                                if ui.button("Awarded To").clicked() {
                                    self.toggle_sort(SortColumn::AwardedTo);
                                }
                            });
                        })
                        .body(|mut body| {
                            for contested_name in &contested_names {
                                body.row(25.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(&contested_name.normalized_contested_name);
                                    });
                                    row.col(|ui| {
                                        if let Some(ended_time) = contested_name.end_time {
                                            // Convert the timestamp to a DateTime object using timestamp_millis_opt
                                            if let LocalResult::Single(datetime) =
                                                Utc.timestamp_millis_opt(ended_time as i64)
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
                                        if let Some(last_updated) = contested_name.last_updated {
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
                                    row.col(|ui| match contested_name.state {
                                        ContestState::Unknown => {
                                            ui.label("Fetching");
                                        }
                                        ContestState::Joinable => {
                                            ui.label("Active");
                                        }
                                        ContestState::Ongoing => {
                                            ui.label("Active");
                                        }
                                        ContestState::WonBy(identifier) => {
                                            ui.label(format!(
                                                "{}",
                                                identifier.to_string(Encoding::Base58),
                                            ));
                                        }
                                        ContestState::Locked => {
                                            ui.label("Locked");
                                        }
                                    });
                                });
                            }
                        });
                });
        });
    }

    fn render_table_local_dpns_names(&mut self, ui: &mut Ui) {
        // Clone and sort a local copy of the `local_dpns_names` vector
        let mut sorted_names = self.local_dpns_names.clone().as_ref().clone();
        sorted_names.sort_by(|a, b| match self.sort_column {
            SortColumn::AwardedTo => {
                let order = a.0.cmp(&b.0); // Sort by Identifier
                if self.sort_order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                }
            }
            SortColumn::ContestedName => {
                let order = a.1.cmp(&b.1); // Sort by DPNS Name
                if self.sort_order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                }
            }
            _ => std::cmp::Ordering::Equal,
        });

        // Render table UI
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
                        .column(Column::initial(500.0).resizable(true)) // Identifier
                        .column(Column::initial(500.0).resizable(true)) // DPNS Name
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                if ui.button("Identifier").clicked() {
                                    self.toggle_sort(SortColumn::AwardedTo); // Toggle sorting for Identifier
                                }
                            });
                            header.col(|ui| {
                                if ui.button("DPNS Name").clicked() {
                                    self.toggle_sort(SortColumn::ContestedName);
                                    // Toggle sorting for DPNS Name
                                }
                            });
                        })
                        .body(|mut body| {
                            for (identifier, name) in sorted_names {
                                body.row(25.0, |mut row| {
                                    // Display Identifier and Name on each row
                                    row.col(|ui| {
                                        ui.label(identifier.to_string(Encoding::Base58));
                                    });
                                    row.col(|ui| {
                                        ui.label(name);
                                    });
                                });
                            }
                        });
                });
        });
    }

    fn show_vote_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        if self.voting_identities.is_empty() {
            ui.label("Please load an Evonode or Masternode first before voting");
            if ui.button("I want to load one now").clicked() {
                self.show_vote_popup_info = None;
                let mut screen = AddExistingIdentityScreen::new(&self.app_context);
                screen.identity_type = IdentityType::Evonode;
                app_action = AppAction::AddScreen(Screen::AddExistingIdentityScreen(screen));
            }
            if ui.button("Cancel").clicked() {
                self.show_vote_popup_info = None;
            }
        } else if let Some((message, action)) = self.show_vote_popup_info.clone() {
            ui.label(message);

            ui.horizontal(|ui| {
                // Only modify `voters` if `action` is `VoteOnDPNSName`
                if let ContestedResourceTask::VoteOnDPNSName(
                    contested_name,
                    vote_choice,
                    mut voters,
                ) = action
                {
                    // Iterate over the voting identities and create a button for each one
                    for identity in self.voting_identities.iter() {
                        if ui.button(identity.display_short_string()).clicked() {
                            // Add the selected identity to the `voters` field
                            voters.push(identity.clone());

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
                        for identity in self.voting_identities.iter() {
                            voters.push(identity.clone());
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
                self.local_dpns_names =
                    Arc::new(self.app_context.local_dpns_names().unwrap_or_default());
            }
        }
    }

    fn refresh_on_arrival(&mut self) {
        self.voting_identities = self
            .app_context
            .db
            .get_local_voting_identities(&self.app_context)
            .unwrap_or_default()
            .into();

        self.user_identities = self
            .app_context
            .db
            .get_local_user_identities(&self.app_context)
            .unwrap_or_default()
            .into();
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.error_message = Some((message.to_string(), message_type, Utc::now()));
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_error_expiration();
        let has_identity_that_can_register = !self.user_identities.is_empty();
        let query = (
            "Refresh",
            DesiredAppAction::BackendTask(BackendTask::ContestedResourceTask(
                ContestedResourceTask::QueryDPNSContestedResources,
            )),
        );
        let right_buttons = if has_identity_that_can_register {
            vec![
                (
                    "Register Name",
                    DesiredAppAction::AddScreenType(ScreenType::RegisterDpnsName),
                ),
                query,
            ]
        } else {
            vec![query]
        };
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Dash Evo Tool", AppAction::None)],
            right_buttons,
        );

        match self.dpns_subscreen {
            DPNSSubscreen::Active => {
                action |= add_left_panel(
                    ctx,
                    &self.app_context,
                    RootScreenType::RootScreenDPNSActiveContests,
                );
            }
            DPNSSubscreen::Past => {
                action |= add_left_panel(
                    ctx,
                    &self.app_context,
                    RootScreenType::RootScreenDPNSPastContests,
                );
            }
            DPNSSubscreen::Owned => {
                action |= add_left_panel(
                    ctx,
                    &self.app_context,
                    RootScreenType::RootScreenDPNSOwnedNames,
                );
            }
        }
        action |= add_dpns_subscreen_chooser_panel(ctx);

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

            // Check if there are any contested names to display
            let has_contested_names = {
                let contested_names = self.contested_names.lock().unwrap();
                !contested_names.is_empty()
            };

            if has_contested_names {
                // Render the table if there are contested names
                match self.dpns_subscreen {
                    DPNSSubscreen::Active => {
                        self.render_table_active_contests(ui);
                    }
                    DPNSSubscreen::Past => {
                        self.render_table_past_contests(ui);
                    }
                    DPNSSubscreen::Owned => {
                        self.render_table_local_dpns_names(ui);
                    }
                }
            } else {
                if self.dpns_subscreen == DPNSSubscreen::Owned && !self.local_dpns_names.is_empty()
                {
                    self.render_table_local_dpns_names(ui);
                }
                // Render the "no active contests" message if none exist
                self.render_no_active_contests(ui);
            }
        });

        action
    }
}
