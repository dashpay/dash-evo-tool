use crate::app::AppAction;
use crate::model::fee_estimation::{PlatformFeeEstimator, format_credits_as_dash};
use crate::ui::identities::add_new_identity_screen::{
    AddNewIdentityScreen, FundingMethod, WalletFundedScreenStep,
};
use egui::{Color32, RichText, Ui};

impl AddNewIdentityScreen {
    fn show_wallet_balance(&self, ui: &mut egui::Ui) {
        if let Some(selected_wallet) = &self.selected_wallet {
            let wallet = selected_wallet.read().unwrap(); // Read lock on the wallet

            let total_balance: u64 = wallet.total_balance_duffs(); // Use stored balance with UTXO fallback

            let dash_balance = total_balance as f64 * 1e-8; // Convert to DASH units

            ui.horizontal(|ui| {
                ui.label(format!("Wallet Balance: {:.8} DASH", dash_balance));
            });
        } else {
            ui.label("No wallet selected");
        }
    }

    pub fn render_ui_by_using_unused_balance(
        &mut self,
        ui: &mut Ui,
        step_number: u32,
    ) -> AppAction {
        let mut action = AppAction::None;

        ui.add_space(10.0);
        ui.heading(format!(
            "{}. How much of your wallet balance would you like to transfer?",
            step_number
        ));

        ui.add_space(10.0);
        self.show_wallet_balance(ui);
        ui.add_space(5.0);

        self.render_funding_amount_input(ui);

        // Extract the step from the RwLock to minimize borrow scope
        let step = *self.step.read().unwrap();

        // Check if we have a valid amount before showing the button
        let has_valid_amount = self
            .funding_amount
            .as_ref()
            .map(|a| a.value() > 0)
            .unwrap_or(false);

        if !has_valid_amount {
            return action;
        }

        // Display estimated fee before action button
        let key_count = self.identity_keys.keys_input.len() + 1; // +1 for master key
        let estimated_fee = PlatformFeeEstimator::new().estimate_identity_create(key_count);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label("Estimated Fee:");
            ui.label(RichText::new(format_credits_as_dash(estimated_fee)).strong());
        });
        ui.add_space(10.0);

        let button = egui::Button::new(RichText::new("Create Identity").color(Color32::WHITE))
            .fill(Color32::from_rgb(0, 128, 255))
            .frame(true)
            .corner_radius(3.0);
        if ui.add(button).clicked() {
            self.error_message = None;
            action = self.register_identity_clicked(FundingMethod::UseWalletBalance);
        }

        ui.add_space(20.0);

        // Only show status messages if there's no error
        if self.error_message.is_none() {
            ui.vertical_centered(|ui| match step {
                WalletFundedScreenStep::WaitingForAssetLock => {
                    ui.heading(
                        "=> Waiting for Core Chain to produce proof of transfer of funds. <=",
                    );
                }
                WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                    ui.heading("=> Waiting for Platform acknowledgement <=");
                }
                WalletFundedScreenStep::Success => {
                    ui.heading("...Success...");
                }
                _ => {}
            });
        }

        ui.add_space(40.0);
        action
    }
}
