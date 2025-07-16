use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::identity::{IdentityTask, IdentityTopUpInfo, TopUpIdentityFundingMethod};
use crate::ui::identities::funding_common::{copy_to_clipboard, generate_qr_code_image};
use crate::ui::identities::top_up_identity_screen::{TopUpIdentityScreen, WalletFundedScreenStep};
use dash_sdk::dashcore_rpc::RpcApi;
use eframe::epaint::TextureHandle;
use egui::{Color32, Ui};
use std::sync::Arc;

impl TopUpIdentityScreen {
    fn render_qr_code(&mut self, ui: &mut egui::Ui, amount: f64) -> Result<(), String> {
        let address = {
            if let Some(wallet_guard) = self.wallet.as_ref() {
                // Get the receive address from the selected wallet
                if self.funding_address.is_none() {
                    let mut wallet = wallet_guard.write().unwrap();
                    let receive_address = wallet.receive_address(
                        self.app_context.network,
                        false,
                        Some(&self.app_context),
                    )?;

                    // Import address to Core if needed for monitoring
                    let core_client = self
                        .app_context
                        .core_client
                        .read()
                        .expect("Core client lock was poisoned");

                    let info = core_client
                        .get_address_info(&receive_address)
                        .map_err(|e| e.to_string())?;

                    if !(info.is_watchonly || info.is_mine) {
                        core_client
                            .import_address(
                                &receive_address,
                                Some("Managed by Dash Evo Tool"),
                                Some(false),
                            )
                            .map_err(|e| e.to_string())?;
                    }

                    drop(core_client);

                    self.funding_address = Some(receive_address.clone());
                    receive_address
                } else {
                    self.funding_address.as_ref().unwrap().clone()
                }
            } else {
                return Err("No wallet selected".to_string());
            }
        };

        let pay_uri = format!("{}?amount={:.4}", address.to_qr_uri(), amount);

        // Generate the QR code image
        if let Ok(qr_image) = generate_qr_code_image(&pay_uri) {
            let texture: TextureHandle =
                ui.ctx()
                    .load_texture("qr_code", qr_image, egui::TextureOptions::LINEAR);
            ui.image(&texture);
        } else {
            ui.label("Failed to generate QR code.");
        }

        ui.add_space(15.0);

        ui.label(&pay_uri);
        ui.add_space(5.0);

        if ui.button("Copy Address").clicked() {
            if let Err(e) = copy_to_clipboard(pay_uri.as_str()) {
                self.copied_to_clipboard = Some(Some(e));
            } else {
                self.copied_to_clipboard = Some(None);
            }
        }

        if let Some(error) = self.copied_to_clipboard.as_ref() {
            ui.add_space(5.0);
            if let Some(error) = error {
                ui.label(format!("Failed to copy to clipboard: {}", error));
            } else {
                ui.label("Address copied to clipboard.");
            }
        }

        Ok(())
    }

    pub fn render_ui_by_wallet_qr_code(&mut self, ui: &mut Ui, step_number: u32) -> AppAction {
        // Extract the step from the RwLock to minimize borrow scope
        let step = *self.step.read().unwrap();

        ui.heading(
            format!(
                "{}. Select how much you would like to transfer?",
                step_number
            )
            .as_str(),
        );

        ui.add_space(8.0);

        self.top_up_funding_amount_input(ui);

        let response = ui.vertical_centered(|ui| {
            // Only try to render QR code if we have a valid amount
            if let Ok(amount_dash) = self.funding_amount.parse::<f64>() {
                if amount_dash > 0.0 {
                    if let Err(e) = self.render_qr_code(ui, amount_dash) {
                        self.error_message = Some(e);
                    }
                } else {
                    ui.label("Please enter an amount greater than 0");
                }
            } else if !self.funding_amount.is_empty() {
                ui.label("Please enter a valid amount");
            }

            ui.add_space(20.0);

            if let Some(error_message) = self.error_message.as_ref() {
                ui.colored_label(Color32::DARK_RED, error_message);
                ui.add_space(20.0);
            }

            match step {
                WalletFundedScreenStep::ChooseFundingMethod => {}
                WalletFundedScreenStep::WaitingOnFunds => {
                    ui.heading("=> Waiting for funds. <=");
                }
                WalletFundedScreenStep::FundsReceived => {
                    let Some(selected_wallet) = &self.wallet else {
                        return AppAction::None;
                    };
                    if let Some((utxo, tx_out, address)) = self.funding_utxo.clone() {
                        let wallet_index = self.identity.wallet_index.unwrap_or(u32::MAX >> 1);
                        let top_up_index = self
                            .identity
                            .top_ups
                            .keys()
                            .max()
                            .cloned()
                            .map(|i| i + 1)
                            .unwrap_or_default();
                        let identity_input = IdentityTopUpInfo {
                            qualified_identity: self.identity.clone(),
                            wallet: Arc::clone(selected_wallet),
                            identity_funding_method: TopUpIdentityFundingMethod::FundWithUtxo(
                                utxo,
                                tx_out,
                                address,
                                wallet_index,
                                top_up_index,
                            ),
                        };

                        let mut step = self.step.write().unwrap();
                        *step = WalletFundedScreenStep::WaitingForAssetLock;

                        return AppAction::BackendTask(BackendTask::IdentityTask(
                            IdentityTask::TopUpIdentity(identity_input),
                        ));
                    }
                }
                WalletFundedScreenStep::ReadyToCreate => {}
                WalletFundedScreenStep::WaitingForAssetLock => {
                    ui.heading("=> Waiting for Core Chain to produce proof of transfer of funds. <=");
                    ui.add_space(20.0);
                    ui.label("NOTE: If this gets stuck, the funds were likely either transferred to the wallet or asset locked,\nand you can use the funding method selector in step 1 to change the method and use those funds to complete the process.");
                }
                WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                    ui.heading("=> Waiting for Platform acknowledgement. <=");
                    ui.add_space(20.0);
                    ui.label("NOTE: If this gets stuck, the funds were likely either transferred to the wallet or asset locked,\nand you can use the funding method selector in step 1 to change the method and use those funds to complete the process.");
                }
                WalletFundedScreenStep::Success => {
                    ui.heading("...Success...");
                }
            }
            AppAction::None
        });

        ui.add_space(40.0);

        response.inner
    }
}
