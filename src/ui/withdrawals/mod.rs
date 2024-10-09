use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::{
    EncryptedPrivateKeyTarget, IdentityType, QualifiedIdentity,
};
use crate::ui::ScreenLike;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::platform::IdentityPublicKey;
use eframe::egui::{self, Context, Ui};
use std::sync::Arc;

pub struct WithdrawalScreen {
    pub identity: QualifiedIdentity,
    selected_key: Option<IdentityPublicKey>,
    withdrawal_amount: String,
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
            withdrawal_amount: String::new(),
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

    fn show_confirmation_popup(&mut self, ui: &mut Ui) {
        egui::Window::new("Confirm Withdrawal")
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.label("Are you sure you want to withdraw the specified amount?");
                if ui.button("Confirm").clicked() {
                    self.confirmation_popup = false;
                    self.make_withdrawal();
                }
                if ui.button("Cancel").clicked() {
                    self.confirmation_popup = false;
                }
            });
    }

    fn make_withdrawal(&self) {
        if let Some(selected_key) = &self.selected_key {
            let amount: u64 = self.withdrawal_amount.parse().unwrap_or(0);

            // TODO: Dispatch backend task for withdrawal using the selected key and amount
            // Example:
            // self.app_context.dispatch_backend_task(...);
        } else {
            // Handle no key selected error case
        }
    }
}

impl ScreenLike for WithdrawalScreen {
    /// Renders the UI components for the withdrawal screen
    fn ui(&mut self, ctx: &Context) -> AppAction {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Withdraw Funds");

            self.render_key_selection(ui);
            self.render_amount_input(ui);

            if ui.button("Withdraw").clicked() {
                self.confirmation_popup = true;
            }

            if self.confirmation_popup {
                self.show_confirmation_popup(ui);
            }
        });
        AppAction::None
    }
}
