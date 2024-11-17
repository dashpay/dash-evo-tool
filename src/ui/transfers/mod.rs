use crate::app::AppAction;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::identities::keys::key_info_screen::KeyInfoScreen;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Context, Ui};
use std::sync::Arc;

pub struct TransferScreen {
    pub identity: QualifiedIdentity,
    selected_key: Option<IdentityPublicKey>,
    receiver_identity_id: String,
    amount: String,
    error_message: Option<String>,
    max_amount: u64,
    pub app_context: Arc<AppContext>,
    confirmation_popup: bool,
}

impl TransferScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        let max_amount = identity.identity.balance();
        Self {
            identity,
            selected_key: None,
            receiver_identity_id: String::new(),
            amount: String::new(),
            error_message: None,
            max_amount,
            app_context: app_context.clone(),
            confirmation_popup: false,
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
                    return;
                } else {
                    match Identifier::from_string_try_encodings(
                        &self.receiver_identity_id,
                        &[Encoding::Base58, Encoding::Hex],
                    ) {
                        Ok(identifier) => identifier,
                        Err(_) => {
                            self.error_message = Some("Invalid identifier".to_string());
                            return;
                        }
                    }
                };

                let Some(selected_key) = self.selected_key.as_ref() else {
                    self.error_message = Some("No selected key".to_string());
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
    fn display_message(&mut self, message: &str, _message_type: MessageType) {
        self.error_message = Some(message.to_string());
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

                self.render_key_selection(ui);
                self.render_amount_input(ui);
                self.render_to_identity_input(ui);

                if ui.button("Transfer").clicked() {
                    self.confirmation_popup = true;
                }

                if self.confirmation_popup {
                    action |= self.show_confirmation_popup(ui);
                }

                if let Some(error_message) = &self.error_message {
                    ui.label(format!("Error: {}", error_message));
                }
            }
        });
        action
    }
}
