use crate::app::AppAction;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::MessageType;
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::identities::add_new_identity_screen::{
    AddNewIdentityScreen, FundingMethod, WalletFundedScreenStep,
};
use crate::ui::theme::DashColors;
use egui::{RichText, Ui};
use platform_wallet::wallet::asset_lock::tracked::{AssetLockStatus, TrackedAssetLock};

impl AddNewIdentityScreen {
    fn render_choose_funding_asset_lock(&mut self, ui: &mut egui::Ui) {
        let Some(selected_wallet) = self.selected_wallet.clone() else {
            ui.label("No wallet selected.");
            return;
        };

        let seed_hash = match selected_wallet.read() {
            Ok(w) => w.seed_hash(),
            Err(_) => {
                ui.label("Wallet is busy. Try again in a moment.");
                return;
            }
        };

        let Some(all_tracked) = self.asset_lock_cache.get(&seed_hash) else {
            ui.label("Loading asset locks…");
            return;
        };

        // Show only locks that are still actionable for a fresh identity
        // (Built / Broadcast / IS-Locked / Chain-Locked). Consumed locks
        // are tracked for history but cannot fund a new identity.
        let tracked: Vec<TrackedAssetLock> = all_tracked
            .iter()
            .filter(|t| !matches!(t.status, AssetLockStatus::Consumed))
            .cloned()
            .collect();

        if tracked.is_empty() {
            ui.label("No unused asset locks available.");
            return;
        }

        ui.heading("Select an unused asset lock:");
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
                                ui.colored_label(DashColors::SUCCESS, "Selected asset lock");
                            }

                            ui.label(format!("TxID: {}", lock.out_point.txid));
                            ui.label(format!("Vout: {}", lock.out_point.vout));
                            ui.label(format!(
                                "Amount: {:.8} DASH",
                                lock.amount as f64 * 1e-8
                            ));
                            ui.label(format!("Status: {:?}", lock.status));

                            ui.add_space(6.0);

                            if lock.proof.is_some() {
                                if ui.button("Select").clicked() {
                                    self.funding_asset_lock = Some(lock.out_point);
                                    let mut step = self.step.write().unwrap();
                                    *step = WalletFundedScreenStep::ReadyToCreate;
                                }
                            } else if ui.button("Select").clicked() {
                                MessageBanner::set_global(
                                    ui.ctx(),
                                    "Asset lock proof is not yet available. Wait for the transaction to chain-lock and try again.",
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
        let step = *self.step.read().unwrap();

        ui.heading(
            format!(
                "{}. Choose the unused asset lock that you would like to use.",
                step_number
            )
            .as_str(),
        );
        ui.add_space(10.0);
        self.render_choose_funding_asset_lock(ui);

        let key_count = self.identity_keys.others.len() + 1; // +1 for master key
        let estimated_fee = self
            .app_context
            .fee_estimator()
            .estimate_identity_create(key_count);
        ui.add_space(10.0);
        let dark_mode = ui.ctx().style().visuals.dark_mode;
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

        if ui.button("Create Identity").clicked() {
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
