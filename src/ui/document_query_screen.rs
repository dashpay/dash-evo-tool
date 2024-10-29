use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::contested_name::ContestedName;
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::BackendTask;
use crate::ui::components::contract_chooser_panel::add_contract_chooser_panel;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use chrono::{DateTime, Utc};
use dash_sdk::dpp::voting::vote_choices::resource_vote_choice::ResourceVoteChoice;
use egui::{Context, Ui};
use std::sync::{Arc, Mutex};

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

pub struct DocumentQueryScreen {
    contested_names: Arc<Mutex<Vec<ContestedName>>>,
    pub app_context: Arc<AppContext>,
    error_message: Option<(String, MessageType, DateTime<Utc>)>,
    sort_column: SortColumn,
    sort_order: SortOrder,
    show_vote_popup: Option<(String, ContestedResourceTask)>,
    contract_search_term: String,
}

impl DocumentQueryScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let contested_names = Arc::new(Mutex::new(
            app_context.all_contested_names().unwrap_or_default(),
        ));
        Self {
            contested_names,
            app_context: app_context.clone(),
            error_message: None,
            sort_column: SortColumn::ContestedName,
            sort_order: SortOrder::Ascending,
            show_vote_popup: None,
            contract_search_term: String::new(),
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
                    self.show_vote_popup = Some((
                        format!(
                            "Confirm Voting for Contestant {} for name \"{}\"",
                            contestant.id, contestant.name
                        ),
                        ContestedResourceTask::VoteOnDPNSName(
                            contested_name.normalized_contested_name.clone(),
                            ResourceVoteChoice::Abstain,
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
        if let Some((message, action)) = self.show_vote_popup.clone() {
            ui.label(message);

            ui.horizontal(|ui| {
                if ui.button("Vote Immediate").clicked() {
                    app_action = AppAction::BackendTask(BackendTask::ContestedResourceTask(action));
                    self.show_vote_popup = None;
                } else if ui.button("Vote Deferred").clicked() {
                    app_action = AppAction::BackendTask(BackendTask::ContestedResourceTask(action));
                    self.show_vote_popup = None;
                } else if ui.button("Cancel").clicked() {
                    self.show_vote_popup = None;
                }
            });
        }
        app_action
    }
}
impl ScreenLike for DocumentQueryScreen {
    fn refresh(&mut self) {
        let mut contested_names = self.contested_names.lock().unwrap();
        *contested_names = self.app_context.all_contested_names().unwrap_or_default();
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
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDocumentQuery,
        );

        action |=
            add_contract_chooser_panel(ctx, &mut self.contract_search_term, &self.app_context);

        action
    }
}
