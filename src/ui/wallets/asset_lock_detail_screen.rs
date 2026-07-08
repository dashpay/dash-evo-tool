use crate::app::AppAction;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::fee_estimation::format_duffs_as_dash;
use crate::ui::components::MessageBanner;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::state::TrackedAssetLockCache;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dpp::dashcore::OutPoint;
use dash_sdk::dpp::prelude::AssetLockProof;
use eframe::egui::{self, Ui};
use egui::{Color32, Frame, Margin, RichText};
use platform_wallet::wallet::asset_lock::tracked::TrackedAssetLock;
use std::sync::Arc;

pub struct AssetLockDetailScreen {
    pub wallet_seed_hash: [u8; 32],
    pub out_point: OutPoint,
    pub app_context: Arc<AppContext>,
    asset_lock_cache: TrackedAssetLockCache,
}

impl AssetLockDetailScreen {
    pub fn new(
        wallet_seed_hash: [u8; 32],
        out_point: OutPoint,
        app_context: &Arc<AppContext>,
    ) -> Self {
        Self {
            wallet_seed_hash,
            out_point,
            app_context: app_context.clone(),
            asset_lock_cache: TrackedAssetLockCache::default(),
        }
    }

    fn load_tracked_lock(&self) -> Option<TrackedAssetLock> {
        self.asset_lock_cache
            .get(&self.wallet_seed_hash)?
            .iter()
            .find(|t| t.out_point == self.out_point)
            .cloned()
    }

    fn render_asset_lock_info(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;

        let Some(lock) = self.load_tracked_lock() else {
            if self.asset_lock_cache.is_failed(&self.wallet_seed_hash) {
                let mut retry = false;
                ui.vertical_centered(|ui| {
                    ui.add_space(50.0);
                    ui.label(
                        RichText::new("Couldn't load asset lock.")
                            .size(16.0)
                            .color(Color32::GRAY),
                    );
                    ui.add_space(8.0);
                    retry = ui.button("Retry").clicked();
                });
                if retry {
                    self.asset_lock_cache.invalidate_one(&self.wallet_seed_hash);
                }
                return;
            }

            let message = if self.asset_lock_cache.is_loading(&self.wallet_seed_hash) {
                "Loading asset lock…"
            } else {
                "Asset lock not found"
            };
            ui.vertical_centered(|ui| {
                ui.add_space(50.0);
                ui.label(RichText::new(message).size(16.0).color(Color32::GRAY));
            });
            return;
        };

        Frame::new()
            .fill(DashColors::surface(dark_mode))
            .corner_radius(5.0)
            .inner_margin(Margin::same(15))
            .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
            .show(ui, |ui| {
                ui.heading(
                    RichText::new("Asset Lock Details").color(DashColors::text_primary(dark_mode)),
                );
                ui.add_space(10.0);

                ui.label(
                    RichText::new("Transaction Information")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.separator();
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Transaction ID:");
                    ui.label(
                        RichText::new(lock.out_point.txid.to_string())
                            .font(egui::FontId::monospace(12.0)),
                    );
                });
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Output Index:");
                    ui.label(
                        RichText::new(lock.out_point.vout.to_string())
                            .font(egui::FontId::monospace(12.0)),
                    );
                });
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Amount:");
                    ui.label(
                        RichText::new(format!(
                            "{} ({} duffs)",
                            format_duffs_as_dash(lock.amount),
                            lock.amount
                        ))
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                    );
                });
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Status:");
                    ui.label(format!("{:?}", lock.status));
                });
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Funding Account Type:");
                    ui.label(format!("{:?}", lock.funding_type));
                });
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Identity Index:");
                    ui.label(lock.identity_index.to_string());
                });
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Asset Lock Proof Type:");
                    let (proof_type, color) = match &lock.proof {
                        Some(AssetLockProof::Instant(_)) => {
                            ("Instant Send Locked", DashColors::success_color(dark_mode))
                        }
                        Some(AssetLockProof::Chain(_)) => {
                            ("Chain Locked", DashColors::success_color(dark_mode))
                        }
                        None => ("Waiting for Lock", DashColors::warning_color(dark_mode)),
                    };
                    ui.label(RichText::new(proof_type).color(color));
                });
                ui.add_space(5.0);

                if let Some(proof) = &lock.proof {
                    ui.add_space(15.0);
                    ui.label(
                        RichText::new("Asset Lock Proof Details")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.separator();
                    ui.add_space(5.0);

                    match proof {
                        AssetLockProof::Instant(instant_proof) => {
                            ui.horizontal(|ui| {
                                ui.label("Type:");
                                ui.label(
                                    RichText::new("Instant Send")
                                        .font(egui::FontId::monospace(12.0)),
                                );
                            });
                            ui.add_space(5.0);

                            ui.horizontal(|ui| {
                                ui.label("InstantLock TxID:");
                                ui.label(
                                    RichText::new(instant_proof.instant_lock.txid.to_string())
                                        .font(egui::FontId::monospace(12.0)),
                                );
                            });
                            ui.add_space(5.0);

                            ui.horizontal(|ui| {
                                ui.label("Output Index:");
                                ui.label(
                                    RichText::new(instant_proof.output_index.to_string())
                                        .font(egui::FontId::monospace(12.0)),
                                );
                            });
                            ui.add_space(5.0);
                        }
                        AssetLockProof::Chain(chain_proof) => {
                            ui.horizontal(|ui| {
                                ui.label("Type:");
                                ui.label(
                                    RichText::new("Chain Lock").font(egui::FontId::monospace(12.0)),
                                );
                            });
                            ui.add_space(5.0);

                            ui.horizontal(|ui| {
                                ui.label("Core Chain Locked Height:");
                                ui.label(
                                    RichText::new(chain_proof.core_chain_locked_height.to_string())
                                        .font(egui::FontId::monospace(12.0)),
                                );
                            });
                            ui.add_space(5.0);

                            ui.horizontal(|ui| {
                                ui.label("OutPoint:");
                                ui.label(
                                    RichText::new(format!(
                                        "{}:{}",
                                        chain_proof.out_point.txid, chain_proof.out_point.vout
                                    ))
                                    .font(egui::FontId::monospace(12.0)),
                                );
                            });
                            ui.add_space(5.0);
                        }
                    }

                    ui.add_space(10.0);
                    let proof_hex = match serde_json::to_vec(proof) {
                        Ok(bytes) => hex::encode(bytes),
                        Err(e) => format!("Error serializing proof: {}", e),
                    };

                    ui.horizontal(|ui| {
                        ui.label("Asset Lock Proof (hex):");
                        if ui.small_button("Copy").clicked() {
                            ui.ctx().copy_text(proof_hex.clone());
                            MessageBanner::set_global(
                                ui.ctx(),
                                "Asset lock proof copied to clipboard",
                                MessageType::Success,
                            );
                        }
                    });
                    ui.add_space(5.0);

                    egui::ScrollArea::horizontal()
                        .id_salt("proof_hex")
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(&proof_hex)
                                    .font(egui::FontId::monospace(10.0))
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                        });

                    ui.add_space(10.0);
                    ui.collapsing("View Raw Proof Details", |ui| {
                        ui.label(
                            RichText::new(format!("{:#?}", proof))
                                .font(egui::FontId::monospace(10.0)),
                        );
                    });
                }
            });
    }
}

impl ScreenLike for AssetLockDetailScreen {
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                (
                    "Wallets",
                    AppAction::SetMainScreenThenGoToMainScreen(
                        RootScreenType::RootScreenWalletsBalances,
                    ),
                ),
                ("Asset Lock Details", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        // Fetch the wallet's tracked locks once (off the UI thread); the lock
        // for this screen is found by out_point in the cached list.
        if let Some(task) = self
            .asset_lock_cache
            .ensure_requested(self.wallet_seed_hash)
        {
            action |= AppAction::BackendTask(task);
        }

        action |= island_central_panel(ui, |ui| {
            let mut inner_action = AppAction::None;
            let dark_mode = ui.style().visuals.dark_mode;

            ui.horizontal(|ui| {
                ui.heading(
                    RichText::new("Asset Lock Information")
                        .color(DashColors::text_primary(dark_mode))
                        .size(24.0),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Back").clicked() {
                        inner_action = AppAction::PopScreenAndRefresh;
                    }
                });
            });
            ui.add_space(10.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    self.render_asset_lock_info(ui);
                });

            inner_action
        });

        action
    }

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {}

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::TrackedAssetLocks { seed_hash, locks } = result {
            self.asset_lock_cache.store(seed_hash, locks);
        }
    }

    fn display_task_error(&mut self, _error: &TaskError) -> bool {
        // Flip the in-flight lock fetch to a retryable state so the screen shows
        // a Retry button instead of a permanent "Loading…".
        self.asset_lock_cache.mark_loading_failed();
        false
    }

    fn refresh_on_arrival(&mut self) {}

    fn refresh(&mut self) {}
}
