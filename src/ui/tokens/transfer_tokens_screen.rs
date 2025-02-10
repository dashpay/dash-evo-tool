use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::keys::add_key_screen::AddKeyScreen;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Context, Ui};
use egui::{Color32, RichText};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;

use crate::ui::identities::get_selected_wallet;

use super::tokens_screen::IdentityTokenBalance;

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
    selected_key: Option<IdentityPublicKey>,
    receiver_identity_id: String,
    amount: String,
    transfer_tokens_status: TransferTokensStatus,
    error_message: Option<String>,
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
        let identity = app_context
            .load_local_qualified_identities()
            .expect("Identities not loaded")
            .iter()
            .find(|identity| identity.identity.id() == identity_token_balance.identity_id)
            .expect("Identity not found")
            .clone();
        let token_balances = app_context
            .db
            .get_identity_token_balances(app_context)
            .expect("Token balances not loaded");
        let max_amount = token_balances
            .iter()
            .find(|balance| balance.identity_id == identity.identity.id())
            .map(|balance| balance.balance)
            .unwrap_or(0);
        let identity_clone = identity.identity.clone();
        let selected_key = identity_clone.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([
                SecurityLevel::HIGH,
                SecurityLevel::MEDIUM,
                SecurityLevel::CRITICAL,
            ]),
            KeyType::all_key_types().into(),
            false,
        );
        let mut error_message = None;
        let selected_wallet =
            get_selected_wallet(&identity, None, selected_key, &mut error_message);
        Self {
            identity,
            identity_token_balance,
            selected_key: selected_key.cloned(),
            receiver_identity_id: String::new(),
            amount: String::new(),
            transfer_tokens_status: TransferTokensStatus::NotStarted,
            error_message: None,
            max_amount,
            app_context: app_context.clone(),
            confirmation_popup: false,
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
        }
    }

    fn render_key_selection(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Select Key:");

            egui::ComboBox::from_id_salt("key_selector")
                .selected_text(match &self.selected_key {
                    Some(key) => format!("Key ID: {}", key.id()),
                    None => "Select a key".to_string(),
                })
                .show_ui(ui, |ui| {
                    if self.app_context.developer_mode {
                        for key in self.identity.identity.public_keys().values() {
                            let label =
                                format!("Key ID: {} (Purpose: {:?})", key.id(), key.purpose());
                            ui.selectable_value(&mut self.selected_key, Some(key.clone()), label);
                        }
                    } else {
                        for key in self.identity.available_authentication_keys() {
                            let label = format!(
                                "Key ID: {} (Purpose: {:?})",
                                key.identity_public_key.id(),
                                key.identity_public_key.purpose()
                            );
                            ui.selectable_value(
                                &mut self.selected_key,
                                Some(key.identity_public_key.clone()),
                                label,
                            );
                        }
                    }
                });
        });
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Amount:");

            ui.text_edit_singleline(&mut self.amount);

            if ui.button("Max").clicked() {
                self.amount = self.max_amount.to_string();
            }
        });
    }

    fn render_to_identity_input(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Receiver Identity Id:");

            ui.text_edit_singleline(&mut self.receiver_identity_id);
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
                    self.error_message = Some("Invalid identifier".to_string());
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
                            self.error_message = Some("Invalid identifier".to_string());
                            self.transfer_tokens_status = TransferTokensStatus::ErrorMessage(
                                "Invalid identifier".to_string(),
                            );
                            self.confirmation_popup = false;
                            return;
                        }
                    }
                };

                if self.selected_key.is_none() {
                    self.error_message = Some("No selected key".to_string());
                    self.transfer_tokens_status =
                        TransferTokensStatus::ErrorMessage("No selected key".to_string());
                    self.confirmation_popup = false;
                    return;
                };

                ui.label(format!(
                    "Are you sure you want to transfer {} tokens to {}",
                    self.amount, self.receiver_identity_id
                ));

                if ui.button("Confirm").clicked() {
                    self.confirmation_popup = false;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    self.transfer_tokens_status = TransferTokensStatus::WaitingForResult(now);
                    let data_contract = self
                        .app_context
                        .get_contracts(None, None)
                        .expect("Contracts not loaded")
                        .iter()
                        .find(|contract| {
                            contract.contract.id() == self.identity_token_balance.data_contract_id
                        })
                        .expect("Data contract not found")
                        .contract
                        .clone();
                    app_action = AppAction::BackendTasks(
                        vec![
                            BackendTask::TokenTask(TokenTask::TransferTokens {
                                sending_identity: self.identity.clone(),
                                recipient_id: identifier,
                                amount: self.amount.parse().unwrap(),
                                data_contract,
                                token_position: self.identity_token_balance.token_position,
                                signing_key: self.selected_key.clone().expect("Expected a key"),
                            }),
                            BackendTask::TokenTask(TokenTask::QueryMyTokenBalances),
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
                action = AppAction::PopScreenAndRefresh;
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
                self.error_message = Some(message.to_string());
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
            .iter()
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
                ("Transfer", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            // Show the success screen if the transfer was successful
            if self.transfer_tokens_status == TransferTokensStatus::Complete {
                action = self.show_success(ui);
                return;
            }

            ui.heading("Transfer Funds");
            ui.add_space(10.0);

            let has_keys = if self.app_context.developer_mode {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self.identity.available_authentication_keys().is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    format!(
                        "You do not have any authentication keys loaded for this {} identity.",
                        self.identity.identity_type
                    ),
                );
                ui.add_space(10.0);

                let key = self.identity.identity.get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([
                        SecurityLevel::HIGH,
                        SecurityLevel::MEDIUM,
                        SecurityLevel::CRITICAL,
                    ]),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = key {
                    if ui.button("Check Keys").clicked() {
                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                if self.selected_wallet.is_some() {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock && !just_unlocked {
                        return;
                    }
                }

                // Select the key to sign with
                ui.heading("1. Select the key to sign the transaction with");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    self.render_key_selection(ui);
                    ui.add_space(5.0);
                    let identity_id_string =
                        self.identity.identity.id().to_string(Encoding::Base58);
                    let identity_display = self
                        .identity
                        .alias
                        .as_deref()
                        .unwrap_or_else(|| &identity_id_string);
                    ui.label(format!("Identity: {}", identity_display));
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input the amount to transfer
                ui.heading("2. Input the amount to transfer");
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

                // Transfer button
                let mut new_style = (**ui.style()).clone();
                new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                ui.set_style(new_style);
                let button = egui::Button::new(RichText::new("Transfer").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .rounding(3.0);
                if ui.add(button).clicked() {
                    self.confirmation_popup = true;
                }

                if self.confirmation_popup {
                    action |= self.show_confirmation_popup(ui);
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
                        ui.colored_label(egui::Color32::DARK_RED, format!("Error: {}", msg));
                    }
                    TransferTokensStatus::Complete => {
                        // Handled above
                    }
                }
            }
        });
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
        self.error_message = error_message;
    }

    fn error_message(&self) -> Option<&String> {
        self.error_message.as_ref()
    }
}
