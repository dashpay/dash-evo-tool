use crate::app::AppAction;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::identities::add_new_identity_screen::FundingMethod;
use crate::ui::identities::top_up_identity_screen::{TopUpIdentityScreen, WalletFundedScreenStep};
use crate::ui::theme::DashColors;
use egui::{Color32, Frame, Margin, RichText, Ui};

impl TopUpIdentityScreen {
    fn show_wallet_balance(&self, ui: &mut egui::Ui) {
        if let Some(selected_wallet) = &self.wallet {
            let total_balance = selected_wallet
                .read()
                .ok()
                .and_then(|g| g.platform_wallet.as_ref().map(|pw| pw.core().balance().total()))
                .unwrap_or(0);

            let dash_balance = total_balance as f64 * 1e-8;

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

        ui.heading(format!(
            "{}. How much of your wallet balance would you like to transfer?",
            step_number
        ));

        ui.add_space(10.0);
        self.show_wallet_balance(ui);
        ui.add_space(5.0);

        self.top_up_funding_amount_input(ui);

        // Extract the step from the RwLock to minimize borrow scope
        let step = *self.step.read().unwrap();

        let Ok(_) = self.funding_amount.parse::<f64>() else {
            return action;
        };

        // Fee estimation display
        let fee_estimator = self.app_context.fee_estimator();
        let estimated_fee = fee_estimator.estimate_identity_topup();

        let dark_mode = ui.ctx().style().visuals.dark_mode;
        Frame::new()
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(10, 8))
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Estimated fee:")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(14.0),
                    );
                    ui.label(
                        RichText::new(format_credits_as_dash(estimated_fee))
                            .color(DashColors::text_primary(dark_mode))
                            .size(14.0),
                    );
                });
            });

        ui.add_space(10.0);

        // Top up button
        let mut new_style = (**ui.style()).clone();
        new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
        ui.set_style(new_style);
        let button = egui::Button::new(RichText::new("Top Up Identity").color(Color32::WHITE))
            .fill(DashColors::DASH_BLUE)
            .frame(true)
            .corner_radius(3.0);
        if ui.add(button).clicked() {
            action = self.top_up_identity_clicked(FundingMethod::UseWalletBalance);
        }

        ui.add_space(20.0);

        ui.vertical_centered(|ui| {
            match step {
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
            };
        });

        ui.add_space(40.0);
        action
    }
}
