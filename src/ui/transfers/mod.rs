use crate::app::AppAction;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
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
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::components::wallet_unlock::ScreenWithWalletUnlock;
use super::identities::register_dpns_name_screen::get_selected_wallet;

pub enum TransferCreditsStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct TransferScreen {
    pub identity: QualifiedIdentity,
    selected_key: Option<IdentityPublicKey>,
    receiver_identity_id: String,
    amount: String,
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
        let max_amount = identity.identity.balance();
        let selected_key = identity
            .identity
            .get_first_public_key_matching(
                Purpose::TRANSFER,
                SecurityLevel::full_range().into(),
                KeyType::all_key_types().into(),
                false,
            )
            .cloned();
        let mut error_message = None;
        let selected_wallet = get_selected_wallet(&identity, app_context, &mut error_message);
        Self {
            identity,
            selected_key,
            receiver_identity_id: String::new(),
            amount: String::new(),
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
                        for key in self.identity.available_transfer_keys() {
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
            ui.label("Amount in Dash:");

            ui.text_edit_singleline(&mut self.amount);

            if ui.button("Max").clicked() {
                let amount_in_dash = self.max_amount as f64 / 100_000_000_000.0 - 0.0001; // Subtract a small amount to cover gas fee which is usually around 0.00002 Dash
                self.amount = format!("{:.8}", amount_in_dash);
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
                    self.transfer_credits_status =
                        TransferCreditsStatus::ErrorMessage("Invalid identifier".to_string());
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
                            return;
                        }
                    }
                };

                let Some(selected_key) = self.selected_key.as_ref() else {
                    self.error_message = Some("No selected key".to_string());
                    self.transfer_credits_status =
                        TransferCreditsStatus::ErrorMessage("No selected key".to_string());
                    return;
                };

                ui.label(format!(
                    "Are you sure you want to transfer {} Dash to {}",
                    self.amount, self.receiver_identity_id
                ));
                let parts: Vec<&str> = self.amount.split('.').collect();
                let mut credits: u128 = 0;

                // Process the whole number part if it exists.
                if let Some(whole) = parts.first() {
                    if let Ok(whole_number) = whole.parse::<u128>() {
                        credits += whole_number * 100_000_000_000; // Whole Dash amount to credits
                    }
                }

                // Process the fractional part if it exists.
                if let Some(fraction) = parts.get(1) {
                    let fraction_length = fraction.len();
                    let fraction_number = fraction.parse::<u128>().unwrap_or(0);
                    // Calculate the multiplier based on the number of digits in the fraction.
                    let multiplier = 10u128.pow(11 - fraction_length as u32);
                    credits += fraction_number * multiplier; // Fractional Dash to credits
                }

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

        egui::CentralPanel::default().show(ctx, |ui| {
            let has_keys = if self.app_context.developer_mode {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self.identity.available_transfer_keys().is_empty()
            };

            if !has_keys {
                ui.heading(format!(
                    "You do not have any transfer keys loaded for this {}.",
                    self.identity.identity_type
                ));

                let key = self.identity.identity.get_first_public_key_matching(
                    Purpose::TRANSFER,
                    SecurityLevel::full_range().into(),
                    KeyType::all_key_types().into(),
                    false,
                );

                if let Some(key) = key {
                    if ui.button("Check Transfer Key").clicked() {
                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity.clone(),
                            key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                }
            } else {
                ui.heading("Transfer Funds");

                if self.selected_wallet.is_some() {
                    let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if needed_unlock && !just_unlocked {
                        return;
                    }
                }

                self.render_key_selection(ui);
                self.render_amount_input(ui);
                self.render_to_identity_input(ui);

                if ui.button("Transfer").clicked() {
                    self.confirmation_popup = true;
                }

                if self.confirmation_popup {
                    action |= self.show_confirmation_popup(ui);
                }

                // Handle transfer status messages
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
                        action = AppAction::PopScreenAndRefresh;
                    }
                }
            }
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
