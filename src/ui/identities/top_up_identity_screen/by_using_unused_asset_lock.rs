use crate::app::AppAction;
use crate::ui::identities::add_new_identity_screen::FundingMethod;
use crate::ui::identities::top_up_identity_screen::{TopUpIdentityScreen, WalletFundedScreenStep};
use egui::Ui;

impl TopUpIdentityScreen {
    fn render_choose_funding_asset_lock(&mut self, ui: &mut egui::Ui) {
        // Ensure a wallet is selected
        let Some(selected_wallet) = self.selected_wallet.clone() else {
            ui.label("No wallet selected.");
            return;
        };

        // Read the wallet to access unused asset locks
        let wallet = selected_wallet.read().unwrap();

        if wallet.unused_asset_locks.is_empty() {
            ui.label("No unused asset locks available.");
            return;
        }

        ui.heading("Select an unused asset lock:");

        // Track the index of the currently selected asset lock (if any)
        let selected_index = self.funding_asset_lock.as_ref().and_then(|(_, proof, _)| {
            wallet
                .unused_asset_locks
                .iter()
                .position(|(_, _, _, _, p)| p.as_ref() == Some(proof))
        });

        // Display the asset locks in a scrollable area
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, (tx, address, amount, islock, proof)) in
                wallet.unused_asset_locks.iter().enumerate()
            {
                ui.horizontal(|ui| {
                    let tx_id = tx.txid().to_string();
                    let lock_amount = *amount as f64 * 1e-8; // Convert to DASH
                    let is_locked = if islock.is_some() { "Yes" } else { "No" };

                    // Display asset lock information with "Selected" if this one is selected
                    let selected_text = if Some(index) == selected_index {
                        " (Selected)"
                    } else {
                        ""
                    };

                    ui.label(format!(
                        "TxID: {}, Address: {}, Amount: {:.8} DASH, InstantLock: {}{}",
                        tx_id, address, lock_amount, is_locked, selected_text
                    ));

                    // Button to select this asset lock
                    if ui.button("Select").clicked() {
                        // Update the selected asset lock
                        self.funding_asset_lock = Some((
                            tx.clone(),
                            proof.clone().expect("Asset lock proof is required"),
                            address.clone(),
                        ));

                        // Update the step to ready to create identity
                        let mut step = self.step.write().unwrap();
                        *step = WalletFundedScreenStep::ReadyToCreate;
                    }
                });

                ui.add_space(5.0); // Add space between each entry
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
        let step = self.step.read().unwrap().clone();

        ui.heading(
            format!(
                "{}. Choose the unused asset lock that you would like to use.",
                step_number
            )
            .as_str(),
        );
        ui.add_space(10.0);
        self.render_choose_funding_asset_lock(ui);

        if ui.button("Create Identity").clicked() {
            self.error_message = None;
            action |= self.register_identity_clicked(FundingMethod::UseUnusedAssetLock);
        }

        match step {
            WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                ui.heading("Waiting for Platform acknowledgement");
            }
            WalletFundedScreenStep::Success => {
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
