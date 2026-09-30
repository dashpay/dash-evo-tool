use super::WalletsBalancesScreen;
use crate::model::fee_estimation::format_duffs_as_dash;
use crate::model::pending_transfers::{PendingPlatformTransfer, TransferStage};
use crate::model::spv_status::SpvStatus;
use crate::ui::components::component_trait::Component;
use crate::ui::theme::ComponentStyles;
use eframe::egui::{self, Ui};

impl WalletsBalancesScreen {
    pub(super) fn render_transfer_history(&mut self, ui: &mut Ui) {
        egui::CollapsingHeader::new("Transaction History")
            .id_salt("wallet_transfer_history")
            .show(ui, |ui| self.render_transactions_section(ui));
        self.render_transfer_details(ui);
    }

    pub(super) fn render_history_status(&mut self, ui: &mut Ui) {
        self.pending_transfer_error.show(ui);
        if self.pending_transfers.show_progress() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Loading wallet records…");
            });
        }
        if self.app_context.connection_status.spv_status() != SpvStatus::Running
            || self.app_context.connection_status.spv_connected_peers() == 0
        {
            ui.label("Wallet records may be out of date. Connect and wait for synchronization.");
        }
        if let Some(assessment) = self.pending_transfers.assessment()
            && !assessment.history_complete
        {
            ui.label("Some saved transactions could not be loaded. Restart the app to try again.");
        }
    }

    pub(super) fn render_transaction_actions(
        &mut self,
        ui: &mut Ui,
        txid: dash_sdk::dpp::dashcore::Txid,
        has_funding: bool,
        unconfirmed: bool,
    ) {
        use crate::ui::theme::ResponseExt;
        ui.horizontal(|ui| {
            if ui.small_button("Copy").clickable_tooltip("Copy transaction ID").clicked() {
                ui.ctx().copy_text(txid.to_string());
            }
            let explorer = match self.app_context.network {
                dash_sdk::dpp::dashcore::Network::Mainnet => Some("https://insight.dash.org/insight/tx/"),
                dash_sdk::dpp::dashcore::Network::Testnet => Some("https://insight.testnet.networks.dash.org/insight/tx/"),
                _ => None,
            };
            if let Some(base) = explorer
                && ui.small_button("View").clickable_tooltip("View on block explorer").clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(format!("{base}{txid}")));
            }
            if has_funding && ui.small_button("Details").clicked() {
                self.transfer_details = Some(txid);
            }
            if has_funding && unconfirmed {
                ui.add_enabled(false, egui::Button::new("Cancel transfer").small())
                    .on_disabled_hover_text("The wallet backend does not support safe cancellation of a broadcast funding transaction. Open Details for the recorded evidence.");
            }
        });
    }

    fn render_transfer_details(&mut self, ui: &mut Ui) {
        let Some(txid) = self.transfer_details else {
            return;
        };
        let transfers: Vec<_> = self
            .pending_transfers
            .assessment()
            .into_iter()
            .flat_map(|a| a.transfers.iter())
            .filter(|t| t.out_point.txid == txid)
            .cloned()
            .collect();
        let mut open = true;
        egui::Window::new("Transfer details")
            .id(egui::Id::new(("transfer_details", self.app_context.network, txid)))
            .open(&mut open).resizable(true).default_width(360.0)
            .max_width((ui.ctx().content_rect().width() - 32.0).max(240.0))
            .vscroll(true).show(ui.ctx(), |ui| {
                ui.label(format!("Network: {network}", network = self.app_context.network));
                ui.label(format!("Transaction ID: {txid}"));
                ui.horizontal_wrapped(|ui| {
                    if ComponentStyles::add_secondary_button(ui, "Copy transaction ID", ui.visuals().dark_mode).clicked() {
                        ui.ctx().copy_text(txid.to_string());
                    }
                    let base = match self.app_context.network {
                        dash_sdk::dpp::dashcore::Network::Mainnet => Some("https://insight.dash.org/insight/tx/"),
                        dash_sdk::dpp::dashcore::Network::Testnet => Some("https://insight.testnet.networks.dash.org/insight/tx/"),
                        _ => None,
                    };
                    if let Some(base) = base
                        && ComponentStyles::add_secondary_button(ui, "Open Core explorer", ui.visuals().dark_mode).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(format!("{base}{txid}")));
                    }
                });
                ui.label("The Core explorer shows confirmation of the funding transaction. It does not verify delivery on Platform.");
                for transfer in &transfers {
                    self.render_funding_details(ui, transfer);
                }
            });
        if !open {
            self.transfer_details = None;
        }
    }

    fn render_funding_details(&self, ui: &mut Ui, transfer: &PendingPlatformTransfer) {
        ui.separator();
        ui.label(format!(
            "Funding amount: {amount}",
            amount = format_duffs_as_dash(transfer.funding_amount)
        ));
        ui.label(match transfer.stage {
            TransferStage::AwaitingConfirmation => "Core confirmation has not been recorded. Keep the wallet connected to receive updates.",
            TransferStage::ConflictObserved => "Another confirmed transaction spends an input used by this transfer. This wallet cannot yet safely cancel the conflicting transfer or release its funds.",
            TransferStage::DeliveryUnknown => "Core funding is confirmed. This wallet cannot verify delivery on Platform; Refresh does not perform that verification.",
            TransferStage::Recovered => "This historical funding record was recovered from the blockchain. Its Platform outcome is unknown; this does not mean a transfer is still pending.",
        });
        ui.label("Recipient: Not recorded.");
        ui.label("The funding amount may include a fee allowance. The originally requested transfer amount is not recorded.");
        ui.label(format!(
            "Core fee: {fee}",
            fee = transfer
                .core_fee
                .map(format_duffs_as_dash)
                .unwrap_or_else(|| "Not recorded".to_string())
        ));
        ui.label(format!(
            "Confirmation date: {date}",
            date = transfer
                .block_time
                .map(|time| format!(
                    "{date} UTC",
                    date = Self::format_transaction_timestamp(time)
                ))
                .unwrap_or_else(|| "Date unavailable".to_string())
        ));
        if matches!(
            transfer.stage,
            TransferStage::AwaitingConfirmation | TransferStage::ConflictObserved
        ) {
            ui.label("Cancellation is unavailable: the wallet backend cannot safely stop this funding transaction and release its inputs.");
        } else {
            ui.label("A confirmed Core funding transaction cannot be undone by deleting its wallet record.");
        }
        ui.label("Age alone does not prove that a transaction expired. Removing this record would not return funds.");
        for conflict in &transfer.conflicts {
            ui.label(format!(
                "Competing transaction: {txid}",
                txid = conflict.competing_txid
            ));
            ui.label(format!(
                "Shared input: {input}; recorded block height: {height}",
                input = conflict.input,
                height = conflict.height
            ));
        }
    }
}
