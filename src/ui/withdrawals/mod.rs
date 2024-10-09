use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::{
    EncryptedPrivateKeyTarget, IdentityType, QualifiedIdentity,
};
use crate::platform::identity::IdentityTask;
use crate::platform::BackendTask;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::ScreenLike;
use dash_sdk::dashcore_rpc::dashcore::address::{Error, NetworkUnchecked};
use dash_sdk::dashcore_rpc::dashcore::Address;
use dash_sdk::dpp::dash_to_credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::Purpose;
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
                    for key in self.identity.available_withdrawal_keys() {
                        let label = format!("Key ID: {} (Purpose: {:?})", key.id(), key.purpose());
                        let selectable =
                            ui.selectable_value(&mut self.selected_key, Some(key.clone()), label);
                        if selectable.hovered() {
                            // Add any additional hover info or color here if needed
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
        if can_have_withdrawal_address {
            ui.horizontal(|ui| {
                ui.label("Address:");

                ui.text_edit_singleline(&mut self.withdrawal_address);
            });
        }
    }

    fn show_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        egui::Window::new("Confirm Withdrawal")
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.label("Are you sure you want to withdraw the specified amount?");
                if ui.button("Confirm").clicked() {
                    self.confirmation_popup = false;
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

                    app_action = AppAction::BackendTask(BackendTask::IdentityTask(
                        IdentityTask::WithdrawFromIdentity(
                            self.identity.clone(),
                            address,
                            dash_to_credits!(self.withdrawal_amount),
                        ),
                    ));
                }
                if ui.button("Cancel").clicked() {
                    self.confirmation_popup = false;
                }
            });
        app_action
    }
}

impl ScreenLike for WithdrawalScreen {
    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Withdraw", AppAction::None),
            ],
            None,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Withdraw Funds");

            self.render_key_selection(ui);
            self.render_amount_input(ui);

            if ui.button("Withdraw").clicked() {
                self.confirmation_popup = true;
            }

            if self.confirmation_popup {
                action |= self.show_confirmation_popup(ui);
            }

            if let Some(error_message) = &self.error_message {
                ui.label(format!("Error: {}", error_message));
            }
        });
        action
    }
}
