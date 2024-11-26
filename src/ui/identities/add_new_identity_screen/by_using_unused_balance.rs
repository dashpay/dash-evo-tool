use crate::app::AppAction;
use crate::ui::identities::add_new_identity_screen::{
    AddNewIdentityScreen, AddNewIdentityWalletFundedScreenStep, FundingMethod,
};
use egui::Ui;

impl AddNewIdentityScreen {
    fn show_wallet_balance(&self, ui: &mut egui::Ui) {
        if let Some(selected_wallet) = &self.selected_wallet {
            let wallet = selected_wallet.read().unwrap(); // Read lock on the wallet

            let total_balance: u64 = wallet.max_balance(); // Sum up all the balances

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
        mut step_number: u32,
    ) -> AppAction {
        let mut action = AppAction::None;

        self.show_wallet_balance(ui);

        ui.add_space(10.0);

        ui.heading(format!(
            "{}. How much of your wallet balance would you like to transfer?",
            step_number
        ));

        step_number += 1;

        self.render_funding_amount_input(ui);

        // Extract the step from the RwLock to minimize borrow scope
        let step = self.step.read().unwrap().clone();

        let Ok(_) = self.funding_amount.parse::<f64>() else {
            return action;
        };

        if ui.button("Create Identity").clicked() {
            self.error_message = None;
            action = self.register_identity_clicked(FundingMethod::UseWalletBalance);
        }

        match step {
            AddNewIdentityWalletFundedScreenStep::WaitingForAssetLock => {
                ui.heading("Waiting for Core Chain to produce proof of transfer of funds.");
            }
            AddNewIdentityWalletFundedScreenStep::WaitingForPlatformAcceptance => {
                ui.heading("Waiting for Platform acknowledgement");
            }
            AddNewIdentityWalletFundedScreenStep::Success => {
                ui.heading("...Success...");
            }
            _ => {}
        }

        if let Some(error_message) = self.error_message.as_ref() {
            ui.heading(error_message);
        }

        action
    }
}
