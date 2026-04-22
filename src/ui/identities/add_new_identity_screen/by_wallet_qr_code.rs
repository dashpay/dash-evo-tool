use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::{
    IdentityRegistrationInfo, IdentityTask, RegisterIdentityFundingMethod,
};
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::identities::add_new_identity_screen::{
    AddNewIdentityScreen, WalletFundedScreenStep,
};
use crate::ui::identities::funding_common::{self, copy_to_clipboard, generate_qr_code_image};
use eframe::epaint::TextureHandle;
use egui::Ui;
use std::sync::Arc;

impl AddNewIdentityScreen {
    fn render_qr_code(&mut self, ui: &mut egui::Ui, amount: f64) -> Result<(), TaskError> {
        let (address, _should_check_balance) = {
            // Scope the write lock to ensure it's dropped before calling `start_balance_check`.

            if let Some(wallet_guard) = self.selected_wallet.as_ref() {
                // Get the receive address
                if self.funding_address.is_none() {
                    let mut wallet = wallet_guard.write().unwrap();
                    let receive_address = wallet
                        .receive_address(self.app_context.network, true, Some(&self.app_context))
                        .map_err(|e| TaskError::WalletAddressDerivationFailed { detail: e })?;
                    let core_wallet_name = wallet.core_wallet_name.clone();
                    drop(wallet);

                    if let Some(has_address) = self.core_has_funding_address {
                        if !has_address {
                            self.app_context.ensure_address_imported(
                                &receive_address,
                                core_wallet_name.as_deref(),
                                Some("Managed by Dash Evo Tool"),
                            )?;
                        }
                        self.funding_address = Some(receive_address);
                    } else {
                        self.app_context.ensure_address_imported(
                            &receive_address,
                            core_wallet_name.as_deref(),
                            Some("Managed by Dash Evo Tool"),
                        )?;
                        self.funding_address = Some(receive_address);
                        self.core_has_funding_address = Some(true);
                    }

                    // Extract the address to return it outside this scope
                    (self.funding_address.as_ref().unwrap().clone(), true)
                } else {
                    (self.funding_address.as_ref().unwrap().clone(), false)
                }
            } else {
                return Err(TaskError::WalletNotFound);
            }
        };

        // if should_check_balance {
        //     // Now `address` is available, and all previous borrows are dropped.
        //     self.start_balance_check(&address, ui.ctx());
        // }

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

        ui.add_space(10.0);

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
        // Update state when funds land on the QR funding address
        if let Some(utxo) = funding_common::capture_qr_funding_utxo_if_available(
            &self.step,
            self.selected_wallet.as_ref(),
            self.funding_address.as_ref(),
        ) {
            self.funding_utxo = Some(utxo);
        }

        // Extract the step from the RwLock to minimize borrow scope
        let step = *self.step.read().unwrap();

        ui.add_space(10.0);

        ui.heading(
            format!(
                "{}. Select how much you would like to transfer?",
                step_number
            )
            .as_str(),
        );

        ui.add_space(8.0);

        self.render_funding_amount_input(ui);

        if step == WalletFundedScreenStep::WaitingOnFunds {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(1));
        }

        // Get the amount in DASH from the Amount struct
        let Some(amount) = &self.funding_amount else {
            return AppAction::None;
        };

        let amount_dash = amount.value() as f64 / 100_000_000_000.0; // credits to DASH

        if amount_dash <= 0.0 {
            return AppAction::None;
        }

        let response = ui.with_layout(
            egui::Layout::top_down(egui::Align::Min).with_cross_align(egui::Align::Center),
            |ui| {
                if let Err(e) = self.render_qr_code(ui, amount_dash) {
                    MessageBanner::set_global(
                        ui.ctx(),
                        e.to_string(),
                        MessageType::Error,
                    );
                }

                ui.add_space(20.0);

                // Handle FundsReceived action regardless of error state
                if step == WalletFundedScreenStep::FundsReceived {
                    let Some(selected_wallet) = &self.selected_wallet else {
                        return AppAction::None;
                    };
                    if let Some((utxo, tx_out, address)) = self.funding_utxo.clone() {
                        let identity_input = IdentityRegistrationInfo {
                            alias_input: self.alias_input.clone(),
                            keys: self.identity_keys.clone(),
                            wallet: Arc::clone(selected_wallet), // Clone the Arc reference
                            wallet_identity_index: self.identity_id_number,
                            identity_funding_method: RegisterIdentityFundingMethod::FundWithUtxo(
                                utxo,
                                tx_out,
                                address,
                                self.identity_id_number,
                            ),
                        };

                        let mut step = self.step.write().unwrap();
                        *step = WalletFundedScreenStep::WaitingForAssetLock;

                        // Create the backend task to register the identity
                        return AppAction::BackendTask(BackendTask::IdentityTask(
                            IdentityTask::RegisterIdentity(identity_input),
                        ));
                    }
                }

                {
                    match step {
                        WalletFundedScreenStep::WaitingOnFunds => {
                            ui.heading("=> Waiting for funds. <=");
                        }
                        WalletFundedScreenStep::WaitingForAssetLock => {
                            ui.heading(
                                "=> Waiting for Core Chain to produce proof of transfer of funds. <=",
                            );
                        }
                        WalletFundedScreenStep::WaitingForPlatformAcceptance => {
                            ui.heading("=> Waiting for Platform acknowledgement. <=");
                        }
                        WalletFundedScreenStep::Success => {
                            ui.heading("...Success...");
                        }
                        _ => {}
                    }
                }
                AppAction::None
            },
        );

        ui.add_space(40.0);

        response.inner
    }
}
