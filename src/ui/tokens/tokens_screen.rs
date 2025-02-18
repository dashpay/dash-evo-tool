use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, CentralPanel, Color32, Context, Frame, Margin, Ui};
use egui::Align;
use egui_extras::{Column, TableBuilder};

use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, Screen, ScreenLike};

use super::burn_tokens_screen::BurnTokensScreen;
use super::destroy_frozen_funds_screen::DestroyFrozenFundsScreen;
use super::freeze_tokens_screen::FreezeTokensScreen;
use super::mint_tokens_screen::MintTokensScreen;
use super::pause_tokens_screen::PauseTokensScreen;
use super::resume_tokens_screen::ResumeTokensScreen;
use super::transfer_tokens_screen::TransferTokensScreen;
use super::unfreeze_tokens_screen::UnfreezeTokensScreen;

/// A token owned by an identity.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityTokenBalance {
    pub token_identifier: Identifier,
    pub token_name: String,
    pub identity_id: Identifier,
    pub balance: u64,
    pub data_contract_id: Identifier,
    pub token_position: u16,
}

/// Which token sub-screen is currently showing.
#[derive(PartialEq)]
pub enum TokensSubscreen {
    MyTokens,
    SearchTokens,
}

impl TokensSubscreen {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::MyTokens => "My Tokens",
            Self::SearchTokens => "Search Tokens",
        }
    }
}

#[derive(PartialEq)]
pub enum RefreshingStatus {
    Refreshing(u64),
    NotRefreshing,
}

/// Represents the status of the user’s search
#[derive(PartialEq, Eq, Clone)]
pub enum TokenSearchStatus {
    NotStarted,
    WaitingForResult(u64),
    Complete,
    ErrorMessage(String),
}

/// Sorting columns
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    TokenName,
    TokenID,
    OwnerIdentity,
    OwnerIdentityAlias,
    Balance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortOrder {
    Ascending,
    Descending,
}

/// The main, combined TokensScreen:
/// - Displays token balances or a search UI
/// - Allows reordering of tokens if desired
pub struct TokensScreen {
    pub app_context: Arc<AppContext>,
    pub tokens_subscreen: TokensSubscreen,
    my_tokens: Arc<Mutex<Vec<IdentityTokenBalance>>>,
    selected_token_id: Option<Identifier>,
    show_token_info: Option<Identifier>,
    backend_message: Option<(String, MessageType, DateTime<Utc>)>,
    pending_backend_task: Option<BackendTask>,
    refreshing_status: RefreshingStatus,

    // Token Search
    token_search_query: Option<String>,
    search_results: Arc<Mutex<Vec<IdentityTokenBalance>>>,
    token_search_status: TokenSearchStatus,
    search_current_page: usize,
    search_has_next_page: bool,
    next_cursors: Vec<Identifier>,
    previous_cursors: Vec<Identifier>,

    /// Sorting
    sort_column: SortColumn,
    sort_order: SortOrder,
    use_custom_order: bool,

    // Remove token
    confirm_remove_token_popup: bool,
    token_to_remove: Option<IdentityTokenBalance>,
}

impl TokensScreen {
    pub fn new(app_context: &Arc<AppContext>, tokens_subscreen: TokensSubscreen) -> Self {
        let my_tokens = Arc::new(Mutex::new(
            app_context.identity_token_balances().unwrap_or_default(),
        ));

        let mut screen = Self {
            app_context: app_context.clone(),
            my_tokens,
            selected_token_id: None,
            show_token_info: None,
            token_search_query: None,
            token_search_status: TokenSearchStatus::NotStarted,
            search_current_page: 1,
            search_has_next_page: false,
            next_cursors: vec![],
            previous_cursors: vec![],
            search_results: Arc::new(Mutex::new(Vec::new())),
            backend_message: None,
            sort_column: SortColumn::TokenName,
            sort_order: SortOrder::Ascending,
            use_custom_order: false,
            pending_backend_task: None,
            tokens_subscreen,
            refreshing_status: RefreshingStatus::NotRefreshing,

            // Remove token
            confirm_remove_token_popup: false,
            token_to_remove: None,
        };

        // if let Ok(saved_ids) = screen.app_context.db.load_token_order() {
        //     screen.reorder_vec_to(saved_ids);
        //     screen.use_custom_order = true;
        // }

        screen
    }

    // ─────────────────────────────────────────────────────────────────
    // Reordering
    // ─────────────────────────────────────────────────────────────────

    /// Reorder `my_tokens` to match a given list of (token_id, identity_id).
    fn reorder_vec_to(&self, new_order: Vec<(Identifier, Identifier)>) {
        let mut lock = self.my_tokens.lock().unwrap();
        for (desired_idx, (token_id, identity_id)) in new_order.iter().enumerate() {
            if let Some(current_idx) = lock
                .iter()
                .position(|t| t.token_identifier == *token_id && t.identity_id == *identity_id)
            {
                if current_idx != desired_idx && current_idx < lock.len() {
                    lock.swap(current_idx, desired_idx);
                }
            }
        }
    }

    /// Save the current vector's order of token IDs to the DB
    fn save_current_order(&self) {
        let lock = self.my_tokens.lock().unwrap();
        let all_ids = lock
            .iter()
            .map(|token| (token.token_identifier.clone(), token.identity_id.clone()))
            .collect::<Vec<_>>();
        drop(lock);
        self.app_context
            .db
            .save_token_order(all_ids)
            .or_else(|e| {
                eprintln!("Error saving token order: {}", e);
                Err(e)
            })
            .ok();
    }

    // ─────────────────────────────────────────────────────────────────
    // Sorting
    // ─────────────────────────────────────────────────────────────────

    /// Sort the vector by the user-specified column/order, overriding any custom order.
    fn sort_vec(&self, list: &mut [IdentityTokenBalance]) {
        list.sort_by(|a, b| {
            let ordering = match self.sort_column {
                SortColumn::Balance => a.balance.cmp(&b.balance),
                SortColumn::OwnerIdentity => a.identity_id.cmp(&b.identity_id),
                SortColumn::OwnerIdentityAlias => {
                    let alias_a = self
                        .app_context
                        .get_alias(&a.identity_id)
                        .expect("Expected to get alias")
                        .unwrap_or("".to_string());
                    let alias_b = self
                        .app_context
                        .get_alias(&b.identity_id)
                        .expect("Expected to get alias")
                        .unwrap_or("".to_string());
                    alias_a.cmp(&alias_b)
                }
                SortColumn::TokenName => a.token_name.cmp(&b.token_name),
                SortColumn::TokenID => a.token_identifier.cmp(&b.token_identifier),
            };
            match self.sort_order {
                SortOrder::Ascending => ordering,
                SortOrder::Descending => ordering.reverse(),
            }
        });
        self.save_current_order();
    }

    fn sort_vec_of_groups(&self, list: &mut [(Identifier, String, u64)]) {
        list.sort_by(|a, b| {
            let ordering = match self.sort_column {
                SortColumn::Balance => a.2.cmp(&b.2),
                SortColumn::TokenName => a.1.cmp(&b.1),
                SortColumn::TokenID => a.0.cmp(&b.0),
                _ => a.0.cmp(&b.0),
            };
            match self.sort_order {
                SortOrder::Ascending => ordering,
                SortOrder::Descending => ordering.reverse(),
            }
        });
    }

    fn toggle_sort(&mut self, column: SortColumn) {
        self.use_custom_order = false;
        if self.sort_column == column {
            self.sort_order = match self.sort_order {
                SortOrder::Ascending => SortOrder::Descending,
                SortOrder::Descending => SortOrder::Ascending,
            };
            self.save_current_order();
        } else {
            self.sort_column = column;
            self.sort_order = SortOrder::Ascending;
            self.save_current_order();
        }
    }

    /// Group all IdentityTokenBalance objects by token_identifier.
    /// Returns a Vec of (token_identifier, token_name, total_balance).
    fn group_tokens_by_identifier(
        &self,
        tokens: &[IdentityTokenBalance],
    ) -> Vec<(Identifier, String, u64)> {
        let mut map: HashMap<Identifier, (String, u64)> = HashMap::new();
        for tb in tokens {
            let entry = map.entry(tb.token_identifier.clone()).or_insert_with(|| {
                // Store (token_name, running_total_balance)
                (tb.token_name.clone(), 0u64)
            });
            entry.1 += tb.balance;
        }

        // Convert to a vec for display
        let mut result = Vec::new();
        for (identifier, (name, total_balance)) in map {
            result.push((identifier, name, total_balance));
        }
        // Sort by token name, for example
        result.sort_by(|a, b| a.1.cmp(&b.1));
        result
    }

    // ─────────────────────────────────────────────────────────────────
    // Message handling
    // ─────────────────────────────────────────────────────────────────

    fn dismiss_message(&mut self) {
        self.backend_message = None;
    }

    fn check_error_expiration(&mut self) {
        if let Some((_, _, timestamp)) = &self.backend_message {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(*timestamp);
            if elapsed.num_seconds() >= 10 {
                self.dismiss_message();
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // Rendering
    // ─────────────────────────────────────────────────────────────────

    /// Renders the top-level token list (one row per unique token).
    /// When the user clicks on a token, we set `selected_token_id`.
    fn render_token_list(&mut self, ui: &mut Ui, tokens: &[IdentityTokenBalance]) {
        let mut grouped = self.group_tokens_by_identifier(tokens);
        if !self.use_custom_order {
            self.sort_vec_of_groups(&mut grouped);
        }

        // Allocate space for refreshing indicator
        let refreshing_height = 33.0;
        let mut max_scroll_height = if let RefreshingStatus::Refreshing(_) = self.refreshing_status
        {
            ui.available_height() - refreshing_height
        } else {
            ui.available_height()
        };

        // Allocate space for backend message
        let backend_message_height = 40.0;
        if let Some((_, _, _)) = self.backend_message.clone() {
            max_scroll_height -= backend_message_height;
        }

        // A simple table with columns: [Token Name | Token ID | Total Balance]
        egui::ScrollArea::vertical()
            .max_height(max_scroll_height)
            .show(ui, |ui| {
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
                            .cell_layout(egui::Layout::left_to_right(Align::Center))
                            .column(Column::initial(150.0).resizable(true)) // Token Name
                            .column(Column::initial(200.0).resizable(true)) // Token ID
                            .column(Column::initial(80.0).resizable(true)) // Total Balance
                            // .column(Column::initial(80.0).resizable(true)) // Token Info
                            .header(30.0, |mut header| {
                                header.col(|ui| {
                                    if ui.button("Token Name").clicked() {
                                        self.toggle_sort(SortColumn::TokenName);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Token ID").clicked() {
                                        self.toggle_sort(SortColumn::TokenID);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Total Balance").clicked() {
                                        self.toggle_sort(SortColumn::Balance);
                                    }
                                });
                                // header.col(|ui| {
                                //     ui.label("Token Info");
                                // });
                            })
                            .body(|mut body| {
                                for (token_id, token_name, total_balance) in grouped {
                                    body.row(25.0, |mut row| {
                                        row.col(|ui| {
                                            // By making the label into a button or using `ui.selectable_label`,
                                            // we can respond to clicks.
                                            if ui.button(&token_name).clicked() {
                                                self.selected_token_id = Some(token_id.clone());
                                            }
                                        });
                                        row.col(|ui| {
                                            ui.label(token_id.to_string(Encoding::Base58));
                                        });
                                        row.col(|ui| {
                                            ui.label(total_balance.to_string());
                                        });
                                        // row.col(|ui| {
                                        //     if ui.button("Info").clicked() {
                                        //         self.show_token_info = Some(token_id.clone());
                                        //     }
                                        // });
                                    });
                                }
                            });
                    });
            });
    }

    /// Renders details for the selected token_id: a row per identity that holds that token.
    fn render_token_details(&mut self, ui: &mut Ui, tokens: &[IdentityTokenBalance]) -> AppAction {
        let mut action = AppAction::None;

        let token_id = self.selected_token_id.as_ref().unwrap();

        // Filter out only the IdentityTokenBalance for this token_id
        let mut detail_list: Vec<IdentityTokenBalance> = tokens
            .iter()
            .filter(|t| &t.token_identifier == token_id)
            .cloned()
            .collect();
        if !self.use_custom_order {
            self.sort_vec(&mut detail_list);
        }

        // This is basically your old `render_table_my_token_balances` logic, but
        // limited to just the single token.
        // Allocate space for refreshing indicator
        let refreshing_height = 33.0;
        let mut max_scroll_height = if let RefreshingStatus::Refreshing(_) = self.refreshing_status
        {
            ui.available_height() - refreshing_height
        } else {
            ui.available_height()
        };

        // Allocate space for backend message
        let backend_message_height = 40.0;
        if let Some((_, _, _)) = self.backend_message.clone() {
            max_scroll_height -= backend_message_height;
        }

        // A simple table with columns: [Token Name | Token ID | Total Balance]
        egui::ScrollArea::vertical()
            .max_height(max_scroll_height)
            .show(ui, |ui| {
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
                            .cell_layout(egui::Layout::left_to_right(Align::Center))
                            .column(Column::initial(200.0).resizable(true)) // Identity Alias
                            .column(Column::initial(200.0).resizable(true)) // Identity ID
                            .column(Column::initial(60.0).resizable(true)) // Balance
                            .column(Column::initial(200.0).resizable(true)) // Actions
                            .header(30.0, |mut header| {
                                header.col(|ui| {
                                    if ui.button("Identity Alias").clicked() {
                                        self.toggle_sort(SortColumn::OwnerIdentityAlias);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Identity ID").clicked() {
                                        self.toggle_sort(SortColumn::OwnerIdentity);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Balance").clicked() {
                                        self.toggle_sort(SortColumn::Balance);
                                    }
                                });
                                header.col(|ui| {
                                    ui.label("Actions");
                                });
                            })
                            .body(|mut body| {
                                for itb in &detail_list {
                                    body.row(25.0, |mut row| {
                                        row.col(|ui| {
                                            // Show identity alias or ID
                                            if let Some(alias) = self
                                                .app_context
                                                .get_alias(&itb.identity_id)
                                                .expect("Expected to get alias")
                                            {
                                                ui.label(alias);
                                            } else {
                                                ui.label("");
                                            }
                                        });
                                        row.col(|ui| {
                                            ui.label(itb.identity_id.to_string(Encoding::Base58));
                                        });
                                        row.col(|ui| {
                                            ui.label(itb.balance.to_string());
                                        });
                                        row.col(|ui| {
                                            ui.horizontal(|ui| {
                                                // Transfer
                                                if ui.button("Transfer").clicked() {
                                                    action = AppAction::AddScreen(
                                                        Screen::TransferTokensScreen(
                                                            TransferTokensScreen::new(
                                                                itb.clone(),
                                                                &self.app_context,
                                                            ),
                                                        ),
                                                    );
                                                }

                                                // Expandable advanced actions menu
                                                ui.menu_button("...", |ui| {
                                                    if ui.button("Mint").clicked() {
                                                        action = AppAction::AddScreen(
                                                            Screen::MintTokensScreen(
                                                                MintTokensScreen::new(
                                                                    itb.clone(),
                                                                    &self.app_context,
                                                                ),
                                                            ),
                                                        );
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Burn").clicked() {
                                                        action = AppAction::AddScreen(
                                                            Screen::BurnTokensScreen(
                                                                BurnTokensScreen::new(
                                                                    itb.clone(),
                                                                    &self.app_context,
                                                                ),
                                                            ),
                                                        );
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Freeze").clicked() {
                                                        action = AppAction::AddScreen(
                                                            Screen::FreezeTokensScreen(
                                                                FreezeTokensScreen::new(
                                                                    itb.clone(),
                                                                    &self.app_context,
                                                                ),
                                                            ),
                                                        );
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Destroy").clicked() {
                                                        action = AppAction::AddScreen(
                                                            Screen::DestroyFrozenFundsScreen(
                                                                DestroyFrozenFundsScreen::new(
                                                                    itb.clone(),
                                                                    &self.app_context,
                                                                ),
                                                            ),
                                                        );
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Unfreeze").clicked() {
                                                        action = AppAction::AddScreen(
                                                            Screen::UnfreezeTokensScreen(
                                                                UnfreezeTokensScreen::new(
                                                                    itb.clone(),
                                                                    &self.app_context,
                                                                ),
                                                            ),
                                                        );
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Pause").clicked() {
                                                        action = AppAction::AddScreen(
                                                            Screen::PauseTokensScreen(
                                                                PauseTokensScreen::new(
                                                                    itb.clone(),
                                                                    &self.app_context,
                                                                ),
                                                            ),
                                                        );
                                                        ui.close_menu();
                                                    }
                                                    if ui.button("Resume").clicked() {
                                                        action = AppAction::AddScreen(
                                                            Screen::ResumeTokensScreen(
                                                                ResumeTokensScreen::new(
                                                                    itb.clone(),
                                                                    &self.app_context,
                                                                ),
                                                            ),
                                                        );
                                                        ui.close_menu();
                                                    }
                                                });
                                            });
                                        });
                                    });
                                }
                            });
                    });
            });

        action
    }

    fn render_token_search(&mut self, ui: &mut Ui) -> AppAction {
        let action = AppAction::None;

        ui.vertical_centered(|ui| {
            ui.add_space(10.0);
            ui.heading("Coming Soon");
            ui.add_space(5.0);

            //     ui.add_space(10.0);
            //     ui.label("Search for tokens by keyword, name, or ID.");
            //     ui.add_space(5.0);

            //     ui.horizontal(|ui| {
            //         ui.label("Search by keyword(s):");
            //         ui.text_edit_singleline(self.token_search_query.get_or_insert_with(String::new));
            //         if ui.button("Go").clicked() {
            //             // 1) Clear old results, set status
            //             let now = Utc::now().timestamp() as u64;
            //             self.token_search_status = TokenSearchStatus::WaitingForResult(now);
            //             {
            //                 let mut sr = self.search_results.lock().unwrap();
            //                 sr.clear();
            //             }
            //             self.search_current_page = 1;
            //             self.next_cursors.clear();
            //             self.previous_cursors.clear();
            //             self.search_has_next_page = false;

            //             // 2) Dispatch backend request
            //             let query_string = self
            //                 .token_search_query
            //                 .as_ref()
            //                 .map(|s| s.clone())
            //                 .unwrap_or_default();

            //             // Example: if you want paged results from the start:
            //             action = AppAction::BackendTask(BackendTask::TokenTask(
            //                 TokenTask::QueryTokensByKeywordPage(query_string, None),
            //             ));
            //         }
            //     });
        });

        // ui.separator();
        // ui.add_space(10.0);

        // // Show results or messages
        // match self.token_search_status {
        //     TokenSearchStatus::WaitingForResult(start_time) => {
        //         let now = Utc::now().timestamp() as u64;
        //         let elapsed = now - start_time;
        //         ui.label(format!("Searching... Time so far: {} seconds", elapsed));
        //         ui.add(egui::widgets::Spinner::default().color(Color32::from_rgb(0, 128, 255)));
        //     }
        //     TokenSearchStatus::Complete => {
        //         // Render the results table
        //         let tokens = self.search_results.lock().unwrap().clone();
        //         if tokens.is_empty() {
        //             ui.label("No tokens match your search.");
        //         } else {
        //             // Possibly add a filter input above the table, if you like
        //             action |= self.render_search_results_table(ui, &tokens);
        //         }

        //         // Then pagination controls
        //         ui.horizontal(|ui| {
        //             // If not on page 1, we can show a “Prev” button
        //             if self.search_current_page > 1 {
        //                 if ui.button("Previous Page").clicked() {
        //                     action |= self.goto_previous_search_page();
        //                 }
        //             }

        //             ui.label(format!("Page {}", self.search_current_page));

        //             // If has_next_page, show “Next Page” button
        //             if self.search_has_next_page {
        //                 if ui.button("Next Page").clicked() {
        //                     action |= self.goto_next_search_page();
        //                 }
        //             }
        //         });
        //     }
        //     TokenSearchStatus::ErrorMessage(ref e) => {
        //         ui.colored_label(Color32::DARK_RED, format!("Error: {}", e));
        //     }
        //     TokenSearchStatus::NotStarted => {
        //         ui.label("Enter keywords above and click Go to search tokens.");
        //     }
        // }

        action
    }

    fn render_search_results_table(
        &mut self,
        ui: &mut Ui,
        search_results: &[IdentityTokenBalance],
    ) -> AppAction {
        let action = AppAction::None;

        // In your DocumentQueryScreen code, you also had a ScrollArea
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
                        .cell_layout(egui::Layout::left_to_right(Align::Center))
                        .column(Column::initial(80.0).resizable(true)) // Token Name
                        .column(Column::initial(330.0).resizable(true)) // Identity
                        .column(Column::initial(60.0).resizable(true)) // Balance
                        .column(Column::initial(80.0).resizable(true)) // Action
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                if ui.button("Token Name").clicked() {
                                    self.toggle_sort(SortColumn::TokenName);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Token ID").clicked() {
                                    self.toggle_sort(SortColumn::TokenID);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Balance").clicked() {
                                    self.toggle_sort(SortColumn::Balance);
                                }
                            });
                            header.col(|ui| {
                                ui.label("Action");
                            });
                        })
                        .body(|mut body| {
                            for token in search_results {
                                body.row(25.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(&token.token_name);
                                    });
                                    row.col(|ui| {
                                        ui.label(token.identity_id.to_string(Encoding::Base58));
                                    });
                                    row.col(|ui| {
                                        ui.label(token.balance.to_string());
                                    });
                                    row.col(|ui| {
                                        if ui.button("Add").clicked() {
                                            // Add to my_tokens
                                            self.add_token_to_my_tokens(token.clone());
                                        }
                                    });
                                });
                            }
                        });
                });
        });

        action
    }

    fn render_no_owned_tokens(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            match self.tokens_subscreen {
                TokensSubscreen::MyTokens => {
                    ui.label(
                        egui::RichText::new("No owned tokens found.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                TokensSubscreen::SearchTokens => {
                    ui.label(
                        egui::RichText::new("No matching tokens found.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
            }
            ui.add_space(10.0);

            ui.label("Please check back later or try refreshing the list.");
            ui.add_space(20.0);
            if ui.button("Refresh").clicked() {
                if let RefreshingStatus::Refreshing(_) = self.refreshing_status {
                    app_action = AppAction::None;
                } else {
                    let now = Utc::now().timestamp() as u64;
                    self.refreshing_status = RefreshingStatus::Refreshing(now);
                    match self.tokens_subscreen {
                        TokensSubscreen::MyTokens => {
                            app_action = AppAction::BackendTask(BackendTask::TokenTask(
                                TokenTask::QueryMyTokenBalances,
                            ));
                        }
                        TokensSubscreen::SearchTokens => {
                            app_action = AppAction::Refresh;
                        }
                    }
                }
            }
        });

        app_action
    }

    fn add_token_to_my_tokens(&self, token: IdentityTokenBalance) {
        let mut my_tokens = self.my_tokens.lock().unwrap();
        // Prevent duplicates
        if !my_tokens
            .iter()
            .any(|t| t.token_identifier == token.token_identifier)
        {
            my_tokens.push(token);
        }
        // Save the new order
        self.save_current_order();
    }

    fn goto_next_search_page(&mut self) -> AppAction {
        // If we have a next cursor:
        if let Some(next_cursor) = self.next_cursors.last().cloned() {
            // set status
            let now = Utc::now().timestamp() as u64;
            self.token_search_status = TokenSearchStatus::WaitingForResult(now);

            // push the current one onto “previous” so we can go back
            // if the user is on page N, and we have a nextCursor in next_cursors[N - 1] or so
            self.previous_cursors.push(next_cursor.clone());

            self.search_current_page += 1;

            // Dispatch
            let query_string = self
                .token_search_query
                .as_ref()
                .map(|s| s.clone())
                .unwrap_or_default();

            return AppAction::BackendTask(BackendTask::TokenTask(
                TokenTask::QueryTokensByKeywordPage(query_string, Some(next_cursor)),
            ));
        }
        AppAction::None
    }

    fn goto_previous_search_page(&mut self) -> AppAction {
        if self.search_current_page > 1 {
            // Move to (page - 1)
            self.search_current_page -= 1;
            let now = Utc::now().timestamp() as u64;
            self.token_search_status = TokenSearchStatus::WaitingForResult(now);

            // The “last” previous_cursors item is the new page’s state
            if let Some(prev_cursor) = self.previous_cursors.pop() {
                // Possibly pop from next_cursors if we want to re-insert it later
                // self.next_cursors.truncate(self.search_current_page - 1);
                let query_string = self
                    .token_search_query
                    .as_ref()
                    .map(|s| s.clone())
                    .unwrap_or_default();
                return AppAction::BackendTask(BackendTask::TokenTask(
                    TokenTask::QueryTokensByKeywordPage(query_string, Some(prev_cursor)),
                ));
            }
        }
        AppAction::None
    }

    fn show_remove_token_popup(&mut self, ui: &mut egui::Ui) {
        // If no token is set, nothing to confirm
        let token_to_remove = match &self.token_to_remove {
            Some(token) => token.clone(),
            None => {
                self.confirm_remove_token_popup = false;
                return;
            }
        };

        let mut is_open = true;

        egui::Window::new("Confirm Remove Balance")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                ui.label(format!(
                    "Are you sure you want to remove identity token balance \"{}\" for identity \"{}\"?",
                    token_to_remove.token_name,
                    token_to_remove.identity_id.to_string(Encoding::Base58)
                ));

                // Confirm button
                if ui.button("Confirm").clicked() {
                    if let Err(e) = self.app_context.remove_token_balance(
                        token_to_remove.token_identifier,
                        token_to_remove.identity_id.clone(),
                    ) {
                        self.backend_message = Some((
                            format!("Error removing token balance: {}", e),
                            MessageType::Error,
                            Utc::now(),
                        ));
                        self.confirm_remove_token_popup = false;
                        self.token_to_remove = None;
                    } else {
                        self.confirm_remove_token_popup = false;
                        self.token_to_remove = None;
                        self.refresh();
                    };
                }

                // Cancel button
                if ui.button("Cancel").clicked() {
                    self.confirm_remove_token_popup = false;
                    self.token_to_remove = None;
                }
            });

        // If user closes the popup window (the [x] button), also reset state
        if !is_open {
            self.confirm_remove_token_popup = false;
            self.token_to_remove = None;
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// ScreenLike implementation
// ─────────────────────────────────────────────────────────────────
impl ScreenLike for TokensScreen {
    fn refresh(&mut self) {
        self.my_tokens = Arc::new(Mutex::new(
            self.app_context
                .identity_token_balances()
                .unwrap_or_default(),
        ));
        match self.app_context.db.load_token_order() {
            Ok(saved_ids) => {
                self.reorder_vec_to(saved_ids);
                self.use_custom_order = true;
            }
            Err(e) => {
                eprintln!("Error loading token order: {}", e);
            }
        }
    }

    fn refresh_on_arrival(&mut self) {
        self.my_tokens = Arc::new(Mutex::new(
            self.app_context
                .identity_token_balances()
                .unwrap_or_default(),
        ));
    }

    fn display_message(&mut self, msg: &str, msg_type: MessageType) {
        // Handle messages from querying My Token Balances
        if msg.contains("Successfully fetched token balances")
            | msg.contains("Failed to fetch token balances")
        {
            self.backend_message = Some((msg.to_string(), msg_type, Utc::now()));
            self.refreshing_status = RefreshingStatus::NotRefreshing;
        }

        // Handle messages from Token Search
        if msg.contains("Error fetching tokens") {
            self.token_search_status = TokenSearchStatus::ErrorMessage(msg.to_string());
            self.backend_message = Some((msg.to_string(), msg_type, Utc::now()));
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::TokensByKeyword(tokens) => {
                // This might be a “full” result (no paging).
                let mut srch = self.search_results.lock().unwrap();
                *srch = tokens;
                self.token_search_status = TokenSearchStatus::Complete;
            }
            BackendTaskSuccessResult::TokensByKeywordPage(tokens, next_cursor) => {
                // Paged result
                let mut srch = self.search_results.lock().unwrap();
                *srch = tokens;
                self.search_has_next_page = next_cursor.is_some();

                if let Some(cursor) = next_cursor {
                    // Save it for “next page” retrieval
                    self.next_cursors.push(cursor);
                }
                self.token_search_status = TokenSearchStatus::Complete;
            }
            _ => {}
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;

        self.check_error_expiration();

        // Build top-right buttons
        let right_buttons = match self.tokens_subscreen {
            TokensSubscreen::MyTokens => vec![(
                "Refresh",
                DesiredAppAction::BackendTask(BackendTask::TokenTask(
                    TokenTask::QueryMyTokenBalances,
                )),
            )],
            TokensSubscreen::SearchTokens => vec![("Refresh", DesiredAppAction::Refresh)],
        };

        // Top panel
        if let Some(token_id) = self.selected_token_id {
            let token_name: String = self
                .my_tokens
                .lock()
                .unwrap()
                .iter()
                .find_map(|t| {
                    if t.token_identifier == token_id {
                        Some(t.token_name.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| token_id.to_string(Encoding::Base58));

            action |= add_top_panel(
                ctx,
                &self.app_context,
                vec![
                    ("Tokens", AppAction::Custom("Back to tokens".to_string())),
                    (&format!("{token_name}"), AppAction::None),
                ],
                right_buttons.clone(),
            );
        } else {
            action |= add_top_panel(
                ctx,
                &self.app_context,
                vec![("Tokens", AppAction::None)],
                right_buttons.clone(),
            );
        }

        // Left panel
        match self.tokens_subscreen {
            TokensSubscreen::MyTokens => {
                action |= add_left_panel(
                    ctx,
                    &self.app_context,
                    RootScreenType::RootScreenMyTokenBalances,
                );
            }
            TokensSubscreen::SearchTokens => {
                action |= add_left_panel(
                    ctx,
                    &self.app_context,
                    RootScreenType::RootScreenTokenSearch,
                );
            }
        }

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        // Main panel
        CentralPanel::default().show(ctx, |ui| {
            match self.tokens_subscreen {
                TokensSubscreen::MyTokens => {
                    let tokens = self.my_tokens.lock().unwrap().clone();
                    if tokens.is_empty() {
                        // If no tokens, show a “no tokens found” message
                        action |= self.render_no_owned_tokens(ui);
                    } else {
                        // Are we showing details for a selected token?
                        if self.selected_token_id.is_some() {
                            // Render detail view for one token
                            action |= self.render_token_details(ui, &tokens);
                        } else {
                            // Otherwise, show the list of all tokens
                            self.render_token_list(ui, &tokens);
                        }
                    }
                }
                TokensSubscreen::SearchTokens => {
                    action |= self.render_token_search(ui);
                }
            }

            // If we are refreshing, show a spinner at the bottom
            if let RefreshingStatus::Refreshing(start_time) = self.refreshing_status {
                ui.add_space(5.0);
                let now = Utc::now().timestamp() as u64;
                let elapsed = now - start_time;
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(format!("Refreshing... Time so far: {}", elapsed));
                    ui.add(egui::widgets::Spinner::default().color(Color32::from_rgb(0, 128, 255)));
                });
                ui.add_space(10.0);
            }

            // If there's a backend message, show it at the bottom
            if let Some((msg, msg_type, timestamp)) = self.backend_message.clone() {
                let color = match msg_type {
                    MessageType::Error => Color32::DARK_RED,
                    MessageType::Info => Color32::BLACK,
                    MessageType::Success => Color32::DARK_GREEN,
                };
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

            if self.confirm_remove_token_popup {
                self.show_remove_token_popup(ui);
            }
        });

        // Post-processing on user actions
        match action {
            AppAction::BackendTask(BackendTask::TokenTask(TokenTask::QueryMyTokenBalances)) => {
                self.refreshing_status =
                    RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
            }
            AppAction::BackendTask(BackendTask::TokenTask(TokenTask::QueryTokensByKeyword(_))) => {
                self.refreshing_status =
                    RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
            }
            AppAction::SetMainScreen(_) => {
                self.refreshing_status = RefreshingStatus::NotRefreshing;
                self.selected_token_id = None;
            }
            AppAction::Custom(ref s) if s == "Back to tokens" => {
                self.selected_token_id = None;
            }
            _ => {}
        }

        if action == AppAction::None {
            if let Some(bt) = self.pending_backend_task.take() {
                action = AppAction::BackendTask(bt);
            }
        }
        action
    }
}
