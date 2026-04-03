use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::secret::Secret;
use crate::model::wallet::Wallet;
use crate::ui::components::MessageBanner;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::theme::{ComponentStyles, DashColors};
use crate::ui::{MessageType, RootScreenType, ScreenLike};
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
    wallet: Option<Arc<RwLock<Wallet>>>,
    password_input: PasswordInput,
    show_private_key_popup: bool,
    private_key_wif: Option<Secret>,
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
            wallet,
            password_input: PasswordInput::new().with_hint_text("Enter password"),
            show_private_key_popup: false,
            private_key_wif: None,
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
                            if ui.small_button("Copy").clicked() {
                                ui.ctx().copy_text(proof_hex.clone());
                                MessageBanner::set_global(ui.ctx(), "Asset lock proof copied to clipboard", MessageType::Success);
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

                    if (!needs_unlock || unlocked)
                        && let Some(wallet_arc) = self.wallet.clone() {
                            let wallet = wallet_arc.read().unwrap();

                            // Find the private key for this address
                            if let Some(derivation_path) = wallet.derivation_path_for_address(&address) {
                                drop(wallet); // Release the read lock before getting write lock

                                ui.horizontal(|ui| {
                                    ui.label("Private Key (WIF):");
                                    ui.label(RichText::new("••••••••••••••••••••").font(egui::FontId::monospace(12.0)).color(DashColors::text_secondary(dark_mode)));
                                    if ui.small_button("View").clicked() {
                                        // Retrieve the private key when View is clicked
                                        let wallet = wallet_arc.write().unwrap();
                                        match wallet.private_key_at_derivation_path(&derivation_path, self.app_context.network) {
                                            Ok(private_key) => {
                                                self.private_key_wif = Some(Secret::new(private_key.to_wif()));
                                                self.show_private_key_popup = true;
                                            }
                                            Err(e) => {
                                                MessageBanner::set_global(ui.ctx(), format!("Error retrieving private key: {}", e), MessageType::Error);
                                            }
                                        }
                                    }
                                });

                                ui.add_space(5.0);
                                ui.label(RichText::new("Warning: Keep this private key secure! Anyone with access to it can spend these funds.")
                                    .color(DashColors::warning_color(dark_mode))
                                    .italics());
                            } else {
                                ui.label(RichText::new("Private key not found for this address")
                                    .color(DashColors::error_color(dark_mode)));
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
}

impl ScreenWithWalletUnlock for AssetLockDetailScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.wallet
    }

    fn password_input(&mut self) -> &mut PasswordInput {
        &mut self.password_input
    }

    fn app_context(&self) -> Arc<AppContext> {
        self.app_context.clone()
    }
}

impl ScreenLike for AssetLockDetailScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
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
                ("Asset Lock Details", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenWalletsBalances,
        );

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;
            let dark_mode = ui.ctx().style().visuals.dark_mode;

            // Header with Back button (outside ScrollArea to avoid scrollbar overlap)
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

            // Message display is handled by the global MessageBanner

            inner_action
        });

        // Private key popup
        if self.show_private_key_popup {
            // Draw dark overlay behind the popup
            let screen_rect = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("private_key_popup_overlay"),
            ));
            painter.rect_filled(screen_rect, 0.0, DashColors::modal_overlay());

            egui::Window::new("Private Key")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.set_min_width(400.0);

                    ui.add_space(10.0);
                    ui.label(RichText::new("⚠ Warning").color(DashColors::WARNING_BRIGHT).strong());
                    ui.label("Keep this private key secure! Anyone with access to it can spend these funds.");
                    ui.add_space(15.0);

                    ui.label("Private Key (WIF):");
                    if let Some(ref wif) = self.private_key_wif {
                        ui.add(egui::TextEdit::multiline(&mut wif.expose_secret())
                            .font(egui::FontId::monospace(12.0))
                            .desired_width(f32::INFINITY)
                            .desired_rows(1));

                        ui.add_space(10.0);
                    }

                    let mut close_popup = false;
                    ui.horizontal(|ui| {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        if ComponentStyles::add_primary_button(ui, "Copy").clicked()
                            && let Some(ref wif) = self.private_key_wif
                        {
                            // SECURITY: clipboard copy inherently exposes plaintext — user-initiated action
                            ui.ctx().copy_text(wif.expose_secret().to_string());
                            MessageBanner::set_global(ctx, "Private key copied to clipboard", MessageType::Success);
                        }
                        if ComponentStyles::add_secondary_button(ui, "Close", dark_mode)
                            .clicked()
                        {
                            close_popup = true;
                        }
                    });
                    if close_popup {
                        self.show_private_key_popup = false;
                        self.private_key_wif = None;
                    }
                    ui.add_space(10.0);
                });
        }

        action
    }

    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Error/success display is handled by the global MessageBanner.
    }

    fn refresh_on_arrival(&mut self) {}

    fn refresh(&mut self) {}
}
