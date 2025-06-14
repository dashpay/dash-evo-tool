use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::helpers::{add_identity_key_chooser, TransactionType};
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Context, Ui};
use egui::{Color32, RichText};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ui::identities::get_selected_wallet;

use super::tokens_screen::IdentityTokenBalance;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;

fn format_token_amount(amount: u64, decimals: u8) -> String {
    if decimals == 0 {
        return amount.to_string();
    }

    let divisor = 10u64.pow(decimals as u32);
    let whole = amount / divisor;
    let fraction = amount % divisor;

    if fraction == 0 {
        whole.to_string()
    } else {
        // Format with the appropriate number of decimal places, removing trailing zeros
        let fraction_str = format!("{:0width$}", fraction, width = decimals as usize);
        let trimmed = fraction_str.trim_end_matches('0');
        format!("{}.{}", whole, trimmed)
    }
}

fn parse_token_amount(input: &str, decimals: u8) -> Result<u64, String> {
    if decimals == 0 {
        return input
            .parse::<u64>()
            .map_err(|_| "Invalid amount: must be a whole number".to_string());
    }

    let parts: Vec<&str> = input.split('.').collect();
    match parts.len() {
        1 => {
            // No decimal point, parse as whole number
            let whole = parts[0]
                .parse::<u64>()
                .map_err(|_| "Invalid amount: must be a number".to_string())?;
            let multiplier = 10u64.pow(decimals as u32);
            whole
                .checked_mul(multiplier)
                .ok_or_else(|| "Amount too large".to_string())
        }
        2 => {
            // Has decimal point
            let whole = if parts[0].is_empty() {
                0
            } else {
                parts[0]
                    .parse::<u64>()
                    .map_err(|_| "Invalid amount: whole part must be a number".to_string())?
            };

            let fraction_str = parts[1];
            if fraction_str.len() > decimals as usize {
                return Err(format!(
                    "Too many decimal places. Maximum allowed: {}",
                    decimals
                ));
            }

            // Pad with zeros if needed
            let padded_fraction = format!("{:0<width$}", fraction_str, width = decimals as usize);
            let fraction = padded_fraction
                .parse::<u64>()
                .map_err(|_| "Invalid amount: decimal part must be a number".to_string())?;

            let multiplier = 10u64.pow(decimals as u32);
            let whole_part = whole
                .checked_mul(multiplier)
                .ok_or_else(|| "Amount too large".to_string())?;

            whole_part
                .checked_add(fraction)
                .ok_or_else(|| "Amount too large".to_string())
        }
        _ => Err("Invalid amount: too many decimal points".to_string()),
    }
}

#[derive(PartialEq)]
pub enum TransferTokensStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct TransferTokensScreen {
    pub identity: QualifiedIdentity,
    pub identity_token_balance: IdentityTokenBalance,
    friend_identities: Vec<(String, Identifier)>,
    selected_friend_index: Option<usize>,
    selected_key: Option<IdentityPublicKey>,
    pub public_note: Option<String>,
    pub receiver_identity_id: String,
    pub amount: String,
    transfer_tokens_status: TransferTokensStatus,
    max_amount: u64,
    pub app_context: Arc<AppContext>,
    confirmation_popup: bool,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
}

impl TransferTokensScreen {
    pub fn new(
        identity_token_balance: IdentityTokenBalance,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let all_identities = app_context
            .load_local_qualified_identities()
            .expect("Identities not loaded");

        let friend_identities: Vec<(String, Identifier)> = all_identities
            .iter()
            .filter(|id| id.identity.id() != identity_token_balance.identity_id)
            .map(|id| {
                let alias = id
                    .alias
                    .clone()
                    .unwrap_or_else(|| id.identity.id().to_string(Encoding::Base58));
                (alias, id.identity.id())
            })
            .collect();

        let identity = all_identities
            .iter()
            .find(|identity| identity.identity.id() == identity_token_balance.identity_id)
            .expect("Identity not found")
            .clone();
        let max_amount = identity_token_balance.balance;
        let identity_clone = identity.identity.clone();
        let selected_key = identity_clone.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::CRITICAL]),
            KeyType::all_key_types().into(),
            false,
        );
        let mut error_message = None;
        let selected_wallet =
            get_selected_wallet(&identity, None, selected_key, &mut error_message);

        let (selected_friend_index, receiver_identity_id) =
            if let Some((_first, identifier)) = friend_identities.first() {
                (Some(0), identifier.to_string(Encoding::Base58))
            } else {
                (None, String::new())
            };
        Self {
            identity,
            identity_token_balance,
            friend_identities,
            selected_friend_index,
            selected_key: selected_key.cloned(),
            public_note: None,
            receiver_identity_id,
            amount: String::new(),
            transfer_tokens_status: TransferTokensStatus::NotStarted,
            max_amount,
            app_context: app_context.clone(),
            confirmation_popup: false,
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
        }
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Amount:");

            ui.text_edit_singleline(&mut self.amount);

            if ui.button("Max").clicked() {
                let decimals = self
                    .identity_token_balance
                    .token_config
                    .conventions()
                    .decimals();
                self.amount = format_token_amount(self.max_amount, decimals);
            }
        });
    }

    fn render_to_identity_input(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            // Dropdown
            egui::ComboBox::from_id_salt("friend_selector")
                .selected_text(
                    self.selected_friend_index
                        .and_then(|i| self.friend_identities.get(i).map(|(name, _)| name.clone()))
                        .unwrap_or_else(|| "Other".to_string()),
                )
                .show_ui(ui, |ui| {
                    for (i, (alias, _)) in self.friend_identities.iter().enumerate() {
                        if ui
                            .selectable_value(&mut self.selected_friend_index, Some(i), alias)
                            .clicked()
                        {
                            self.receiver_identity_id =
                                self.friend_identities[i].1.to_string(Encoding::Base58);
                        }
                    }

                    if ui
                        .selectable_value(&mut self.selected_friend_index, None, "Other")
                        .clicked()
                    {
                        // Clear the text box to avoid confusion
                        self.receiver_identity_id.clear();
                    }
                });

            // Text box
            let prev_text = self.receiver_identity_id.clone();
            ui.text_edit_singleline(&mut self.receiver_identity_id);
            if self.receiver_identity_id != prev_text {
                self.selected_friend_index = None;
            }
        });
    }

    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let mut is_open = true;
        egui::Window::new("Confirm Transfer")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                let identifier = if self.receiver_identity_id.is_empty() {
                    self.transfer_tokens_status =
                        TransferTokensStatus::ErrorMessage("Invalid identifier".to_string());
                    self.confirmation_popup = false;
                    return;
                } else {
                    match Identifier::from_string_try_encodings(
                        &self.receiver_identity_id,
                        &[Encoding::Base58, Encoding::Hex],
                    ) {
                        Ok(identifier) => identifier,
                        Err(_) => {
                            self.transfer_tokens_status = TransferTokensStatus::ErrorMessage(
                                "Invalid identifier".to_string(),
                            );
                            self.confirmation_popup = false;
                            return;
                        }
                    }
                };

                if self.selected_key.is_none() {
                    self.transfer_tokens_status =
                        TransferTokensStatus::ErrorMessage("No selected key".to_string());
                    self.confirmation_popup = false;
                    return;
                };

                ui.label(format!(
                    "Are you sure you want to transfer {} {} to {}?",
                    self.amount, self.identity_token_balance.token_alias, self.receiver_identity_id
                ));

                if ui.button("Confirm").clicked() {
                    self.confirmation_popup = false;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    self.transfer_tokens_status = TransferTokensStatus::WaitingForResult(now);
                    let data_contract = Arc::new(
                        self.app_context
                            .get_unqualified_contract_by_id(
                                &self.identity_token_balance.data_contract_id,
                            )
                            .expect("Contracts not loaded")
                            .expect("Data contract not found"),
                    );
                    app_action |= AppAction::BackendTasks(
                        vec![
                            BackendTask::TokenTask(Box::new(TokenTask::TransferTokens {
                                sending_identity: self.identity.clone(),
                                recipient_id: identifier,
                                amount: {
                                    let decimals = self
                                        .identity_token_balance
                                        .token_config
                                        .conventions()
                                        .decimals();
                                    parse_token_amount(&self.amount, decimals)
                                        .expect("Amount should be valid at this point")
                                },
                                data_contract,
                                token_position: self.identity_token_balance.token_position,
                                signing_key: self.selected_key.clone().expect("Expected a key"),
                                public_note: self.public_note.clone(),
                            })),
                            BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)),
                        ],
                        BackendTasksExecutionMode::Sequential,
                    );
                }
                if ui.button("Cancel").clicked() {
                    self.confirmation_popup = false;
                }
            });
        if !is_open {
            self.confirmation_popup = false;
        }
        app_action
    }

    pub fn show_success(&self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Center the content vertically and horizontally
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Success!");

            ui.add_space(20.0);

            // Display the "Back to Identities" button
            if ui.button("Back to Tokens").clicked() {
                // Handle navigation back to the identities screen
                action |= AppAction::PopScreenAndRefresh;
            }
        });

        action
    }
}

impl ScreenLike for TransferTokensScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message == "TransferTokens" {
                    self.transfer_tokens_status = TransferTokensStatus::Complete;
                }
            }
            MessageType::Info => {}
            MessageType::Error => {
                // It's not great because the error message can be coming from somewhere else if there are other processes happening
                self.transfer_tokens_status =
                    TransferTokensStatus::ErrorMessage(message.to_string());
            }
        }
    }

    fn refresh(&mut self) {
        // Refresh the identity because there might be new keys
        self.identity = self
            .app_context
            .load_local_qualified_identities()
            .unwrap()
            .into_iter()
            .find(|identity| identity.identity.id() == self.identity.identity.id())
            .unwrap();
        let token_balances = self
            .app_context
            .db
            .get_identity_token_balances(&self.app_context)
            .expect("Token balances not loaded");
        self.max_amount = token_balances
            .values()
            .find(|balance| balance.identity_id == self.identity.identity.id())
            .map(|balance| balance.balance)
            .unwrap_or(0);
    }

    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Tokens", AppAction::GoToMainScreen),
                (
                    &self.identity_token_balance.token_alias,
                    AppAction::PopScreen,
                ),
                ("Transfer", AppAction::None),
            ],
            vec![],
        );

        // Left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenMyTokenBalances,
        );

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ctx, &self.app_context);

        let central_panel_action = island_central_panel(ctx, |ui| {
            let dark_mode = ui.ctx().style().visuals.dark_mode;
            
            // Show the success screen if the transfer was successful
            if self.transfer_tokens_status == TransferTokensStatus::Complete {
                return self.show_success(ui);
            }

            ui.heading(format!(
                "Transfer {}",
                self.identity_token_balance.token_alias
            ));
            ui.add_space(10.0);

            let has_keys = if self.app_context.developer_mode.load(Ordering::Relaxed) {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self
                    .identity
                    .available_authentication_keys_with_critical_security_level()
                    .is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    DashColors::error_color(dark_mode),
                    format!(
                        "You do not have any authentication keys with CRITICAL security level loaded for this {} identity.",
                        self.identity.identity_type
                    ),
                );
                ui.add_space(10.0);

                let key = self.identity.identity.get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([SecurityLevel::CRITICAL]),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = key {
                    if ui.button("Check Keys").clicked() {
                        return AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    return AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                if self.selected_wallet.is_some() {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock && !just_unlocked {
                        return AppAction::None;
                    }
                }

                // Select the key to sign with
                ui.heading("1. Select the key to sign the transaction with");
                ui.add_space(10.0);

                let mut selected_identity = Some(self.identity.clone());
                add_identity_key_chooser(
                    ui,
                    &self.app_context,
                    std::iter::once(&self.identity),
                    &mut selected_identity,
                    &mut self.selected_key,
                    TransactionType::TokenTransfer,
                );

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input the amount to transfer
                ui.heading("2. Input the amount to transfer");
                ui.add_space(5.0);

                // Show available balance
                let decimals = self
                    .identity_token_balance
                    .token_config
                    .conventions()
                    .decimals();
                let formatted_balance = format_token_amount(self.max_amount, decimals);
                ui.label(format!(
                    "Available balance: {} {}",
                    formatted_balance, self.identity_token_balance.token_alias
                ));
                ui.add_space(5.0);

                self.render_amount_input(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input the ID of the identity to transfer to
                ui.heading("3. ID of the identity to transfer to");
                ui.add_space(5.0);
                self.render_to_identity_input(ui);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render text input for the public note
                ui.heading("4. Public note (optional)");
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.label("Public note (optional):");
                    ui.add_space(10.0);
                    let mut txt = self.public_note.clone().unwrap_or_default();
                    if ui
                        .text_edit_singleline(&mut txt)
                        .on_hover_text(
                            "A note about the transaction that can be seen by the public.",
                        )
                        .changed()
                    {
                        self.public_note = Some(txt);
                    }
                });
                ui.add_space(10.0);

                // Transfer button
                let mut new_style = (**ui.style()).clone();
                new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                ui.set_style(new_style);
                let button = egui::Button::new(RichText::new("Transfer").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .corner_radius(3.0);
                if ui.add(button).clicked() {
                    let decimals = self
                        .identity_token_balance
                        .token_config
                        .conventions()
                        .decimals();
                    match parse_token_amount(&self.amount, decimals) {
                        Ok(parsed_amount) => {
                            if parsed_amount > self.max_amount {
                                self.transfer_tokens_status = TransferTokensStatus::ErrorMessage(
                                    "Amount exceeds available balance".to_string(),
                                );
                            } else if parsed_amount == 0 {
                                self.transfer_tokens_status = TransferTokensStatus::ErrorMessage(
                                    "Amount must be greater than zero".to_string(),
                                );
                            } else {
                                self.confirmation_popup = true;
                            }
                        }
                        Err(e) => {
                            self.transfer_tokens_status = TransferTokensStatus::ErrorMessage(e);
                        }
                    }
                }

                if self.confirmation_popup {
                    return self.show_confirmation_popup(ui);
                }

                // Handle transfer status messages
                ui.add_space(5.0);
                match &self.transfer_tokens_status {
                    TransferTokensStatus::NotStarted => {
                        // Do nothing
                    }
                    TransferTokensStatus::WaitingForResult(start_time) => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        let elapsed_seconds = now - start_time;

                        let display_time = if elapsed_seconds < 60 {
                            format!(
                                "{} second{}",
                                elapsed_seconds,
                                if elapsed_seconds == 1 { "" } else { "s" }
                            )
                        } else {
                            let minutes = elapsed_seconds / 60;
                            let seconds = elapsed_seconds % 60;
                            format!(
                                "{} minute{} and {} second{}",
                                minutes,
                                if minutes == 1 { "" } else { "s" },
                                seconds,
                                if seconds == 1 { "" } else { "s" }
                            )
                        };

                        ui.label(format!(
                            "Transferring... Time taken so far: {}",
                            display_time
                        ));
                    }
                    TransferTokensStatus::ErrorMessage(msg) => {
                        ui.colored_label(DashColors::error_color(dark_mode), format!("Error: {}", msg));
                    }
                    TransferTokensStatus::Complete => {
                        // Handled above
                    }
                }
            }

            AppAction::None
        });
        action |= central_panel_action;
        action
    }
}

impl ScreenWithWalletUnlock for TransferTokensScreen {
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
        if let Some(error_message) = error_message {
            self.transfer_tokens_status = TransferTokensStatus::ErrorMessage(error_message);
        }
    }

    fn error_message(&self) -> Option<&String> {
        if let TransferTokensStatus::ErrorMessage(error_message) = &self.transfer_tokens_status {
            Some(error_message)
        } else {
            None
        }
    }
}
