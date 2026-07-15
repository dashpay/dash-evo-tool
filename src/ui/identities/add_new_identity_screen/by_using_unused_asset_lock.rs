use crate::app::AppAction;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::MessageType;
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::identities::add_new_identity_screen::{
    AddNewIdentityScreen, FundingMethod, WalletFundedScreenStep,
};
use crate::ui::identities::funding_common::{
    FundingAssetLockPicker, actionable_asset_locks, asset_lock_address, asset_lock_status_label,
};
use crate::ui::theme::{ComponentStyles, DashColors};
use egui::{Color32, RichText, Ui};

impl AddNewIdentityScreen {
    fn render_choose_funding_asset_lock(&mut self, ui: &mut egui::Ui) {
        let tracked = match actionable_asset_locks(
            ui,
            &mut self.asset_lock_cache,
            self.selected_wallet.as_ref(),
        ) {
            FundingAssetLockPicker::Handled => return,
            FundingAssetLockPicker::Available(tracked) => tracked,
        };

        ui.heading("Select the unfinished funding to use:");
        ui.add_space(8.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .min_scrolled_height(180.0)
            .show(ui, |ui| {
                for lock in &tracked {
                    let is_selected = self.funding_asset_lock == Some(lock.out_point);
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            if is_selected {
                                ui.colored_label(DashColors::SUCCESS, "Selected");
                            }

                            if self.show_advanced_options {
                                ui.label(format!("TxID: {}", lock.out_point.txid));
                                ui.label(format!("Vout: {}", lock.out_point.vout));
                            }
                            if let Some(address) =
                                asset_lock_address(lock, self.app_context.network)
                            {
                                ui.label(format!("Address: {address}"));
                            }
                            ui.label(format!(
                                "Amount: {:.8} DASH",
                                lock.amount as f64 * 1e-8
                            ));
                            ui.label(format!("Status: {}", asset_lock_status_label(&lock.status)));

                            ui.add_space(6.0);

                            if lock.proof.is_some() {
                                if ui.button("Select").clicked() {
                                    self.funding_asset_lock = Some(lock.out_point);
                                    if let Ok(mut step) = self.step.write() {
                                        *step = WalletFundedScreenStep::ReadyToCreate;
                                    }
                                }
                            } else if ui.button("Select").clicked() {
                                MessageBanner::set_global(
                                    ui.ctx(),
                                    "This funding isn't ready to use yet. Wait for it to be confirmed on the Dash network, then try again.",
                                    MessageType::Warning,
                                );
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
            });
    }

    pub fn render_ui_by_using_unused_asset_lock(
        &mut self,
        ui: &mut Ui,
        step_number: u32,
    ) -> AppAction {
        let mut action = AppAction::None;
        let step = self
            .step
            .read()
            .map(|s| *s)
            .unwrap_or(WalletFundedScreenStep::ChooseFundingMethod);

        ui.heading(
            format!("{step_number}. Choose the unfinished funding you'd like to use.").as_str(),
        );
        ui.add_space(10.0);
        self.render_choose_funding_asset_lock(ui);

        let key_count = self.identity_keys.others.len() + 1; // +1 for master key
        let estimated_fee = self
            .app_context
            .fee_estimator()
            .estimate_identity_create(key_count);
        ui.add_space(10.0);
        let dark_mode = ui.style().visuals.dark_mode;
        egui::Frame::new()
            .fill(DashColors::surface(dark_mode))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Estimated Fee:")
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(format_credits_as_dash(estimated_fee))
                            .color(DashColors::text_primary(dark_mode))
                            .strong(),
                    );
                });
            });
        ui.add_space(10.0);

        self.render_alias_input(ui, step_number + 1);

        let can_create = self.funding_asset_lock.is_some();
        let button = egui::Button::new(RichText::new("Create Identity").color(Color32::WHITE))
            .fill(if can_create {
                DashColors::DASH_BLUE
            } else {
                ComponentStyles::button_disabled_fill(dark_mode)
            })
            .frame(true)
            .corner_radius(3.0);
        if ui.add_enabled(can_create, button).clicked() {
            action |= self.register_identity_clicked(FundingMethod::UseUnusedAssetLock);
        }

        ui.add_space(20.0);

        ui.vertical_centered(|ui| match step {
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                ui.heading("=> Waiting for Platform acknowledgement <=");
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
