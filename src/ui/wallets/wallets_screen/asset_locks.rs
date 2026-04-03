use crate::app::AppAction;
use crate::model::wallet::DerivationPathHelpers;
use crate::ui::ScreenType;
use crate::ui::theme::{DashColors, ResponseExt};
use eframe::egui::{self, Ui};
use egui::{Color32, Frame, Margin, RichText};
use egui_extras::{Column, TableBuilder};

use super::WalletsBalancesScreen;

impl WalletsBalancesScreen {
    pub(super) fn render_wallet_asset_locks(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let mut open_fund_dialog_for_idx: Option<(usize, Vec<(String, u64)>)> = None;
        let mut recover_asset_locks_clicked = false;

        if let Some(arc_wallet) = &self.selected_wallet {
            let wallet = arc_wallet.read().unwrap();

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

                    if wallet.unused_asset_locks.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(20.0);
                            ui.label(RichText::new("No asset locks found").color(Color32::GRAY).size(14.0));
                            ui.add_space(10.0);
                            ui.label(RichText::new("Asset locks are special transactions that can be used to create identities or fund Platform addresses").color(Color32::GRAY).size(12.0));
                            ui.add_space(20.0);
                        });
                    } else {
                        // Collect Platform addresses from PlatformWallet
                        let network = self.app_context.network;
                        let platform_addresses: Vec<(String, u64)> = wallet
                            .all_addresses_info()
                            .into_iter()
                            .filter(|a| a.derivation_path.is_platform_payment(network))
                            .filter_map(|a| {
                                use dash_sdk::dpp::address_funds::PlatformAddress;
                                let balance = wallet
                                    .get_platform_address_info(&a.address)
                                    .map(|info| info.balance)
                                    .unwrap_or(0);
                                PlatformAddress::try_from(a.address)
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
                        .column(Column::initial(100.0)) // Usable status
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
                                ui.label("Usable");
                            });
                            header.col(|ui| {
                                ui.label("Actions");
                            });
                        })
                        .body(|mut body| {
                            for (index, (tx, address, amount, islock, proof)) in wallet.unused_asset_locks.iter().enumerate() {
                                body.row(25.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(tx.txid().to_string());
                                    });
                                    row.col(|ui| {
                                        ui.label(address.to_string());
                                    });
                                    row.col(|ui| {
                                        ui.label(format!("{}", amount));
                                    });
                                    row.col(|ui| {
                                        let status = if islock.is_some() { "Yes" } else { "No" };
                                        ui.label(status);
                                    });
                                    row.col(|ui| {
                                        let status = if proof.is_some() { "Yes" } else { "No" };
                                        ui.label(status);
                                    });
                                    row.col(|ui| {
                                        if ui.small_button("View").clickable_tooltip("View full asset lock details").clicked() {
                                            app_action = AppAction::AddScreen(
                                                ScreenType::AssetLockDetail(
                                                    wallet.seed_hash(),
                                                    index
                                                ).create_screen(&self.app_context)
                                            );
                                        }
                                        if proof.is_some()
                                            && ui.small_button("Fund").clickable_tooltip("Fund a Platform address with this asset lock").clicked() {
                                                open_fund_dialog_for_idx = Some((index, platform_addresses.clone()));
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
        if let Some((idx, platform_addresses)) = open_fund_dialog_for_idx {
            self.fund_platform_dialog.selected_asset_lock_index = Some(idx);
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
