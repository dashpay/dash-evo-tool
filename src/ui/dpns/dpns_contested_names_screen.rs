use crate::wallet_backend::poison::MutexRecover;
use std::sync::{Arc, Mutex};
use tracing::error;

use chrono::{DateTime, LocalResult, TimeZone, Utc};
use chrono_humanize::HumanTime;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Button, Color32, ComboBox, Label, RichText, Ui};
use egui_extras::{Column, TableBuilder};
use itertools::Itertools;

use crate::app::{
    AppAction, BackendTasksExecutionMode, DesiredAppAction, scheduled_vote_sweep_is_quiet,
};
use crate::backend_task::BackendTask;
use crate::backend_task::contested_names::{ContestedResourceTask, ScheduledDPNSVote};
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::IdentityTask;
use crate::context::{AppContext, DpnsOperatorRoute};
use crate::model::contested_name::{ContestState, ContestedName};
use crate::model::dpns_voting::{
    DpnsCurrentVoteState, DpnsVoteOperation, DpnsVoteTarget, DpnsVoteTargetKey,
    DpnsVoteTargetStatus, VoteTiming,
};
use crate::model::qualified_identity::{DPNSNameInfo, QualifiedIdentity};
use crate::ui::components::dpns_subscreen_chooser_panel::add_dpns_subscreen_chooser_panel;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::{StyledButton, island_central_panel};
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::{add_top_panel_with_global_nav, subdued_everyday_spec};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::identities::register_dpns_name_screen::RegisterDpnsNameSource;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, ScreenLike, ScreenType};

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
    NoVote,
    CastNow,
    Scheduled { days: u32, hours: u32, minutes: u32 },
}

/// Tracks the casting status for each scheduled vote item.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScheduledVoteCastingStatus {
    NotStarted,
    InProgress,
    Failed,
    Completed,
}

#[derive(PartialEq)]
pub enum VoteHandlingStatus {
    NotStarted,
    CastingVotes,
    SchedulingVotes,
    Completed,
    Failed(String),
}

#[derive(PartialEq)]
pub enum RefreshingStatus {
    Refreshing,
    NotRefreshing,
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

/// The main, combined DPNSScreen:
/// - Displays active/past/owned DPNS contests
/// - Allows clicking selection of votes (bulk scheduling)
/// - Allows single immediate vote or single schedule
/// - Shows scheduled votes listing
pub struct DPNSScreen {
    voting_identities: Vec<QualifiedIdentity>,
    user_identities: Vec<QualifiedIdentity>,
    contested_names: Arc<Mutex<Vec<ContestedName>>>,
    local_dpns_names: Arc<Mutex<Vec<(Identifier, DPNSNameInfo)>>>,
    pub scheduled_votes: Arc<Mutex<Vec<(ScheduledDPNSVote, ScheduledVoteCastingStatus)>>>,
    pub scheduled_vote_cast_in_progress: bool,
    pub selected_votes: Vec<SelectedVote>,
    pub app_context: Arc<AppContext>,
    pending_backend_task: Option<BackendTask>,

    /// Sorting
    sort_column: SortColumn,
    sort_order: SortOrder,
    active_filter_term: String,
    past_filter_term: String,
    owned_filter_term: String,

    /// Which sub-screen is active: Active contests, Past, Owned, or Scheduled
    pub dpns_subscreen: DPNSSubscreen,
    refreshing_status: RefreshingStatus,
    refresh_banner: Option<BannerHandle>,

    /// Selected vote handling
    show_bulk_schedule_popup: bool,
    bulk_identity_options: Vec<VoteOption>,
    bulk_schedule_message: Option<(MessageType, String)>,
    bulk_vote_handling_status: VoteHandlingStatus,
    vote_banner: Option<BannerHandle>,
    set_all_option: VoteOption,
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
                        (vote.clone(), ScheduledVoteCastingStatus::Completed)
                    } else {
                        (vote.clone(), ScheduledVoteCastingStatus::NotStarted)
                    }
                })
                .collect::<Vec<_>>(),
        ));

        let voting_identities = app_context
            .load_local_voting_identities()
            .unwrap_or_default();
        let user_identities = app_context.load_local_user_identities().unwrap_or_default();

        // Initialize vote handling pop-up state to hidden
        let identity_count = voting_identities.len();
        let bulk_identity_options = vec![VoteOption::CastNow; identity_count];

        Self {
            voting_identities,
            user_identities,
            contested_names,
            local_dpns_names,
            scheduled_votes: scheduled_votes_with_status,
            selected_votes: Vec::new(),
            app_context: app_context.clone(),
            sort_column: SortColumn::ContestedName,
            sort_order: SortOrder::Ascending,
            active_filter_term: String::new(),
            past_filter_term: String::new(),
            owned_filter_term: String::new(),
            scheduled_vote_cast_in_progress: false,
            pending_backend_task: None,
            dpns_subscreen,
            refreshing_status: RefreshingStatus::NotRefreshing,
            refresh_banner: None,

            // Vote handling
            show_bulk_schedule_popup: false,
            bulk_identity_options,
            bulk_schedule_message: None,
            bulk_vote_handling_status: VoteHandlingStatus::NotStarted,
            vote_banner: None,
            set_all_option: VoteOption::CastNow,
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

    fn sort_contested_names(&self, contested_names: &mut [ContestedName]) {
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
                let dark_mode = ui.style().visuals.dark_mode;
                ui.label(RichText::new("Please check back later or try refreshing the list.").color(DashColors::text_primary(dark_mode)));
                ui.add_space(20.0);
                if StyledButton::primary("Refresh").show(ui).clicked() {
                    if let RefreshingStatus::Refreshing = self.refreshing_status {
                        app_action = AppAction::None;
                    } else {
                        self.refreshing_status = RefreshingStatus::Refreshing;
                        match self.dpns_subscreen {
                            DPNSSubscreen::Active | DPNSSubscreen::Past => {
                                app_action = AppAction::BackendTask(BackendTask::ContestedResourceTask(
                                    ContestedResourceTask::QueryDPNSContests,
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
                let dark_mode = ui.style().visuals.dark_mode;
                let text_color = DashColors::text_primary(dark_mode);
                ui.label(
                    RichText::new("To schedule votes, go to the Active Contests subscreen, click your choices, and then click the 'Vote' button in the top-right.").color(text_color)
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
        ui.horizontal(|ui| {
            let dark_mode = ui.style().visuals.dark_mode;
            ui.label(RichText::new("Filter by name:").color(DashColors::text_primary(dark_mode)));
            ui.text_edit_singleline(&mut self.active_filter_term);
        });

        let contested_names = {
            let guard = self.contested_names.lock_recover();
            let mut cn = guard.clone();
            if !self.active_filter_term.is_empty() {
                let mut filter_lc = self.active_filter_term.to_lowercase();
                // Convert o and O to 0 and l to 1 in filter_lc
                filter_lc = filter_lc
                    .chars()
                    .map(|c| match c {
                        'o' | 'O' => '0',
                        'l' => '1',
                        _ => c,
                    })
                    .collect();
                cn.retain(|c| {
                    c.normalized_contested_name
                        .to_lowercase()
                        .contains(&filter_lc)
                });
            }
            self.sort_contested_names(&mut cn);
            cn
        };

        // Space allocation for UI elements is handled by the layout system

        egui::ScrollArea::both().show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(false)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().resizable(true)) // Contested Name
                .column(Column::auto().resizable(true)) // Locked
                .column(Column::auto().resizable(true)) // Abstain
                .column(Column::auto().resizable(true)) // Ending Time
                .column(Column::auto().resizable(true)) // Last Updated
                .column(Column::auto().resizable(true)) // Contestants
                .header(30.0, |mut header| {
                    header.col(|ui| {
                        if ui.button("Name").clicked() {
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
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.heading(
                            RichText::new("Contestants").color(DashColors::text_primary(dark_mode)),
                        );
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
                                let (used_name, highlighted) =
                                    if let Some(contestants) = &contested_name.contestants {
                                        if let Some(first) = contestants.first() {
                                            if contestants.iter().all(|c| c.name == first.name) {
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
                                            (contested_name.normalized_contested_name.clone(), None)
                                        }
                                    } else {
                                        (contested_name.normalized_contested_name.clone(), None)
                                    };

                                let dark_mode = ui.style().visuals.dark_mode;
                                let label_response = ui.label(
                                    RichText::new(used_name)
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                                if let Some(tooltip) = highlighted {
                                    label_response.info_tooltip(tooltip);
                                }
                            });

                            // LOCK button
                            row.col(|ui| {
                                let label_text = format!("{}", locked_votes);
                                let dark_green = Color32::from_rgb(0, 100, 0);
                                let dark_mode = ui.style().visuals.dark_mode;
                                let normal_color = DashColors::text_primary(dark_mode);
                                let text_widget = if is_locked_votes_bold {
                                    RichText::new(label_text).strong().color(dark_green)
                                } else {
                                    RichText::new(label_text).color(normal_color)
                                };

                                // See if this (LOCK) is selected
                                let is_selected = self.selected_votes.iter().any(|sv| {
                                    sv.contested_name == contested_name.normalized_contested_name
                                        && sv.vote_choice == ResourceVoteChoice::Lock
                                });

                                let button = if is_selected {
                                    Button::new(text_widget).fill(Color32::from_rgb(0, 150, 255))
                                } else {
                                    Button::new(text_widget)
                                };
                                let resp = ui.add(button);
                                if resp.clicked() {
                                    // Is there already a selection for this contested name?
                                    if let Some(existing_index) =
                                        self.selected_votes.iter().position(|sv| {
                                            sv.contested_name
                                                == contested_name.normalized_contested_name
                                        })
                                    {
                                        // If the user clicked the same choice, that toggles it off (unselect).
                                        if self.selected_votes[existing_index].vote_choice
                                            == ResourceVoteChoice::Lock
                                        {
                                            // Remove it entirely -> no selection
                                            self.selected_votes.remove(existing_index);
                                        } else {
                                            // Otherwise replace the old choice with Lock
                                            self.selected_votes[existing_index].vote_choice =
                                                ResourceVoteChoice::Lock;
                                        }
                                    } else {
                                        // No existing selection for this name, so add this new Lock
                                        self.selected_votes.push(SelectedVote {
                                            contested_name: contested_name
                                                .normalized_contested_name
                                                .clone(),
                                            vote_choice: ResourceVoteChoice::Lock,
                                            end_time: contested_name.end_time,
                                        });
                                    }
                                }
                            });

                            // ABSTAIN button
                            row.col(|ui| {
                                let abstain_votes = contested_name.abstain_votes.unwrap_or(0);
                                let label_text = format!("{}", abstain_votes);

                                let is_selected = self.selected_votes.iter().any(|sv| {
                                    sv.contested_name == contested_name.normalized_contested_name
                                        && sv.vote_choice == ResourceVoteChoice::Abstain
                                });

                                let button = if is_selected {
                                    Button::new(label_text).fill(Color32::from_rgb(0, 150, 255))
                                } else {
                                    Button::new(label_text)
                                };
                                let resp = ui.add(button);
                                if resp.clicked() {
                                    // Is there already a selection for this contested name?
                                    if let Some(existing_index) =
                                        self.selected_votes.iter().position(|sv| {
                                            sv.contested_name
                                                == contested_name.normalized_contested_name
                                        })
                                    {
                                        // If the user clicked the same choice, that toggles it off (unselect).
                                        if self.selected_votes[existing_index].vote_choice
                                            == ResourceVoteChoice::Abstain
                                        {
                                            // Remove it entirely -> no selection
                                            self.selected_votes.remove(existing_index);
                                        } else {
                                            // Otherwise replace the old choice with Abstain
                                            self.selected_votes[existing_index].vote_choice =
                                                ResourceVoteChoice::Abstain;
                                        }
                                    } else {
                                        // No existing selection for this name, so add this new Abstain
                                        self.selected_votes.push(SelectedVote {
                                            contested_name: contested_name
                                                .normalized_contested_name
                                                .clone(),
                                            vote_choice: ResourceVoteChoice::Abstain,
                                            end_time: contested_name.end_time,
                                        });
                                    }
                                }
                            });

                            // Ending Time
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                if let Some(ending_time) = contested_name.end_time {
                                    if let LocalResult::Single(dt) =
                                        Utc.timestamp_millis_opt(ending_time as i64)
                                    {
                                        let iso_date = dt.format("%Y-%m-%d %H:%M:%S");
                                        let relative_time = HumanTime::from(dt).to_string();
                                        let text = format!("{} ({})", iso_date, relative_time);
                                        ui.label(
                                            RichText::new(text)
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new("Invalid timestamp")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                } else {
                                    ui.label(
                                        RichText::new("Fetching")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                }
                            });

                            // Last Updated
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                if let Some(last_updated) = contested_name.last_updated {
                                    if let LocalResult::Single(dt) =
                                        Utc.timestamp_opt(last_updated as i64, 0)
                                    {
                                        let rel_time = HumanTime::from(dt).to_string();
                                        if rel_time.contains("seconds") {
                                            ui.label(
                                                RichText::new("now")
                                                    .color(DashColors::text_primary(dark_mode)),
                                            );
                                        } else {
                                            ui.label(
                                                RichText::new(rel_time)
                                                    .color(DashColors::text_primary(dark_mode)),
                                            );
                                        }
                                    } else {
                                        ui.label(
                                            RichText::new("Invalid timestamp")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                } else {
                                    ui.label(
                                        RichText::new("Fetching")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
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
    }

    /// Show a Past Contests table
    fn render_table_past_contests(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let dark_mode = ui.style().visuals.dark_mode;
            ui.label(RichText::new("Filter by name:").color(DashColors::text_primary(dark_mode)));
            ui.text_edit_singleline(&mut self.past_filter_term);
        });

        let contested_names = {
            let guard = self.contested_names.lock_recover();
            let mut cn = guard.clone();
            cn.retain(|c| c.awarded_to.is_some() || c.state == ContestState::Locked);
            // 1) Filter by `past_filter_term`
            if !self.past_filter_term.is_empty() {
                let mut filter_lc = self.past_filter_term.to_lowercase();
                // Convert o and O to 0 and l to 1 in filter_lc
                filter_lc = filter_lc
                    .chars()
                    .map(|c| match c {
                        'o' | 'O' => '0',
                        'l' => '1',
                        _ => c,
                    })
                    .collect();

                cn.retain(|c| {
                    c.normalized_contested_name
                        .to_lowercase()
                        .contains(&filter_lc)
                });
            }
            self.sort_contested_names(&mut cn);
            cn
        };

        // Allocate space for refreshing indicator
        // Space allocation for UI elements is handled by the layout system

        egui::ScrollArea::both().show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(false)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().resizable(true)) // Name
                .column(Column::auto().resizable(true)) // Ended Time
                .column(Column::auto().resizable(true)) // Last Updated
                .column(Column::auto().resizable(true)) // Awarded To
                .header(30.0, |mut header| {
                    header.col(|ui| {
                        if ui.button("Name").clicked() {
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
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.label(
                                    RichText::new(&contested_name.normalized_contested_name)
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            });
                            // Ended Time
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                if let Some(ended_time) = contested_name.end_time {
                                    if let LocalResult::Single(dt) =
                                        Utc.timestamp_millis_opt(ended_time as i64)
                                    {
                                        let iso = dt.format("%Y-%m-%d %H:%M:%S").to_string();
                                        let relative = HumanTime::from(dt).to_string();
                                        ui.label(
                                            RichText::new(format!("{} ({})", iso, relative))
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new("Invalid timestamp")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                } else {
                                    ui.label(
                                        RichText::new("Fetching")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                }
                            });
                            // Last Updated
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                if let Some(last_updated) = contested_name.last_updated {
                                    if let LocalResult::Single(dt) =
                                        Utc.timestamp_opt(last_updated as i64, 0)
                                    {
                                        let rel = HumanTime::from(dt).to_string();
                                        if rel.contains("seconds") {
                                            ui.label(
                                                RichText::new("now")
                                                    .color(DashColors::text_primary(dark_mode)),
                                            );
                                        } else {
                                            ui.label(
                                                RichText::new(rel)
                                                    .color(DashColors::text_primary(dark_mode)),
                                            );
                                        }
                                    } else {
                                        ui.label(
                                            RichText::new("Invalid timestamp")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                } else {
                                    ui.label(
                                        RichText::new("Fetching")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                }
                            });
                            // Awarded To
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                match contested_name.state {
                                    ContestState::Unknown => {
                                        ui.label(
                                            RichText::new("Fetching")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                    ContestState::Joinable | ContestState::Ongoing => {
                                        ui.label(
                                            RichText::new("Active")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                    ContestState::WonBy(identifier) => {
                                        ui.add(
                                            egui::Label::new(
                                                identifier.to_string(Encoding::Base58),
                                            )
                                            .sense(egui::Sense::hover())
                                            .truncate(),
                                        );
                                    }
                                    ContestState::Locked => {
                                        ui.label(
                                            RichText::new("Locked")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                }
                            });
                        });
                    }
                });
        });
    }

    /// Show the Owned DPNS names table
    fn render_table_local_dpns_names(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let dark_mode = ui.style().visuals.dark_mode;
            ui.label(RichText::new("Filter by name:").color(DashColors::text_primary(dark_mode)));
            ui.text_edit_singleline(&mut self.owned_filter_term);
        });

        let mut filtered_names = {
            let guard = self.local_dpns_names.lock_recover();
            let mut name_infos = guard.clone();
            if !self.owned_filter_term.is_empty() {
                let filter_lc = self.owned_filter_term.to_lowercase();
                name_infos.retain(|c| c.1.name.to_lowercase().contains(&filter_lc));
            }
            name_infos
        };
        // Sort
        filtered_names.sort_by(|a, b| match self.sort_column {
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

        // Space allocation for UI elements is handled by the layout system

        egui::ScrollArea::both().show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(false)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().resizable(true)) // DPNS Name
                .column(Column::auto().resizable(true)) // Owner ID
                .column(Column::auto().resizable(true)) // Acquired At
                .column(Column::auto().resizable(true)) // Actions
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
                    header.col(|ui| {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.label(
                            RichText::new("Actions").color(DashColors::text_primary(dark_mode)),
                        );
                    });
                })
                .body(|mut body| {
                    for (identifier, dpns_info) in filtered_names {
                        let name_for_alias = dpns_info.name.clone();
                        // Display name with .dash suffix
                        let display_name = if name_for_alias.ends_with(".dash") {
                            name_for_alias.clone()
                        } else {
                            format!("{}.dash", name_for_alias)
                        };
                        body.row(25.0, |mut row| {
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.label(
                                    RichText::new(&display_name)
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            });
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.label(
                                    RichText::new(identifier.to_string(Encoding::Base58))
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            });
                            let dt = DateTime::from_timestamp(
                                dpns_info.acquired_at as i64 / 1000,
                                ((dpns_info.acquired_at % 1000) * 1_000_000) as u32,
                            )
                            .map(|dt| dt.to_string())
                            .unwrap_or_else(|| "Invalid timestamp".to_string());
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.label(
                                    RichText::new(dt).color(DashColors::text_primary(dark_mode)),
                                );
                            });
                            row.col(|ui| {
                                if ui.small_button("Set Alias").clicked() {
                                    // Append .dash suffix for DPNS names
                                    let alias_with_suffix = if name_for_alias.ends_with(".dash") {
                                        name_for_alias.clone()
                                    } else {
                                        format!("{}.dash", name_for_alias)
                                    };
                                    if let Err(e) = self
                                        .app_context
                                        .set_identity_alias(&identifier, Some(&alias_with_suffix))
                                    {
                                        MessageBanner::set_global(
                                            ui.ctx(),
                                            format!("Failed to set alias: {}", e),
                                            MessageType::Error,
                                        );
                                    } else {
                                        MessageBanner::set_global(
                                            ui.ctx(),
                                            format!(
                                                "Alias set to '{}' for identity {}",
                                                alias_with_suffix,
                                                identifier.to_string(Encoding::Base58)
                                            ),
                                            MessageType::Success,
                                        );
                                    }
                                }
                            });
                        });
                    }
                });
        });
    }

    /// Show the Scheduled Votes table
    fn render_table_scheduled_votes(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut sorted_votes = {
            let guard = self.scheduled_votes.lock_recover();
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

        egui::ScrollArea::both().show(ui, |ui| {
            TableBuilder::new(ui)
                .striped(false)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::auto().resizable(true)) // ContestedName
                .column(Column::auto().resizable(true)) // Voter
                .column(Column::auto().resizable(true)) // Choice
                .column(Column::auto().resizable(true)) // Time
                .column(Column::auto().resizable(true)) // Status
                .column(Column::auto().resizable(true)) // Actions
                .header(30.0, |mut header| {
                    header.col(|ui| {
                        if ui.button("Name").clicked() {
                            self.toggle_sort(SortColumn::ContestedName);
                        }
                    });
                    header.col(|ui| {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.heading(
                            RichText::new("Voter").color(DashColors::text_primary(dark_mode)),
                        );
                    });
                    header.col(|ui| {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.heading(
                            RichText::new("Vote Choice").color(DashColors::text_primary(dark_mode)),
                        );
                    });
                    header.col(|ui| {
                        if ui.button("Scheduled Time").clicked() {
                            self.toggle_sort(SortColumn::EndingTime);
                        }
                    });
                    header.col(|ui| {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.heading(
                            RichText::new("Status").color(DashColors::text_primary(dark_mode)),
                        );
                    });
                    header.col(|ui| {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.heading(
                            RichText::new("Actions").color(DashColors::text_primary(dark_mode)),
                        );
                    });
                })
                .body(|mut body| {
                    for vote in sorted_votes.iter_mut() {
                        let operation_status = self
                            .app_context
                            .dpns_vote_poll_id(&vote.0.contested_name)
                            .ok()
                            .and_then(|vote_poll_id| {
                                self.app_context
                                    .dpns_vote_target_status(&DpnsVoteTargetKey {
                                        network: self.app_context.network(),
                                        voter_id: vote.0.voter_id,
                                        vote_poll_id,
                                    })
                                    .ok()
                                    .flatten()
                            });
                        body.row(25.0, |mut row| {
                            // Contested name
                            row.col(|ui| {
                                ui.add(Label::new(&vote.0.contested_name));
                            });
                            // Voter
                            row.col(|ui| {
                                ui.add(
                                    Label::new(vote.0.voter_id.to_string(Encoding::Hex)).truncate(),
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
                                let dark_mode = ui.style().visuals.dark_mode;
                                if let LocalResult::Single(dt) =
                                    Utc.timestamp_millis_opt(vote.0.unix_timestamp as i64)
                                {
                                    let iso = dt.format("%Y-%m-%d %H:%M:%S").to_string();
                                    let rel_time = HumanTime::from(dt).to_string();
                                    let relative = if rel_time.contains("seconds") {
                                        "now".to_string()
                                    } else {
                                        rel_time
                                    };
                                    let text = format!("{} ({})", iso, relative);
                                    ui.label(
                                        RichText::new(text)
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                } else {
                                    ui.label(
                                        RichText::new("Invalid timestamp")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                }
                            });
                            // Status
                            row.col(|ui| {
                                let dark_mode = ui.style().visuals.dark_mode;
                                if matches!(
                                    operation_status,
                                    Some(DpnsVoteTargetStatus::Queued)
                                        | Some(DpnsVoteTargetStatus::Submitting)
                                        | Some(DpnsVoteTargetStatus::Confirming)
                                ) {
                                    ui.label(
                                        RichText::new("Submitting…")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                    return;
                                }
                                if operation_status == Some(DpnsVoteTargetStatus::Unconfirmed) {
                                    ui.colored_label(
                                        DashColors::warning_color(dark_mode),
                                        "Checking result",
                                    );
                                    return;
                                }
                                match vote.1 {
                                    ScheduledVoteCastingStatus::NotStarted => {
                                        ui.label(
                                            RichText::new("Pending")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                    ScheduledVoteCastingStatus::InProgress => {
                                        ui.label(
                                            RichText::new("Casting...")
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                    }
                                    ScheduledVoteCastingStatus::Failed => {
                                        ui.colored_label(Color32::DARK_RED, "Failed");
                                    }
                                    ScheduledVoteCastingStatus::Completed => {
                                        ui.colored_label(Color32::DARK_GREEN, "Casted");
                                    }
                                }
                            });
                            // Actions
                            row.col(|ui| {
                                let target_is_busy = matches!(
                                    operation_status,
                                    Some(DpnsVoteTargetStatus::Queued)
                                        | Some(DpnsVoteTargetStatus::Submitting)
                                        | Some(DpnsVoteTargetStatus::Confirming)
                                        | Some(DpnsVoteTargetStatus::Unconfirmed)
                                );
                                if ui
                                    .add_enabled(!target_is_busy, Button::new("Remove"))
                                    .disabled_tooltip(
                                        "This scheduled vote cannot be removed while its result is being checked.",
                                    )
                                    .clicked()
                                {
                                    action =
                                        AppAction::BackendTask(BackendTask::ContestedResourceTask(
                                            ContestedResourceTask::DeleteScheduledVote(
                                                vote.0.voter_id,
                                                vote.0.contested_name.clone(),
                                            ),
                                        ));
                                }
                                // If the user wants to do "Cast Now" from here, they can
                                // if NotStarted or Failed. If in progress or done, disabled.
                                let cast_button_enabled = matches!(
                                    vote.1,
                                    ScheduledVoteCastingStatus::NotStarted
                                        | ScheduledVoteCastingStatus::Failed
                                ) && !target_is_busy;

                                let cast_button = if cast_button_enabled {
                                    Button::new("Cast Now")
                                } else {
                                    Button::new("Cast Now").sense(egui::Sense::hover())
                                };

                                if ui.add(cast_button).clicked() && cast_button_enabled {
                                    self.scheduled_vote_cast_in_progress = true;
                                    vote.1 = ScheduledVoteCastingStatus::InProgress;

                                    // Mark in our Arc as well
                                    if let Ok(mut sched_guard) = self.scheduled_votes.lock()
                                        && let Some(t) = sched_guard.iter_mut().find(|(sv, _)| {
                                            sv.voter_id == vote.0.voter_id
                                                && sv.contested_name == vote.0.contested_name
                                        })
                                    {
                                        t.1 = ScheduledVoteCastingStatus::InProgress;
                                    }
                                    // dispatch the actual cast
                                    let local_ids =
                                        match self.app_context.load_local_voting_identities() {
                                            Ok(ids) => ids,
                                            Err(e) => {
                                                error!("{}", e);
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
                                                    Box::new(found.clone()),
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
                    // Is there already a selection for this contested name?
                    if let Some(existing_index) = self.selected_votes.iter().position(|sv| {
                        sv.contested_name == contested_name.normalized_contested_name
                    }) {
                        // If the user clicked the same choice, that toggles it off (unselect).
                        if self.selected_votes[existing_index].vote_choice
                            == ResourceVoteChoice::TowardsIdentity(contestant.id)
                        {
                            // Remove it entirely -> no selection
                            self.selected_votes.remove(existing_index);
                        } else {
                            // Otherwise replace the old choice with TowardsIdentity
                            self.selected_votes[existing_index].vote_choice =
                                ResourceVoteChoice::TowardsIdentity(contestant.id);
                        }
                    } else {
                        // No existing selection for this name, so add this new TowardsIdentity
                        self.selected_votes.push(SelectedVote {
                            contested_name: contested_name.normalized_contested_name.clone(),
                            vote_choice: ResourceVoteChoice::TowardsIdentity(contestant.id),
                            end_time: contested_name.end_time,
                        });
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

        let dark_mode = ui.style().visuals.dark_mode;
        ui.heading(
            RichText::new("Cast or Schedule Votes").color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(10.0);

        // If self.bulk_vote_handling_status is Complete, show completed message
        if self.bulk_vote_handling_status == VoteHandlingStatus::Completed {
            action |= self.show_bulk_vote_handling_complete(ui);
            return action;
        }

        // If no voting identities are loaded, display a message and return
        if self.voting_identities.is_empty() {
            ui.add_space(5.0);
            ui.colored_label(Color32::DARK_RED, "No masternode identities loaded. Please go to the Identities screen to load your masternodes.");
            ui.add_space(10.0);
            let dark_mode = ui.style().visuals.dark_mode;
            if ComponentStyles::add_secondary_button(ui, "Close", dark_mode).clicked() {
                self.show_bulk_schedule_popup = false;
            }
            return action;
        }

        // If no votes are selected, display a message and return
        if self.selected_votes.is_empty() {
            ui.add_space(5.0);
            ui.colored_label(Color32::DARK_RED, "No votes selected. Please click the votes you want to cast or schedule in the Active Contests screen.");
            ui.add_space(10.0);
            let dark_mode = ui.style().visuals.dark_mode;
            if ComponentStyles::add_secondary_button(ui, "Close", dark_mode).clicked() {
                self.show_bulk_schedule_popup = false;
            }
            return action;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            // Show which votes were clicked
            ui.group(|ui| {
                let dark_mode = ui.style().visuals.dark_mode;
                ui.heading(
                    RichText::new("Selected Votes:").color(DashColors::text_primary(dark_mode)),
                );
                ui.separator();
                for sv in &self.selected_votes {
                    // Convert end_time -> readable
                    let end_str = if let Some(e) = sv.end_time {
                        if let LocalResult::Single(dt) = Utc.timestamp_millis_opt(e as i64) {
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
                        ResourceVoteChoice::TowardsIdentity(id) => id.to_string(Encoding::Base58),
                        other => other.to_string(),
                    };
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.label(
                        RichText::new(format!(
                            "{}   =>   {}   |   Contest ends at {}",
                            sv.contested_name, display_text, end_str
                        ))
                        .color(DashColors::text_primary(dark_mode)),
                    );
                }
            });

            ui.add_space(10.0);

            // Show each identity + let user pick None / Immediate / Scheduled
            let dark_mode = ui.style().visuals.dark_mode;
            ui.heading(
                RichText::new("Select cast method for each node:")
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.add_space(10.0);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.label(RichText::new("Set all:").color(DashColors::text_primary(dark_mode)));

                    // A ComboBox to pick No Vote / Cast Now / Schedule
                    ComboBox::from_id_salt("set_all_combo")
                        .width(120.0)
                        .selected_text(match self.set_all_option {
                            VoteOption::NoVote => "No Vote".to_string(),
                            VoteOption::CastNow => "Cast Now".to_string(),
                            VoteOption::Scheduled { .. } => "Schedule".to_string(),
                        })
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(
                                    matches!(self.set_all_option, VoteOption::NoVote),
                                    "No Vote",
                                )
                                .clicked()
                            {
                                self.set_all_option = VoteOption::NoVote;
                            }
                            if ui
                                .selectable_label(
                                    matches!(self.set_all_option, VoteOption::CastNow),
                                    "Cast Now",
                                )
                                .clicked()
                            {
                                self.set_all_option = VoteOption::CastNow;
                            }
                            if ui
                                .selectable_label(
                                    matches!(self.set_all_option, VoteOption::Scheduled { .. }),
                                    "Schedule",
                                )
                                .clicked()
                            {
                                // Default scheduled values if none set yet
                                let (d, h, m) = match &self.set_all_option {
                                    VoteOption::Scheduled {
                                        days,
                                        hours,
                                        minutes,
                                    } => (*days, *hours, *minutes),
                                    _ => (0, 0, 0),
                                };
                                self.set_all_option = VoteOption::Scheduled {
                                    days: d,
                                    hours: h,
                                    minutes: m,
                                };
                            }
                        });

                    // If scheduling, show the days/hours/minutes widgets inline
                    if let VoteOption::Scheduled {
                        ref mut days,
                        ref mut hours,
                        ref mut minutes,
                    } = self.set_all_option
                    {
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.label(
                            RichText::new("Schedule In:")
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.add(egui::DragValue::new(days).prefix("Days: ").range(0..=14));
                        ui.add(egui::DragValue::new(hours).prefix("Hours: ").range(0..=23));
                        ui.add(egui::DragValue::new(minutes).prefix("Min: ").range(0..=59));
                    }

                    // Button to apply the "Set all" choice to each identity in bulk_identity_options
                    if ui.button("Apply to All").clicked() {
                        for option in &mut self.bulk_identity_options {
                            *option = self.set_all_option.clone();
                        }
                    }
                });
            });
            ui.add_space(10.0);
            for (i, identity) in self.voting_identities.iter().enumerate() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        let label = identity
                            .alias
                            .clone()
                            .unwrap_or_else(|| identity.identity.id().to_string(Encoding::Base58));
                        let dark_mode = ui.style().visuals.dark_mode;
                        ui.label(
                            RichText::new(format!("Identity: {}", label))
                                .color(DashColors::text_primary(dark_mode)),
                        );

                        // This is a hack
                        // I'm seeing a panic if I load the app in mainnet context where I have no voting identities,
                        // and then switch to testnet and pressed "Vote".
                        if self.bulk_identity_options.len() <= i {
                            let voting_identities = self
                                .app_context
                                .load_local_voting_identities()
                                .unwrap_or_default();
                            // Initialize ephemeral bulk-schedule state to hidden
                            let identity_count = voting_identities.len();
                            self.bulk_identity_options = vec![VoteOption::CastNow; identity_count];
                        }

                        let current_option = &mut self.bulk_identity_options[i];
                        ComboBox::from_id_salt(format!("combo_bulk_identity_{}", i))
                            .width(120.0)
                            .selected_text(match current_option {
                                VoteOption::NoVote => "No Vote".to_string(),
                                VoteOption::CastNow => "Cast Now".to_string(),
                                VoteOption::Scheduled { .. } => "Schedule".to_string(),
                            })
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        matches!(current_option, VoteOption::NoVote),
                                        "No Vote",
                                    )
                                    .clicked()
                                {
                                    *current_option = VoteOption::NoVote;
                                }
                                if ui
                                    .selectable_label(
                                        matches!(current_option, VoteOption::CastNow),
                                        "Cast Now",
                                    )
                                    .clicked()
                                {
                                    *current_option = VoteOption::CastNow;
                                }
                                if ui
                                    .selectable_label(
                                        matches!(current_option, VoteOption::Scheduled { .. }),
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
                            let dark_mode = ui.style().visuals.dark_mode;
                            ui.label(
                                RichText::new("Schedule In:")
                                    .color(DashColors::text_primary(dark_mode)),
                            );
                            ui.add(egui::DragValue::new(days).prefix("Days: ").range(0..=14));
                            ui.add(egui::DragValue::new(hours).prefix("Hours: ").range(0..=23));
                            ui.add(egui::DragValue::new(minutes).prefix("Min: ").range(0..=59));
                        }
                    });
                });
                ui.add_space(10.0);
            }
        });

        // If any selected votes are scheduled, show a warning
        if self
            .bulk_identity_options
            .iter()
            .any(|o| matches!(o, VoteOption::Scheduled { .. }))
        {
            ui.colored_label(Color32::DARK_RED, "NOTE: Dash Evo Tool must remain running and connected for scheduled votes to execute on time.");
            ui.add_space(10.0);
        }

        let operation_in_progress = matches!(
            self.bulk_vote_handling_status,
            VoteHandlingStatus::CastingVotes | VoteHandlingStatus::SchedulingVotes
        );
        if operation_in_progress {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Submitting votes…");
            });
        }
        // "Apply Votes" button
        if ComponentStyles::add_primary_button_enabled(
            ui,
            !operation_in_progress,
            if operation_in_progress {
                "Submitting votes…"
            } else {
                "Apply Votes"
            },
        )
        .disabled_tooltip("The selected votes are already being submitted.")
        .clicked()
        {
            action = self.bulk_apply_votes();
            if self.bulk_vote_handling_status == VoteHandlingStatus::CastingVotes {
                self.vote_banner.take_and_clear();
                let handle =
                    MessageBanner::set_global(ui.ctx(), "Casting votes...", MessageType::Info);
                handle.with_elapsed();
                self.vote_banner = Some(handle);
            }
        }

        ui.add_space(5.0);
        let dark_mode = ui.style().visuals.dark_mode;
        if ui
            .add_enabled(
                !operation_in_progress,
                ComponentStyles::secondary_button("Cancel", dark_mode),
            )
            .disabled_tooltip("Submitted votes cannot be cancelled.")
            .clicked()
        {
            self.selected_votes.clear();
            self.show_bulk_schedule_popup = false;
            self.bulk_schedule_message = None;
            self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
            self.vote_banner.take_and_clear();
        }

        // Handle status
        ui.add_space(10.0);
        match &self.bulk_vote_handling_status {
            VoteHandlingStatus::NotStarted => {}
            VoteHandlingStatus::CastingVotes => {
                // Elapsed time is shown in the global banner
            }
            VoteHandlingStatus::SchedulingVotes => {
                let dark_mode = ui.style().visuals.dark_mode;
                ui.label(
                    RichText::new("Scheduling votes...").color(DashColors::text_primary(dark_mode)),
                );
            }
            VoteHandlingStatus::Completed => {
                // handled above
            }
            VoteHandlingStatus::Failed(message) => {
                ui.colored_label(
                    Color32::RED,
                    format!("Error casting/scheduling votes: {}", message),
                );
            }
        }

        action
    }

    /// The logic that was in BulkScheduleVoteScreen::schedule_votes
    fn bulk_apply_votes(&mut self) -> AppAction {
        let mut targets = Vec::new();
        let mut selected_voters = Vec::new();
        let mut has_immediate = false;
        for (identity, option) in self
            .voting_identities
            .iter()
            .zip(&self.bulk_identity_options)
        {
            let timing = match option {
                VoteOption::NoVote => continue,
                VoteOption::CastNow => {
                    has_immediate = true;
                    VoteTiming::Now
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
                    VoteTiming::Scheduled((now + offset).timestamp_millis() as u64)
                }
            };
            selected_voters.push(identity.clone());
            for selected_vote in &self.selected_votes {
                let voter_id = identity.identity.id();
                let vote_poll_id = match self
                    .app_context
                    .dpns_vote_poll_id(&selected_vote.contested_name)
                {
                    Ok(vote_poll_id) => vote_poll_id,
                    Err(error) => {
                        self.bulk_vote_handling_status =
                            VoteHandlingStatus::Failed(error.to_string());
                        return AppAction::None;
                    }
                };
                let target_key = DpnsVoteTargetKey {
                    network: self.app_context.network(),
                    voter_id,
                    vote_poll_id,
                };
                if self
                    .app_context
                    .dpns_vote_target_status(&target_key)
                    .ok()
                    .flatten()
                    .is_some()
                {
                    self.bulk_vote_handling_status = VoteHandlingStatus::Failed(format!(
                        "This node's vote for {} is already in progress. Check its result before submitting again.",
                        selected_vote.contested_name
                    ));
                    return AppAction::None;
                }
                let current_choice = match self
                    .app_context
                    .dpns_current_vote_state(voter_id, vote_poll_id)
                {
                    Ok(DpnsCurrentVoteState::Available(choice)) => choice,
                    Ok(DpnsCurrentVoteState::Checking | DpnsCurrentVoteState::Unavailable)
                    | Err(_) => {
                        self.bulk_vote_handling_status = VoteHandlingStatus::Failed(
                            "Current vote state is unavailable. Refresh voting before applying votes."
                                .to_owned(),
                        );
                        return AppAction::None;
                    }
                };
                targets.push(DpnsVoteTarget {
                    key: target_key,
                    voter_alias: identity.alias.clone(),
                    contested_name: selected_vote.contested_name.clone(),
                    requested_choice: selected_vote.vote_choice,
                    current_choice,
                    timing,
                });
            }
        }

        if targets.is_empty() {
            self.bulk_vote_handling_status = VoteHandlingStatus::Failed(
                "No votes selected. Choose at least one node and contest.".to_owned(),
            );
            return AppAction::None;
        }
        let operation = DpnsVoteOperation::new(targets);
        if operation.targets.is_empty() {
            self.bulk_vote_handling_status = VoteHandlingStatus::Failed(
                "Every selected node already has the requested vote. Nothing will be submitted."
                    .to_owned(),
            );
            return AppAction::None;
        }
        self.bulk_vote_handling_status = if has_immediate {
            VoteHandlingStatus::CastingVotes
        } else {
            VoteHandlingStatus::SchedulingVotes
        };
        AppAction::BackendTask(BackendTask::ContestedResourceTask(
            ContestedResourceTask::SubmitDpnsVoteOperation(operation, selected_voters),
        ))
    }

    /// If voting/scheduling is successful, show success message
    fn show_bulk_vote_handling_complete(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        self.selected_votes.clear();

        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            match &self.bulk_vote_handling_status {
                VoteHandlingStatus::Completed => {
                    // This means DET side was successful, but Platform may have returned errors
                    if let Some(message) = &self.bulk_schedule_message {
                        match message.0 {
                            MessageType::Error => {
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.heading(
                                    RichText::new("❌").color(DashColors::text_primary(dark_mode)),
                                );
                                if message.1.contains("Successes") {
                                    let dark_mode = ui.style().visuals.dark_mode;
                                    ui.heading(
                                        RichText::new("Only some votes succeeded")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                } else {
                                    let dark_mode = ui.style().visuals.dark_mode;
                                    ui.heading(
                                        RichText::new("No votes succeeded")
                                            .color(DashColors::text_primary(dark_mode)),
                                    );
                                }
                                ui.add_space(10.0);
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.label(
                                    RichText::new(message.1.clone())
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            }
                            MessageType::Success => {
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.heading(
                                    RichText::new("🎉").color(DashColors::text_primary(dark_mode)),
                                );
                                let dark_mode = ui.style().visuals.dark_mode;
                                ui.heading(
                                    RichText::new("Successfully casted and scheduled all votes")
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            }
                            _ => {}
                        }
                    }
                }
                VoteHandlingStatus::Failed(message) => {
                    // This means there was a DET-side error, not Platform-side
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.heading(RichText::new("❌").color(DashColors::text_primary(dark_mode)));
                    ui.heading(
                        RichText::new("Error casting and scheduling votes (DET-side)")
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(10.0);
                    ui.label(RichText::new(message).color(DashColors::text_primary(dark_mode)));
                }
                _ => {
                    // this should not occur
                }
            }

            ui.add_space(20.0);
            let dark_mode = ui.style().visuals.dark_mode;
            if ComponentStyles::add_primary_button(ui, "Go back to Active Contests").clicked() {
                self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
                self.show_bulk_schedule_popup = false;
                action = AppAction::BackendTask(BackendTask::ContestedResourceTask(
                    ContestedResourceTask::QueryDPNSContests,
                ))
            }
            ui.add_space(5.0);
            if ComponentStyles::add_secondary_button(ui, "Go to Scheduled Votes Screen", dark_mode)
                .clicked()
            {
                self.show_bulk_schedule_popup = false;
                self.bulk_vote_handling_status = VoteHandlingStatus::NotStarted;
                action = AppAction::SetMainScreenThenPopScreen(
                    RootScreenType::RootScreenDPNSScheduledVotes,
                );
            }
        });

        action
    }
}

// ---------------------------
// ScreenLike implementation
// ---------------------------
impl ScreenLike for DPNSScreen {
    fn refresh(&mut self) {
        self.scheduled_vote_cast_in_progress = false;
        let mut contested_names = self.contested_names.lock_recover();
        let mut dpns_names = self.local_dpns_names.lock_recover();
        let mut scheduled_votes = self.scheduled_votes.lock_recover();

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
                            (newv.clone(), ScheduledVoteCastingStatus::Completed)
                        } else if let Some(existing) = scheduled_votes.iter().find(|(old, _)| {
                            old.contested_name == newv.contested_name
                                && old.voter_id == newv.voter_id
                        }) {
                            // preserve old status if InProgress/Failed
                            match existing.1 {
                                ScheduledVoteCastingStatus::InProgress => {
                                    (newv.clone(), ScheduledVoteCastingStatus::InProgress)
                                }
                                ScheduledVoteCastingStatus::Failed => {
                                    (newv.clone(), ScheduledVoteCastingStatus::Failed)
                                }
                                _ => (newv.clone(), ScheduledVoteCastingStatus::NotStarted),
                            }
                        } else {
                            (newv.clone(), ScheduledVoteCastingStatus::NotStarted)
                        }
                    })
                    .collect();
            }
        }
    }

    fn refresh_on_arrival(&mut self) {
        self.voting_identities = self
            .app_context
            .load_local_voting_identities()
            .unwrap_or_default();
        self.user_identities = self
            .app_context
            .load_local_user_identities()
            .unwrap_or_default();
        self.refresh();
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // Banner display is handled globally by AppState; this is only for side-effects.
        if matches!(message_type, MessageType::Error | MessageType::Warning) {
            self.refresh_banner.take_and_clear();
            self.vote_banner.take_and_clear();
        }
    }

    fn display_task_error(&mut self, error: &TaskError) -> bool {
        let handled = scheduled_vote_sweep_is_quiet(error);
        if matches!(
            error,
            TaskError::ScheduledVoteRejected { .. }
                | TaskError::ScheduledVoteResultUnavailable
                | TaskError::ScheduledVoteSweepFailed { .. }
        ) {
            self.scheduled_vote_cast_in_progress = false;
            if let Ok(mut guard) = self.scheduled_votes.lock() {
                for vote in guard.iter_mut() {
                    if vote.1 == ScheduledVoteCastingStatus::InProgress {
                        vote.1 = ScheduledVoteCastingStatus::Failed;
                    }
                }
            }
        }
        handled
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::DpnsVoteOperationUpdated(_) => {
                self.vote_banner.take_and_clear();
                self.bulk_vote_handling_status = VoteHandlingStatus::Completed;
                self.refresh();
            }
            BackendTaskSuccessResult::ScheduledVotesInProgress(votes) => {
                // The periodic sweep is about to cast these votes; reflect that
                // in the list so the user sees them move before results land.
                self.scheduled_vote_cast_in_progress = true;
                if let Ok(mut guard) = self.scheduled_votes.lock() {
                    for vote in &votes {
                        if let Some((_, status)) = guard.iter_mut().find(|(v, _)| {
                            v.contested_name == vote.contested_name && v.voter_id == vote.voter_id
                        }) {
                            *status = ScheduledVoteCastingStatus::InProgress;
                        }
                    }
                }
            }
            BackendTaskSuccessResult::RefreshedDpnsContests
            | BackendTaskSuccessResult::RefreshedOwnedDpnsNames => {
                self.refresh_banner.take_and_clear();
                self.refreshing_status = RefreshingStatus::NotRefreshing;
            }
            _ => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        if self.dpns_subscreen == DPNSSubscreen::ScheduledVotes {
            self.app_context
                .route_to_dpns_operator(DpnsOperatorRoute::Scheduled);
            return AppAction::SetMainScreen(RootScreenType::RootScreenMasternodes);
        }
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let has_identity_that_can_register = !self.user_identities.is_empty();
        let has_active_contests = {
            let guard = self.contested_names.lock_recover();
            !guard.is_empty()
        };

        // Build top-right buttons
        let mut right_buttons = match self.dpns_subscreen {
            DPNSSubscreen::Active => {
                let refresh_button = (
                    "Refresh",
                    DesiredAppAction::BackendTask(Box::new(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::QueryDPNSContests,
                    ))),
                );
                if has_active_contests {
                    vec![
                        refresh_button,
                        (
                            "Vote with masternodes",
                            DesiredAppAction::Custom("Vote".to_string()),
                        ),
                    ]
                } else {
                    vec![refresh_button]
                }
            }
            DPNSSubscreen::Past => {
                let refresh_button = (
                    "Refresh",
                    DesiredAppAction::BackendTask(Box::new(BackendTask::ContestedResourceTask(
                        ContestedResourceTask::QueryDPNSContests,
                    ))),
                );
                vec![refresh_button]
            }
            DPNSSubscreen::Owned => {
                let refresh_button = (
                    "Refresh",
                    DesiredAppAction::BackendTask(Box::new(BackendTask::IdentityTask(
                        IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
                    ))),
                );
                vec![refresh_button]
            }
            DPNSSubscreen::ScheduledVotes => {
                vec![
                    (
                        "Clear All",
                        DesiredAppAction::BackendTask(Box::new(
                            BackendTask::ContestedResourceTask(
                                ContestedResourceTask::ClearAllScheduledVotes,
                            ),
                        )),
                    ),
                    (
                        "Clear Casted",
                        DesiredAppAction::BackendTask(Box::new(
                            BackendTask::ContestedResourceTask(
                                ContestedResourceTask::ClearExecutedScheduledVotes,
                            ),
                        )),
                    ),
                ]
            }
        };

        if has_identity_that_can_register && self.dpns_subscreen != DPNSSubscreen::ScheduledVotes {
            // "Register Name" button on the left
            right_buttons.insert(
                0,
                (
                    "Register Name",
                    DesiredAppAction::AddScreenType(Box::new(ScreenType::RegisterDpnsName(
                        RegisterDpnsNameSource::Dpns,
                    ))),
                ),
            );
        }

        // TODO: wire wallet/identity selection consumption for the DPNS page.
        let mut action = add_top_panel_with_global_nav(
            ui,
            &self.app_context,
            subdued_everyday_spec("DPNS", RootScreenType::RootScreenDPNSActiveContests),
            right_buttons,
        );

        // If user clicked "Apply Votes" in the top bar
        if action == AppAction::Custom("Vote".to_string()) {
            self.app_context
                .route_to_dpns_operator(DpnsOperatorRoute::Voting {
                    choices: self
                        .selected_votes
                        .iter()
                        .map(|vote| (vote.contested_name.clone(), vote.vote_choice))
                        .collect(),
                });
            action = AppAction::SetMainScreen(RootScreenType::RootScreenMasternodes);
        }

        // Left panel
        action |= add_left_panel(
            ui,
            &self.app_context,
            RootScreenType::RootScreenToolsPlatformInfoScreen,
        );

        // Tools area chooser
        action |= add_tools_subscreen_chooser_panel(ui, self.app_context.as_ref());

        // DPNS subscreen chooser
        action |= add_dpns_subscreen_chooser_panel(ui, self.app_context.as_ref());

        // Main panel
        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;
            // Bulk-schedule ephemeral popup
            if self.show_bulk_schedule_popup {
                egui::Window::new("Voting")
                    .collapsible(false)
                    .resizable(true)
                    .vscroll(true)
                    .show(ui.ctx(), |ui| {
                        inner_action |= self.show_bulk_schedule_popup_window(ui);
                    });
            }

            // Render sub-screen
            match self.dpns_subscreen {
                DPNSSubscreen::Active => {
                    let has_any = {
                        let guard = self.contested_names.lock_recover();
                        !guard.is_empty()
                    };
                    if has_any {
                        self.render_table_active_contests(ui);
                    } else {
                        inner_action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::Past => {
                    let has_any = {
                        let guard = self.contested_names.lock_recover();
                        !guard.is_empty()
                    };
                    if has_any {
                        self.render_table_past_contests(ui);
                    } else {
                        inner_action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::Owned => {
                    let has_any = {
                        let guard = self.local_dpns_names.lock_recover();
                        !guard.is_empty()
                    };
                    if has_any {
                        self.render_table_local_dpns_names(ui);
                    } else {
                        inner_action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
                DPNSSubscreen::ScheduledVotes => {
                    let has_any = {
                        let guard = self.scheduled_votes.lock_recover();
                        !guard.is_empty()
                    };
                    if has_any {
                        inner_action |= self.render_table_scheduled_votes(ui);
                    } else {
                        inner_action |= self.render_no_active_contests_or_owned_names(ui);
                    }
                }
            }

            // Refreshing indicator is shown via the global banner
            // (no inline elapsed rendering needed)
            inner_action
        });

        // Extra handling for actions
        match action {
            // If refreshing contested names, set self.refreshing = true
            AppAction::BackendTask(BackendTask::ContestedResourceTask(
                ContestedResourceTask::QueryDPNSContests,
            )) => {
                self.refresh_banner.take_and_clear();
                let handle = MessageBanner::set_global(
                    ctx,
                    "Refreshing contested names...",
                    MessageType::Info,
                );
                handle.with_elapsed();
                self.refresh_banner = Some(handle);
                self.refreshing_status = RefreshingStatus::Refreshing;
            }
            // If refreshing owned names, set self.refreshing = true
            AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::RefreshLoadedIdentitiesOwnedDPNSNames,
            )) => {
                self.refresh_banner.take_and_clear();
                let handle = MessageBanner::set_global(
                    ctx,
                    "Refreshing contested names...",
                    MessageType::Info,
                );
                handle.with_elapsed();
                self.refresh_banner = Some(handle);
                self.refreshing_status = RefreshingStatus::Refreshing;
            }
            AppAction::SetMainScreen(_) => {
                self.refresh_banner.take_and_clear();
                self.refreshing_status = RefreshingStatus::NotRefreshing;
            }
            _ => {}
        }

        // If we have a pending backend task from scheduling (e.g. after immediate votes)
        if action == AppAction::None
            && let Some(bt) = self.pending_backend_task.take()
        {
            action = AppAction::BackendTask(bt);
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::model::user_role::UserRoleCell;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::Network;

    fn offline_ctx() -> (Arc<AppContext>, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        crate::app_dir::ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Regtest,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            UserRoleCell::default(),
        )
        .expect("offline regtest AppContext::new");
        (ctx, temp_dir)
    }

    #[test]
    fn scheduled_vote_sweep_error_is_handled_after_cleanup() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::ScheduledVotes);
        let error = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::NoVotingIdentity {
                identity_id: "voter-id".to_string(),
            }),
        };

        screen.scheduled_vote_cast_in_progress = true;
        assert!(screen.display_task_error(&error));
        assert!(!screen.scheduled_vote_cast_in_progress);
    }

    #[test]
    fn direct_scheduled_vote_error_remains_available_to_global_handling() {
        let (ctx, _temp_dir) = offline_ctx();
        let mut screen = DPNSScreen::new(&ctx, DPNSSubscreen::ScheduledVotes);

        screen.scheduled_vote_cast_in_progress = true;
        assert!(!screen.display_task_error(&TaskError::ScheduledVoteResultUnavailable));
        assert!(!screen.scheduled_vote_cast_in_progress);

        let sweep_error = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::ScheduledVoteResultUnavailable),
        };
        assert!(!screen.display_task_error(&sweep_error));
    }
}
