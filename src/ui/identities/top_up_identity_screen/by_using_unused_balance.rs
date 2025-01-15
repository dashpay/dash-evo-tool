use crate::app::AppAction;
use crate::ui::identities::add_new_identity_screen::FundingMethod;
use crate::ui::identities::top_up_identity_screen::{TopUpIdentityScreen, WalletFundedScreenStep};
use egui::{Color32, RichText, Ui};

impl TopUpIdentityScreen {
    fn show_wallet_balance(&self, ui: &mut egui::Ui) {
        if let Some(selected_wallet) = &self.wallet {
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

        ui.heading(format!(
            "{}. How much of your wallet balance would you like to transfer?",
            step_number
        ));

        ui.add_space(10.0);

        self.show_wallet_balance(ui);

        ui.add_space(10.0);

        step_number += 1;

        self.top_up_funding_amount_input(ui);

        // Extract the step from the RwLock to minimize borrow scope
        let step = self.step.read().unwrap().clone();

        let Ok(_) = self.funding_amount.parse::<f64>() else {
            return action;
        };

        ui.add_space(10.0);

        // Top up button
        let mut new_style = (**ui.style()).clone();
        new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
        ui.set_style(new_style);
        let button = egui::Button::new(RichText::new("Top Up Identity").color(Color32::WHITE))
            .fill(Color32::from_rgb(0, 128, 255))
            .frame(true)
            .rounding(3.0);
        if ui.add(button).clicked() {
            self.error_message = None;
            action = self.top_up_identity_clicked(FundingMethod::UseWalletBalance);
        }

        ui.add_space(20.0);

        if let Some(error_message) = self.error_message.as_ref() {
            ui.colored_label(Color32::DARK_RED, error_message);
            ui.add_space(20.0);
        }

        ui.vertical_centered(|ui| {
            match step {
                WalletFundedScreenStep::WaitingForAssetLock => {
                    ui.heading("=> Waiting for Core Chain to produce proof of transfer of funds. <=");
                    ui.add_space(20.0);
                    ui.label("NOTE: If this gets stuck, the funds were likely either transferred to the wallet or asset locked,\nand you can use the funding method selector in step 1 to change the method and use those funds to complete the process.");
                }
                WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                    ui.heading("=> Waiting for Platform acknowledgement <=");
                    ui.add_space(20.0);
                    ui.label("NOTE: If this gets stuck, the funds were likely either transferred to the wallet or asset locked,\nand you can use the funding method selector in step 1 to change the method and use those funds to complete the process.");
                }
                WalletFundedScreenStep::Success => {
                    ui.heading("...Success...");
                }
                _ => {}
            };
        });

        ui.add_space(40.0);
        action
    }
}
