use crate::app::AppAction;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::MessageType;
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::identities::add_new_identity_screen::FundingMethod;
use crate::ui::identities::top_up_identity_screen::{TopUpIdentityScreen, WalletFundedScreenStep};
use crate::ui::theme::DashColors;
use egui::{Color32, Frame, Margin, RichText, Ui};
use platform_wallet::wallet::asset_lock::tracked::{AssetLockStatus, TrackedAssetLock};

impl TopUpIdentityScreen {
    fn render_choose_funding_asset_lock(&mut self, ui: &mut egui::Ui) {
        let Some(selected_wallet) = self.wallet.clone() else {
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

        let backend = match self.app_context.wallet_backend() {
            Ok(b) => b,
            Err(_) => {
                ui.label("Wallet backend is not ready yet. Try again in a moment.");
                return;
            }
        };

        let tracked: Vec<TrackedAssetLock> = backend
            .list_tracked_asset_locks_blocking(&seed_hash)
            .into_iter()
            .filter(|t| !matches!(t.status, AssetLockStatus::Consumed))
            .collect();

        if tracked.is_empty() {
            ui.label("No unused asset locks available.");
            return;
        }

        ui.heading("Select an unused asset lock:");

        egui::ScrollArea::vertical().show(ui, |ui| {
            for lock in &tracked {
                ui.horizontal(|ui| {
                    let selected_text = if self.funding_asset_lock == Some(lock.out_point) {
                        " (Selected)"
                    } else {
                        ""
                    };
                    ui.label(format!(
                        "TxID: {}, Vout: {}, Amount: {:.8} DASH, Status: {:?}{}",
                        lock.out_point.txid,
                        lock.out_point.vout,
                        lock.amount as f64 * 1e-8,
                        lock.status,
                        selected_text,
                    ));
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
        ui.add_space(10.0);

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

        let mut new_style = (**ui.style()).clone();
        new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
        ui.set_style(new_style);
        let button = egui::Button::new(RichText::new("Top Up Identity").color(Color32::WHITE))
            .fill(DashColors::DASH_BLUE)
            .frame(true)
            .corner_radius(3.0);
        if ui.add(button).clicked() {
            action |= self.top_up_identity_clicked(FundingMethod::UseUnusedAssetLock);
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
