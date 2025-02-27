use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Utc};
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::SecurityLevel;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, CentralPanel, Color32, Context, Frame, Margin, Ui};
use egui::{Align, RichText};
use egui_extras::{Column, TableBuilder};

use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
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
    TokenCreator,
}

impl TokensSubscreen {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::MyTokens => "My Tokens",
            Self::SearchTokens => "Search Tokens",
            Self::TokenCreator => "Token Creator",
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

#[derive(Debug, PartialEq)]
pub enum TokenCreatorStatus {
    NotStarted,
    WaitingForResult(u64),
    Complete,
    ErrorMessage(String),
}

impl Default for TokenCreatorStatus {
    fn default() -> Self {
        Self::NotStarted
    }
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
    pub selected_token_id: Option<Identifier>,
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
    confirm_remove_identity_token_balance_popup: bool,
    identity_token_balance_to_remove: Option<IdentityTokenBalance>,
    confirm_remove_token_popup: bool,
    token_to_remove: Option<Identifier>,

    /// Token Creator
    selected_identity: Option<QualifiedIdentity>,
    selected_key: Option<IdentityPublicKey>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    token_name_input: String,
    should_capitalize_input: bool,
    decimals_input: String,
    base_supply_input: String,
    max_supply_input: String,
    start_as_paused_input: bool,
    show_advanced_creator_settings: bool,
    show_token_creator_confirmation_popup: bool,
    token_creator_status: TokenCreatorStatus,
    token_creator_error_message: Option<String>,
    token_keeps_history: bool,
    authorized_manual_mint: AuthorizedActionTakers,
    manual_mint_authorized_identity: Option<String>,
    manual_mint_authorized_group: Option<String>,
    authorized_manual_burn: AuthorizedActionTakers,
    manual_burn_authorized_identity: Option<String>,
    manual_burn_authorized_group: Option<String>,
    authorized_freeze: AuthorizedActionTakers,
    freeze_authorized_identity: Option<String>,
    freeze_authorized_group: Option<String>,
    authorized_unfreeze: AuthorizedActionTakers,
    unfreeze_authorized_identity: Option<String>,
    unfreeze_authorized_group: Option<String>,
    authorized_destroy_frozen_funds: AuthorizedActionTakers,
    destroy_frozen_funds_authorized_identity: Option<String>,
    destroy_frozen_funds_authorized_group: Option<String>,
    authorized_pause_resume: AuthorizedActionTakers,
    pause_resume_authorized_identity: Option<String>,
    pause_resume_authorized_group: Option<String>,
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
            confirm_remove_identity_token_balance_popup: false,
            identity_token_balance_to_remove: None,
            confirm_remove_token_popup: false,
            token_to_remove: None,

            // Token Creator
            selected_identity: None,
            selected_key: None,
            selected_wallet: None,
            wallet_password: String::new(),
            show_password: false,
            show_advanced_creator_settings: false,
            show_token_creator_confirmation_popup: false,
            token_creator_status: TokenCreatorStatus::NotStarted,
            token_creator_error_message: None,
            token_name_input: String::new(),
            should_capitalize_input: false,
            decimals_input: "8".to_string(),
            base_supply_input: "1000000".to_string(),
            max_supply_input: "5000000".to_string(),
            start_as_paused_input: false,
            token_keeps_history: false,
            authorized_manual_mint: AuthorizedActionTakers::NoOne,
            manual_mint_authorized_identity: None,
            manual_mint_authorized_group: None,
            authorized_manual_burn: AuthorizedActionTakers::NoOne,
            manual_burn_authorized_identity: None,
            manual_burn_authorized_group: None,
            authorized_freeze: AuthorizedActionTakers::NoOne,
            freeze_authorized_identity: None,
            freeze_authorized_group: None,
            authorized_unfreeze: AuthorizedActionTakers::NoOne,
            unfreeze_authorized_identity: None,
            unfreeze_authorized_group: None,
            authorized_destroy_frozen_funds: AuthorizedActionTakers::NoOne,
            destroy_frozen_funds_authorized_identity: None,
            destroy_frozen_funds_authorized_group: None,
            authorized_pause_resume: AuthorizedActionTakers::NoOne,
            pause_resume_authorized_identity: None,
            pause_resume_authorized_group: None,
        };

        if let Ok(saved_ids) = screen.app_context.db.load_token_order() {
            screen.reorder_vec_to(saved_ids);
            screen.use_custom_order = true;
        }

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
                            .column(Column::initial(80.0).resizable(true)) // Actions
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
                                header.col(|ui| {
                                    ui.label("Actions");
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
                                        row.col(|ui| {
                                            // Remove
                                            if ui
                                                .button("X")
                                                .on_hover_text("Remove token from DET")
                                                .clicked()
                                            {
                                                self.confirm_remove_token_popup = true;
                                                self.token_to_remove = Some(token_id.clone());
                                            }
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
                                                // Remove
                                                if ui
                                                    .button("X")
                                                    .on_hover_text(
                                                        "Remove identity token balance from DET",
                                                    )
                                                    .clicked()
                                                {
                                                    self.confirm_remove_identity_token_balance_popup = true;
                                                    self.identity_token_balance_to_remove = Some(itb.clone());
                                                }

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

    pub fn render_token_creator(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;

        // 1) If we've successfully completed contract creation, show a success UI
        if self.token_creator_status == TokenCreatorStatus::Complete {
            self.render_token_creator_success_screen(ui);
            return action;
        }

        // 2) Choose identity & key
        //    We'll show a dropdown of local QualifiedIdentities, then a sub-dropdown of keys

        // Identity selection
        ui.add_space(10.0);
        let all_identities = match self.app_context.load_local_qualified_identities() {
            Ok(ids) => ids,
            Err(_) => {
                ui.colored_label(egui::Color32::RED, "Error loading identities from local DB");
                return action;
            }
        };
        if all_identities.is_empty() {
            ui.colored_label(
                        Color32::DARK_RED,
                        "No identities loaded. Please load or create one to register the token contract with first.",
                    );
            return action;
        }

        ui.heading("1. Select an identity and key to register the token contract with:");
        ui.add_space(5.0);

        ui.horizontal(|ui| {
            ui.label("Identity:");
            egui::ComboBox::from_id_salt("token_creator_identity_selector")
                .selected_text(
                    self.selected_identity
                        .as_ref()
                        .map(|qi| {
                            qi.alias
                                .clone()
                                .unwrap_or_else(|| qi.identity.id().to_string(Encoding::Base58))
                        })
                        .unwrap_or_else(|| "Select Identity".to_owned()),
                )
                .show_ui(ui, |ui| {
                    for identity in all_identities.iter() {
                        let display = identity
                            .alias
                            .clone()
                            .unwrap_or_else(|| identity.identity.id().to_string(Encoding::Base58));
                        if ui
                            .selectable_label(
                                Some(identity) == self.selected_identity.as_ref(),
                                display,
                            )
                            .clicked()
                        {
                            // On select, store it
                            self.selected_identity = Some(identity.clone());
                            // Clear the selected key & wallet
                            self.selected_key = None;
                            self.selected_wallet = None;
                            self.token_creator_error_message = None;
                        }
                    }
                });
        });

        // Key selection
        ui.add_space(3.0);
        if let Some(ref qid) = self.selected_identity {
            // Attempt to list available keys (only auth keys in normal mode)
            let keys = if self.app_context.developer_mode {
                qid.identity
                    .public_keys()
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                qid.available_authentication_keys()
                    .into_iter()
                    .filter_map(|k| {
                        if k.identity_public_key.security_level() == SecurityLevel::CRITICAL
                            || k.identity_public_key.security_level() == SecurityLevel::HIGH
                        {
                            Some(k.identity_public_key.clone())
                        } else {
                            None
                        }
                    })
                    .collect()
            };

            ui.horizontal(|ui| {
                ui.label("Key:");
                egui::ComboBox::from_id_salt("token_creator_key_selector")
                    .selected_text(match &self.selected_key {
                        Some(k) => format!(
                            "Key {} (Purpose: {:?}, Security Level: {:?})",
                            k.id(),
                            k.purpose(),
                            k.security_level()
                        ),
                        None => "Select Key".to_owned(),
                    })
                    .show_ui(ui, |ui| {
                        for k in keys {
                            let label = format!(
                                "Key {} (Purpose: {:?}, Security Level: {:?})",
                                k.id(),
                                k.purpose(),
                                k.security_level()
                            );
                            if ui
                                .selectable_label(
                                    Some(k.id()) == self.selected_key.as_ref().map(|kk| kk.id()),
                                    label,
                                )
                                .clicked()
                            {
                                self.selected_key = Some(k.clone());

                                // If the key belongs to a wallet, set that wallet reference:
                                self.selected_wallet = crate::ui::identities::get_selected_wallet(
                                    qid,
                                    None,
                                    Some(&k),
                                    &mut self.token_creator_error_message,
                                );
                            }
                        }
                    });
            });
        }

        if self.selected_key.is_none() {
            return action;
        }

        ui.add_space(8.0);
        ui.separator();

        // 3) If the wallet is locked, show unlock
        //    But only do this step if we actually have a wallet reference:
        let mut need_unlock = false;
        let mut just_unlocked = false;

        if let Some(_) = self.selected_wallet {
            let (n, j) = self.render_wallet_unlock_if_needed(ui);
            need_unlock = n;
            just_unlocked = j;
        }

        if need_unlock && !just_unlocked {
            // We must wait for unlock before continuing
            return action;
        }

        // 4) Show input fields for token name, decimals, base supply, etc.
        ui.add_space(8.0);
        ui.heading("2. Enter basic token info:");
        ui.add_space(4.0);

        // Token name
        ui.horizontal(|ui| {
            ui.label("Token Name:");
            ui.text_edit_singleline(&mut self.token_name_input);
        });

        // Base Supply
        ui.horizontal(|ui| {
            ui.label("Base Supply:");
            ui.text_edit_singleline(&mut self.base_supply_input);
        });

        // Max Supply
        ui.horizontal(|ui| {
            ui.label("Max Supply:");
            ui.text_edit_singleline(&mut self.max_supply_input);
        });

        // Name should be capitalized
        ui.add_space(3.0);
        ui.checkbox(
            &mut self.should_capitalize_input,
            "Name should be capitalized",
        );

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        // 5) Advanced settings toggle
        ui.collapsing("Advanced Settings", |ui| {
            // Start as paused
            ui.checkbox(&mut self.start_as_paused_input, "Start as paused");

            // 1) Keep history?
            ui.checkbox(&mut self.token_keeps_history, "Keep history");

            // Decimals
            ui.horizontal(|ui| {
                ui.label("Decimals:");
                ui.text_edit_singleline(&mut self.decimals_input);
            });

            // 2) Manual minting
            ui.horizontal(|ui| {
                ui.label("Allow manual minting:");
                egui::ComboBox::from_id_salt("manual_mint_selector")
                    .selected_text(format!("{:?}", self.authorized_manual_mint))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.authorized_manual_mint,
                            AuthorizedActionTakers::NoOne,
                            "No One",
                        );
                        ui.selectable_value(
                            &mut self.authorized_manual_mint,
                            AuthorizedActionTakers::ContractOwner,
                            "Contract Owner",
                        );
                        // When selecting Identity, initialize with a default Identifier (or keep the current one)
                        ui.selectable_value(
                            &mut self.authorized_manual_mint,
                            AuthorizedActionTakers::Identity(Identifier::default()),
                            "Identity",
                        );
                        ui.selectable_value(
                            &mut self.authorized_manual_mint,
                            AuthorizedActionTakers::MainGroup,
                            "Main Group",
                        );
                        // When selecting Group, initialize with a default GroupContractPosition (u16)
                        ui.selectable_value(
                            &mut self.authorized_manual_mint,
                            AuthorizedActionTakers::Group(0),
                            "Group",
                        );
                    });
                // If the selected variant requires extra input, render an adjacent field:
                match &mut self.authorized_manual_mint {
                    AuthorizedActionTakers::Identity(_) => {
                        // Initialize if needed
                        if self.manual_mint_authorized_identity.is_none() {
                            self.manual_mint_authorized_identity = Some(String::new());
                        }
                        if let Some(ref mut id) = self.manual_mint_authorized_identity {
                            ui.add(egui::TextEdit::singleline(id).hint_text("Enter base58 id"));
                        }
                    }
                    AuthorizedActionTakers::Group(_) => {
                        // Initialize if needed
                        if self.manual_mint_authorized_group.is_none() {
                            self.manual_mint_authorized_group = Some(0.to_string());
                        }
                        if let Some(ref mut group) = self.manual_mint_authorized_group {
                            ui.add(
                                egui::TextEdit::singleline(group)
                                    .hint_text("Enter group contract position"),
                            );
                        }
                    }
                    _ => {}
                }
            });

            // 2) Manual burning
            ui.horizontal(|ui| {
                ui.label("Allow manual burning:");
                egui::ComboBox::from_id_salt("manual_burn_selector")
                    .selected_text(format!("{:?}", self.authorized_manual_burn))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.authorized_manual_burn,
                            AuthorizedActionTakers::NoOne,
                            "No One",
                        );
                        ui.selectable_value(
                            &mut self.authorized_manual_burn,
                            AuthorizedActionTakers::ContractOwner,
                            "Contract Owner",
                        );
                        ui.selectable_value(
                            &mut self.authorized_manual_burn,
                            AuthorizedActionTakers::Identity(Identifier::default()),
                            "Identity",
                        );
                        ui.selectable_value(
                            &mut self.authorized_manual_burn,
                            AuthorizedActionTakers::MainGroup,
                            "Main Group",
                        );
                        ui.selectable_value(
                            &mut self.authorized_manual_burn,
                            AuthorizedActionTakers::Group(0),
                            "Group",
                        );
                    });
                match &mut self.authorized_manual_burn {
                    AuthorizedActionTakers::Identity(_) => {
                        // Initialize if needed
                        if self.manual_burn_authorized_identity.is_none() {
                            self.manual_burn_authorized_identity = Some(String::new());
                        }
                        if let Some(ref mut id) = self.manual_burn_authorized_identity {
                            ui.add(egui::TextEdit::singleline(id).hint_text("Enter base58 id"));
                        }
                    }
                    AuthorizedActionTakers::Group(_) => {
                        // Initialize if needed
                        if self.manual_burn_authorized_group.is_none() {
                            self.manual_burn_authorized_group = Some(0.to_string());
                        }
                        if let Some(ref mut group) = self.manual_burn_authorized_group {
                            ui.add(
                                egui::TextEdit::singleline(group)
                                    .hint_text("Enter group contract position"),
                            );
                        }
                    }
                    _ => {}
                }
            });

            // 3) Freeze/Unfreeze (Freeze example)
            ui.horizontal(|ui| {
                ui.label("Allow identity freeze:");
                egui::ComboBox::from_id_salt("freeze_selector")
                    .selected_text(format!("{:?}", self.authorized_freeze))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.authorized_freeze,
                            AuthorizedActionTakers::NoOne,
                            "No One",
                        );
                        ui.selectable_value(
                            &mut self.authorized_freeze,
                            AuthorizedActionTakers::ContractOwner,
                            "Contract Owner",
                        );
                        ui.selectable_value(
                            &mut self.authorized_freeze,
                            AuthorizedActionTakers::Identity(Identifier::default()),
                            "Identity",
                        );
                        ui.selectable_value(
                            &mut self.authorized_freeze,
                            AuthorizedActionTakers::MainGroup,
                            "Main Group",
                        );
                        ui.selectable_value(
                            &mut self.authorized_freeze,
                            AuthorizedActionTakers::Group(0),
                            "Group",
                        );
                    });
                match &mut self.authorized_freeze {
                    AuthorizedActionTakers::Identity(_) => {
                        // Initialize if needed
                        if self.freeze_authorized_identity.is_none() {
                            self.freeze_authorized_identity = Some(String::new());
                        }
                        if let Some(ref mut id) = self.freeze_authorized_identity {
                            ui.add(egui::TextEdit::singleline(id).hint_text("Enter base58 id"));
                        }
                    }
                    AuthorizedActionTakers::Group(_) => {
                        // Initialize if needed
                        if self.freeze_authorized_group.is_none() {
                            self.freeze_authorized_group = Some(0.to_string());
                        }
                        if let Some(ref mut group) = self.freeze_authorized_group {
                            ui.add(
                                egui::TextEdit::singleline(group)
                                    .hint_text("Enter group contract position"),
                            );
                        }
                    }
                    _ => {}
                }
            });

            // 3) Freeze/Unfreeze (Unfreeze example)
            ui.horizontal(|ui| {
                ui.label("Allow identity unfreeze:");
                egui::ComboBox::from_id_salt("unfreeze_selector")
                    .selected_text(format!("{:?}", self.authorized_unfreeze))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.authorized_unfreeze,
                            AuthorizedActionTakers::NoOne,
                            "No One",
                        );
                        ui.selectable_value(
                            &mut self.authorized_unfreeze,
                            AuthorizedActionTakers::ContractOwner,
                            "Contract Owner",
                        );
                        ui.selectable_value(
                            &mut self.authorized_unfreeze,
                            AuthorizedActionTakers::Identity(Identifier::default()),
                            "Identity",
                        );
                        ui.selectable_value(
                            &mut self.authorized_unfreeze,
                            AuthorizedActionTakers::MainGroup,
                            "Main Group",
                        );
                        ui.selectable_value(
                            &mut self.authorized_unfreeze,
                            AuthorizedActionTakers::Group(0),
                            "Group",
                        );
                    });
                match &mut self.authorized_unfreeze {
                    AuthorizedActionTakers::Identity(_) => {
                        // Initialize if needed
                        if self.unfreeze_authorized_identity.is_none() {
                            self.unfreeze_authorized_identity = Some(String::new());
                        }
                        if let Some(ref mut id) = self.unfreeze_authorized_identity {
                            ui.add(egui::TextEdit::singleline(id).hint_text("Enter base58 id"));
                        }
                    }
                    AuthorizedActionTakers::Group(_) => {
                        // Initialize if needed
                        if self.unfreeze_authorized_group.is_none() {
                            self.unfreeze_authorized_group = Some(0.to_string());
                        }
                        if let Some(ref mut group) = self.unfreeze_authorized_group {
                            ui.add(
                                egui::TextEdit::singleline(group)
                                    .hint_text("Enter group contract position"),
                            );
                        }
                    }
                    _ => {}
                }
            });

            // 4) Destroy Frozen Funds
            ui.horizontal(|ui| {
                ui.label("Allow destroy frozen identity funds:");
                egui::ComboBox::from_id_salt("destroy_frozen_selector")
                    .selected_text(format!("{:?}", self.authorized_destroy_frozen_funds))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.authorized_destroy_frozen_funds,
                            AuthorizedActionTakers::NoOne,
                            "No One",
                        );
                        ui.selectable_value(
                            &mut self.authorized_destroy_frozen_funds,
                            AuthorizedActionTakers::ContractOwner,
                            "Contract Owner",
                        );
                        ui.selectable_value(
                            &mut self.authorized_destroy_frozen_funds,
                            AuthorizedActionTakers::Identity(Identifier::default()),
                            "Identity",
                        );
                        ui.selectable_value(
                            &mut self.authorized_destroy_frozen_funds,
                            AuthorizedActionTakers::MainGroup,
                            "Main Group",
                        );
                        ui.selectable_value(
                            &mut self.authorized_destroy_frozen_funds,
                            AuthorizedActionTakers::Group(0),
                            "Group",
                        );
                    });
                match &mut self.authorized_destroy_frozen_funds {
                    AuthorizedActionTakers::Identity(_) => {
                        // Initialize if needed
                        if self.destroy_frozen_funds_authorized_identity.is_none() {
                            self.destroy_frozen_funds_authorized_identity = Some(String::new());
                        }
                        if let Some(ref mut id) = self.destroy_frozen_funds_authorized_identity {
                            ui.add(egui::TextEdit::singleline(id).hint_text("Enter base58 id"));
                        }
                    }
                    AuthorizedActionTakers::Group(_) => {
                        // Initialize if needed
                        if self.destroy_frozen_funds_authorized_group.is_none() {
                            self.destroy_frozen_funds_authorized_group = Some(0.to_string());
                        }
                        if let Some(ref mut group) = self.destroy_frozen_funds_authorized_group {
                            ui.add(
                                egui::TextEdit::singleline(group)
                                    .hint_text("Enter group contract position"),
                            );
                        }
                    }
                    _ => {}
                }
            });

            // Token pause and resume
            ui.horizontal(|ui| {
                ui.label("Allow token pause and resume:");
                egui::ComboBox::from_id_salt("pause_resume_selector")
                    .selected_text(format!("{:?}", self.authorized_pause_resume))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.authorized_pause_resume,
                            AuthorizedActionTakers::NoOne,
                            "No One",
                        );
                        ui.selectable_value(
                            &mut self.authorized_pause_resume,
                            AuthorizedActionTakers::ContractOwner,
                            "Contract Owner",
                        );
                        ui.selectable_value(
                            &mut self.authorized_pause_resume,
                            AuthorizedActionTakers::Identity(Identifier::default()),
                            "Identity",
                        );
                        ui.selectable_value(
                            &mut self.authorized_pause_resume,
                            AuthorizedActionTakers::MainGroup,
                            "Main Group",
                        );
                        ui.selectable_value(
                            &mut self.authorized_pause_resume,
                            AuthorizedActionTakers::Group(0),
                            "Group",
                        );
                    });
                match &mut self.authorized_pause_resume {
                    AuthorizedActionTakers::Identity(_) => {
                        // Initialize if needed
                        if self.pause_resume_authorized_identity.is_none() {
                            self.pause_resume_authorized_identity = Some(String::new());
                        }
                        if let Some(ref mut id) = self.pause_resume_authorized_identity {
                            ui.add(egui::TextEdit::singleline(id).hint_text("Enter base58 id"));
                        }
                    }
                    AuthorizedActionTakers::Group(_) => {
                        // Initialize if needed
                        if self.pause_resume_authorized_group.is_none() {
                            self.pause_resume_authorized_group = Some(0.to_string());
                        }
                        if let Some(ref mut group) = self.pause_resume_authorized_group {
                            ui.add(
                                egui::TextEdit::singleline(group)
                                    .hint_text("Enter group contract position"),
                            );
                        }
                    }
                    _ => {}
                }
            });
        });

        // 6) "Register Token Contract" button
        ui.add_space(8.0);
        let button =
            egui::Button::new(RichText::new("Register Token Contract").color(Color32::WHITE))
                .fill(Color32::from_rgb(0, 128, 255))
                .frame(true)
                .rounding(3.0);
        if ui.add(button).clicked() {
            // Validate input & if valid, show confirmation
            self.show_token_creator_confirmation_popup = true;
        }

        // 7) If the user pressed "Register Token Contract," show a popup confirmation
        if self.show_token_creator_confirmation_popup {
            action |= self.render_token_creator_confirmation_popup(ui);
        }

        // 8) If we are waiting, show spinner / time elapsed
        if let TokenCreatorStatus::WaitingForResult(start_time) = self.token_creator_status {
            let now = chrono::Utc::now().timestamp() as u64;
            let elapsed = now - start_time;
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Registering token contract... elapsed {}s",
                    elapsed
                ));
                ui.add(egui::widgets::Spinner::default());
            });
        }

        // Show an error if we have one
        if let Some(err_msg) = &self.token_creator_error_message {
            ui.add_space(10.0);
            ui.colored_label(egui::Color32::RED, format!("Error: {err_msg}"));
            ui.add_space(10.0);
        }

        action
    }

    /// Shows a popup "Are you sure?" for creating the token contract
    fn render_token_creator_confirmation_popup(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut is_open = true;

        egui::Window::new("Confirm Token Contract Registration")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                ui.label(
                    "Are you sure you want to register a new token contract with these settings?",
                );
                ui.monospace(format!(
                    "Name: {}\nDecimals: {}\nBase Supply: {}\nMax Supply: {}\nPaused: {}",
                    self.token_name_input,
                    self.decimals_input,
                    self.base_supply_input,
                    self.max_supply_input,
                    self.start_as_paused_input
                ));

                ui.add_space(10.0);

                // Confirm
                if ui.button("Confirm").clicked() {
                    // Attempt to parse fields
                    let decimals = if let Ok(dec) = self.decimals_input.parse::<u8>() {
                        dec
                    } else {
                        self.token_creator_error_message = Some("Invalid decimals".to_string());
                        self.show_token_creator_confirmation_popup = false;
                        return;
                    };
                    let base_supply = if let Ok(base) = self.base_supply_input.parse::<u64>() {
                        base
                    } else {
                        self.token_creator_error_message = Some("Invalid base supply".to_string());
                        self.show_token_creator_confirmation_popup = false;
                        return;
                    };
                    let max_supply = if let Ok(max) = self.max_supply_input.parse::<u64>() {
                        max
                    } else {
                        self.token_creator_error_message = Some("Invalid max supply".to_string());
                        self.show_token_creator_confirmation_popup = false;
                        return;
                    };

                    // We now dispatch a backend task for actually registering the contract.
                    use crate::app::BackendTasksExecutionMode;
                    use crate::backend_task::tokens::TokenTask;
                    use crate::backend_task::BackendTask;

                    // We'll switch status to "Waiting"
                    self.token_creator_status =
                        TokenCreatorStatus::WaitingForResult(chrono::Utc::now().timestamp() as u64);
                    self.show_token_creator_confirmation_popup = false;

                    // Build a new DataContract on the fly (or ask the backend task to do it).
                    // For example:
                    let identity = self.selected_identity.clone().unwrap();
                    let key = self.selected_key.clone().unwrap();

                    let start_paused = self.start_as_paused_input;
                    let token_name = self.token_name_input.clone();

                    match self.authorized_manual_mint {
                        AuthorizedActionTakers::Identity(id) => {
                            if let Some(ref id_str) = self.manual_mint_authorized_identity {
                                if let Ok(id) = Identifier::from_string(id_str, Encoding::Base58) {
                                    self.authorized_manual_mint = AuthorizedActionTakers::Identity(id);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid base58 identifier for manual mint authorized identity".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        AuthorizedActionTakers::Group(_) => {
                            if let Some(ref group_str) = self.manual_mint_authorized_group {
                                if let Ok(group) = group_str.parse::<u16>() {
                                    self.authorized_manual_mint = AuthorizedActionTakers::Group(group);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid group contract position for manual mint authorized group".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }
                    match self.authorized_manual_burn {
                        AuthorizedActionTakers::Identity(id) => {
                            if let Some(ref id_str) = self.manual_burn_authorized_identity {
                                if let Ok(id) = Identifier::from_string(id_str, Encoding::Base58) {
                                    self.authorized_manual_burn = AuthorizedActionTakers::Identity(id);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid base58 identifier for manual burn authorized identity".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        AuthorizedActionTakers::Group(_) => {
                            if let Some(ref group_str) = self.manual_burn_authorized_group {
                                if let Ok(group) = group_str.parse::<u16>() {
                                    self.authorized_manual_burn = AuthorizedActionTakers::Group(group);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid group contract position for manual burn authorized group".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }
                    match self.authorized_freeze {
                        AuthorizedActionTakers::Identity(id) => {
                            if let Some(ref id_str) = self.freeze_authorized_identity {
                                if let Ok(id) = Identifier::from_string(id_str, Encoding::Base58) {
                                    self.authorized_freeze = AuthorizedActionTakers::Identity(id);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid base58 identifier for freeze authorized identity".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        AuthorizedActionTakers::Group(_) => {
                            if let Some(ref group_str) = self.freeze_authorized_group {
                                if let Ok(group) = group_str.parse::<u16>() {
                                    self.authorized_freeze = AuthorizedActionTakers::Group(group);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid group contract position for freeze authorized group".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }
                    match self.authorized_unfreeze {
                        AuthorizedActionTakers::Identity(id) => {
                            if let Some(ref id_str) = self.unfreeze_authorized_identity {
                                if let Ok(id) = Identifier::from_string(id_str, Encoding::Base58) {
                                    self.authorized_unfreeze = AuthorizedActionTakers::Identity(id);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid base58 identifier for unfreeze authorized identity".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        AuthorizedActionTakers::Group(_) => {
                            if let Some(ref group_str) = self.unfreeze_authorized_group {
                                if let Ok(group) = group_str.parse::<u16>() {
                                    self.authorized_unfreeze = AuthorizedActionTakers::Group(group);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid group contract position for unfreeze authorized group".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }
                    match self.authorized_destroy_frozen_funds {
                        AuthorizedActionTakers::Identity(id) => {
                            if let Some(ref id_str) = self.destroy_frozen_funds_authorized_identity {
                                if let Ok(id) = Identifier::from_string(id_str, Encoding::Base58) {
                                    self.authorized_destroy_frozen_funds = AuthorizedActionTakers::Identity(id);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid base58 identifier for destroy frozen funds authorized identity".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        AuthorizedActionTakers::Group(_) => {
                            if let Some(ref group_str) = self.destroy_frozen_funds_authorized_group {
                                if let Ok(group) = group_str.parse::<u16>() {
                                    self.authorized_destroy_frozen_funds = AuthorizedActionTakers::Group(group);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid group contract position for destroy frozen funds authorized group".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }
                    match self.authorized_pause_resume {
                        AuthorizedActionTakers::Identity(id) => {
                            if let Some(ref id_str) = self.pause_resume_authorized_identity {
                                if let Ok(id) = Identifier::from_string(id_str, Encoding::Base58) {
                                    self.authorized_pause_resume = AuthorizedActionTakers::Identity(id);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid base58 identifier for pause resume authorized identity".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        AuthorizedActionTakers::Group(_) => {
                            if let Some(ref group_str) = self.pause_resume_authorized_group {
                                if let Ok(group) = group_str.parse::<u16>() {
                                    self.authorized_pause_resume = AuthorizedActionTakers::Group(group);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid group contract position for pause resume authorized group".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }

                    let tasks = vec![
                        BackendTask::TokenTask(TokenTask::RegisterTokenContract {
                            identity,
                            signing_key: key,
                            token_name,
                            should_capitalize: self.should_capitalize_input,
                            decimals,
                            base_supply,
                            max_supply,
                            start_paused,
                            keeps_history: self.token_keeps_history,
                            manual_mint_authorized: self.authorized_manual_mint.clone(),
                            manual_burn_authorized: self.authorized_manual_burn.clone(),
                            freeze_authorized: self.authorized_freeze.clone(),
                            unfreeze_authorized: self.authorized_unfreeze.clone(),
                            destroy_frozen_funds_authorized: self
                                .authorized_destroy_frozen_funds
                                .clone(),
                            pause_and_resume_authorized: self.authorized_pause_resume.clone(),
                        }),
                        BackendTask::TokenTask(TokenTask::QueryMyTokenBalances),
                    ];

                    action = AppAction::BackendTasks(tasks, BackendTasksExecutionMode::Sequential);
                }

                // Cancel
                if ui.button("Cancel").clicked() {
                    self.show_token_creator_confirmation_popup = false;
                }
            });

        if !is_open {
            self.show_token_creator_confirmation_popup = false;
        }

        action
    }

    /// Once the contract creation is done (status=Complete),
    /// render a simple "Success" screen
    fn render_token_creator_success_screen(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);
            ui.heading("Token Contract Created Successfully! 🎉");
            ui.add_space(10.0);
            if ui.button("Back").clicked() {
                self.reset_token_creator();
            }
        });
    }

    fn reset_token_creator(&mut self) {
        self.selected_identity = None;
        self.selected_key = None;
        self.token_creator_status = TokenCreatorStatus::NotStarted;
        self.token_name_input = "".to_string();
        self.decimals_input = "8".to_string();
        self.base_supply_input = "1000000".to_string();
        self.max_supply_input = "5000000".to_string();
        self.start_as_paused_input = false;
        self.should_capitalize_input = false;
        self.token_keeps_history = false;
        self.authorized_manual_mint = AuthorizedActionTakers::NoOne;
        self.authorized_manual_burn = AuthorizedActionTakers::NoOne;
        self.authorized_freeze = AuthorizedActionTakers::NoOne;
        self.authorized_unfreeze = AuthorizedActionTakers::NoOne;
        self.authorized_destroy_frozen_funds = AuthorizedActionTakers::NoOne;
        self.authorized_pause_resume = AuthorizedActionTakers::NoOne;
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
                TokensSubscreen::TokenCreator => {
                    ui.label(
                        egui::RichText::new("Cannot render token creator for some reason")
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
                        TokensSubscreen::TokenCreator => {
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

    fn show_remove_identity_token_balance_popup(&mut self, ui: &mut egui::Ui) {
        // If no token is set, nothing to confirm
        let token_to_remove = match &self.identity_token_balance_to_remove {
            Some(token) => token.clone(),
            None => {
                self.confirm_remove_identity_token_balance_popup = false;
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
                        self.confirm_remove_identity_token_balance_popup = false;
                        self.identity_token_balance_to_remove = None;
                    } else {
                        self.confirm_remove_identity_token_balance_popup = false;
                        self.identity_token_balance_to_remove = None;
                        self.refresh();
                    };
                }

                // Cancel button
                if ui.button("Cancel").clicked() {
                    self.confirm_remove_identity_token_balance_popup = false;
                    self.identity_token_balance_to_remove = None;
                }
            });

        // If user closes the popup window (the [x] button), also reset state
        if !is_open {
            self.confirm_remove_identity_token_balance_popup = false;
            self.identity_token_balance_to_remove = None;
        }
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

        // find the token name from one of the identity token balances in my tokens
        let token_name = self
            .my_tokens
            .lock()
            .unwrap()
            .iter()
            .find_map(|t| {
                if t.token_identifier == token_to_remove {
                    Some(t.token_name.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| token_to_remove.to_string(Encoding::Base58));

        let mut is_open = true;

        egui::Window::new("Confirm Remove Token")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                ui.label(format!(
                    "Are you sure you want to remove token \"{}\" for all identities?",
                    token_name,
                ));

                // Confirm button
                if ui.button("Confirm").clicked() {
                    for identity in self
                        .app_context
                        .load_local_qualified_identities()
                        .expect("Expected to load local qualified identities")
                    {
                        if let Err(e) = self.app_context.remove_token_balance(
                            token_to_remove.clone(),
                            identity.identity.id().clone(),
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
                        }
                    }
                }

                // Cancel button
                if ui.button("Cancel").clicked() {
                    self.confirm_remove_token_popup = false;
                    self.token_to_remove = None;
                }
            });

        // If user closes the popup window (the [x] button), also reset state
        if !is_open {
            self.confirm_remove_identity_token_balance_popup = false;
            self.identity_token_balance_to_remove = None;
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
        self.selected_token_id = None;
        self.my_tokens = Arc::new(Mutex::new(
            self.app_context
                .identity_token_balances()
                .unwrap_or_default(),
        ));
    }

    fn display_message(&mut self, msg: &str, msg_type: MessageType) {
        if self.tokens_subscreen == TokensSubscreen::TokenCreator {
            // Handle messages from Token Creator
            if msg.contains("Successfully registered token contract") {
                self.token_creator_status = TokenCreatorStatus::Complete;
            } else if msg.contains("Error registering token contract") {
                self.token_creator_status = TokenCreatorStatus::ErrorMessage(msg.to_string());
                self.token_creator_error_message = Some(msg.to_string());
            } else {
                return;
            }
        }

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
            TokensSubscreen::TokenCreator => vec![],
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
            TokensSubscreen::TokenCreator => {
                action |= add_left_panel(
                    ctx,
                    &self.app_context,
                    RootScreenType::RootScreenTokenCreator,
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
                TokensSubscreen::TokenCreator => {
                    action |= self.render_token_creator(ui);
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

            if self.confirm_remove_identity_token_balance_popup {
                self.show_remove_identity_token_balance_popup(ui);
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
                self.reset_token_creator();
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

impl ScreenWithWalletUnlock for TokensScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.selected_wallet
    }

    fn wallet_password_ref(&self) -> &String {
        &self.wallet_password
    }

    fn wallet_password_mut(&mut self) -> &mut String {
        &mut self.wallet_password
    }

    fn show_password(&self) -> bool {
        self.show_password
    }

    fn show_password_mut(&mut self) -> &mut bool {
        &mut self.show_password
    }

    fn set_error_message(&mut self, error_message: Option<String>) {
        self.token_creator_error_message = error_message;
    }

    fn error_message(&self) -> Option<&String> {
        self.token_creator_error_message.as_ref()
    }
}
