use crate::app::AppAction;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::ui::tokens::tokens_screen::{
    ContractDescriptionInfo, ContractSearchStatus, TokensScreen,
};
use chrono::Utc;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use eframe::emath::Align;
use eframe::epaint::{Color32, Margin};
use egui::{Frame, Ui};
use egui_extras::{Column, TableBuilder};

impl TokensScreen {
    pub(super) fn render_keyword_search(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // 1) Input & “Go” button
        ui.heading("Search Tokens by Keyword");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Enter Keyword:");
            let query_ref = self
                .token_search_query
                .get_or_insert_with(|| "".to_string());
            let text_edit_response = ui.text_edit_singleline(query_ref);

            let go_clicked = ui.button("Search").clicked();
            let enter_pressed =
                text_edit_response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

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
                let keyword = query_ref.to_lowercase();
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
                    ui.add(egui::widgets::Spinner::default().color(Color32::from_rgb(0, 128, 255)));
                });
            }
            ContractSearchStatus::Complete => {
                // Show the results
                let results = self.search_results.lock().unwrap().clone();
                if results.is_empty() {
                    ui.label("No tokens match your keyword.");
                } else {
                    action |= self.render_search_results_table(ui, &results);
                }

                // Pagination controls
                ui.horizontal(|ui| {
                    if self.search_current_page > 1 && ui.button("Previous").clicked() {
                        // Go to previous page
                        action = self.goto_previous_search_page();
                    }

                    if !(self.next_cursors.is_empty() && self.previous_cursors.is_empty()) {
                        ui.label(format!("Page {}", self.search_current_page));
                    }

                    if self.search_has_next_page && ui.button("Next").clicked() {
                        // Go to next page
                        action = self.goto_next_search_page();
                    }
                });
            }
            ContractSearchStatus::ErrorMessage(e) => {
                ui.colored_label(Color32::RED, format!("Error: {}", e));
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

        egui::ScrollArea::vertical().show(ui, |ui| {
            Frame::group(ui.style())
                .fill(ui.visuals().panel_fill)
                .stroke(egui::Stroke::new(
                    1.0,
                    ui.visuals().widgets.inactive.bg_stroke.color,
                ))
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    TableBuilder::new(ui)
                        .striped(true)
                        .resizable(true)
                        .cell_layout(egui::Layout::left_to_right(Align::Center))
                        .column(Column::initial(60.0).resizable(true)) // Contract ID
                        .column(Column::initial(200.0).resizable(true)) // Contract Description
                        .column(Column::initial(80.0).resizable(true)) // Action
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                ui.label("Contract ID");
                            });
                            header.col(|ui| {
                                ui.label("Contract Description");
                            });
                            header.col(|ui| {
                                ui.label("Action");
                            });
                        })
                        .body(|mut body| {
                            for contract in search_results {
                                body.row(25.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(
                                            contract.data_contract_id.to_string(Encoding::Base58),
                                        );
                                    });
                                    row.col(|ui| {
                                        ui.label(contract.description.clone());
                                    });
                                    row.col(|ui| {
                                        // Example "Add" button
                                        if ui.button("More Info").clicked() {
                                            // Show more info about the token
                                            self.selected_contract_id =
                                                Some(contract.data_contract_id);
                                            // Set loading state to true
                                            self.contract_details_loading = true;
                                            // Clear previous data
                                            self.selected_contract_description = None;
                                            self.selected_token_infos.clear();
                                            action = AppAction::BackendTask(
                                                BackendTask::ContractTask(Box::new(
                                                    ContractTask::FetchContractsWithDescriptions(
                                                        vec![contract.data_contract_id],
                                                    ),
                                                )),
                                            );
                                        }
                                    });
                                });
                            }
                        });
                });
        });

        action
    }
}
