use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use chrono::{DateTime, Utc};
use dash_sdk::dashcore_rpc::dashcore::{Address, InstantLock, Transaction};
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::prelude::AssetLockProof;
use eframe::egui::{self, Context, Ui};
use egui::{Color32, Frame, Margin, RichText};
use std::sync::{Arc, RwLock};

pub struct AssetLockDetailScreen {
    pub wallet_seed_hash: [u8; 32],
    pub asset_lock_index: usize,
    pub app_context: Arc<AppContext>,
    message: Option<(String, MessageType, DateTime<Utc>)>,
    wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
}

impl AssetLockDetailScreen {
    pub fn new(
        wallet_seed_hash: [u8; 32],
        asset_lock_index: usize,
        app_context: &Arc<AppContext>,
    ) -> Self {
        // Find the wallet by seed hash
        let wallet = app_context
            .wallets
            .read()
            .unwrap()
            .values()
            .find(|w| w.read().unwrap().seed_hash() == wallet_seed_hash)
            .cloned();

        Self {
            wallet_seed_hash,
            asset_lock_index,
            app_context: app_context.clone(),
            message: None,
            wallet,
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
        }
    }

    #[allow(clippy::type_complexity)]
    fn get_asset_lock_data(
        &self,
    ) -> Option<(
        Transaction,
        Address,
        Credits,
        Option<InstantLock>,
        Option<AssetLockProof>,
    )> {
        self.wallet.as_ref().and_then(|wallet| {
            let wallet = wallet.read().unwrap();
            wallet
                .unused_asset_locks
                .get(self.asset_lock_index)
                .cloned()
        })
    }

    fn render_asset_lock_info(&mut self, ui: &mut Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        if let Some((tx, address, amount, _islock, proof)) = self.get_asset_lock_data() {
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .corner_radius(5.0)
                .inner_margin(Margin::same(15))
                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .show(ui, |ui| {
                    ui.heading(RichText::new("Asset Lock Details").color(DashColors::text_primary(dark_mode)));
                    ui.add_space(10.0);

                    // Transaction Information
                    ui.label(RichText::new("Transaction Information").strong().color(DashColors::text_primary(dark_mode)));
                    ui.separator();
                    ui.add_space(5.0);

                    ui.horizontal(|ui| {
                        ui.label("Transaction ID:");
                        ui.label(RichText::new(tx.txid().to_string()).font(egui::FontId::monospace(12.0)));
                    });
                    ui.add_space(5.0);

                    ui.horizontal(|ui| {
                        ui.label("Address:");
                        ui.label(RichText::new(address.to_string()).font(egui::FontId::monospace(12.0)));
                    });
                    ui.add_space(5.0);

                    ui.horizontal(|ui| {
                        ui.label("Amount:");
                        let dash_amount = amount.to_string().parse::<u64>().unwrap_or(0) as f64 * 1e-8;
                        ui.label(RichText::new(format!("{:.8} DASH ({} duffs)", dash_amount, amount))
                            .strong()
                            .color(DashColors::text_primary(dark_mode)));
                    });
                    ui.add_space(5.0);

                    ui.horizontal(|ui| {
                        ui.label("Asset Lock Proof Type:");
                        let (proof_type, color) = match &proof {
                            Some(AssetLockProof::Instant(_)) => ("Instant Send Locked", DashColors::success_color(dark_mode)),
                            Some(AssetLockProof::Chain(_)) => ("Chain Locked", DashColors::success_color(dark_mode)),
                            None => ("Waiting for Lock", DashColors::warning_color(dark_mode)),
                        };
                        ui.label(RichText::new(proof_type).color(color));
                    });
                    ui.add_space(5.0);

                    // Asset Lock Proof Details
                    if let Some(proof) = &proof {
                        ui.add_space(15.0);
                        ui.label(RichText::new("Asset Lock Proof Details").strong().color(DashColors::text_primary(dark_mode)));
                        ui.separator();
                        ui.add_space(5.0);

                        // Show specific proof details based on type
                        match proof {
                            AssetLockProof::Instant(instant_proof) => {
                                ui.horizontal(|ui| {
                                    ui.label("Type:");
                                    ui.label(RichText::new("Instant Send").font(egui::FontId::monospace(12.0)));
                                });
                                ui.add_space(5.0);

                                // The instant lock is in the instant_proof
                                ui.horizontal(|ui| {
                                    ui.label("InstantLock TxID:");
                                    ui.label(RichText::new(instant_proof.instant_lock.txid.to_string()).font(egui::FontId::monospace(12.0)));
                                });
                                ui.add_space(5.0);

                                ui.horizontal(|ui| {
                                    ui.label("Output Index:");
                                    ui.label(RichText::new(instant_proof.output_index.to_string()).font(egui::FontId::monospace(12.0)));
                                });
                                ui.add_space(5.0);
                            }
                            AssetLockProof::Chain(chain_proof) => {
                                ui.horizontal(|ui| {
                                    ui.label("Type:");
                                    ui.label(RichText::new("Chain Lock").font(egui::FontId::monospace(12.0)));
                                });
                                ui.add_space(5.0);

                                ui.horizontal(|ui| {
                                    ui.label("Core Chain Locked Height:");
                                    ui.label(RichText::new(chain_proof.core_chain_locked_height.to_string()).font(egui::FontId::monospace(12.0)));
                                });
                                ui.add_space(5.0);

                                ui.horizontal(|ui| {
                                    ui.label("OutPoint:");
                                    ui.label(RichText::new(format!("{}:{}", chain_proof.out_point.txid, chain_proof.out_point.vout)).font(egui::FontId::monospace(12.0)));
                                });
                                ui.add_space(5.0);
                            }
                        }

                        // Asset Lock Proof Hex
                        ui.add_space(10.0);

                        // Serialize the proof to get hex
                        let proof_hex = match serde_json::to_vec(proof) {
                            Ok(bytes) => hex::encode(bytes),
                            Err(e) => format!("Error serializing proof: {}", e),
                        };

                        ui.horizontal(|ui| {
                            ui.label("Asset Lock Proof (hex):");
                            if ui.button("📋").on_hover_text("Copy to clipboard").clicked() {
                                ui.ctx().copy_text(proof_hex.clone());
                                self.display_message("Asset lock proof copied to clipboard", MessageType::Success);
                            }
                        });
                        ui.add_space(5.0);

                        // Display hex in a scrollable area with monospace font
                        egui::ScrollArea::horizontal()
                            .id_salt("proof_hex")
                            .show(ui, |ui| {
                                ui.label(RichText::new(&proof_hex).font(egui::FontId::monospace(10.0)).color(DashColors::text_secondary(dark_mode)));
                            });

                        ui.add_space(10.0);
                        ui.collapsing("View Raw Proof Details", |ui| {
                            ui.label(RichText::new(format!("{:#?}", proof)).font(egui::FontId::monospace(10.0)));
                        });
                    }

                    // Private Key Section (requires wallet unlock)
                    ui.add_space(20.0);
                    ui.label(RichText::new("Private Key Information").strong().color(DashColors::text_primary(dark_mode)));
                    ui.separator();
                    ui.add_space(5.0);

                    let (needs_unlock, unlocked) = self.render_wallet_unlock_if_needed(ui);

                    if !needs_unlock || unlocked {
                        if let Some(wallet_arc) = self.wallet.clone() {
                            let wallet = wallet_arc.read().unwrap();

                            // Find the private key for this address
                            if let Some(derivation_path) = wallet.known_addresses.get(&address).cloned() {
                                drop(wallet); // Release the read lock before getting write lock
                                let wallet = wallet_arc.write().unwrap();
                                match wallet.private_key_at_derivation_path(&derivation_path) {
                                    Ok(private_key) => {
                                        let wif = private_key.to_wif();
                                        drop(wallet); // Release lock before UI operations
                                        ui.horizontal(|ui| {
                                            ui.label("Private Key (WIF):");
                                            ui.label(RichText::new(&wif).font(egui::FontId::monospace(12.0)).color(DashColors::warning_color(dark_mode)));
                                            if ui.button("📋").on_hover_text("Copy to clipboard").clicked() {
                                                ui.ctx().copy_text(wif);
                                                self.display_message("Private key copied to clipboard", MessageType::Success);
                                            }
                                        });

                                        ui.add_space(5.0);
                                        ui.label(RichText::new("⚠️ Keep this private key secure! Anyone with access to it can spend these funds.")
                                            .color(DashColors::warning_color(dark_mode))
                                            .italics());
                                    }
                                    Err(e) => {
                                        ui.label(RichText::new(format!("Error retrieving private key: {}", e))
                                            .color(DashColors::error_color(dark_mode)));
                                    }
                                }
                            } else {
                                ui.label(RichText::new("Private key not found for this address")
                                    .color(DashColors::error_color(dark_mode)));
                            }
                        }
                    }
                });
        } else {
            ui.vertical_centered(|ui| {
                ui.add_space(50.0);
                ui.label(
                    RichText::new("Asset lock not found")
                        .size(16.0)
                        .color(Color32::GRAY),
                );
            });
        }
    }

    fn check_message_expiration(&mut self) {
        if let Some((_, _, timestamp)) = &self.message {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(*timestamp);

            if elapsed.num_seconds() >= 10 {
                self.message = None;
            }
        }
    }
}

impl ScreenWithWalletUnlock for AssetLockDetailScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.wallet
    }

    fn wallet_password_ref(&self) -> &String {
        &self.wallet_password
    }

    fn wallet_password_mut(&mut self) -> &mut String {
        &mut self.wallet_password
    }

    fn show_password(&self) -> bool {
        self.show_password
    }

    fn show_password_mut(&mut self) -> &mut bool {
        &mut self.show_password
    }

    fn set_error_message(&mut self, error_message: Option<String>) {
        self.error_message = error_message;
    }

    fn error_message(&self) -> Option<&String> {
        self.error_message.as_ref()
    }
}

impl ScreenLike for AssetLockDetailScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_message_expiration();

        let wallet_name = self
            .wallet
            .as_ref()
            .and_then(|w| w.read().ok()?.alias.clone())
            .unwrap_or_else(|| "Unknown Wallet".to_string());

        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                (
                    "Wallets",
                    AppAction::SetMainScreenThenGoToMainScreen(
                        RootScreenType::RootScreenWalletsBalances,
                    ),
                ),
                (
                    &format!("{} / Asset Lock Details", wallet_name),
                    AppAction::None,
                ),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        action |= island_central_panel(ctx, |ui| {
            let inner_action = AppAction::None;
            let dark_mode = ui.ctx().style().visuals.dark_mode;

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    ui.heading(
                        RichText::new("Asset Lock Information")
                            .color(DashColors::text_primary(dark_mode))
                            .size(24.0),
                    );
                    ui.add_space(10.0);

                    self.render_asset_lock_info(ui);
                });

            // Display messages
            if let Some((message, message_type, timestamp)) = &self.message {
                let message_color = match message_type {
                    MessageType::Error => egui::Color32::DARK_RED,
                    MessageType::Info => DashColors::text_primary(dark_mode),
                    MessageType::Success => egui::Color32::DARK_GREEN,
                };

                ui.add_space(25.0);
                ui.horizontal(|ui| {
                    ui.add_space(10.0);

                    let now = Utc::now();
                    let elapsed = now.signed_duration_since(*timestamp);
                    let remaining = (10 - elapsed.num_seconds()).max(0);

                    let full_msg = format!("{} ({}s)", message, remaining);
                    ui.label(egui::RichText::new(full_msg).color(message_color));
                });
                ui.add_space(2.0);
            }

            inner_action
        });

        action
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.message = Some((message.to_string(), message_type, Utc::now()));
    }

    fn refresh_on_arrival(&mut self) {}

    fn refresh(&mut self) {}
}
