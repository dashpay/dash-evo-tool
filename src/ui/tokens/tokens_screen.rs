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
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, ScreenLike};

/// A token owned by an identity.
#[derive(Clone)]
pub struct IdentityTokenBalance {
    pub token_identifier: Identifier,
    pub token_name: String,
    pub identity_id: Identifier,
    pub balance: u64,
}

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
/// - Displays token balances or a search UI
/// - Allows reordering of tokens if desired
pub struct TokensScreen {
    pub app_context: Arc<AppContext>,

    // Identities you might own; not strictly necessary but included from your original example
    user_identities: Vec<QualifiedIdentity>,

    // CHANGED: Instead of IndexMap, a simple Vec:
    my_tokens: Arc<Mutex<Vec<IdentityTokenBalance>>>,

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
            .load_local_qualified_identities()
            .unwrap_or_default();
        let my_tokens = Arc::new(Mutex::new(
            app_context.identity_token_balances().unwrap_or_default(),
        ));

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

        // CHANGED: Load your saved order from DB and reorder the Vec accordingly
        if let Ok(saved_ids) = screen.app_context.db.load_token_order() {
            screen.reorder_vec_to(saved_ids);
            screen.use_custom_order = true;
        }

        screen
    }

    // ─────────────────────────────────────────────────────────────────
    // Reordering
    // ─────────────────────────────────────────────────────────────────

    /// Reorder `my_tokens` to match a given list of token_identifiers.
    fn reorder_vec_to(&self, new_order: Vec<Identifier>) {
        let mut lock = self.my_tokens.lock().unwrap();
        for (desired_idx, id) in new_order.iter().enumerate() {
            // CHANGED: Use Vec::position:
            if let Some(current_idx) = lock.iter().position(|t| t.token_identifier == *id) {
                if current_idx != desired_idx && current_idx < lock.len() {
                    lock.swap(current_idx, desired_idx);
                }
            }
        }
    }

    /// Save the current vector's order of token IDs to the DB
    fn save_current_order(&self) {
        let lock = self.my_tokens.lock().unwrap();
        // CHANGED: gather token_identifier from each item in the Vec
        let all_ids = lock
            .iter()
            .map(|token| token.token_identifier.clone())
            .collect::<Vec<_>>();
        drop(lock);
        self.app_context.db.save_token_order(all_ids).ok();
    }

    // If user toggles away from sorting to a custom order,
    // but we've already displayed an ephemeral sort,
    // we can unify that ephemeral order with the underlying Vec
    // *before* doing an up/down swap.
    fn update_vec_to_current_ephemeral(&self, ephemeral_list: Vec<IdentityTokenBalance>) {
        let mut lock = self.my_tokens.lock().unwrap();
        // Reorder `lock` to match ephemeral_list:
        for (desired_idx, ephemeral_token) in ephemeral_list.into_iter().enumerate() {
            if let Some(current_idx) = lock
                .iter()
                .position(|t| t.token_identifier == ephemeral_token.token_identifier)
            {
                if current_idx != desired_idx {
                    lock.swap(current_idx, desired_idx);
                }
            }
        }
    }

    // Move a token up
    fn move_token_up(&mut self, token_id: &Identifier) {
        let mut lock = self.my_tokens.lock().unwrap();
        if let Some(idx) = lock.iter().position(|t| &t.token_identifier == token_id) {
            if idx > 0 {
                lock.swap(idx, idx - 1);
            }
        }
        drop(lock);
        self.save_current_order();
    }

    // Move a token down
    fn move_token_down(&mut self, token_id: &Identifier) {
        let mut lock = self.my_tokens.lock().unwrap();
        if let Some(idx) = lock.iter().position(|t| &t.token_identifier == token_id) {
            if idx + 1 < lock.len() {
                lock.swap(idx, idx + 1);
            }
        }
        drop(lock);
        self.save_current_order();
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
                SortColumn::TokenName => a.token_name.cmp(&b.token_name),
            };
            match self.sort_order {
                SortOrder::Ascending => ordering,
                SortOrder::Descending => ordering.reverse(),
            }
        });
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

    // ─────────────────────────────────────────────────────────────────
    // Rendering
    // ─────────────────────────────────────────────────────────────────

    fn render_table_my_token_balances(
        &mut self,
        ui: &mut Ui,
        tokens: &[IdentityTokenBalance],
    ) -> AppAction {
        let mut action = AppAction::None;

        let mut display_list = tokens.to_vec();
        // If user hasn't chosen to keep a custom order, do a normal sort:
        if !self.use_custom_order {
            self.sort_vec(&mut display_list);
        }

        // We'll use a scroll area for large lists.
        let mut max_scroll_height = ui.available_height();

        // If we are refreshing, we want space for that spinner, etc.
        if let RefreshingStatus::Refreshing(_) = self.refreshing_status {
            max_scroll_height -= 33.0;
        }
        // If there's a message at bottom, allocate space for that.
        if self.message.is_some() {
            max_scroll_height -= 47.0;
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
                                for token in &display_list {
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
                                            ui.spacing_mut().item_spacing.x = 3.0;

                                            ui.horizontal(|ui| {
                                                if ui.button("Remove").clicked() {
                                                    // If you need a removal, handle here
                                                }
                                            });

                                            ui.horizontal(|ui| {
                                                let up_btn = ui.button("⬆").on_hover_text(
                                                    "Move this token up in the list",
                                                );
                                                let down_btn = ui.button("⬇").on_hover_text(
                                                    "Move this token down in the list",
                                                );

                                                if up_btn.clicked() {
                                                    // CHANGED: If we are currently sorted, unify the ephemeral sort:
                                                    if !self.use_custom_order {
                                                        self.update_vec_to_current_ephemeral(
                                                            display_list.clone(),
                                                        );
                                                    }
                                                    self.use_custom_order = true;
                                                    self.move_token_up(&token.token_identifier);
                                                }
                                                if down_btn.clicked() {
                                                    if !self.use_custom_order {
                                                        self.update_vec_to_current_ephemeral(
                                                            display_list.clone(),
                                                        );
                                                    }
                                                    self.use_custom_order = true;
                                                    self.move_token_down(&token.token_identifier);
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
                                .as_ref()
                                .unwrap_or(&String::new())
                                .clone(),
                        )));
                }
            });
        });

        action
    }

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

// ─────────────────────────────────────────────────────────────────
// ScreenLike implementation
// ─────────────────────────────────────────────────────────────────
impl ScreenLike for TokensScreen {
    fn refresh(&mut self) {}

    fn refresh_on_arrival(&mut self) {}

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {}

    fn display_task_result(&mut self, _backend_task_success_result: BackendTaskSuccessResult) {}

    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_error_expiration();

        // Build top-right buttons
        let right_buttons = match self.tokens_subscreen {
            TokensSubscreen::MyBalances => vec![(
                "Refresh",
                DesiredAppAction::BackendTask(BackendTask::TokenTask(
                    TokenTask::QueryMyTokenBalances,
                )),
            )],
            TokensSubscreen::TokenSearch => vec![("Refresh", DesiredAppAction::Refresh)],
        };

        // Top panel
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
            match self.tokens_subscreen {
                TokensSubscreen::MyBalances => {
                    let tokens_empty = {
                        let guard = self.my_tokens.lock().unwrap();
                        guard.is_empty()
                    };

                    if tokens_empty {
                        action |= self.render_no_owned_tokens(ui);
                    } else {
                        let tokens = {
                            let guard = self.my_tokens.lock().unwrap();
                            guard.clone()
                        };
                        action |= self.render_table_my_token_balances(ui, &tokens);
                    }
                }
                TokensSubscreen::TokenSearch => {
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

        // Post-processing on user actions
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
