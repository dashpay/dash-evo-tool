use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::identity::IdentityTask;
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Context, Ui};
use egui::{Color32, RichText};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::helpers::{TransactionType, add_identity_key_chooser};

use super::get_selected_wallet;
use super::keys::add_key_screen::AddKeyScreen;

#[derive(PartialEq)]
pub enum TransferCreditsStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct TransferScreen {
    pub identity: QualifiedIdentity,
    selected_key: Option<IdentityPublicKey>,
    known_identities: Vec<QualifiedIdentity>,
    receiver_identity_id: String,
    amount: Amount,
    amount_input: Option<AmountInput>,
    transfer_credits_status: TransferCreditsStatus,
    error_message: Option<String>,
    max_amount: u64,
    pub app_context: Arc<AppContext>,
    confirmation_popup: bool,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
}

impl TransferScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        let known_identities = app_context
            .load_local_qualified_identities()
            .expect("Identities not loaded");

        let max_amount = identity.identity.balance();
        let identity_clone = identity.identity.clone();
        let selected_key = identity_clone.get_first_public_key_matching(
            Purpose::TRANSFER,
            SecurityLevel::full_range().into(),
            KeyType::all_key_types().into(),
            false,
        );
        let mut error_message = None;
        let selected_wallet =
            get_selected_wallet(&identity, None, selected_key, &mut error_message);
        Self {
            identity,
            selected_key: selected_key.cloned(),
            known_identities,
            receiver_identity_id: String::new(),
            amount: Amount::dash(0),
            amount_input: None,
            transfer_credits_status: TransferCreditsStatus::NotStarted,
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
        let mut selected_identity = Some(self.identity.clone());
        add_identity_key_chooser(
            ui,
            &self.app_context,
            std::iter::once(&self.identity),
            &mut selected_identity,
            &mut self.selected_key,
            TransactionType::Transfer,
        );
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        // Show available balance
        let balance_in_dash = self.max_amount as f64 / 100_000_000_000.0;
        ui.label(format!("Available balance: {:.8} DASH", balance_in_dash));
        ui.add_space(5.0);

        // Calculate max amount minus fee for the "Max" button
        let max_amount_minus_fee = (self.max_amount as f64 / 100_000_000_000.0 - 0.0001).max(0.0);
        let max_amount_credits = (max_amount_minus_fee * 100_000_000_000.0) as u64;

        // Update max amount dynamically (since it can change)
        let amount_input = self.amount_input.get_or_insert_with(|| {
            AmountInput::new(Amount::dash(0))
                .label("Amount:")
                .max_button(true)
                .max_amount(Some(max_amount_credits))
        });

        // Disable the input when operation is in progress
        match self.transfer_credits_status {
            TransferCreditsStatus::WaitingForResult(_) | TransferCreditsStatus::Complete => {
                amount_input.set_enabled(false);
            }
            TransferCreditsStatus::NotStarted | TransferCreditsStatus::ErrorMessage(_) => {
                amount_input.set_enabled(true);
                amount_input.set_max_amount(Some(max_amount_credits));
            }
        }

        let response = amount_input.show(ui);

        // Update the amount if it was changed
        if let Some(parsed_amount) = response.inner.parsed_amount {
            self.amount = parsed_amount.with_unit_name("DASH".to_string());
        }

        if let Some(error) = &response.inner.error_message {
            ui.colored_label(egui::Color32::DARK_RED, error);
        }
    }

    fn render_to_identity_input(&mut self, ui: &mut Ui) {
        ui.add(
            IdentitySelector::new(
                "transfer_recipient_selector",
                &mut self.receiver_identity_id,
                &self.known_identities,
            )
            .width(300.0)
            .label("Receiver Identity ID:")
            .exclude(&[self.identity.identity.id()]),
        );
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
                    self.transfer_credits_status =
                        TransferCreditsStatus::ErrorMessage("Invalid identifier".to_string());
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
                            self.transfer_credits_status = TransferCreditsStatus::ErrorMessage(
                                "Invalid identifier".to_string(),
                            );
                            self.confirmation_popup = false;
                            return;
                        }
                    }
                };

                let Some(selected_key) = self.selected_key.as_ref() else {
                    self.error_message = Some("No selected key".to_string());
                    self.transfer_credits_status =
                        TransferCreditsStatus::ErrorMessage("No selected key".to_string());
                    self.confirmation_popup = false;
                    return;
                };

                ui.label(format!(
                    "Are you sure you want to transfer {} to {}",
                    self.amount, self.receiver_identity_id
                ));

                // Use the amount directly since it's already an Amount struct
                let credits = self.amount.value() as u128;

                if ui.button("Confirm").clicked() {
                    self.confirmation_popup = false;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    self.transfer_credits_status = TransferCreditsStatus::WaitingForResult(now);
                    app_action =
                        AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::Transfer(
                            self.identity.clone(),
                            identifier,
                            credits as Credits,
                            Some(selected_key.id()),
                        )));
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
            if ui.button("Back to Identities").clicked() {
                // Handle navigation back to the identities screen
                action = AppAction::PopScreenAndRefresh;
            }
        });

        action
    }
}

impl ScreenLike for TransferScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message == "Successfully transferred credits" {
                    self.transfer_credits_status = TransferCreditsStatus::Complete;
                }
            }
            MessageType::Info => {}
            MessageType::Error => {
                // It's not great because the error message can be coming from somewhere else if there are other processes happening
                self.transfer_credits_status =
                    TransferCreditsStatus::ErrorMessage(message.to_string());
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
        self.max_amount = self.identity.identity.balance();
    }

    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Transfer", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentities,
        );

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            // Show the success screen if the transfer was successful
            if self.transfer_credits_status == TransferCreditsStatus::Complete {
                inner_action |= self.show_success(ui);
                return inner_action;
            }

            ui.heading("Transfer Funds");
            ui.add_space(10.0);

            let has_keys = if self.app_context.is_developer_mode() {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self.identity.available_transfer_keys().is_empty()
            };

            if !has_keys {
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    format!(
                        "You do not have any transfer keys loaded for this {} identity.",
                        self.identity.identity_type
                    ),
                );
                ui.add_space(10.0);

                let key = self.identity.identity.get_first_public_key_matching(
                    Purpose::TRANSFER,
                    SecurityLevel::full_range().into(),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = key {
                    if ui.button("Check Transfer Key").clicked() {
                        inner_action |=
                            AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                                self.identity.clone(),
                                key.clone(),
                                None,
                                &self.app_context,
                            )));
                    }
                    ui.add_space(5.0);
                }

                if ui.button("Add key").clicked() {
                    inner_action |= AppAction::AddScreen(Screen::AddKeyScreen(AddKeyScreen::new(
                        self.identity.clone(),
                        &self.app_context,
                    )));
                }
            } else {
                if self.selected_wallet.is_some() {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock && !just_unlocked {
                        return inner_action;
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
                    .corner_radius(3.0);
                if ui.add(button).clicked() {
                    self.confirmation_popup = true;
                }

                if self.confirmation_popup {
                    inner_action |= self.show_confirmation_popup(ui);
                }

                // Handle transfer status messages
                ui.add_space(5.0);
                match &self.transfer_credits_status {
                    TransferCreditsStatus::NotStarted => {
                        // Do nothing
                    }
                    TransferCreditsStatus::WaitingForResult(start_time) => {
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
                    TransferCreditsStatus::ErrorMessage(msg) => {
                        ui.colored_label(egui::Color32::RED, format!("Error: {}", msg));
                    }
                    TransferCreditsStatus::Complete => {
                        // Handled above
                    }
                }
            }

            inner_action
        });
        action
    }
}

impl ScreenWithWalletUnlock for TransferScreen {
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
