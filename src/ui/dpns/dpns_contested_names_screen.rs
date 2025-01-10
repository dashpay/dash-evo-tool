use std::sync::{Arc, Mutex};

use chrono::{DateTime, LocalResult, TimeZone, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use eframe::egui::{
    self, Button, CentralPanel, Color32, ComboBox, Context, Frame, Label, Margin, RichText, Ui,
};
use egui_extras::{Column, TableBuilder};
use itertools::Itertools;

use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::contested_names::{ContestedResourceTask, ScheduledDPNSVote};
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::contested_name::{ContestState, ContestedName};
use crate::model::qualified_identity::{DPNSNameInfo, IdentityType, QualifiedIdentity};
use crate::ui::components::dpns_subscreen_chooser_panel::add_dpns_subscreen_chooser_panel;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::add_existing_identity_screen::AddExistingIdentityScreen;
use crate::ui::{
    BackendTaskSuccessResult, MessageType, RootScreenType, Screen, ScreenLike, ScreenType,
};

/// Which DPNS sub-screen is currently showing.
#[derive(PartialEq)]
pub enum DPNSSubscreen {
    Active,
    Past,
    Owned,
    ScheduledVotes,
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

/// Minimal object for storing the user’s currently selected vote on a single contested name.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectedVote {
    pub contested_name: String,
    pub vote_choice: ResourceVoteChoice,
    pub end_time: Option<u64>,
}

#[derive(Clone)]
pub enum VoteOption {
    None,
    Immediate,
    Scheduled { days: u32, hours: u32, minutes: u32 },
}

/// Tracks the casting status for each scheduled vote item.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum IndividualVoteCastingStatus {
    NotStarted,
    InProgress,
    Failed,
    Completed,
}

/// The main, combined DPNSScreen:
/// - Displays active/past/owned DPNS contests
/// - Allows SHIFT-click selection of votes (bulk scheduling)
/// - Allows single immediate vote or single schedule
/// - Shows scheduled votes listing
pub struct DPNSScreen {
    voting_identities: Vec<QualifiedIdentity>,
    user_identities: Vec<QualifiedIdentity>,
    contested_names: Arc<Mutex<Vec<ContestedName>>>,
    local_dpns_names: Arc<Mutex<Vec<(Identifier, DPNSNameInfo)>>>,
    pub scheduled_votes: Arc<Mutex<Vec<(ScheduledDPNSVote, IndividualVoteCastingStatus)>>>,
    pub selected_votes: Vec<SelectedVote>,
    pub app_context: Arc<AppContext>,
    message: Option<(String, MessageType, DateTime<Utc>)>,

    /// Sorting
    sort_column: SortColumn,
    sort_order: SortOrder,

    /// If user has clicked one of the immediate “Vote” buttons in the row, we show a confirmation popup.
    show_vote_popup_info: Option<(String, ContestedResourceTask)>,
    popup_pending_vote_action: Option<ContestedResourceTask>,
    pub vote_cast_in_progress: bool,
    pending_backend_task: Option<BackendTask>,

    /// Which sub-screen is active: Active contests, Past, Owned, or Scheduled
    pub dpns_subscreen: DPNSSubscreen,
    refreshing: bool,

    /// True if we should display the ephemeral Bulk Scheduling UI (replaces the old separate screen).
    show_bulk_schedule_popup: bool,
    /// The "VoteOption" (None, Immediate, Scheduled) for each identity in the ephemeral UI.
    bulk_identity_options: Vec<VoteOption>,
    /// Any message from the ephemeral scheduling process
    bulk_schedule_message: Option<(MessageType, String)>,
    /// If immediate casting finishes and we still have scheduled votes to add, store them here
    bulk_pending_scheduled: Option<Vec<ScheduledDPNSVote>>,

    /// If the user wants to schedule exactly 1 name+choice with multiple identities,
    /// store that ephemeral UI state here:
    show_single_schedule_popup: bool,
    single_schedule_contested_name: String,
    single_schedule_ending_time: u64,
    single_schedule_identities: Vec<QualifiedIdentity>,
    single_schedule_choice: ResourceVoteChoice,
    single_schedule_identity_options: Vec<VoteOption>,
    single_schedule_message: Option<(MessageType, String)>,
}

/// Sorting columns
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

impl DPNSScreen {
    pub fn new(app_context: &Arc<AppContext>, dpns_subscreen: DPNSSubscreen) -> Self {
        // Load contested names, local dpns, scheduled, etc.:
        let contested_names = Arc::new(Mutex::new(match dpns_subscreen {
            DPNSSubscreen::Active => app_context.ongoing_contested_names().unwrap_or_default(),
            DPNSSubscreen::Past => app_context.all_contested_names().unwrap_or_default(),
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
                .map(|vote| {
                    if vote.executed_successfully {
                        (vote.clone(), IndividualVoteCastingStatus::Completed)
                    } else {
                        (vote.clone(), IndividualVoteCastingStatus::NotStarted)
                    }
                })
                .collect::<Vec<_>>(),
        ));

        let voting_identities = app_context
            .db
            .get_local_voting_identities(app_context)
            .unwrap_or_default();
        let user_identities = app_context
            .db
            .get_local_user_identities(app_context)
            .unwrap_or_default();

        // Initialize ephemeral bulk-schedule state to hidden
        let identity_count = voting_identities.len();
        let bulk_identity_options = vec![VoteOption::Immediate; identity_count];

        // For single-scheduling ephemeral popup, default hidden
        Self {
            voting_identities,
            user_identities,
            contested_names,
            local_dpns_names,
            scheduled_votes: scheduled_votes_with_status,
            selected_votes: Vec::new(),
            app_context: app_context.clone(),
            message: None,
            sort_column: SortColumn::ContestedName,
            sort_order: SortOrder::Ascending,
            show_vote_popup_info: None,
            popup_pending_vote_action: None,
            vote_cast_in_progress: false,
            pending_backend_task: None,
            dpns_subscreen,
            refreshing: false,

            // Bulk-schedule
            show_bulk_schedule_popup: false,
            bulk_identity_options,
            bulk_schedule_message: None,
            bulk_pending_scheduled: None,

            // Single-schedule
            show_single_schedule_popup: false,
            single_schedule_contested_name: String::new(),
            single_schedule_ending_time: 0,
            single_schedule_identities: Vec::new(),
            single_schedule_choice: ResourceVoteChoice::Abstain,
            single_schedule_identity_options: Vec::new(),
            single_schedule_message: None,
        }
    }

    // ---------------------------
    // Error handling
    // ---------------------------
    fn dismiss_message(&mut self) {
        self.message = None;
    }

    fn check_error_expiration(&mut self) {
        if let Some((_, _, timestamp)) = &self.message {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(*timestamp);
            if elapsed.num_seconds() >= 10 {
                self.dismiss_message();
            }
        }
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

    // ---------------------------
    // Rendering: Empty states
    // ---------------------------
    fn render_no_active_contests_or_owned_names(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            match self.dpns_subscreen {
                DPNSSubscreen::Active => {
                    ui.label(
                        egui::RichText::new("No active contests at the moment.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                DPNSSubscreen::Past => {
                    ui.label(
                        egui::RichText::new("No active or past contests at the moment.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                DPNSSubscreen::Owned => {
                    ui.label(
                        egui::RichText::new("No owned usernames.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                DPNSSubscreen::ScheduledVotes => {
                    ui.label(
                        egui::RichText::new("No scheduled votes.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
            }
            ui.add_space(10.0);

            if self.dpns_subscreen != DPNSSubscreen::ScheduledVotes {
                ui.label("Please check back later or try refreshing the list.");
                ui.add_space(20.0);
                if ui.button("Refresh").clicked() {
                    if self.refreshing {
                        app_action = AppAction::None;
                    } else {
                        self.refreshing = true;
                        match self.dpns_subscreen {
                            DPNSSubscreen::Active | DPNSSubscreen::Past => {
                                app_action = AppAction::BackendTask(BackendTask::ContestedResourceTask(
                                    ContestedResourceTask::QueryDPNSContestedResources,
                                ));
                            }
                            DPNSSubscreen::Owned => {
                                app_action = AppAction::BackendTask(BackendTask::IdentityTask(
                                    IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
                                ));
                            }
                            _ => {
                                app_action = AppAction::Refresh;
                            }
                        }
                    }
                }
            } else {
                ui.label(
                    "To schedule votes, go to the Active Contests subscreen, shift-click your choices, and then click the 'Apply Votes' button in the top-right.",
                );
            }
        });

        app_action
    }

    // ---------------------------
    // Rendering: Active, Past, Owned, Scheduled
    // ---------------------------

    /// Show the Active Contests table
    fn render_table_active_contests(&mut self, ui: &mut Ui) {
        let contested_names = {
            let guard = self.contested_names.lock().unwrap();
            let mut cn = guard.clone();
            self.sort_contested_names(&mut cn);
            cn
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
                        .column(Column::initial(100.0).resizable(true)) // Locked
                        .column(Column::initial(100.0).resizable(true)) // Abstain
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
                                    let max_contestant_votes = contested_name
                                        .contestants
                                        .as_ref()
                                        .map(|contestants| {
                                            contestants.iter().map(|c| c.votes).max().unwrap_or(0)
                                        })
                                        .unwrap_or(0);
                                    let is_locked_votes_bold = locked_votes > max_contestant_votes;

                                    // Contested Name
                                    row.col(|ui| {
                                        let (used_name, highlighted) = if let Some(contestants) =
                                            &contested_name.contestants
                                        {
                                            if let Some(first) = contestants.first() {
                                                if contestants.iter().all(|c| c.name == first.name)
                                                {
                                                    // Everyone has same name
                                                    (
                                                        first.name.clone(),
                                                        Some(
                                                            contested_name
                                                                .normalized_contested_name
                                                                .clone(),
                                                        ),
                                                    )
                                                } else {
                                                    // Multiple different names
                                                    (
                                                        contestants
                                                            .iter()
                                                            .map(|c| c.name.clone())
                                                            .join(" or "),
                                                        Some(
                                                            contestants
                                                                .iter()
                                                                .map(|c| {
                                                                    format!(
                                                                        "{} trying to get {}",
                                                                        c.id,
                                                                        c.name.clone()
                                                                    )
                                                                })
                                                                .join(" and "),
                                                        ),
                                                    )
                                                }
                                            } else {
                                                (
                                                    contested_name
                                                        .normalized_contested_name
                                                        .clone(),
                                                    None,
                                                )
                                            }
                                        } else {
                                            (contested_name.normalized_contested_name.clone(), None)
                                        };

                                        let label_response = ui.label(used_name);
                                        if let Some(tooltip) = highlighted {
                                            label_response.on_hover_text(tooltip);
                                        }
                                    });

                                    // LOCK button
                                    row.col(|ui| {
                                        let label_text = format!("{}", locked_votes);
                                        let text_widget = if is_locked_votes_bold {
                                            RichText::new(label_text).strong()
                                        } else {
                                            RichText::new(label_text)
                                        };

                                        // See if this (LOCK) is selected
                                        let is_selected = self.selected_votes.iter().any(|sv| {
                                            sv.contested_name
                                                == contested_name.normalized_contested_name
                                                && sv.vote_choice == ResourceVoteChoice::Lock
                                        });

                                        let button = if is_selected {
                                            Button::new(text_widget)
                                                .fill(Color32::from_rgb(0, 150, 255))
                                        } else {
                                            Button::new(text_widget)
                                        };
                                        let resp = ui.add(button);
                                        if resp.clicked() {
                                            let shift_held = ui.input(|i| i.modifiers.shift_only());
                                            if shift_held {
                                                // SHIFT logic => toggle or replace
                                                if let Some(pos) =
                                                    self.selected_votes.iter().position(|sv| {
                                                        sv.contested_name
                                                            == contested_name
                                                                .normalized_contested_name
                                                    })
                                                {
                                                    if self.selected_votes[pos].vote_choice
                                                        == ResourceVoteChoice::Lock
                                                    {
                                                        // same => remove
                                                        self.selected_votes.remove(pos);
                                                    } else {
                                                        // different => remove old, add new
                                                        self.selected_votes.remove(pos);
                                                        self.selected_votes.push(SelectedVote {
                                                            contested_name: contested_name
                                                                .normalized_contested_name
                                                                .clone(),
                                                            vote_choice: ResourceVoteChoice::Lock,
                                                            end_time: contested_name.end_time,
                                                        });
                                                    }
                                                } else {
                                                    // no existing => add
                                                    self.selected_votes.push(SelectedVote {
                                                        contested_name: contested_name
                                                            .normalized_contested_name
                                                            .clone(),
                                                        vote_choice: ResourceVoteChoice::Lock,
                                                        end_time: contested_name.end_time,
                                                    });
                                                }
                                            } else {
                                                // immediate vote
                                                self.show_vote_popup_info = Some((
                                                    format!(
                                                        "Confirm Voting to Lock the name \"{}\".",
                                                        contested_name.normalized_contested_name
                                                    ),
                                                    ContestedResourceTask::VoteOnDPNSName(
                                                        contested_name
                                                            .normalized_contested_name
                                                            .clone(),
                                                        ResourceVoteChoice::Lock,
                                                        vec![],
                                                    ),
                                                ));
                                            }
                                        }
                                    });

                                    // ABSTAIN button
                                    row.col(|ui| {
                                        let abstain_votes =
                                            contested_name.abstain_votes.unwrap_or(0);
                                        let label_text = format!("{}", abstain_votes);

                                        let is_selected = self.selected_votes.iter().any(|sv| {
                                            sv.contested_name
                                                == contested_name.normalized_contested_name
                                                && sv.vote_choice == ResourceVoteChoice::Abstain
                                        });

                                        let button = if is_selected {
                                            Button::new(label_text)
                                                .fill(Color32::from_rgb(0, 150, 255))
                                        } else {
                                            Button::new(label_text)
                                        };
                                        let resp = ui.add(button);
                                        if resp.clicked() {
                                            let shift_held = ui.input(|i| i.modifiers.shift_only());
                                            if shift_held {
                                                if let Some(pos) =
                                                    self.selected_votes.iter().position(|sv| {
                                                        sv.contested_name
                                                            == contested_name
                                                                .normalized_contested_name
                                                    })
                                                {
                                                    if self.selected_votes[pos].vote_choice
                                                        == ResourceVoteChoice::Abstain
                                                    {
                                                        self.selected_votes.remove(pos);
                                                    } else {
                                                        self.selected_votes.remove(pos);
                                                        self.selected_votes.push(SelectedVote {
                                                            contested_name: contested_name
                                                                .normalized_contested_name
                                                                .clone(),
                                                            vote_choice:
                                                                ResourceVoteChoice::Abstain,
                                                            end_time: contested_name.end_time,
                                                        });
                                                    }
                                                } else {
                                                    self.selected_votes.push(SelectedVote {
                                                        contested_name: contested_name
                                                            .normalized_contested_name
                                                            .clone(),
                                                        vote_choice: ResourceVoteChoice::Abstain,
                                                        end_time: contested_name.end_time,
                                                    });
                                                }
                                            } else {
                                                self.show_vote_popup_info = Some((
                                                    format!(
                                                        "Confirm Voting to Abstain on \"{}\".",
                                                        contested_name.normalized_contested_name
                                                    ),
                                                    ContestedResourceTask::VoteOnDPNSName(
                                                        contested_name
                                                            .normalized_contested_name
                                                            .clone(),
                                                        ResourceVoteChoice::Abstain,
                                                        vec![],
                                                    ),
                                                ));
                                            }
                                        }
                                    });

                                    // Ending Time
                                    row.col(|ui| {
                                        if let Some(ending_time) = contested_name.end_time {
                                            if let LocalResult::Single(dt) =
                                                Utc.timestamp_millis_opt(ending_time as i64)
                                            {
                                                let iso_date = dt.format("%Y-%m-%d %H:%M:%S");
                                                let relative_time = HumanTime::from(dt).to_string();
                                                let text =
                                                    format!("{} ({})", iso_date, relative_time);
                                                ui.label(text);
                                            } else {
                                                ui.label("Invalid timestamp");
                                            }
                                        } else {
                                            ui.label("Fetching");
                                        }
                                    });

                                    // Last Updated
                                    row.col(|ui| {
                                        if let Some(last_updated) = contested_name.last_updated {
                                            if let LocalResult::Single(dt) =
                                                Utc.timestamp_opt(last_updated as i64, 0)
                                            {
                                                let rel_time = HumanTime::from(dt).to_string();
                                                if rel_time.contains("seconds") {
                                                    ui.label("now");
                                                } else {
                                                    ui.label(rel_time);
                                                }
                                            } else {
                                                ui.label("Invalid timestamp");
                                            }
                                        } else {
                                            ui.label("Fetching");
                                        }
                                    });

                                    // Contestants
                                    row.col(|ui| {
                                        self.show_contestants_for_contested_name(
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

    /// Show a Past Contests table
    fn render_table_past_contests(&mut self, ui: &mut Ui) {
        let contested_names = {
            let guard = self.contested_names.lock().unwrap();
            let mut cn = guard.clone();
            cn.retain(|c| c.awarded_to.is_some() || c.state == ContestState::Locked);
            self.sort_contested_names(&mut cn);
            cn
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
                        .column(Column::initial(200.0).resizable(true)) // Name
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
                                    // Name
                                    row.col(|ui| {
                                        ui.label(&contested_name.normalized_contested_name);
                                    });
                                    // Ended Time
                                    row.col(|ui| {
                                        if let Some(ended_time) = contested_name.end_time {
                                            if let LocalResult::Single(dt) =
                                                Utc.timestamp_millis_opt(ended_time as i64)
                                            {
                                                let iso =
                                                    dt.format("%Y-%m-%d %H:%M:%S").to_string();
                                                let relative = HumanTime::from(dt).to_string();
                                                ui.label(format!("{} ({})", iso, relative));
                                            } else {
                                                ui.label("Invalid timestamp");
                                            }
                                        } else {
                                            ui.label("Fetching");
                                        }
                                    });
                                    // Last Updated
                                    row.col(|ui| {
                                        if let Some(last_updated) = contested_name.last_updated {
                                            if let LocalResult::Single(dt) =
                                                Utc.timestamp_opt(last_updated as i64, 0)
                                            {
                                                let rel = HumanTime::from(dt).to_string();
                                                if rel.contains("seconds") {
                                                    ui.label("now");
                                                } else {
                                                    ui.label(rel);
                                                }
                                            } else {
                                                ui.label("Invalid timestamp");
                                            }
                                        } else {
                                            ui.label("Fetching");
                                        }
                                    });
                                    // Awarded To
                                    row.col(|ui| match contested_name.state {
                                        ContestState::Unknown => {
                                            ui.label("Fetching");
                                        }
                                        ContestState::Joinable | ContestState::Ongoing => {
                                            ui.label("Active");
                                        }
                                        ContestState::WonBy(identifier) => {
                                            ui.label(identifier.to_string(Encoding::Base58));
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

    /// Show the Owned DPNS names table
    fn render_table_local_dpns_names(&mut self, ui: &mut Ui) {
        let mut sorted_names = {
            let guard = self.local_dpns_names.lock().unwrap();
            guard.clone()
        };
        // Sort
        sorted_names.sort_by(|a, b| match self.sort_column {
            SortColumn::ContestedName => {
                let order = a.1.name.cmp(&b.1.name);
                if self.sort_order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                }
            }
            SortColumn::AwardedTo => {
                let order = a.0.cmp(&b.0);
                if self.sort_order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                }
            }
            SortColumn::EndingTime => {
                let order = a.1.acquired_at.cmp(&b.1.acquired_at);
                if self.sort_order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                }
            }
            _ => std::cmp::Ordering::Equal,
        });

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
                        .column(Column::initial(400.0).resizable(true)) // Owner ID
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
                                    let dt = DateTime::from_timestamp(
                                        dpns_info.acquired_at as i64 / 1000,
                                        ((dpns_info.acquired_at % 1000) * 1_000_000) as u32,
                                    )
                                    .map(|dt| dt.to_string())
                                    .unwrap_or_else(|| "Invalid timestamp".to_string());
                                    row.col(|ui| {
                                        ui.label(dt);
                                    });
                                });
                            }
                        });
                });
        });
    }

    /// Show the Scheduled Votes table
    fn render_table_scheduled_votes(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut sorted_votes = {
            let guard = self.scheduled_votes.lock().unwrap();
            guard.clone()
        };
        // Sort by contested_name or time
        sorted_votes.sort_by(|a, b| {
            let order = a.0.contested_name.cmp(&b.0.contested_name);
            if self.sort_order == SortOrder::Descending {
                order.reverse()
            } else {
                order
            }
        });

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
                        .column(Column::initial(100.0).resizable(true)) // ContestedName
                        .column(Column::initial(200.0).resizable(true)) // Voter
                        .column(Column::initial(200.0).resizable(true)) // Choice
                        .column(Column::initial(200.0).resizable(true)) // Time
                        .column(Column::initial(100.0).resizable(true)) // Status
                        .column(Column::initial(100.0).resizable(true)) // Actions
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                if ui.button("Contested Name").clicked() {
                                    self.toggle_sort(SortColumn::ContestedName);
                                }
                            });
                            header.col(|ui| {
                                ui.heading("Voter");
                            });
                            header.col(|ui| {
                                ui.heading("Vote Choice");
                            });
                            header.col(|ui| {
                                if ui.button("Scheduled Time").clicked() {
                                    self.toggle_sort(SortColumn::EndingTime);
                                }
                            });
                            header.col(|ui| {
                                ui.heading("Status");
                            });
                            header.col(|ui| {
                                ui.heading("Actions");
                            });
                        })
                        .body(|mut body| {
                            for vote in sorted_votes.iter_mut() {
                                body.row(25.0, |mut row| {
                                    // Contested name
                                    row.col(|ui| {
                                        ui.add(Label::new(&vote.0.contested_name));
                                    });
                                    // Voter
                                    row.col(|ui| {
                                        ui.add(
                                            Label::new(vote.0.voter_id.to_string(Encoding::Hex))
                                                .truncate(),
                                        );
                                    });
                                    // Choice
                                    row.col(|ui| {
                                        let display_text = match &vote.0.choice {
                                            ResourceVoteChoice::TowardsIdentity(id) => {
                                                id.to_string(Encoding::Base58)
                                            }
                                            other => other.to_string(),
                                        };
                                        ui.add(Label::new(display_text));
                                    });
                                    // Time
                                    row.col(|ui| {
                                        if let LocalResult::Single(dt) =
                                            Utc.timestamp_millis_opt(vote.0.unix_timestamp as i64)
                                        {
                                            let iso = dt.format("%Y-%m-%d %H:%M:%S").to_string();
                                            let relative = HumanTime::from(dt).to_string();
                                            let text = format!("{} ({})", iso, relative);
                                            ui.label(text);
                                        } else {
                                            ui.label("Invalid timestamp");
                                        }
                                    });
                                    // Status
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
                                            ui.colored_label(Color32::DARK_GREEN, "Casted");
                                        }
                                    });
                                    // Actions
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
                                        // If the user wants to do "Cast Now" from here, they can
                                        // if NotStarted or Failed. If in progress or done, disabled.
                                        let cast_button_enabled = matches!(
                                            vote.1,
                                            IndividualVoteCastingStatus::NotStarted
                                                | IndividualVoteCastingStatus::Failed
                                        ) && !self.vote_cast_in_progress;

                                        let cast_button = if cast_button_enabled {
                                            Button::new("Cast Now")
                                        } else {
                                            Button::new("Cast Now").sense(egui::Sense::hover())
                                        };

                                        if ui.add(cast_button).clicked() && cast_button_enabled {
                                            self.vote_cast_in_progress = true;
                                            vote.1 = IndividualVoteCastingStatus::InProgress;

                                            // Mark in our Arc as well
                                            if let Ok(mut sched_guard) = self.scheduled_votes.lock()
                                            {
                                                if let Some(t) =
                                                    sched_guard.iter_mut().find(|(sv, _)| {
                                                        sv.voter_id == vote.0.voter_id
                                                            && sv.contested_name
                                                                == vote.0.contested_name
                                                    })
                                                {
                                                    t.1 = IndividualVoteCastingStatus::InProgress;
                                                }
                                            }
                                            // dispatch the actual cast
                                            let local_ids = match self
                                                .app_context
                                                .load_local_voting_identities()
                                            {
                                                Ok(ids) => ids,
                                                Err(e) => {
                                                    eprintln!("Error: {}", e);
                                                    return;
                                                }
                                            };
                                            if let Some(found) = local_ids
                                                .iter()
                                                .find(|i| i.identity.id() == vote.0.voter_id)
                                            {
                                                action = AppAction::BackendTask(
                                                    BackendTask::ContestedResourceTask(
                                                        ContestedResourceTask::CastScheduledVote(
                                                            vote.0.clone(),
                                                            found.clone(),
                                                        ),
                                                    ),
                                                );
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

    /// For each contested name row, show the possible contestants. This is the old `show_contested_name_details` function.
    fn show_contestants_for_contested_name(
        &mut self,
        ui: &mut Ui,
        contested_name: &ContestedName,
        is_locked_votes_bold: bool,
        max_contestant_votes: u32,
    ) {
        if let Some(contestants) = &contested_name.contestants {
            for contestant in contestants {
                let first_6_chars: String = contestant
                    .id
                    .to_string(Encoding::Base58)
                    .chars()
                    .take(6)
                    .collect();
                let button_text = format!("{}... - {} votes", first_6_chars, contestant.votes);

                // Bold if highest
                let text = if contestant.votes == max_contestant_votes && !is_locked_votes_bold {
                    RichText::new(button_text)
                        .strong()
                        .color(Color32::from_rgb(0, 100, 0))
                } else {
                    RichText::new(button_text)
                };

                // Check if selected
                let is_selected = self.selected_votes.iter().any(|sv| {
                    sv.contested_name == contested_name.normalized_contested_name
                        && sv.vote_choice == ResourceVoteChoice::TowardsIdentity(contestant.id)
                });

                let button = if is_selected {
                    Button::new(text).fill(Color32::from_rgb(0, 150, 255))
                } else {
                    Button::new(text)
                };
                let resp = ui.add(button);
                if resp.clicked() {
                    let shift_held = ui.input(|i| i.modifiers.shift_only());
                    if shift_held {
                        if let Some(pos) = self.selected_votes.iter().position(|sv| {
                            sv.contested_name == contested_name.normalized_contested_name
                        }) {
                            let existing_choice = &self.selected_votes[pos].vote_choice;
                            let new_choice = ResourceVoteChoice::TowardsIdentity(contestant.id);

                            if *existing_choice == new_choice {
                                // remove
                                self.selected_votes.remove(pos);
                            } else {
                                // replace
                                self.selected_votes.remove(pos);
                                self.selected_votes.push(SelectedVote {
                                    contested_name: contested_name
                                        .normalized_contested_name
                                        .clone(),
                                    vote_choice: new_choice,
                                    end_time: contested_name.end_time,
                                });
                            }
                        } else {
                            // no existing => add
                            self.selected_votes.push(SelectedVote {
                                contested_name: contested_name.normalized_contested_name.clone(),
                                vote_choice: ResourceVoteChoice::TowardsIdentity(contestant.id),
                                end_time: contested_name.end_time,
                            });
                        }
                    } else {
                        // immediate vote
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
    }

    // ---------------------------
    // Bulk scheduling ephemeral UI
    // ---------------------------
    fn show_bulk_schedule_popup_window(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.heading("Cast or Schedule Votes");
        ui.add_space(10.0);

        if self.selected_votes.is_empty() {
            ui.colored_label(
                Color32::DARK_RED,
                "No votes selected. SHIFT-click on vote choices in the Active Contests table first.",
            );
            return action;
        }

        ui.label("NOTE: Dash Evo Tool must remain running and connected for scheduled votes to execute on time.");
        ui.add_space(10.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            // Define a frame with custom background color and border
            Frame::group(ui.style())
                .fill(ui.visuals().panel_fill) // Use panel fill color
                .stroke(egui::Stroke::new(
                    1.0,
                    ui.visuals().widgets.inactive.bg_stroke.color,
                ))
                .inner_margin(Margin::same(8.0))
                .show(ui, |ui| {
                    // Show which votes were SHIFT-clicked
                    ui.group(|ui| {
                        ui.heading("Selected Votes:");
                        ui.separator();
                        for sv in &self.selected_votes {
                            // Convert end_time -> readable
                            let end_str = if let Some(e) = sv.end_time {
                                if let LocalResult::Single(dt) = Utc.timestamp_millis_opt(e as i64)
                                {
                                    let iso = dt.format("%Y-%m-%d %H:%M:%S").to_string();
                                    let rel = HumanTime::from(dt).to_string();
                                    format!("{} ({})", iso, rel)
                                } else {
                                    "Invalid timestamp".to_string()
                                }
                            } else {
                                "N/A".to_string()
                            };
                            let display_text = match &sv.vote_choice {
                                ResourceVoteChoice::TowardsIdentity(id) => {
                                    id.to_string(Encoding::Base58)
                                }
                                other => other.to_string(),
                            };
                            ui.label(format!(
                                "{}   =>   {}, ends at {}",
                                sv.contested_name, display_text, end_str
                            ));
                        }
                    });

                    ui.add_space(10.0);

                    // Show each identity + let user pick None / Immediate / Scheduled
                    ui.heading("Select cast method for each node:");
                    for (i, identity) in self.voting_identities.iter().enumerate() {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let label = identity.alias.clone().unwrap_or_else(|| {
                                    identity.identity.id().to_string(Encoding::Base58)
                                });
                                ui.label(format!("Identity: {}", label));

                                let current_option = &mut self.bulk_identity_options[i];
                                ComboBox::from_id_salt(format!("combo_bulk_identity_{}", i))
                                    .width(120.0)
                                    .selected_text(match current_option {
                                        VoteOption::None => "No Vote".to_string(),
                                        VoteOption::Immediate => "Cast Now".to_string(),
                                        VoteOption::Scheduled { .. } => "Schedule".to_string(),
                                    })
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(
                                                matches!(current_option, VoteOption::None),
                                                "No Vote",
                                            )
                                            .clicked()
                                        {
                                            *current_option = VoteOption::None;
                                        }
                                        if ui
                                            .selectable_label(
                                                matches!(current_option, VoteOption::Immediate),
                                                "Cast Now",
                                            )
                                            .clicked()
                                        {
                                            *current_option = VoteOption::Immediate;
                                        }
                                        if ui
                                            .selectable_label(
                                                matches!(
                                                    current_option,
                                                    VoteOption::Scheduled { .. }
                                                ),
                                                "Schedule",
                                            )
                                            .clicked()
                                        {
                                            let (d, h, m) = match current_option {
                                                VoteOption::Scheduled {
                                                    days,
                                                    hours,
                                                    minutes,
                                                } => (*days, *hours, *minutes),
                                                _ => (0, 0, 0),
                                            };
                                            *current_option = VoteOption::Scheduled {
                                                days: d,
                                                hours: h,
                                                minutes: m,
                                            };
                                        }
                                    });

                                if let VoteOption::Scheduled {
                                    days,
                                    hours,
                                    minutes,
                                } = current_option
                                {
                                    ui.label("Schedule In:");
                                    ui.add(
                                        egui::DragValue::new(days).prefix("Days: ").range(0..=14),
                                    );
                                    ui.add(
                                        egui::DragValue::new(hours).prefix("Hours: ").range(0..=23),
                                    );
                                    ui.add(
                                        egui::DragValue::new(minutes).prefix("Min: ").range(0..=59),
                                    );
                                }
                            });
                        });
                        ui.add_space(10.0);
                    }
                })
        });

        // "Apply Votes" button
        let button = egui::Button::new(RichText::new("Apply Votes").color(Color32::WHITE))
            .fill(Color32::from_rgb(0, 128, 255))
            .rounding(3.0);
        if ui.add(button).clicked() {
            action = self.bulk_schedule_votes();
        }

        // Show any message
        if let Some((msg_type, msg_str)) = &self.bulk_schedule_message {
            ui.add_space(10.0);
            match msg_type {
                MessageType::Error => {
                    ui.colored_label(Color32::RED, msg_str);
                }
                MessageType::Success => {
                    if msg_str.contains("Votes scheduled") {
                        // Show success screen
                        action |= self.show_bulk_schedule_success(ui);
                    } else {
                        ui.colored_label(Color32::DARK_GREEN, msg_str);
                    }
                }
                MessageType::Info => {
                    ui.colored_label(Color32::YELLOW, msg_str);
                }
            }
        }

        ui.separator();
        if ui.button("Cancel").clicked() {
            self.show_bulk_schedule_popup = false;
            self.bulk_schedule_message = None;
        }

        action
    }

    /// The logic that was in BulkScheduleVoteScreen::schedule_votes
    fn bulk_schedule_votes(&mut self) -> AppAction {
        // Partition immediate vs scheduled
        let mut immediate_list = Vec::new();
        let mut scheduled_list = Vec::new();

        for (identity, option) in self
            .voting_identities
            .iter()
            .zip(&self.bulk_identity_options)
        {
            match option {
                VoteOption::None => {}
                VoteOption::Immediate => {
                    immediate_list.push(identity.clone());
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
                        let new_vote = ScheduledDPNSVote {
                            contested_name: sv.contested_name.clone(),
                            voter_id: identity.identity.id().clone(),
                            choice: sv.vote_choice.clone(),
                            unix_timestamp: scheduled_time,
                            executed_successfully: false,
                        };
                        scheduled_list.push(new_vote);
                    }
                }
            }
        }

        if immediate_list.is_empty() && scheduled_list.is_empty() {
            self.bulk_schedule_message = Some((
                MessageType::Error,
                "No votes selected (or all set to None).".to_string(),
            ));
            return AppAction::None;
        }

        // 1) If immediate_list is not empty, vote now
        if !immediate_list.is_empty() {
            let votes_for_all: Vec<(String, ResourceVoteChoice)> = self
                .selected_votes
                .iter()
                .map(|sv| (sv.contested_name.clone(), sv.vote_choice))
                .collect();
            if !scheduled_list.is_empty() {
                // store scheduled for after immediate completes
                self.bulk_pending_scheduled = Some(scheduled_list);
            }
            return AppAction::BackendTask(BackendTask::ContestedResourceTask(
                ContestedResourceTask::VoteOnMultipleDPNSNames(votes_for_all, immediate_list),
            ));
        } else {
            // 2) Otherwise just schedule them
            return AppAction::BackendTask(BackendTask::ContestedResourceTask(
                ContestedResourceTask::ScheduleDPNSVotes(scheduled_list),
            ));
        }
    }

    /// If scheduling is successful, show success message with link to go to Scheduled
    fn show_bulk_schedule_success(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.heading("🎉 Successfully scheduled votes.");

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

    // ---------------------------
    // Single scheduling ephemeral UI
    // ---------------------------
    fn show_single_schedule_popup_window(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.heading("Cast or Schedule Votes");
        ui.separator();
        ui.label("NOTE: Dash Evo Tool must remain running and connected for scheduled votes to execute on time.");

        egui::ScrollArea::vertical().show(ui, |ui| {
            Frame::group(ui.style())
                .fill(ui.visuals().panel_fill) // Use panel fill color
                .stroke(egui::Stroke::new(
                    1.0,
                    ui.visuals().widgets.inactive.bg_stroke.color,
                ))
                .inner_margin(Margin::same(8.0))
                .show(ui, |ui| {
                    // Show the vote choice
                    ui.label(format!(
                        "Vote Choice: {}",
                        match self.single_schedule_choice {
                            ResourceVoteChoice::TowardsIdentity(id) => {
                                id.to_string(Encoding::Base58)
                            }
                            other => other.to_string(),
                        }
                    ));

                    // Show the contest end time
                    if let LocalResult::Single(end_dt) =
                        Utc.timestamp_millis_opt(self.single_schedule_ending_time as i64)
                    {
                        let iso = end_dt.format("%Y-%m-%d %H:%M:%S").to_string();
                        let rel = HumanTime::from(end_dt).to_string();
                        ui.label(format!(
                            "Contest for name {} ends at {} ({})",
                            self.single_schedule_contested_name, iso, rel
                        ));
                    } else {
                        ui.colored_label(Color32::DARK_RED, "Error reading contest end time");
                    }

                    ui.add_space(10.0);

                    // For each identity, show row with None/Immediate/Scheduled
                    for (i, identity) in self.single_schedule_identities.iter().enumerate() {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                let label = identity.alias.clone().unwrap_or_else(|| {
                                    identity.identity.id().to_string(Encoding::Base58)
                                });
                                ui.label(format!("Identity: {}", label));

                                let opt = &mut self.single_schedule_identity_options[i];
                                ComboBox::from_id_salt(format!("single_sch_combo_{}", i))
                                    .selected_text(match opt {
                                        VoteOption::None => "No Vote".to_string(),
                                        VoteOption::Immediate => "Cast Now".to_string(),
                                        VoteOption::Scheduled { .. } => "Schedule".to_string(),
                                    })
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(
                                                matches!(opt, VoteOption::None),
                                                "No Vote",
                                            )
                                            .clicked()
                                        {
                                            *opt = VoteOption::None;
                                        }
                                        if ui
                                            .selectable_label(
                                                matches!(opt, VoteOption::Immediate),
                                                "Cast Now",
                                            )
                                            .clicked()
                                        {
                                            *opt = VoteOption::Immediate;
                                        }
                                        if ui
                                            .selectable_label(
                                                matches!(opt, VoteOption::Scheduled { .. }),
                                                "Schedule",
                                            )
                                            .clicked()
                                        {
                                            let (d, h, m) = match opt {
                                                VoteOption::Scheduled {
                                                    days,
                                                    hours,
                                                    minutes,
                                                } => (*days, *hours, *minutes),
                                                _ => (0, 0, 0),
                                            };
                                            *opt = VoteOption::Scheduled {
                                                days: d,
                                                hours: h,
                                                minutes: m,
                                            };
                                        }
                                    });
                                if let VoteOption::Scheduled {
                                    days,
                                    hours,
                                    minutes,
                                } = opt
                                {
                                    ui.label("Schedule In:");
                                    ui.add(
                                        egui::DragValue::new(days).range(0..=14).prefix("Days: "),
                                    );
                                    ui.add(
                                        egui::DragValue::new(hours).range(0..=23).prefix("Hours: "),
                                    );
                                    ui.add(
                                        egui::DragValue::new(minutes).range(0..=59).prefix("Min: "),
                                    );
                                }
                            });
                        });
                        ui.add_space(10.0);
                    }
                })
        });

        let button = egui::Button::new(RichText::new("Apply Votes").color(Color32::WHITE))
            .fill(Color32::from_rgb(0, 128, 255))
            .rounding(3.0);
        if ui.add(button).clicked() {
            action = self.cast_single_schedule_votes();
        }
        ui.add_space(10.0);

        if let Some((msg_type, msg_str)) = &self.single_schedule_message {
            match msg_type {
                MessageType::Error => {
                    ui.colored_label(Color32::DARK_RED, msg_str);
                }
                MessageType::Success => {
                    if msg_str.contains("Votes scheduled") {
                        action = self.show_single_schedule_success(ui);
                    } else {
                        ui.colored_label(Color32::DARK_GREEN, msg_str);
                    }
                }
                MessageType::Info => {
                    ui.colored_label(Color32::DARK_BLUE, msg_str);
                }
            }
            ui.add_space(10.0);
        }

        ui.separator();
        if ui.button("Cancel").clicked() {
            self.show_single_schedule_popup = false;
            self.single_schedule_message = None;
        }

        action
    }

    fn cast_single_schedule_votes(&mut self) -> AppAction {
        let mut scheduled = Vec::new();
        for (identity, option) in self
            .single_schedule_identities
            .iter()
            .zip(&self.single_schedule_identity_options)
        {
            match option {
                VoteOption::None => {}
                VoteOption::Immediate => {
                    let vote = ScheduledDPNSVote {
                        contested_name: self.single_schedule_contested_name.clone(),
                        voter_id: identity.identity.id().clone(),
                        choice: self.single_schedule_choice,
                        unix_timestamp: Utc::now().timestamp_millis() as u64,
                        executed_successfully: false,
                    };
                    scheduled.push(vote);
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
                    let st = now + offset;
                    let chosen_time = st.timestamp_millis() as u64;

                    if chosen_time > self.single_schedule_ending_time {
                        self.single_schedule_message = Some((
                            MessageType::Error,
                            "Scheduled time is after the contest end time.".to_string(),
                        ));
                        return AppAction::None;
                    }
                    let vote = ScheduledDPNSVote {
                        contested_name: self.single_schedule_contested_name.clone(),
                        voter_id: identity.identity.id().clone(),
                        choice: self.single_schedule_choice,
                        unix_timestamp: chosen_time,
                        executed_successfully: false,
                    };
                    scheduled.push(vote);
                }
            }
        }

        if scheduled.is_empty() {
            self.single_schedule_message = Some((
                MessageType::Error,
                "No votes selected for scheduling.".to_string(),
            ));
            return AppAction::None;
        }

        AppAction::BackendTask(BackendTask::ContestedResourceTask(
            ContestedResourceTask::ScheduleDPNSVotes(scheduled),
        ))
    }

    fn show_single_schedule_success(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.heading("🎉 Successfully scheduled votes.");

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

    // ---------------------------
    // Immediate vote confirmation popup
    // ---------------------------
    fn show_vote_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;

        if self.voting_identities.is_empty() {
            ui.label("Please load an Evonode or Masternode first before voting.");
            if ui.button("I want to load one now").clicked() {
                self.show_vote_popup_info = None;
                let mut screen = AddExistingIdentityScreen::new(&self.app_context);
                screen.identity_type = IdentityType::Evonode;
                app_action = AppAction::AddScreen(Screen::AddExistingIdentityScreen(screen));
            }
            if ui.button("Cancel").clicked() {
                self.show_vote_popup_info = None;
                self.popup_pending_vote_action = None;
            }
            return app_action;
        }

        if let Some((message, action)) = self.show_vote_popup_info.clone() {
            ui.label(message);
            ui.add_space(10.0);

            if self.popup_pending_vote_action.is_none() {
                ui.label("Select the identity to vote with (or 'All'): ");
            } else {
                ui.label("Vote now or schedule it?");
            }
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                if let ContestedResourceTask::VoteOnDPNSName(name, choice, mut voters) = action {
                    if self.popup_pending_vote_action.is_none() {
                        // Show each identity + an "All" button
                        for identity in &self.voting_identities {
                            if ui.button(identity.display_short_string()).clicked() {
                                voters.push(identity.clone());
                                self.popup_pending_vote_action =
                                    Some(ContestedResourceTask::VoteOnDPNSName(
                                        name.clone(),
                                        choice.clone(),
                                        voters.clone(),
                                    ));
                            }
                        }
                        if ui.button("All").clicked() {
                            voters.extend(self.voting_identities.clone());
                            self.popup_pending_vote_action =
                                Some(ContestedResourceTask::VoteOnDPNSName(
                                    name.clone(),
                                    choice.clone(),
                                    voters,
                                ));
                        }
                    } else {
                        // If we already have a pending action, we ask: "Vote Now" or "Schedule"
                        if ui.button("Vote Now").clicked() {
                            // dispatch immediate
                            app_action =
                                AppAction::BackendTask(BackendTask::ContestedResourceTask(
                                    self.popup_pending_vote_action.take().unwrap(),
                                ));
                            self.show_vote_popup_info = None;
                        }
                        if ui.button("Schedule").clicked() {
                            // single schedule ephemeral UI
                            let pending = self.popup_pending_vote_action.take().unwrap();
                            if let ContestedResourceTask::VoteOnDPNSName(nm, vote_choice, voters) =
                                pending
                            {
                                // Try to get the end_time
                                let end_time = {
                                    let cns = self.contested_names.lock().unwrap();
                                    if let Some(found) =
                                        cns.iter().find(|cn| cn.normalized_contested_name == nm)
                                    {
                                        found.end_time.unwrap_or_default()
                                    } else {
                                        0
                                    }
                                };
                                self.single_schedule_contested_name = nm.clone();
                                self.single_schedule_ending_time = end_time;
                                self.single_schedule_choice = vote_choice;
                                self.single_schedule_identities = voters;
                                self.single_schedule_identity_options = self
                                    .single_schedule_identities
                                    .iter()
                                    .map(|_| VoteOption::Scheduled {
                                        days: 0,
                                        hours: 0,
                                        minutes: 0,
                                    })
                                    .collect();
                                self.show_single_schedule_popup = true;
                            }
                            self.show_vote_popup_info = None;
                        }
                    }
                }
                // Cancel
                if ui.button("Cancel").clicked() {
                    self.show_vote_popup_info = None;
                    self.popup_pending_vote_action = None;
                }
            });
        }

        app_action
    }
}

// ---------------------------
// ScreenLike implementation
// ---------------------------
impl ScreenLike for DPNSScreen {
    fn refresh(&mut self) {
        self.vote_cast_in_progress = false;
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
                let new_scheduled = self.app_context.get_scheduled_votes().unwrap_or_default();
                *scheduled_votes = new_scheduled
                    .iter()
                    .map(|newv| {
                        if newv.executed_successfully {
                            (newv.clone(), IndividualVoteCastingStatus::Completed)
                        } else if let Some(existing) = scheduled_votes.iter().find(|(old, _)| {
                            old.contested_name == newv.contested_name
                                && old.voter_id == newv.voter_id
                        }) {
                            // preserve old status if InProgress/Failed
                            match existing.1 {
                                IndividualVoteCastingStatus::InProgress => {
                                    (newv.clone(), IndividualVoteCastingStatus::InProgress)
                                }
                                IndividualVoteCastingStatus::Failed => {
                                    (newv.clone(), IndividualVoteCastingStatus::Failed)
                                }
                                _ => (newv.clone(), IndividualVoteCastingStatus::NotStarted),
                            }
                        } else {
                            (newv.clone(), IndividualVoteCastingStatus::NotStarted)
                        }
                    })
                    .collect();
            }
        }
    }

    fn refresh_on_arrival(&mut self) {
        self.voting_identities = self
            .app_context
            .db
            .get_local_voting_identities(&self.app_context)
            .unwrap_or_default();
        self.user_identities = self
            .app_context
            .db
            .get_local_user_identities(&self.app_context)
            .unwrap_or_default();
        self.refresh();
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        // Sync error states
        if message.contains("Error casting scheduled vote") {
            self.vote_cast_in_progress = false;
            if let Ok(mut guard) = self.scheduled_votes.lock() {
                for vote in guard.iter_mut() {
                    if vote.1 == IndividualVoteCastingStatus::InProgress {
                        vote.1 = IndividualVoteCastingStatus::Failed;
                    }
                }
            }
        }
        if message.contains("Successfully cast scheduled vote") {
            self.vote_cast_in_progress = false;
        }
        // If it's from a DPNS query or identity refresh, remove refreshing state
        if message.contains("Finished querying DPNS contested resources")
            || message.contains("Successfully refreshed loaded identities dpns names")
            || message.contains("Contested resource query failed")
            || message.contains("Error refreshing owned DPNS names")
        {
            self.refreshing = false;
        }

        // If from BulkSchedule or single schedule
        if let Some((_, m)) = &self.bulk_schedule_message {
            if m.contains("Votes scheduled") {
                // already success
            }
        }
        // Save into general error_message for top-of-screen
        self.message = Some((message.to_string(), message_type, Utc::now()));
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            // If immediate cast finished, see if we have pending to schedule next
            BackendTaskSuccessResult::MultipleDPNSVotesCast(_results) => {
                if let Some(pending) = self.bulk_pending_scheduled.take() {
                    self.pending_backend_task = Some(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::ScheduleDPNSVotes(pending),
                    ));
                    self.bulk_schedule_message = Some((
                        MessageType::Info,
                        "Immediate votes cast. Scheduling the remainder...".to_string(),
                    ));
                } else {
                    // Done
                    self.bulk_schedule_message = Some((
                        MessageType::Success,
                        "All votes cast immediately.".to_string(),
                    ));
                }
            }
            // If scheduling succeeded
            BackendTaskSuccessResult::Message(msg) => {
                // Could be "Votes scheduled" or something else
                if msg.contains("Votes scheduled") {
                    self.bulk_schedule_message =
                        Some((MessageType::Success, "Votes scheduled".to_string()));
                    self.single_schedule_message =
                        Some((MessageType::Success, "Votes scheduled".to_string()));
                }
            }
            _ => {}
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_error_expiration();
        let has_identity_that_can_register = !self.user_identities.is_empty();
        let has_selected_votes = !self.selected_votes.is_empty();

        // Build top-right buttons
        let mut right_buttons = match self.dpns_subscreen {
            DPNSSubscreen::Active => {
                let refresh_button = (
                    "Refresh",
                    DesiredAppAction::BackendTask(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::QueryDPNSContestedResources,
                    )),
                );
                // If we have selected SHIFT-click votes, show "Apply Votes"
                if has_selected_votes {
                    vec![
                        refresh_button,
                        (
                            "Apply Votes",
                            DesiredAppAction::Custom("Apply Votes".to_string()),
                        ), // We'll open our ephemeral bulk UI
                    ]
                } else {
                    vec![refresh_button]
                }
            }
            DPNSSubscreen::Past => {
                let refresh_button = (
                    "Refresh",
                    DesiredAppAction::BackendTask(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::QueryDPNSContestedResources,
                    )),
                );
                vec![refresh_button]
            }
            DPNSSubscreen::Owned => {
                let refresh_button = (
                    "Refresh",
                    DesiredAppAction::BackendTask(BackendTask::IdentityTask(
                        IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
                    )),
                );
                vec![refresh_button]
            }
            DPNSSubscreen::ScheduledVotes => {
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
            // "Register Name" button on the left
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

        // If user clicked "Apply Votes" in the top bar
        if action == AppAction::Custom("Apply Votes".to_string()) {
            // That means the user clicked "Apply Votes"
            self.show_bulk_schedule_popup = true;
            action = AppAction::None; // clear it out so we don't re-trigger
        }

        // Left panel
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

        // Subscreen chooser
        action |= add_dpns_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        // Main panel
        CentralPanel::default().show(ctx, |ui| {
            // If an immediate vote popup is active
            if self.show_vote_popup_info.is_some() {
                egui::Window::new("Vote Confirmation")
                    .collapsible(false)
                    .show(ui.ctx(), |ui| {
                        action |= self.show_vote_popup(ui);
                    });
            }

            // Bulk-schedule ephemeral popup
            if self.show_bulk_schedule_popup {
                egui::Window::new("Voting")
                    .collapsible(false)
                    .resizable(true)
                    .vscroll(true)
                    .show(ui.ctx(), |ui| {
                        action |= self.show_bulk_schedule_popup_window(ui);
                    });
            }

            // Single schedule ephemeral popup
            if self.show_single_schedule_popup {
                egui::Window::new("Voting")
                    .collapsible(false)
                    .resizable(true)
                    .vscroll(true)
                    .show(ui.ctx(), |ui| {
                        action |= self.show_single_schedule_popup_window(ui);
                    });
            }

            // Render sub-screen
            match self.dpns_subscreen {
                DPNSSubscreen::Active => {
                    let has_any = {
                        let guard = self.contested_names.lock().unwrap();
                        !guard.is_empty()
                    };
                    if has_any {
                        self.render_table_active_contests(ui);
                    } else {
                        action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::Past => {
                    let has_any = {
                        let guard = self.contested_names.lock().unwrap();
                        !guard.is_empty()
                    };
                    if has_any {
                        self.render_table_past_contests(ui);
                    } else {
                        action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::Owned => {
                    let has_any = {
                        let guard = self.local_dpns_names.lock().unwrap();
                        !guard.is_empty()
                    };
                    if has_any {
                        self.render_table_local_dpns_names(ui);
                    } else {
                        action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::ScheduledVotes => {
                    let has_any = {
                        let guard = self.scheduled_votes.lock().unwrap();
                        !guard.is_empty()
                    };
                    if has_any {
                        action |= self.render_table_scheduled_votes(ui);
                    } else {
                        action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
            }

            // If we are refreshing, show a message and spinner
            if self.refreshing {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.label("Refreshing. Please wait... ");
                    // Loading spinner
                    ui.add(egui::widgets::Spinner::default());
                });
            }

            // If there's a backend message, show it at the bottom
            if let Some((msg, msg_type, timestamp)) = self.message.clone() {
                ui.add_space(10.0);
                let color = match msg_type {
                    MessageType::Error => Color32::DARK_RED,
                    MessageType::Info => Color32::BLACK,
                    MessageType::Success => Color32::DARK_GREEN,
                };
                ui.add_space(10.0);
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(color, &msg);
                        let now = Utc::now();
                        let elapsed = now.signed_duration_since(timestamp);
                        if ui
                            .button(format!("Dismiss ({})", 10 - elapsed.num_seconds()))
                            .clicked()
                        {
                            self.dismiss_message();
                        }
                    });
                });
            }
        });

        // Extra handling for actions
        match action {
            // If refreshing contested names, set self.refreshing = true
            AppAction::BackendTask(BackendTask::ContestedResourceTask(
                ContestedResourceTask::QueryDPNSContestedResources,
            )) => {
                self.refreshing = true;
            }
            // If refreshing owned names, set self.refreshing = true
            AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
            )) => {
                self.refreshing = true;
            }
            _ => {}
        }

        // If we have a pending backend task from scheduling (e.g. after immediate votes)
        if action == AppAction::None {
            if let Some(bt) = self.pending_backend_task.take() {
                action = AppAction::BackendTask(bt);
            }
        }
        action
    }
}
