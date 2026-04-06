use crate::app::AppAction;
use crate::ui::ScreenType;
use crate::ui::theme::{DashColors, ResponseExt};
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::transaction::special_transaction::TransactionPayload;
use dash_sdk::dpp::dashcore::Address;
use eframe::egui::{self, Ui};
use egui::{Color32, Frame, Margin, RichText};
use egui_extras::{Column, TableBuilder};

use super::WalletsBalancesScreen;

impl WalletsBalancesScreen {
    pub(super) fn render_wallet_asset_locks(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let mut open_fund_dialog_for_txid: Option<([u8; 32], Vec<(String, u64)>)> = None;
        let mut recover_asset_locks_clicked = false;

        if let Some(arc_wallet) = &self.selected_wallet {
            let wallet = arc_wallet.read().unwrap();

            let network = self.app_context.network;

            // Read asset locks from the database (source of truth, includes consumed locks).
            let locks = self
                .app_context
                .db
                .get_asset_lock_transactions_for_wallet(&wallet.seed_hash(), network)
                .unwrap_or_default();

            let dark_mode = ui.ctx().style().visuals.dark_mode;
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .corner_radius(5.0)
                .inner_margin(Margin::same(15))
                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .show(ui, |ui| {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    ui.horizontal(|ui| {
                        ui.heading(RichText::new("Asset Locks").color(DashColors::text_primary(dark_mode)));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Create Asset Lock").clicked() {
                                app_action = AppAction::AddScreen(
                                    ScreenType::CreateAssetLock(arc_wallet.clone()).create_screen(&self.app_context)
                                );
                            }
                            if ui.button("Search for Unused").clickable_tooltip("Scan Core wallet for untracked asset locks").clicked() {
                                recover_asset_locks_clicked = true;
                            }
                        });
                    });
                    ui.add_space(10.0);

                    if locks.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(RichText::new("No asset locks found").color(Color32::GRAY).size(14.0));
                            ui.add_space(10.0);
                            ui.label(RichText::new("Asset locks are special transactions that can be used to create identities or fund Platform addresses").color(Color32::GRAY).size(12.0));
                            ui.add_space(20.0);
                        });
                    } else {
                        // Collect Platform addresses with balances from DB
                        let platform_addresses: Vec<(String, u64)> = self
                            .app_context
                            .db
                            .get_all_platform_address_info(&wallet.seed_hash(), &network)
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|(core_addr, balance, _nonce)| {
                                use dash_sdk::dpp::address_funds::PlatformAddress;
                                PlatformAddress::try_from(core_addr)
                                    .ok()
                                    .map(|pa| (pa.to_bech32m_string(network), balance))
                            })
                            .collect();

                        egui::ScrollArea::both()
                            .id_salt("asset_locks_table")
                            .min_scrolled_height(200.0)
                            .show(ui, |ui| {
                                TableBuilder::new(ui)
                        .striped(false)
                        .resizable(true)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                        .column(Column::initial(200.0)) // Transaction ID
                        .column(Column::initial(100.0)) // Address
                        .column(Column::initial(100.0)) // Amount (Duffs)
                        .column(Column::initial(100.0)) // InstantLock status
                        .column(Column::initial(100.0)) // Consumed
                        .column(Column::initial(200.0)) // Actions
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                ui.label("Transaction ID");
                            });
                            header.col(|ui| {
                                ui.label("Address");
                            });
                            header.col(|ui| {
                                ui.label("Amount (Duffs)");
                            });
                            header.col(|ui| {
                                ui.label("InstantLock");
                            });
                            header.col(|ui| {
                                ui.label("Status");
                            });
                            header.col(|ui| {
                                ui.label("Actions");
                            });
                        })
                        .body(|mut body| {
                            for (tx, amount, islock, _chain_locked_height, identity_id) in &locks {
                                let txid = tx.txid();
                                let txid_bytes = txid.to_byte_array();

                                let address_str = if let Some(TransactionPayload::AssetLockPayloadType(payload)) = &tx.special_transaction_payload {
                                    payload.credit_outputs.first()
                                        .and_then(|output| Address::from_script(&output.script_pubkey, network).ok())
                                        .map(|a| a.to_string())
                                        .unwrap_or_else(|| "Unknown".to_string())
                                } else {
                                    "Unknown".to_string()
                                };

                                let is_locked = islock.is_some();
                                let is_consumed = identity_id.is_some();

                                body.row(25.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(txid.to_string());
                                    });
                                    row.col(|ui| {
                                        ui.label(&address_str);
                                    });
                                    row.col(|ui| {
                                        ui.label(format!("{}", amount));
                                    });
                                    row.col(|ui| {
                                        let status = if is_locked { "Yes" } else { "No" };
                                        ui.label(status);
                                    });
                                    row.col(|ui| {
                                        if is_consumed {
                                            ui.label(RichText::new("Used").color(Color32::GRAY));
                                        } else {
                                            ui.label(RichText::new("Available").color(DashColors::SUCCESS));
                                        }
                                    });
                                    row.col(|ui| {
                                        if ui.small_button("View").clickable_tooltip("View full asset lock details").clicked() {
                                            app_action = AppAction::AddScreen(
                                                ScreenType::AssetLockDetail(
                                                    wallet.seed_hash(),
                                                    txid_bytes,
                                                ).create_screen(&self.app_context)
                                            );
                                        }
                                        if !is_consumed && is_locked
                                            && ui.small_button("Fund").clickable_tooltip("Fund a Platform address with this asset lock").clicked() {
                                                open_fund_dialog_for_txid = Some((txid_bytes, platform_addresses.clone()));
                                            }
                                    });
                                });
                            }
                        });
                    });
                    }
                });
        } else {
            ui.label("No wallet selected.");
        }

        // Handle dialog opening outside the borrow
        if let Some((txid, platform_addresses)) = open_fund_dialog_for_txid {
            self.fund_platform_dialog.selected_asset_lock_txid = Some(txid);
            self.fund_platform_dialog.is_open = true;
            self.fund_platform_dialog.platform_addresses = platform_addresses;
            self.fund_platform_dialog.selected_platform_address = None;
            self.fund_platform_dialog.status = None;
            self.fund_platform_dialog.is_processing = false;
        }

        // Handle recover asset locks button click - use custom action to check lock status
        if recover_asset_locks_clicked {
            app_action = AppAction::Custom("SearchAssetLocks".to_string());
        }

        app_action
    }
}
