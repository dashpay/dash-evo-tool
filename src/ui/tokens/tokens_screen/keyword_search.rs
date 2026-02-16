use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::tokens::TokenTask;
use crate::ui::theme::DashColors;
use crate::ui::tokens::tokens_screen::{
    ContractDescriptionInfo, ContractSearchStatus, TokensScreen,
};
use chrono::Utc;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::emath::Align;
use egui::{Frame, Margin, RichText, Ui};
use egui_extras::{Column, TableBuilder};

const KEYWORD_SEARCH_INFO_TEXT: &str = "Keyword Search allows you to find tokens by searching their associated keywords.\n\n\
    When token creators register tokens on Dash Platform, they can add searchable keywords \
    to make their tokens discoverable.\n\n\
    Tips:\n\n\
    - Try common terms like 'game', 'music', 'art', etc.\n\n\
    - Keywords are case-insensitive.\n\n\
    - Each keyword costs 0.1 Dash to register, so creators choose them carefully.";

impl TokensScreen {
    pub(super) fn render_keyword_search(&mut self, ui: &mut Ui) -> AppAction {
        ui.set_min_width(ui.available_width());
        ui.set_max_width(ui.available_width());

        let mut action = AppAction::None;

        // 1) Input & "Go" button
        ui.horizontal(|ui| {
            ui.heading("Search Tokens by Keyword");
            if crate::ui::helpers::info_icon_button(ui, KEYWORD_SEARCH_INFO_TEXT).clicked() {
                self.show_pop_up_info = Some(KEYWORD_SEARCH_INFO_TEXT.to_string());
            }
        });
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Enter Keyword:");
            // Extract the query string as a clone to avoid multiple mutable borrows
            let query_ref = self
                .token_search_query
                .get_or_insert_with(|| "".to_string());
            let text_edit_response = ui.text_edit_singleline(query_ref);

            // Clone the current query for use in the closure below
            let current_query = query_ref.clone();

            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                ui.add_space(-6.0);

                ui.horizontal(|ui| {
                    let go_clicked = ui.button("Search").clicked();

                    let enter_pressed = text_edit_response.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    if go_clicked || enter_pressed {
                        // Clear old results, set status
                        self.search_results.lock().unwrap().clear();
                        let now = Utc::now().timestamp() as u64;
                        self.contract_search_status = ContractSearchStatus::WaitingForResult(now);
                        self.search_current_page = 1;
                        self.next_cursors.clear();
                        self.previous_cursors.clear();
                        self.search_has_next_page = false;

                        // Dispatch a backend task to do the actual keyword => token retrieval
                        let keyword = current_query.to_lowercase();
                        action = AppAction::BackendTask(BackendTask::TokenTask(Box::new(
                            TokenTask::QueryDescriptionsByKeyword(keyword, None),
                        )));
                    }

                    // Clear button
                    if ui.button("Clear").clicked() {
                        // Clear the search input
                        self.token_search_query = Some("".to_string());
                        // Clear the search results
                        self.search_results.lock().unwrap().clear();
                        // Reset the search status
                        self.contract_search_status = ContractSearchStatus::NotStarted;
                        // Clear pagination state
                        self.search_current_page = 1;
                        self.next_cursors.clear();
                        self.previous_cursors.clear();
                        self.search_has_next_page = false;
                        // Clear any selected contract and loading state
                        self.selected_contract_id = None;
                        self.contract_details_loading = false;
                        self.selected_contract_description = None;
                        self.selected_token_infos.clear();
                    }
                })
            });
        });

        ui.add_space(10.0);

        // 2) Display status
        match &self.contract_search_status {
            ContractSearchStatus::NotStarted => {
                // Nothing
            }
            ContractSearchStatus::WaitingForResult(start_time) => {
                let now = Utc::now().timestamp() as u64;
                let elapsed = now - start_time;
                ui.horizontal(|ui| {
                    ui.label(format!("Searching... {} seconds", elapsed));
                    ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
                });
            }
            ContractSearchStatus::Complete => {
                // Show the results
                let results = self.search_results.lock().unwrap().clone();
                if results.is_empty() {
                    ui.label("No tokens match your keyword.");
                } else {
                    action |= self.render_search_results_table(ui, &results);
                    // Pagination controls
                    if self.search_has_next_page || self.search_current_page > 1 {
                        ui.horizontal(|ui| {
                            if self.search_current_page > 1 && ui.button("Previous").clicked() {
                                // Go to previous page
                                action |= self.goto_previous_search_page();
                            }

                            if !(self.next_cursors.is_empty() && self.previous_cursors.is_empty()) {
                                ui.label(format!("Page {}", self.search_current_page));
                            }

                            if self.search_has_next_page && ui.button("Next").clicked() {
                                // Go to next page
                                action |= self.goto_next_search_page();
                            }
                        });
                    }
                }
            }
            ContractSearchStatus::ErrorMessage(e) => {
                let error_color = DashColors::error_color(ui.visuals().dark_mode);
                let msg = e.clone();
                Frame::new()
                    .fill(error_color.gamma_multiply(0.1))
                    .inner_margin(Margin::symmetric(10, 8))
                    .corner_radius(5.0)
                    .stroke(egui::Stroke::new(1.0, error_color))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("Error: {}", msg)).color(error_color));
                            ui.add_space(10.0);
                            if ui.small_button("Dismiss").clicked() {
                                self.contract_search_status = ContractSearchStatus::NotStarted;
                            }
                        });
                    });
            }
        }

        action
    }

    pub(super) fn render_search_results_table(
        &mut self,
        ui: &mut Ui,
        search_results: &[ContractDescriptionInfo],
    ) -> AppAction {
        let mut action = AppAction::None;

        let dark_mode = ui.visuals().dark_mode;

        egui::ScrollArea::both().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_max_width(ui.available_width());

            TableBuilder::new(ui)
                .striped(false)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(Align::Center))
                .column(Column::initial(60.0).resizable(true)) // Contract ID
                .column(Column::initial(200.0).resizable(true)) // Contract Description
                .column(Column::initial(80.0).resizable(true)) // Action
                .header(30.0, |mut header| {
                    header.col(|ui| {
                        ui.label(
                            RichText::new("Contract ID")
                                .strong()
                                .size(14.0)
                                .color(DashColors::text_primary(dark_mode)),
                        );
                    });
                    header.col(|ui| {
                        ui.label(
                            RichText::new("Contract Description")
                                .strong()
                                .size(14.0)
                                .color(DashColors::text_primary(dark_mode)),
                        );
                    });
                    header.col(|ui| {
                        ui.label(
                            RichText::new("Action")
                                .strong()
                                .size(14.0)
                                .color(DashColors::text_primary(dark_mode)),
                        );
                    });
                })
                .body(|mut body| {
                    for contract in search_results {
                        body.row(25.0, |mut row| {
                            row.col(|ui| {
                                ui.label(contract.data_contract_id.to_string(Encoding::Base58));
                            });
                            row.col(|ui| {
                                let description = if contract.description.trim().is_empty() {
                                    "None".to_string()
                                } else {
                                    contract.description.clone()
                                };

                                ui.label(description);
                            });
                            row.col(|ui| {
                                // Example "Add" button
                                if ui.button("More Info").clicked() {
                                    // Show more info about the token
                                    self.selected_contract_id = Some(contract.data_contract_id);
                                    // Set loading state to true
                                    self.contract_details_loading = true;
                                    // Clear previous data
                                    self.selected_contract_description = None;
                                    self.selected_token_infos.clear();
                                    action = AppAction::BackendTask(BackendTask::ContractTask(
                                        Box::new(ContractTask::FetchContractsWithDescriptions(
                                            vec![contract.data_contract_id],
                                        )),
                                    ));
                                }
                            });
                        });
                    }
                });
        });

        action
    }
}
