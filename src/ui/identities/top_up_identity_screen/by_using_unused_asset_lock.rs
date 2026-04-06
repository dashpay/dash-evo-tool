use crate::app::AppAction;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::ui::MessageType;
use crate::ui::components::message_banner::MessageBanner;
use crate::ui::identities::add_new_identity_screen::FundingMethod;
use crate::ui::identities::top_up_identity_screen::{TopUpIdentityScreen, WalletFundedScreenStep};
use crate::ui::theme::DashColors;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::dashcore::{Address, OutPoint};
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::InstantAssetLockProof;
use dash_sdk::dpp::identity::state_transition::asset_lock_proof::chain::ChainAssetLockProof;
use dash_sdk::dpp::prelude::AssetLockProof;
use egui::{Color32, Frame, Margin, RichText, Ui};

impl TopUpIdentityScreen {
    fn render_choose_funding_asset_lock(&mut self, ui: &mut egui::Ui) {
        // Ensure a wallet is selected
        let Some(selected_wallet) = self.wallet.clone() else {
            ui.label("No wallet selected.");
            return;
        };

        let network = self.app_context.network;

        // Read unused asset locks from the database (identity_id IS NULL).
        let wallet = selected_wallet.read().unwrap();
        let locks = self
            .app_context
            .db
            .get_unused_asset_lock_transactions_for_wallet(&wallet.seed_hash(), network)
            .unwrap_or_default();
        drop(wallet);

        if locks.is_empty() {
            ui.label("No unused asset locks available.");
            return;
        }

        ui.heading("Select an unused asset lock:");

        // Track the txid of the currently selected asset lock (if any)
        let selected_txid = self
            .funding_asset_lock
            .as_ref()
            .map(|(tx, _, _)| tx.txid());

        // Display the asset locks in a scrollable area
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (tx, amount, islock, chain_locked_height) in &locks {
                let txid = tx.txid();

                ui.horizontal(|ui| {
                    let lock_amount = *amount as f64 * 1e-8; // Convert to DASH
                    let is_locked = islock.is_some() || chain_locked_height.is_some();

                    let address_str = if let Some(TransactionPayload::AssetLockPayloadType(payload)) = &tx.special_transaction_payload {
                        payload.credit_outputs.first()
                            .and_then(|output| Address::from_script(&output.script_pubkey, network).ok())
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| "Unknown".to_string())
                    } else {
                        "Unknown".to_string()
                    };

                    // Display asset lock information with "Selected" if this one is selected
                    let selected_text = if selected_txid == Some(txid) {
                        " (Selected)"
                    } else {
                        ""
                    };

                    ui.label(format!(
                        "TxID: {}, Address: {}, Amount: {:.8} DASH, InstantLock: {}{}",
                        txid, address_str, lock_amount, if is_locked { "Yes" } else { "No" }, selected_text
                    ));

                    // Build proof from IS-lock or chain-locked height
                    let proof = if let Some(islock) = islock {
                        Some(AssetLockProof::Instant(InstantAssetLockProof::new(
                            islock.clone(),
                            tx.clone(),
                            0,
                        )))
                    } else {
                        chain_locked_height.map(|height| {
                            AssetLockProof::Chain(ChainAssetLockProof {
                                core_chain_locked_height: height,
                                out_point: OutPoint::new(txid, 0),
                            })
                        })
                    };

                    if let Some(asset_lock_proof) = proof {
                        if ui.button("Select").clicked() {
                            let address = if let Some(TransactionPayload::AssetLockPayloadType(payload)) = &tx.special_transaction_payload {
                                payload.credit_outputs.first()
                                    .and_then(|output| Address::from_script(&output.script_pubkey, network).ok())
                                    .unwrap_or_else(|| Address::from_script(&tx.output[0].script_pubkey, network).unwrap())
                            } else {
                                Address::from_script(&tx.output[0].script_pubkey, network).unwrap()
                            };

                            self.funding_asset_lock =
                                Some((tx.clone(), asset_lock_proof, address));

                            let mut step = self.step.write().unwrap();
                            *step = WalletFundedScreenStep::ReadyToCreate;
                        }
                    } else if ui.button("Select").clicked() {
                        MessageBanner::set_global(
                            ui.ctx(),
                            "Asset lock proof is not yet available — the transaction may not be chain-locked yet. Please try again later.",
                            MessageType::Warning,
                        );
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
