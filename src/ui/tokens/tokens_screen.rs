use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, CentralPanel, Color32, Context, Frame, Margin, Ui};
use egui::Align;
use egui_extras::{Column, TableBuilder};

use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, ScreenLike};

/// Which DPNS sub-screen is currently showing.
#[derive(PartialEq)]
pub enum TokensSubscreen {
    MyBalances,
    TokenSearch,
}

impl TokensSubscreen {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::MyBalances => "My Balances",
            Self::TokenSearch => "Token Search",
        }
    }
}

#[derive(PartialEq)]
pub enum RefreshingStatus {
    Refreshing(u64),
    NotRefreshing,
}

/// Sorting columns
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    TokenName,
    OwnerIdentity,
    Balance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortOrder {
    Ascending,
    Descending,
}

/// The main, combined TokensScreen:
/// - Displays active/past/owned DPNS contests
/// - Allows clicking selection of votes (bulk scheduling)
/// - Allows single immediate vote or single schedule
/// - Shows scheduled votes listing
pub struct TokensScreen {
    pub app_context: Arc<AppContext>,
    user_identities: Vec<QualifiedIdentity>,
    my_tokens: Arc<Mutex<Vec<Token>>>,
    token_search_query: Option<String>,
    message: Option<(String, MessageType, DateTime<Utc>)>,
    pending_backend_task: Option<BackendTask>,
    pub tokens_subscreen: TokensSubscreen,
    refreshing_status: RefreshingStatus,

    /// Sorting
    sort_column: SortColumn,
    sort_order: SortOrder,
    use_custom_order: bool,
}

impl TokensScreen {
    pub fn new(app_context: &Arc<AppContext>, tokens_subscreen: TokensSubscreen) -> Self {
        let user_identities = app_context
            .db
            .get_local_user_identities(app_context)
            .unwrap_or_default();
        // let my_tokens =

        let mut screen = Self {
            app_context: app_context.clone(),
            user_identities,
            my_tokens,
            token_search_query: None,
            message: None,
            sort_column: SortColumn::TokenName,
            sort_order: SortOrder::Ascending,
            use_custom_order: true,
            pending_backend_task: None,
            tokens_subscreen,
            refreshing_status: RefreshingStatus::NotRefreshing,
        };

        if let Ok(saved_ids) = screen.app_context.db.load_token_order() {
            // reorder the IndexMap
            screen.reorder_map_to(saved_ids);
            screen.use_custom_order = true;
        }

        screen
    }

    /// Reorder the underlying IndexMap to match a list of IDs
    fn reorder_map_to(&self, new_order: Vec<Identifier>) {
        let mut lock = self.my_tokens.lock().unwrap();
        for (desired_idx, id) in new_order.iter().enumerate() {
            if let Some(current_idx) = lock.get_index_of(id) {
                if current_idx != desired_idx && current_idx < lock.len() {
                    lock.swap_indices(current_idx, desired_idx);
                }
            }
        }
    }

    /// Sorts a list of QIs
    fn sort_vec(&self, list: &mut [Token]) {
        list.sort_by(|a, b| {
            let ordering = match self.sort_column {
                SortColumn::Balance => {
                    let balance_a = a.balance.as_deref().unwrap_or("");
                    let balance_b = b.balance.as_deref().unwrap_or("");
                    balance_a.cmp(balance_b)
                }
                SortColumn::OwnerIdentity => {
                    let short_a: String = a
                        .identity
                        .id()
                        .to_string(a.identity_type.default_encoding())
                        .chars()
                        .take(6)
                        .collect();
                    let short_b: String = b
                        .identity
                        .id()
                        .to_string(b.identity_type.default_encoding())
                        .chars()
                        .take(6)
                        .collect();
                    short_a.cmp(&short_b)
                }
                SortColumn::TokenName => {
                    let name_a = a.token_name.to_string();
                    let name_b = b.token_name.to_string();
                    name_a.cmp(&name_b)
                }
            };
            match self.sort_order {
                SortOrder::Ascending => ordering,
                SortOrder::Descending => ordering.reverse(),
            }
        });
        let mut lock = self.my_tokens.lock().unwrap();
        *lock = list
            .iter()
            .map(|token| (token.identity.id(), token.clone()))
            .collect();
        drop(lock);
        // self.save_current_order();
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

    fn sort_my_tokens(&self, tokens: &mut Vec<Token>) {
        tokens.sort_by(|a, b| {
            let order = match self.sort_column {
                SortColumn::TokenName => a.token_name.cmp(&b.token_name),
                SortColumn::OwnerIdentity => a.owner_identity.cmp(&b.owner_identity),
                SortColumn::Balance => a.balance.cmp(&b.balance),
            };
            if self.sort_order == SortOrder::Descending {
                order.reverse()
            } else {
                order
            }
        });
    }

    /// This method merges the ephemeral-sorted `Vec` back into the IndexMap
    /// so the IndexMap is updated to the user’s currently displayed order.
    fn update_index_map_to_current_ephemeral(&self, ephemeral_list: Vec<QualifiedIdentity>) {
        let mut lock = self.my_tokens.lock().unwrap();
        // basically reorder the underlying IndexMap to match ephemeral_list
        for (desired_idx, qi) in ephemeral_list.into_iter().enumerate() {
            let id = qi.identity.id();
            if let Some(current_idx) = lock.get_index_of(&id) {
                if current_idx != desired_idx {
                    lock.swap_indices(current_idx, desired_idx);
                }
            }
        }
    }

    // Up/down reorder methods
    fn move_token_up(&mut self, identity_id: &Identifier) {
        let mut lock = self.my_tokens.lock().unwrap();
        if let Some(idx) = lock.get_index_of(identity_id) {
            if idx > 0 {
                lock.swap_indices(idx, idx - 1);
            }
        }
        drop(lock);
        self.save_current_order();
    }

    // arrow down
    fn move_token_down(&mut self, identity_id: &Identifier) {
        let mut lock = self.my_tokens.lock().unwrap();
        if let Some(idx) = lock.get_index_of(identity_id) {
            if idx + 1 < lock.len() {
                lock.swap_indices(idx, idx + 1);
            }
        }
        drop(lock);
        self.save_current_order();
    }

    // Save the current index order to DB
    fn save_current_order(&self) {
        let lock = self.my_tokens.lock().unwrap();
        let all_ids = lock.keys().cloned().collect::<Vec<_>>();
        drop(lock);
        self.app_context.db.save_identity_order(all_ids).ok();
    }

    fn render_table_my_token_balances(&mut self, ui: &mut Ui, tokens: &[Token]) -> AppAction {
        let mut action = AppAction::None;

        let mut my_tokens = tokens.to_vec();
        if !self.use_custom_order {
            self.sort_vec(&mut my_tokens);
        }

        // Allocate space for refreshing status
        let refreshing_height = 33.0;
        let mut max_scroll_height = if let RefreshingStatus::Refreshing(_) = self.refreshing_status
        {
            ui.available_height() - refreshing_height
        } else {
            ui.available_height()
        };

        // Allocate space for backend message
        let backend_message_height = 47.0;
        if let Some((_, _, _)) = self.message.clone() {
            max_scroll_height -= backend_message_height;
        }

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
                            .column(Column::initial(80.0).resizable(true)) // Token Name
                            .column(Column::initial(330.0).resizable(true)) // Owner Identity ID
                            .column(Column::initial(60.0).resizable(true)) // Balance
                            .column(Column::initial(80.0).resizable(true)) // Actions
                            .header(30.0, |mut header| {
                                header.col(|ui| {
                                    if ui.button("Token Name").clicked() {
                                        self.toggle_sort(SortColumn::TokenName);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Owner ID").clicked() {
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
                                for token in &my_tokens {
                                    body.row(25.0, |mut row| {
                                        row.col(|ui| {
                                            self.show_token_name(ui, token);
                                        });
                                        row.col(|ui| {
                                            Self::show_owner_identity_id(ui, token);
                                        });
                                        row.col(|ui| {
                                            TokensScreen::show_balance(ui, token);
                                        });
                                        row.col(|ui| {
                                            ui.spacing_mut().item_spacing.x = 3.0;

                                            ui.horizontal(|ui| {
                                                // Remove
                                                if ui.button("Remove").clicked() {
                                                    // nothing.. placeholder
                                                }
                                            });

                                            ui.horizontal(|ui| {
                                                // Up arrow
                                                let up_btn = ui.button("⬆").on_hover_text(
                                                    "Move this token up in the list",
                                                );
                                                // Down arrow
                                                let down_btn = ui.button("⬇").on_hover_text(
                                                    "Move this token down in the list",
                                                );

                                                if up_btn.clicked() {
                                                    // If we are currently sorted (not custom),
                                                    // unify the IndexMap to reflect that ephemeral sort
                                                    if !self.use_custom_order {
                                                        self.update_index_map_to_current_ephemeral(
                                                            my_tokens.clone(),
                                                        );
                                                    }
                                                    // Now do the swap
                                                    self.use_custom_order = true;
                                                    self.move_token_up(&token.id());
                                                }
                                                if down_btn.clicked() {
                                                    if !self.use_custom_order {
                                                        self.update_index_map_to_current_ephemeral(
                                                            my_tokens.clone(),
                                                        );
                                                    }
                                                    self.use_custom_order = true;
                                                    self.move_token_down(&token.id());
                                                }
                                            });
                                        });
                                    });
                                }
                            });
                    });
            });

        action
    }

    fn show_token_name(&self, ui: &mut Ui, token: &Token) {
        ui.label(token.token_name.to_string());
    }

    fn show_owner_identity_id(ui: &mut Ui, token: &Token) {
        ui.label(token.owner_identity.id().to_string());
    }

    fn show_balance(ui: &mut Ui, token: &Token) {
        ui.label(token.balance.to_string());
    }

    fn render_token_search(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            ui.label("Search for tokens by name or owner identity ID.");
            ui.add_space(10.0);

            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(self.token_search_query.get_or_insert_with(String::new));
                if ui.button("Go").clicked() {
                    let now = Utc::now().timestamp() as u64;
                    self.refreshing_status = RefreshingStatus::Refreshing(now);
                    action =
                        AppAction::BackendTask(BackendTask::TokenTask(TokenTask::QueryTokens(
                            self.token_search_query
                                .get_or_insert_with(String::new)
                                .to_string(),
                        )));
                }
            });
        });

        action
    }

    // ---------------------------
    // Rendering: Empty states
    // ---------------------------
    fn render_no_owned_tokens(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            match self.tokens_subscreen {
                TokensSubscreen::MyBalances => {
                    ui.label(
                        egui::RichText::new("No owned tokens found.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                TokensSubscreen::TokenSearch => {
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
                        TokensSubscreen::MyBalances => {
                            app_action = AppAction::BackendTask(BackendTask::TokenTask(
                                TokenTask::QueryMyTokenBalances,
                            ));
                        }
                        TokensSubscreen::TokenSearch => {
                            app_action = AppAction::Refresh;
                        }
                    }
                }
            }
        });

        app_action
    }
}

// ---------------------------
// ScreenLike implementation
// ---------------------------
impl ScreenLike for TokensScreen {
    fn refresh(&mut self) {}

    fn refresh_on_arrival(&mut self) {}

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {}

    fn display_task_result(&mut self, _backend_task_success_result: BackendTaskSuccessResult) {}

    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_error_expiration();

        // Build top-right buttons
        let mut right_buttons = match self.tokens_subscreen {
            TokensSubscreen::MyBalances => vec![(
                "Refresh",
                DesiredAppAction::BackendTask(BackendTask::TokenTask(
                    TokenTask::QueryMyTokenBalances,
                )),
            )],
            TokensSubscreen::TokenSearch => vec![("Refresh", DesiredAppAction::Refresh)],
        };

        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("DPNS", AppAction::None)],
            right_buttons,
        );

        // Left panel
        match self.tokens_subscreen {
            TokensSubscreen::MyBalances => {
                action |= add_left_panel(
                    ctx,
                    &self.app_context,
                    RootScreenType::RootScreenMyTokenBalances,
                );
            }
            TokensSubscreen::TokenSearch => {
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
            // Render sub-screen
            match self.tokens_subscreen {
                TokensSubscreen::MyBalances => {
                    let has_any = {
                        let guard = self.my_tokens.lock().unwrap();
                        !guard.is_empty()
                    };
                    if has_any {
                        self.render_table_my_token_balances(ui);
                    } else {
                        action |= self.render_no_owned_tokens(ui);
                    }
                }
                TokensSubscreen::TokenSearch => {
                    self.render_token_search(ui);
                }
            }

            // If we are refreshing, show a spinner at the bottom
            if let RefreshingStatus::Refreshing(start_time) = self.refreshing_status {
                ui.add_space(5.0);
                let now = Utc::now().timestamp() as u64;
                let elapsed = now - start_time;
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(format!("Refreshing... Time taken so far: {}", elapsed)); // Can add "time taken so far" later
                    ui.add(egui::widgets::Spinner::default().color(Color32::from_rgb(0, 128, 255)));
                });
                ui.add_space(10.0);
            }

            // If there's a backend message, show it at the bottom
            if let Some((msg, msg_type, timestamp)) = self.message.clone() {
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
        });

        // Extra handling for actions
        match action {
            AppAction::BackendTask(BackendTask::TokenTask(TokenTask::QueryMyTokenBalances)) => {
                self.refreshing_status =
                    RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
            }
            AppAction::BackendTask(BackendTask::TokenTask(TokenTask::QueryTokens(_))) => {
                self.refreshing_status =
                    RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
            }
            AppAction::SetMainScreen(_) => {
                self.refreshing_status = RefreshingStatus::NotRefreshing;
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
