use super::components::dpns_subscreen_chooser_panel::add_dpns_subscreen_chooser_panel;
use super::dpns_vote_scheduling_screen::ScheduleVoteScreen;
use super::{Screen, ScreenType};
use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::contested_names::ScheduledDPNSVote;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::contested_name::{ContestState, ContestedName};
use crate::model::qualified_identity::{DPNSNameInfo, IdentityType, QualifiedIdentity};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::add_existing_identity_screen::AddExistingIdentityScreen;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use chrono::{DateTime, LocalResult, TimeZone, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use egui::{Color32, Context, Frame, Margin, Ui};
use egui_extras::{Column, TableBuilder};
use itertools::Itertools;
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
    ScheduledVotes,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IndividualVoteCastingStatus {
    NotStarted,
    InProgress,
    Failed,
    Completed,
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

pub struct DPNSContestedNamesScreen {
    // No need for Mutex as this can only refresh when entering screen
    voting_identities: Vec<QualifiedIdentity>,
    user_identities: Vec<QualifiedIdentity>,
    contested_names: Arc<Mutex<Vec<ContestedName>>>,
    local_dpns_names: Arc<Mutex<Vec<(Identifier, DPNSNameInfo)>>>,
    scheduled_votes: Arc<Mutex<Vec<(ScheduledDPNSVote, IndividualVoteCastingStatus)>>>,
    pub app_context: Arc<AppContext>,
    error_message: Option<(String, MessageType, DateTime<Utc>)>,
    sort_column: SortColumn,
    sort_order: SortOrder,
    show_vote_popup_info: Option<(String, ContestedResourceTask)>,
    pending_vote_action: Option<ContestedResourceTask>,
    screen_casting_vote_in_progress: bool,
    pub dpns_subscreen: DPNSSubscreen,
    refreshing: bool,
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
            DPNSSubscreen::ScheduledVotes => Vec::new(),
        }));
        let local_dpns_names = Arc::new(Mutex::new(match dpns_subscreen {
            DPNSSubscreen::Active => Vec::new(),
            DPNSSubscreen::Past => Vec::new(),
            DPNSSubscreen::Owned => app_context.local_dpns_names().unwrap_or_default(),
            DPNSSubscreen::ScheduledVotes => Vec::new(),
        }));
        let scheduled_votes = app_context.get_scheduled_votes().unwrap_or_default();
        let scheduled_votes_with_status = Arc::new(Mutex::new(
            scheduled_votes
                .iter()
                .map(|vote| match vote.executed_successfully {
                    true => (vote.clone(), IndividualVoteCastingStatus::Completed),
                    false => (vote.clone(), IndividualVoteCastingStatus::NotStarted),
                })
                .collect::<Vec<_>>(),
        ));
        let voting_identities = app_context
            .db
            .get_local_voting_identities(&app_context)
            .unwrap_or_default();
        let user_identities = app_context
            .db
            .get_local_user_identities(&app_context)
            .unwrap_or_default();
        Self {
            voting_identities,
            user_identities,
            contested_names,
            local_dpns_names,
            scheduled_votes: scheduled_votes_with_status,
            app_context: app_context.clone(),
            error_message: None,
            sort_column: SortColumn::ContestedName,
            sort_order: SortOrder::Ascending,
            show_vote_popup_info: None,
            pending_vote_action: None,
            screen_casting_vote_in_progress: false,
            dpns_subscreen,
            refreshing: false,
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
                            "Confirm Voting for Contestant {} for name \"{}\".",
                            contestant.id, contestant.name
                        ),
                        ContestedResourceTask::VoteOnDPNSName(
                            contested_name.normalized_contested_name.clone(),
                            ResourceVoteChoice::TowardsIdentity(contestant.id),
                            vec![],
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

            // Automatically dismiss the error message after 10 seconds
            if elapsed.num_seconds() >= 10 {
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

    fn render_no_active_contests_or_owned_names(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
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
                DPNSSubscreen::ScheduledVotes => {
                    ui.label(
                        egui::RichText::new("No scheduled votes.")
                            .heading()
                            .strong()
                            .color(egui::Color32::GRAY),
                    );
                }
            }
            ui.add_space(10.0);
            if self.dpns_subscreen != DPNSSubscreen::ScheduledVotes {
                ui.label("Please check back later or try refreshing the list.");
                ui.add_space(20.0);
                if ui.button("Refresh").clicked() {
                    if self.refreshing {
                        app_action |= AppAction::None;
                    } else {
                        match self.dpns_subscreen {
                            DPNSSubscreen::Active | DPNSSubscreen::Past => {
                                app_action |=
                                    AppAction::BackendTask(BackendTask::ContestedResourceTask(
                                        ContestedResourceTask::QueryDPNSContestedResources,
                                    ));
                            }
                            DPNSSubscreen::Owned => {
                                app_action |= AppAction::BackendTask(BackendTask::IdentityTask(
                                    IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
                                ));
                            }
                            _ => {
                                app_action |= AppAction::Refresh;
                            }
                        }
                    }
                }
            } else {
                ui.label("Go to the Active Contests subscreen to schedule votes.");
            }
        });

        app_action
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
                                        let (used_name, highlighted) = if let Some(contestants) = &contested_name.contestants {
                                            if let Some(first_contestant) = contestants.first() {
                                                if contestants.iter().all(|contestant|contestant.name == first_contestant.name) {
                                                    (first_contestant.name.clone(), Some(contested_name.normalized_contested_name.clone()))
                                                } else {
                                                    (contestants.iter().map(|contestant| contestant.name.clone()).join(" or "),
                                                    Some(contestants.iter().map(|contestant| format!("{} trying to get {}",contestant.id, contestant.name.clone())).join(" and ")))
                                                }
                                            } else {
                                                (contested_name.normalized_contested_name.clone(), None)
                                            }
                                        } else {
                                            (contested_name.normalized_contested_name.clone(), None)
                                        };
                                        let label_response = ui.label(used_name);
                                        if let Some(highlighted_text) = highlighted {
                                            label_response.on_hover_text(highlighted_text);
                                        }
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
                                            self.show_vote_popup_info = Some((format!("Confirm Voting to Lock the name \"{}\".", contested_name.normalized_contested_name.clone()), ContestedResourceTask::VoteOnDPNSName(contested_name.normalized_contested_name.clone(), ResourceVoteChoice::Lock, vec![])));
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
                                            self.show_vote_popup_info = Some((format!("Confirm Voting to Abstain on distribution of \"{}\".", contested_name.normalized_contested_name.clone()), ContestedResourceTask::VoteOnDPNSName(contested_name.normalized_contested_name.clone(), ResourceVoteChoice::Abstain, vec![])));
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

                                                if relative_time.contains("seconds") {
                                                    ui.label("now");
                                                } else {
                                                    ui.label(relative_time);
                                                }
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
            contested_names.retain(|contested_name| {
                contested_name.awarded_to.is_some() || contested_name.state == ContestState::Locked
            });
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

                                                if relative_time.contains("seconds") {
                                                    ui.label("now");
                                                } else {
                                                    ui.label(relative_time);
                                                }
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
        let mut sorted_names = {
            let dpns_names_guard = self.local_dpns_names.lock().unwrap();
            let dpns_names = dpns_names_guard.clone();
            dpns_names
        };

        sorted_names.sort_by(|a, b| match self.sort_column {
            SortColumn::ContestedName => {
                let order = a.1.name.cmp(&b.1.name); // Sort by DPNS Name
                if self.sort_order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                }
            }
            SortColumn::AwardedTo => {
                let order = a.0.cmp(&b.0); // Sort by Identifier
                if self.sort_order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                }
            }
            SortColumn::EndingTime => {
                let order = a.1.acquired_at.cmp(&b.1.acquired_at); // Sort by Acquired At
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
                        .column(Column::initial(200.0).resizable(true)) // DPNS Name
                        .column(Column::initial(400.0).resizable(true)) // Owner Identifier
                        .column(Column::initial(300.0).resizable(true)) // Acquired At
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
                        })
                        .body(|mut body| {
                            for (identifier, dpns_info) in sorted_names {
                                body.row(25.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(dpns_info.name);
                                    });
                                    row.col(|ui| {
                                        ui.label(identifier.to_string(Encoding::Base58));
                                    });

                                    let datetime = DateTime::from_timestamp(
                                        dpns_info.acquired_at as i64 / 1000,
                                        ((dpns_info.acquired_at % 1000) * 1_000_000) as u32,
                                    )
                                    .map(|dt| dt.to_string())
                                    .unwrap_or_else(|| "Invalid timestamp".to_string());

                                    row.col(|ui| {
                                        ui.label(datetime);
                                    });
                                });
                            }
                        });
                });
        });
    }

    fn render_table_scheduled_votes(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut sorted_votes = {
            let scheduled_votes_guard = self.scheduled_votes.lock().unwrap();
            let scheduled_votes = scheduled_votes_guard.clone();
            scheduled_votes
        };

        sorted_votes.sort_by(|a, b| match self.sort_column {
            SortColumn::ContestedName => {
                let order = a.0.contested_name.cmp(&b.0.contested_name); // Sort by DPNS Name
                if self.sort_order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                }
            }
            SortColumn::EndingTime => {
                let order = a.0.unix_timestamp.cmp(&b.0.unix_timestamp); // Sort by Vote Time
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
                        .column(Column::initial(100.0).resizable(true)) // DPNS Name
                        .column(Column::initial(200.0).resizable(true)) // Voter ID
                        .column(Column::initial(200.0).resizable(true)) // Choice
                        .column(Column::initial(200.0).resizable(true)) // Scheduled vote time
                        .column(Column::initial(100.0).resizable(true)) // Executed?
                        .column(Column::initial(100.0).resizable(true)) // Actions
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                if ui.button("Contested Name").clicked() {
                                    self.toggle_sort(SortColumn::ContestedName);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Voter").clicked() {
                                    self.toggle_sort(SortColumn::ContestedName);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Vote").clicked() {
                                    self.toggle_sort(SortColumn::ContestedName);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Scheduled Time").clicked() {
                                    self.toggle_sort(SortColumn::ContestedName);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Status").clicked() {
                                    self.toggle_sort(SortColumn::ContestedName);
                                }
                            });
                            header.col(|ui| {
                                ui.label("Actions");
                            });
                        })
                        .body(|mut body| {
                            for vote in sorted_votes.iter_mut() {
                                body.row(25.0, |mut row| {
                                    row.col(|ui| {
                                        ui.add(
                                            egui::Label::new(vote.0.contested_name.clone())
                                                .truncate(),
                                        );
                                    });
                                    row.col(|ui| {
                                        ui.add(
                                            egui::Label::new(
                                                vote.0.voter_id.to_string(Encoding::Hex),
                                            )
                                            .truncate(),
                                        );
                                    });
                                    row.col(|ui| {
                                        let display_text = match &vote.0.choice {
                                            ResourceVoteChoice::TowardsIdentity(identifier) => {
                                                identifier.to_string(Encoding::Base58)
                                            }
                                            other => other.to_string(),
                                        };
                                        ui.add(egui::Label::new(display_text).truncate());
                                    });
                                    row.col(|ui| {
                                        if let LocalResult::Single(datetime) =
                                            Utc.timestamp_millis_opt(vote.0.unix_timestamp as i64)
                                        {
                                            let iso_date =
                                                datetime.format("%Y-%m-%d %H:%M:%S").to_string();
                                            let relative_time =
                                                HumanTime::from(datetime).to_string();
                                            let display_text =
                                                format!("{} ({})", iso_date, relative_time);
                                            ui.add(egui::Label::new(display_text).truncate());
                                        } else {
                                            ui.label("Invalid timestamp");
                                        }
                                    });
                                    row.col(|ui| match vote.1 {
                                        IndividualVoteCastingStatus::NotStarted => {
                                            ui.label("Pending");
                                        }
                                        IndividualVoteCastingStatus::InProgress => {
                                            ui.label("Casting...");
                                        }
                                        IndividualVoteCastingStatus::Failed => {
                                            ui.colored_label(Color32::DARK_RED, "Failed");
                                        }
                                        IndividualVoteCastingStatus::Completed => {
                                            ui.colored_label(Color32::DARK_GREEN, "Complete");
                                        }
                                    });
                                    row.col(|ui| {
                                        if ui.button("Remove").clicked() {
                                            action = AppAction::BackendTask(
                                                BackendTask::ContestedResourceTask(
                                                    ContestedResourceTask::DeleteScheduledVote(
                                                        vote.0.voter_id.clone(),
                                                        vote.0.contested_name.clone(),
                                                    ),
                                                ),
                                            );
                                        }

                                        let cast_button = match vote.1 {
                                            IndividualVoteCastingStatus::NotStarted => egui::Button::new("Cast Now"),
                                            IndividualVoteCastingStatus::InProgress => egui::Button::new("Casting..."),
                                            IndividualVoteCastingStatus::Failed => egui::Button::new("Cast Now"),
                                            IndividualVoteCastingStatus::Completed => egui::Button::new("Completed"),
                                        };

                                        if !self.screen_casting_vote_in_progress && (vote.1 == IndividualVoteCastingStatus::NotStarted || vote.1 == IndividualVoteCastingStatus::Failed) {
                                            if ui.add(cast_button).clicked() {
                                                self.screen_casting_vote_in_progress = true;

                                                // Update the local vote
                                                vote.1 = IndividualVoteCastingStatus::InProgress;

                                                // Now also update self.scheduled_votes
                                                if let Ok(mut scheduled_guard) = self.scheduled_votes.lock() {
                                                    if let Some(sched_vote) = scheduled_guard.iter_mut().find(|(sv, _)| {
                                                        sv.voter_id == vote.0.voter_id && sv.contested_name == vote.0.contested_name
                                                    }) {
                                                        sched_vote.1 = IndividualVoteCastingStatus::InProgress;
                                                    }
                                                }

                                                // Trigger the CastScheduledVote task
                                                let local_identities =
                                                    match self.app_context.load_local_voting_identities() {
                                                        Ok(identities) => identities,
                                                        Err(e) => {
                                                            eprintln!("Error querying local voting identities: {}", e);
                                                            return;
                                                        }
                                                    };

                                                if let Some(voter) = local_identities
                                                    .iter()
                                                    .find(|i| i.identity.id() == vote.0.voter_id)
                                                {
                                                    action = AppAction::BackendTask(
                                                        BackendTask::ContestedResourceTask(
                                                            ContestedResourceTask::CastScheduledVote(vote.0.clone(), voter.clone()),
                                                        ),
                                                    );
                                                }
                                            }
                                        }
                                    });
                                });
                            }
                        });
                });
        });

        action
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
                self.pending_vote_action = None;
            }
        } else if let Some((message, action)) = self.show_vote_popup_info.clone() {
            ui.label(message);

            ui.add_space(10.0);

            if self.pending_vote_action.is_none() {
                ui.label("Select the identity to vote with:");
            } else {
                ui.label("Would you like to vote now or schedule your votes?");
            }

            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if let ContestedResourceTask::VoteOnDPNSName(
                    contested_name,
                    vote_choice,
                    mut voters,
                ) = action
                {
                    // If we haven't yet chosen any voters (pending_vote_action is None), we show the identities
                    if self.pending_vote_action.is_none() {
                        // Iterate over the voting identities and create a button for each one
                        for identity in self.voting_identities.iter() {
                            if ui.button(identity.display_short_string()).clicked() {
                                // Add the selected identity to the `voters` field
                                voters.push(identity.clone());

                                // Store the updated action, but don't finalize yet
                                let updated_action = ContestedResourceTask::VoteOnDPNSName(
                                    contested_name.clone(),
                                    vote_choice.clone(),
                                    voters.clone(),
                                );
                                self.pending_vote_action = Some(updated_action);
                            }
                        }

                        // Vote with all identities
                        if ui.button("All").clicked() {
                            voters.extend(self.voting_identities.iter().cloned());
                            let updated_action = ContestedResourceTask::VoteOnDPNSName(
                                contested_name.clone(),
                                vote_choice.clone(),
                                voters.clone(),
                            );
                            self.pending_vote_action = Some(updated_action);
                        }
                    } else {
                        // If we have a pending vote action, ask whether to vote now or schedule
                        if ui.button("Vote Now").clicked() {
                            // Finalize the vote now
                            app_action =
                                AppAction::BackendTask(BackendTask::ContestedResourceTask(
                                    self.pending_vote_action.take().unwrap(),
                                ));
                            self.show_vote_popup_info = None;
                        }
                        if ui.button("Schedule").clicked() {
                            // Move to a scheduling screen instead
                            let pending = self.pending_vote_action.take().unwrap();
                            if let ContestedResourceTask::VoteOnDPNSName(
                                name_string,
                                vote_choice,
                                voters,
                            ) = pending
                            {
                                // Lock and get a reference to the contested names
                                let contested_names = self.contested_names.lock().unwrap();

                                // Find the contested name that matches the given name_string
                                let ending_time = contested_names
                                    .iter()
                                    .find(|cn| cn.normalized_contested_name == name_string)
                                    .and_then(|cn| cn.end_time)
                                    .unwrap_or_default();
                                let contested_name = name_string.clone();
                                let schedule_screen = ScheduleVoteScreen::new(
                                    &self.app_context,
                                    contested_name,
                                    ending_time,
                                    voters,
                                    vote_choice,
                                );
                                app_action = AppAction::AddScreen(Screen::ScheduleVoteScreen(
                                    schedule_screen,
                                ));
                            }
                            self.show_vote_popup_info = None;
                        }
                    }
                }

                // Add the "Cancel" button
                if ui.button("Cancel").clicked() {
                    self.show_vote_popup_info = None;
                    self.pending_vote_action = None;
                }
            });
        }

        app_action
    }
}

impl ScreenLike for DPNSContestedNamesScreen {
    fn refresh(&mut self) {
        let mut contested_names = self.contested_names.lock().unwrap();
        let mut dpns_names = self.local_dpns_names.lock().unwrap();
        let mut scheduled_votes = self.scheduled_votes.lock().unwrap();
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
                *dpns_names = self.app_context.local_dpns_names().unwrap_or_default();
            }
            DPNSSubscreen::ScheduledVotes => {
                *scheduled_votes = {
                    let new_scheduled_votes =
                        self.app_context.get_scheduled_votes().unwrap_or_default();
                    new_scheduled_votes
                        .iter()
                        .map(|new_vote| match new_vote.executed_successfully {
                            true => (new_vote.clone(), IndividualVoteCastingStatus::Completed),
                            false => scheduled_votes
                                .iter()
                                .find(|(old_vote, _)| {
                                    old_vote.contested_name == new_vote.contested_name
                                        && old_vote.voter_id == new_vote.voter_id
                                })
                                .map(|(_, status)| {
                                    if status == &IndividualVoteCastingStatus::InProgress {
                                        (new_vote.clone(), IndividualVoteCastingStatus::InProgress)
                                    } else {
                                        (new_vote.clone(), IndividualVoteCastingStatus::NotStarted)
                                    }
                                })
                                .unwrap_or_else(|| {
                                    (new_vote.clone(), IndividualVoteCastingStatus::NotStarted)
                                }),
                        })
                        .collect::<Vec<_>>()
                }
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

        let mut contested_names = self.contested_names.lock().unwrap();
        let mut dpns_names = self.local_dpns_names.lock().unwrap();
        let mut scheduled_votes = self.scheduled_votes.lock().unwrap();
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
                *dpns_names = self.app_context.local_dpns_names().unwrap_or_default();
            }
            DPNSSubscreen::ScheduledVotes => {
                *scheduled_votes = {
                    let new_scheduled_votes =
                        self.app_context.get_scheduled_votes().unwrap_or_default();
                    new_scheduled_votes
                        .iter()
                        .map(|new_vote| match new_vote.executed_successfully {
                            true => (new_vote.clone(), IndividualVoteCastingStatus::Completed),
                            // If false, it could be failed, in progress, or not started
                            // Check screen state to see if vote is in progress
                            false => scheduled_votes
                                .iter()
                                .find(|(old_vote, _)| {
                                    old_vote.contested_name == new_vote.contested_name
                                        && old_vote.voter_id == new_vote.voter_id
                                })
                                .map(|(_, status)| {
                                    if status == &IndividualVoteCastingStatus::InProgress {
                                        (new_vote.clone(), IndividualVoteCastingStatus::InProgress)
                                    } else if status == &IndividualVoteCastingStatus::Failed {
                                        (new_vote.clone(), IndividualVoteCastingStatus::Failed)
                                    } else {
                                        (new_vote.clone(), IndividualVoteCastingStatus::NotStarted)
                                    }
                                })
                                .unwrap_or_else(|| {
                                    (new_vote.clone(), IndividualVoteCastingStatus::NotStarted)
                                }),
                        })
                        .collect::<Vec<_>>()
                }
            }
        }
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if message.contains("Finished querying DPNS contested resources")
            || message.contains("Successfully refreshed loaded identities dpns names")
            || message.contains("Contested resource query failed")
            || message.contains("Error refreshing owned DPNS names")
        {
            self.refreshing = false;
        }
        if message.contains("Error casting scheduled vote") {
            self.screen_casting_vote_in_progress = false;
            let mut scheduled_votes = self.scheduled_votes.lock().unwrap();
            for vote in scheduled_votes.iter_mut() {
                if vote.1 == IndividualVoteCastingStatus::InProgress {
                    vote.1 = IndividualVoteCastingStatus::Failed;
                }
            }
        }
        if message.contains("Successfully cast scheduled vote") {
            self.screen_casting_vote_in_progress = false;
        }
        self.error_message = Some((message.to_string(), message_type, Utc::now()));
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_error_expiration();

        let has_identity_that_can_register = !self.user_identities.is_empty();

        // Determine the right-side buttons based on the current DPNSSubscreen
        let mut right_buttons = match self.dpns_subscreen {
            DPNSSubscreen::Active => {
                let refresh_button = if self.refreshing {
                    ("Refreshing...", DesiredAppAction::None)
                } else {
                    (
                        "Refresh",
                        DesiredAppAction::BackendTask(BackendTask::ContestedResourceTask(
                            ContestedResourceTask::QueryDPNSContestedResources,
                        )),
                    )
                };

                vec![refresh_button]
            }

            DPNSSubscreen::Past => {
                // Past contests: similar to Active
                let refresh_button = if self.refreshing {
                    ("Refreshing...", DesiredAppAction::None)
                } else {
                    (
                        "Refresh",
                        DesiredAppAction::BackendTask(BackendTask::ContestedResourceTask(
                            ContestedResourceTask::QueryDPNSContestedResources,
                        )),
                    )
                };

                vec![refresh_button]
            }

            DPNSSubscreen::Owned => {
                // Owned names: refresh or refreshing
                let refresh_button = if self.refreshing {
                    ("Refreshing...", DesiredAppAction::None)
                } else {
                    (
                        "Refresh",
                        DesiredAppAction::BackendTask(BackendTask::IdentityTask(
                            IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
                        )),
                    )
                };

                vec![refresh_button]
            }

            DPNSSubscreen::ScheduledVotes => {
                // Scheduled votes: "Refresh", "Clear All", and "Clear Executed"
                vec![
                    ("Refresh", DesiredAppAction::Refresh),
                    (
                        "Clear All",
                        DesiredAppAction::BackendTask(BackendTask::ContestedResourceTask(
                            ContestedResourceTask::ClearAllScheduledVotes,
                        )),
                    ),
                    (
                        "Clear Executed",
                        DesiredAppAction::BackendTask(BackendTask::ContestedResourceTask(
                            ContestedResourceTask::ClearExecutedScheduledVotes,
                        )),
                    ),
                ]
            }
        };

        if has_identity_that_can_register {
            right_buttons.insert(
                0,
                (
                    "Register Name",
                    DesiredAppAction::AddScreenType(ScreenType::RegisterDpnsName),
                ),
            );
        }

        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("DPNS", AppAction::None)],
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
            DPNSSubscreen::ScheduledVotes => {
                action |= add_left_panel(
                    ctx,
                    &self.app_context,
                    RootScreenType::RootScreenDPNSScheduledVotes,
                );
            }
        }
        action |= add_dpns_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        // Render the UI with the cloned contested_names vector
        egui::CentralPanel::default().show(ctx, |ui| {
            let error_message = self.error_message.clone();
            if let Some((message, message_type, timestamp)) = error_message {
                if message_type != MessageType::Success {
                    let message_color = match message_type {
                        MessageType::Error => egui::Color32::RED,
                        MessageType::Info => egui::Color32::BLACK,
                        MessageType::Success => unreachable!(),
                    };

                    ui.add_space(10.0);
                    ui.allocate_ui(egui::Vec2::new(ui.available_width(), 50.0), |ui| {
                        ui.group(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(message).color(message_color));
                                let now = Utc::now();
                                let elapsed = now.signed_duration_since(timestamp);
                                if ui
                                    .button(format!("Dismiss ({})", 10 - elapsed.num_seconds()))
                                    .clicked()
                                {
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
            // Check if there are any owned dpns names to display
            let has_dpns_names = {
                let dpns_names = self.local_dpns_names.lock().unwrap();
                !dpns_names.is_empty()
            };
            // Check if there are any scheduled votes to display
            let has_scheduled_votes = {
                let scheduled_votes = self.scheduled_votes.lock().unwrap();
                !scheduled_votes.is_empty()
            };

            // Render the proper table
            match self.dpns_subscreen {
                DPNSSubscreen::Active => {
                    if has_contested_names {
                        self.render_table_active_contests(ui);
                    } else {
                        action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::Past => {
                    if has_contested_names {
                        self.render_table_past_contests(ui);
                    } else {
                        action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::Owned => {
                    if has_dpns_names {
                        self.render_table_local_dpns_names(ui);
                    } else {
                        action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::ScheduledVotes => {
                    if has_scheduled_votes {
                        action |= self.render_table_scheduled_votes(ui);
                    } else {
                        action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
            }
        });

        match action {
            AppAction::BackendTask(BackendTask::ContestedResourceTask(
                ContestedResourceTask::QueryDPNSContestedResources,
            ))
            | AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
            )) => {
                self.refreshing = true;
            }
            AppAction::SetMainScreen(_) => {
                self.refreshing = false;
            }
            _ => {}
        }

        action
    }
}
