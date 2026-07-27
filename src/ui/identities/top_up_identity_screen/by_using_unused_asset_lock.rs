use crate::app::AppAction;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::MessageType;
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::identities::funding_common::{
    FundingAssetLockPicker, FundingMethod, actionable_asset_locks, asset_lock_address,
    asset_lock_status_label,
};
use crate::ui::identities::top_up_identity_screen::{TopUpIdentityScreen, WalletFundedScreenStep};
use crate::ui::theme::DashColors;
use egui::{Color32, Frame, Margin, RichText, Ui};

impl TopUpIdentityScreen {
    fn render_choose_funding_asset_lock(&mut self, ui: &mut egui::Ui) {
        let tracked =
            match actionable_asset_locks(ui, &mut self.asset_lock_cache, self.wallet.as_ref()) {
                FundingAssetLockPicker::Handled => return,
                FundingAssetLockPicker::Available(tracked) => tracked,
            };

        ui.heading("Select the unfinished funding to use:");

        egui::ScrollArea::vertical().show(ui, |ui| {
            for lock in &tracked {
                ui.horizontal(|ui| {
                    let selected_text = if self.funding_asset_lock == Some(lock.out_point) {
                        " (Selected)"
                    } else {
                        ""
                    };
                    let address_text = match asset_lock_address(lock, self.app_context.network) {
                        Some(address) => format!(", Address: {address}"),
                        None => String::new(),
                    };
                    ui.label(format!(
                        "TxID: {}, Vout: {}{}, Amount: {:.8} DASH, Status: {}{}",
                        lock.out_point.txid,
                        lock.out_point.vout,
                        address_text,
                        lock.amount as f64 * 1e-8,
                        asset_lock_status_label(&lock.status),
                        selected_text,
                    ));
                    if lock.proof.is_some() {
                        if ui.button("Select").clicked() {
                            self.funding_asset_lock = Some(lock.out_point);
                            self.set_step(WalletFundedScreenStep::ReadyToCreate);
                        }
                    } else if ui.button("Select").clicked() {
                        MessageBanner::set_global(
                            ui.ctx(),
                            "This funding isn't ready to use yet. Wait for it to be confirmed on the Dash network, then try again.",
                            MessageType::Warning,
                        );
                    }
                });
                ui.add_space(5.0);
            }
        });
    }

    pub fn render_ui_by_using_unused_asset_lock(
        &mut self,
        ui: &mut Ui,
        step_number: u32,
    ) -> AppAction {
        let mut action = AppAction::None;
        let step = self.current_step();

        ui.heading(
            format!("{step_number}. Choose the unfinished funding you'd like to use.").as_str(),
        );
        ui.add_space(10.0);
        self.render_choose_funding_asset_lock(ui);
        ui.add_space(10.0);

        let fee_estimator = self.app_context.fee_estimator();
        let estimated_fee = fee_estimator.estimate_identity_topup();

        let dark_mode = ui.style().visuals.dark_mode;
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

        let mut new_style = (**ui.style()).clone();
        new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
        ui.set_style(new_style);
        let button = egui::Button::new(RichText::new("Add funds").color(Color32::WHITE))
            .fill(DashColors::DASH_BLUE)
            .frame(true)
            .corner_radius(3.0);
        if ui.add(button).clicked() {
            action |= self.top_up_identity_clicked(FundingMethod::UseUnusedAssetLock);
        }

        ui.add_space(20.0);

        ui.vertical_centered(|ui| match step {
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                ui.heading("Waiting for Platform to add the funds to the identity.");
            }
            WalletFundedScreenStep::Success => {
                ui.heading("...Success...");
            }
            _ => {}
        });

        ui.add_space(40.0);
        action
    }
}
