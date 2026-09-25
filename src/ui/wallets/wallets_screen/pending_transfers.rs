use super::WalletsBalancesScreen;
use crate::model::fee_estimation::format_duffs_as_dash;
use crate::model::pending_transfers::{PendingPlatformTransfer, TransferStage};
use crate::model::spv_status::SpvStatus;
use crate::ui::components::component_trait::Component;
use crate::ui::theme::{ComponentStyles, DashColors, Spacing};
use eframe::egui::{self, RichText, Ui};

impl WalletsBalancesScreen {
    pub(super) fn render_unfinished_summary(&mut self, ui: &mut Ui) {
        let assessment = self.pending_transfers.assessment();
        let count = assessment.map(|assessment| assessment.transfers.len());
        let incomplete = assessment.is_some_and(|assessment| !assessment.history_complete);
        if count == Some(0) && !incomplete && !self.pending_transfers.is_failed() {
            return;
        }
        ui.add_space(Spacing::SM);
        ui.horizontal_wrapped(|ui| {
            match count {
                Some(count) => {
                    ui.strong(format!("Unfinished transfers: {count}"));
                }
                None => {
                    ui.label("Unfinished transfers: Status not yet available");
                }
            }
            if ComponentStyles::add_secondary_button(ui, "View transfers", ui.visuals().dark_mode)
                .clicked()
            {
                self.show_transfer_history = true;
            }
        });
        if self.pending_transfers.is_failed() {
            ui.label(
                "Transfer status could not be checked. Open the transfer history and try again.",
            );
        } else if incomplete {
            ui.label("Some saved transactions could not be loaded. Open the transfer history to review the available information.");
        } else if count.is_some_and(|count| count > 0) {
            ui.label("Some funds may be unavailable while these transfers are unresolved. Review the transfers before sending again.");
        }
    }

    pub(super) fn render_transfer_history(&mut self, ui: &mut Ui) {
        let open = self.show_transfer_history.then_some(true);
        let response = egui::CollapsingHeader::new("Transaction History")
            .id_salt("wallet_transfer_history")
            .open(open)
            .show(ui, |ui| {
                self.render_pending_transfers(ui);
                self.render_transactions_section(ui);
            });
        if self.show_transfer_history {
            response
                .header_response
                .scroll_to_me(Some(egui::Align::Min));
            self.show_transfer_history = false;
        }
    }

    fn render_pending_transfers(&mut self, ui: &mut Ui) {
        self.pending_transfer_error.show(ui);
        let assessment = self.pending_transfers.assessment().cloned();
        ui.horizontal_wrapped(|ui| {
            ui.strong("Unfinished transfers");
            let loading = self.pending_transfers.is_loading();
            if ui
                .add_enabled_ui(!loading, |ui| {
                    ComponentStyles::add_secondary_button(
                        ui,
                        "Check status",
                        ui.visuals().dark_mode,
                    )
                })
                .inner
                .clicked()
            {
                self.pending_transfers.refresh();
            }
            if self.pending_transfers.show_progress() {
                ui.spinner();
                ui.label("Checking transfer status…");
            }
        });
        ui.label("This view checks the latest information saved by your wallet. Keep the wallet connected to receive network updates.");
        if self.app_context.connection_status.spv_status() != SpvStatus::Running
            || self.app_context.connection_status.spv_connected_peers() == 0
        {
            ui.label("Status may be out of date. Connect and wait for wallet synchronization, then check again.");
        }
        if self.pending_transfers.is_failed() {
            ui.label("Transfer status could not be checked. Try again. Previously loaded information is still shown.");
        }
        let Some(assessment) = assessment else {
            ui.label("Transfer information is not yet available. Wait for the wallet to load, then check again.");
            return;
        };
        ui.label(format!(
            "Last wallet check: {time} UTC",
            time = Self::format_transaction_timestamp(assessment.checked_at)
        ));
        if !assessment.history_complete {
            ui.label("Some saved transactions could not be loaded. Restart the app to try again; missing records can hide payment conflicts.");
        }
        if assessment.transfers.is_empty() {
            ui.label("No unfinished Platform transfers were found in the loaded wallet records.");
        }
        for transfer in &assessment.transfers {
            self.render_pending_transfer(ui, transfer);
        }
    }

    fn render_pending_transfer(&self, ui: &mut Ui, transfer: &PendingPlatformTransfer) {
        let dark_mode = ui.visuals().dark_mode;
        let (status, explanation) = match transfer.stage {
            TransferStage::AwaitingConfirmation => (
                "Awaiting confirmation",
                "We have not confirmed this transfer yet. Check its status before sending again.",
            ),
            TransferStage::ConflictObserved => (
                "Checking a payment conflict",
                "Another payment may have used funds needed for this transfer. Check its status before trying again. Funds cannot yet be safely released.",
            ),
            TransferStage::DeliveryUnknown => (
                "Delivery not verified",
                "The funding transaction was confirmed, but Platform delivery has not been verified. Check its status before sending again.",
            ),
        };
        egui::Frame::group(ui.style())
            .inner_margin(Spacing::SM)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("Transfer to Platform");
                    ui.label(RichText::new(status).color(DashColors::WARNING));
                });
                ui.label(format!("Funding amount: {amount}", amount = format_duffs_as_dash(transfer.funding_amount)));
                ui.label(explanation);
                egui::CollapsingHeader::new("View details")
                    .id_salt((self.app_context.network, transfer.out_point))
                    .show(ui, |ui| {
                        ui.label(format!("Network: {network}", network = self.app_context.network));
                        ui.label("Recipient: Not recorded. The original Platform recipient cannot be inferred from the funding transaction.");
                        ui.label("The funding amount may include a fee allowance. The originally requested transfer amount is not recorded.");
                        ui.label("Temporarily unavailable: Not yet known. Additional funds may be unavailable while this transfer is unresolved.");
                        ui.label(format!("Core fee: {fee}", fee = transfer.core_fee.map(format_duffs_as_dash).unwrap_or_else(|| "Not yet known".to_string())));
                        ui.label("Platform fee: Not yet known.");
                        ui.label(format!("Confirmation date: {date}", date = transfer.block_time.map(|time| format!("{date} UTC", date = Self::format_transaction_timestamp(time))).unwrap_or_else(|| "Date unavailable".to_string())));
                        ui.label("Cancellation is not available for this transfer. Check its status for updates.");
                        ui.horizontal_wrapped(|ui| {
                            let txid = transfer.out_point.txid.to_string();
                            ui.label(RichText::new(&txid).monospace().color(DashColors::text_secondary(dark_mode)));
                            if ComponentStyles::add_secondary_button(ui, "Copy transaction ID", dark_mode).clicked() {
                                ui.ctx().copy_text(txid);
                            }
                        });
                        for conflict in &transfer.conflicts {
                            ui.label(format!("Competing transaction: {txid}", txid = conflict.competing_txid));
                            ui.label(format!("Shared input: {input}; recorded block height: {height}", input = conflict.input, height = conflict.height));
                        }
                    });
            });
        ui.add_space(Spacing::SM);
    }
}
