use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::platform::identity::IdentityTask;
use crate::platform::BackendTask;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::key_info_screen::KeyInfoScreen;
use crate::ui::{MessageType, Screen, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::platform::IdentityPublicKey;
use eframe::egui::{self, Context, Ui};
use std::str::FromStr;
use std::sync::Arc;

pub struct WithdrawalScreen {
    pub identity: QualifiedIdentity,
    selected_key: Option<IdentityPublicKey>,
    withdrawal_address: String,
    withdrawal_amount: String,
    error_message: Option<String>,
    max_amount: u64,
    pub app_context: Arc<AppContext>,
    confirmation_popup: bool,
}

impl WithdrawalScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        let max_amount = identity.identity.balance();
        Self {
            identity,
            selected_key: None,
            withdrawal_address: String::new(),
            withdrawal_amount: String::new(),
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
                        for key in self.identity.available_withdrawal_keys() {
                            let label =
                                format!("Key ID: {} (Purpose: {:?})", key.id(), key.purpose());
                            ui.selectable_value(&mut self.selected_key, Some(key.clone()), label);
                        }
                    }
                });
        });
    }

    fn render_amount_input(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label("Amount:");

            ui.text_edit_singleline(&mut self.withdrawal_amount);

            if ui.button("Max").clicked() {
                self.withdrawal_amount = self.max_amount.to_string();
            }
        });
    }

    fn render_address_input(&mut self, ui: &mut Ui) {
        let can_have_withdrawal_address = if let Some(key) = self.selected_key.as_ref() {
            key.purpose() != Purpose::OWNER
        } else {
            true
        };
        if can_have_withdrawal_address || self.app_context.developer_mode {
            ui.horizontal(|ui| {
                ui.label("Address:");

                ui.text_edit_singleline(&mut self.withdrawal_address);
            });
        }
    }

    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let mut is_open = true;
        egui::Window::new("Confirm Withdrawal")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                let address = if self.withdrawal_address.is_empty() {
                    None
                } else {
                    match Address::from_str(&self.withdrawal_address) {
                        Ok(address) => Some(address.assume_checked()),
                        Err(_) => {
                            self.error_message = Some("Invalid withdrawal address".to_string());
                            None
                        }
                    }
                };

                let message_address = if address.is_some() {
                    self.withdrawal_address.clone()
                } else if let Some(payout_address) = self
                    .identity
                    .masternode_payout_address(self.app_context.network)
                {
                    format!("masternode payout address {}", payout_address)
                } else if !self.app_context.developer_mode {
                    self.error_message = Some("No masternode payout address".to_string());
                    return;
                } else {
                    "to default address".to_string()
                };

                let Some(selected_key) = self.selected_key.as_ref() else {
                    self.error_message = Some("No selected key".to_string());
                    return;
                };

                ui.label(format!(
                    "Are you sure you want to withdraw {} Dash to {}",
                    self.withdrawal_amount, message_address
                ));
                let parts: Vec<&str> = self.withdrawal_amount.split('.').collect();
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
                    app_action = AppAction::BackendTask(BackendTask::IdentityTask(
                        IdentityTask::WithdrawFromIdentity(
                            self.identity.clone(),
                            address,
                            credits as Credits,
                            Some(selected_key.id()),
                        ),
                    ));
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

impl ScreenLike for WithdrawalScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.error_message = Some(message.to_string());
    }

    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Withdraw", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            let has_keys = if self.app_context.developer_mode {
                !self.identity.identity.public_keys().is_empty()
            } else {
                !self.identity.available_withdrawal_keys().is_empty()
            };

            if !has_keys {
                ui.heading(format!("You do not have any withdrawal keys loaded for this {}.", self.identity.identity_type));

                if self.identity.identity_type != IdentityType::User {
                    ui.heading("An evonode can withdraw with the payout address private key or the owner key.".to_string());
                    ui.heading("If the owner key is used you can only withdraw to the Dash Core payout address (where you get your Core rewards).".to_string());
                }

                let owner_key = self.identity.identity.get_first_public_key_matching(Purpose::OWNER, SecurityLevel::full_range().into(), KeyType::all_key_types().into(), false);

                let transfer_key = self.identity.identity.get_first_public_key_matching(Purpose::TRANSFER, SecurityLevel::full_range().into(), KeyType::all_key_types().into(), false);

                if let Some(owner_key) = owner_key {
                    if ui.button("Check Owner Key").clicked() {
                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity.clone(),
                            owner_key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                }

                if let Some(transfer_key) = transfer_key {
                    if ui.button("Check Payout Address Key").clicked() {
                        action |= AppAction::AddScreen(Screen::KeyInfoScreen(KeyInfoScreen::new(
                            self.identity.clone(),
                            transfer_key.clone(),
                            None,
                            &self.app_context,
                        )));
                    }
                }
            } else {
                ui.heading("Withdraw Funds");

                self.render_key_selection(ui);
                self.render_amount_input(ui);
                self.render_address_input(ui);

                if ui.button("Withdraw").clicked() {
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
