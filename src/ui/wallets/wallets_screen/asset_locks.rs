use crate::app::AppAction;
use crate::model::wallet::DerivationPathHelpers;
use crate::ui::ScreenType;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::wallet_backend::poison::RwLockRecover;
use eframe::egui::{self, Ui};
use egui::{Color32, Frame, Margin, RichText};
use egui_extras::{Column, TableBuilder};
use platform_wallet::wallet::asset_lock::tracked::TrackedAssetLock;

use super::WalletsBalancesScreen;

impl WalletsBalancesScreen {
    pub(super) fn render_wallet_asset_locks(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let mut open_fund_dialog_for_op: Option<(
            dash_sdk::dpp::dashcore::OutPoint,
            Vec<(String, u64)>,
        )> = None;

        let Some(arc_wallet) = &self.selected_wallet else {
            ui.label("No wallet selected.");
            return app_action;
        };

        let (seed_hash, platform_addresses) = {
            let wallet = arc_wallet.read_recover();
            let network = self.app_context.network;
            let platform_addresses: Vec<(String, u64)> = wallet
                .known_addresses
                .iter()
                .filter(|(_, path)| path.is_platform_payment(network))
                .filter_map(|(addr, _)| {
                    use dash_sdk::dpp::address_funds::PlatformAddress;
                    let balance = wallet
                        .get_platform_address_info(addr)
                        .map(|info| info.balance)
                        .unwrap_or(0);
                    PlatformAddress::try_from(addr.clone())
                        .ok()
                        .map(|pa| (pa.to_bech32m_string(network), balance))
                })
                .collect();
            (wallet.seed_hash(), platform_addresses)
        };

        // Fetch the locks off the UI thread via the App Task System; render
        // from the cache. `None` ⇒ the fetch is loading or failed.
        if let Some(task) = self.asset_lock_cache.ensure_requested(seed_hash) {
            app_action = AppAction::BackendTask(task);
        }
        let tracked: Option<Vec<TrackedAssetLock>> =
            self.asset_lock_cache.get(&seed_hash).map(<[_]>::to_vec);
        let load_failed = self.asset_lock_cache.is_failed(&seed_hash);
        let mut retry_clicked = false;

        let dark_mode = ui.style().visuals.dark_mode;
        Frame::new()
            .fill(DashColors::surface(dark_mode))
            .corner_radius(5.0)
            .inner_margin(Margin::same(15))
            .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
            .show(ui, |ui| {
                let dark_mode = ui.style().visuals.dark_mode;
                ui.heading(
                    RichText::new("Asset Locks").color(DashColors::text_primary(dark_mode)),
                );
                ui.add_space(10.0);

                let Some(tracked) = tracked.as_deref() else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(20.0);
                        if load_failed {
                            ui.label(
                                RichText::new("Couldn't load asset locks.")
                                    .color(Color32::GRAY)
                                    .size(14.0),
                            );
                            ui.add_space(8.0);
                            if ui.button("Retry").clicked() {
                                retry_clicked = true;
                            }
                        } else {
                            ui.label(
                                RichText::new("Loading asset locks…")
                                    .color(Color32::GRAY)
                                    .size(14.0),
                            );
                        }
                        ui.add_space(20.0);
                    });

                    ui.add_space(10.0);
                    if ComponentStyles::add_primary_button(ui, "Create Asset Lock").clicked() {
                        app_action = AppAction::AddScreen(
                            ScreenType::CreateAssetLock(arc_wallet.clone())
                                .create_screen(&self.app_context),
                        );
                    }
                    return;
                };

                if tracked.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(20.0);
                        ui.label(
                            RichText::new("No asset locks found")
                                .color(Color32::GRAY)
                                .size(14.0),
                        );
                        ui.add_space(10.0);
                        ui.label(RichText::new("Asset locks are special transactions that can be used to create identities or fund Platform addresses").color(Color32::GRAY).size(12.0));
                        ui.add_space(20.0);
                    });
                } else {
                    egui::ScrollArea::both()
                        .id_salt("asset_locks_table")
                        .min_scrolled_height(200.0)
                        .show(ui, |ui| {
                            TableBuilder::new(ui)
                                .striped(false)
                                .resizable(true)
                                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                                .column(Column::initial(200.0)) // Transaction ID
                                .column(Column::initial(60.0))  // Vout
                                .column(Column::initial(120.0)) // Amount (Duffs)
                                .column(Column::initial(140.0)) // Status
                                .column(Column::initial(100.0)) // Usable
                                .column(Column::initial(200.0)) // Actions
                                .header(30.0, |mut header| {
                                    header.col(|ui| { ui.label("Transaction ID"); });
                                    header.col(|ui| { ui.label("Vout"); });
                                    header.col(|ui| { ui.label("Amount (Duffs)"); });
                                    header.col(|ui| { ui.label("Status"); });
                                    header.col(|ui| { ui.label("Usable"); });
                                    header.col(|ui| { ui.label("Actions"); });
                                })
                                .body(|mut body| {
                                    for lock in tracked {
                                        body.row(25.0, |mut row| {
                                            row.col(|ui| {
                                                ui.label(lock.out_point.txid.to_string());
                                            });
                                            row.col(|ui| {
                                                ui.label(lock.out_point.vout.to_string());
                                            });
                                            row.col(|ui| {
                                                ui.label(format!("{}", lock.amount));
                                            });
                                            row.col(|ui| {
                                                ui.label(format!("{:?}", lock.status));
                                            });
                                            row.col(|ui| {
                                                let usable = if lock.proof.is_some() { "Yes" } else { "No" };
                                                ui.label(usable);
                                            });
                                            row.col(|ui| {
                                                if ui.small_button("View")
                                                    .clickable_tooltip("View full asset lock details")
                                                    .clicked()
                                                {
                                                    app_action = AppAction::AddScreen(
                                                        ScreenType::AssetLockDetail(
                                                            seed_hash,
                                                            lock.out_point,
                                                        )
                                                        .create_screen(&self.app_context),
                                                    );
                                                }
                                                if lock.proof.is_some()
                                                    && ui.small_button("Fund")
                                                        .clickable_tooltip("Fund a Platform address with this asset lock")
                                                        .clicked()
                                                {
                                                    open_fund_dialog_for_op = Some((
                                                        lock.out_point,
                                                        platform_addresses.clone(),
                                                    ));
                                                }
                                            });
                                        });
                                    }
                                });
                        });
                }

                ui.add_space(10.0);
                if ComponentStyles::add_primary_button(ui, "Create Asset Lock").clicked() {
                    app_action = AppAction::AddScreen(
                        ScreenType::CreateAssetLock(arc_wallet.clone())
                            .create_screen(&self.app_context),
                    );
                }
            });

        if retry_clicked {
            self.asset_lock_cache.invalidate_one(&seed_hash);
        }

        if let Some((out_point, platform_addresses)) = open_fund_dialog_for_op {
            self.fund_platform_dialog.selected_asset_lock_out_point = Some(out_point);
            self.fund_platform_dialog.is_open = true;
            self.fund_platform_dialog.platform_addresses = platform_addresses;
            self.fund_platform_dialog.selected_platform_address = None;
            self.fund_platform_dialog.status = None;
            self.fund_platform_dialog.is_processing = false;
        }

        app_action
    }
}
