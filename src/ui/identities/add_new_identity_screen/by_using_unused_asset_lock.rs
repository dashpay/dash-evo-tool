use crate::app::AppAction;
use crate::lock_helper::RwLockExt;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::identities::add_new_identity_screen::{
    AddNewIdentityScreen, FundingMethod, WalletFundedScreenStep,
};
use egui::{Color32, RichText, Ui};

impl AddNewIdentityScreen {
    fn render_choose_funding_asset_lock(&mut self, ui: &mut egui::Ui) {
        // Ensure a wallet is selected
        let Some(selected_wallet) = self.selected_wallet.clone() else {
            ui.label("No wallet selected.");
            return;
        };

        // Read the wallet to access unused asset locks
        let wallet = selected_wallet.read_or_recover();

        if wallet.unused_asset_locks.is_empty() {
            ui.label("No unused asset locks available.");
            return;
        }

        ui.heading("Select an unused asset lock:");
        ui.add_space(8.0);

        // Track the index of the currently selected asset lock (if any)
        let selected_index = self.funding_asset_lock.as_ref().and_then(|(_, proof, _)| {
            wallet
                .unused_asset_locks
                .iter()
                .position(|(_, _, _, _, p)| p.as_ref() == Some(proof))
        });

        // Display the asset locks in a scrollable area
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .min_scrolled_height(180.0)
            .show(ui, |ui| {
                for (index, (tx, address, amount, islock, proof)) in
                    wallet.unused_asset_locks.iter().enumerate()
                {
                    ui.group(|ui| {
                        ui.vertical(|ui| {
                            let tx_id = tx.txid().to_string();
                            let lock_amount = *amount as f64 * 1e-8; // Convert to DASH
                            let is_locked = if islock.is_some() { "Yes" } else { "No" };

                            // Display asset lock information with "Selected" if this one is selected
                            if Some(index) == selected_index {
                                ui.colored_label(
                                    Color32::from_rgb(0, 130, 90),
                                    "Selected asset lock",
                                );
                            }

                            ui.label(format!("TxID: {}", tx_id));
                            ui.label(format!("Address: {}", address));
                            ui.label(format!("Amount: {:.8} DASH", lock_amount));
                            ui.label(format!("InstantLock: {}", is_locked));

                            ui.add_space(6.0);

                            // Button to select this asset lock stays visible regardless of wrapping
                            if ui.button("Select").clicked() {
                                // Update the selected asset lock
                                self.funding_asset_lock = Some((
                                    tx.clone(),
                                    proof.clone().expect("Asset lock proof is required"),
                                    address.clone(),
                                ));

                                // Update the step to ready to create identity
                                let mut step = self.step.write_or_recover();
                                *step = WalletFundedScreenStep::ReadyToCreate;
                            }
                        });
                    });

                    ui.add_space(6.0); // Add space between each entry
                }
            });
    }

    pub fn render_ui_by_using_unused_asset_lock(
        &mut self,
        ui: &mut Ui,
        step_number: u32,
    ) -> AppAction {
        let mut action = AppAction::None;

        // Extract the step from the RwLock to minimize borrow scope
        let step = *self.step.read_or_recover();

        ui.heading(
            format!(
                "{}. Choose the unused asset lock that you would like to use.",
                step_number
            )
            .as_str(),
        );
        ui.add_space(10.0);
        self.render_choose_funding_asset_lock(ui);

        // Display estimated fee before action button
        let key_count = self.identity_keys.keys_input.len() + 1; // +1 for master key
        let estimated_fee = self
            .app_context
            .fee_estimator()
            .estimate_identity_create(key_count);
        ui.add_space(10.0);
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        egui::Frame::new()
            .fill(crate::ui::theme::DashColors::surface(dark_mode))
            .inner_margin(egui::Margin::symmetric(10, 8))
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Estimated Fee:")
                            .color(crate::ui::theme::DashColors::text_secondary(dark_mode)),
                    );
                    ui.label(
                        RichText::new(format_credits_as_dash(estimated_fee))
                            .color(crate::ui::theme::DashColors::text_primary(dark_mode))
                            .strong(),
                    );
                });
            });
        ui.add_space(10.0);

        if ui.button("Create Identity").clicked() {
            self.error_message = None;
            action |= self.register_identity_clicked(FundingMethod::UseUnusedAssetLock);
        }

        ui.add_space(20.0);

        // Only show status messages if there's no error
        if self.error_message.is_none() {
            ui.vertical_centered(|ui| match step {
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
