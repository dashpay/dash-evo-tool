use crate::app::AppAction;
use crate::spv::CoreBackendMode;
use crate::ui::ScreenType;
use crate::ui::theme::DashColors;
use crate::ui::wallets::{
    SINGLE_KEY_REQUIRES_CORE_MESSAGE, render_single_key_requires_core_banner,
};
use eframe::egui;
use egui::{Frame, Margin, RichText, Ui};

use super::WalletsBalancesScreen;

impl WalletsBalancesScreen {
    /// Render the detail view for a selected single key wallet
    pub(super) fn render_single_key_wallet_view(
        &mut self,
        ui: &mut Ui,
        dark_mode: bool,
    ) -> AppAction {
        let mut action = AppAction::None;

        let wallet_arc = match &self.selected_single_key_wallet {
            Some(w) => w.clone(),
            None => return action,
        };

        let wallet = wallet_arc.read().unwrap();
        let address = wallet.address.to_string();
        let alias = wallet
            .alias
            .clone()
            .unwrap_or_else(|| "Unnamed Key".to_string());
        let balance_duffs = wallet.total_balance_duffs();
        let balance_dash = balance_duffs as f64 * 1e-8;
        let utxo_count = wallet.utxos.len();
        let utxos: Vec<_> = wallet.utxos.iter().map(|(o, t)| (*o, t.clone())).collect();
        drop(wallet);

        let text_color = DashColors::text_primary(dark_mode);
        let is_rpc_mode = self.app_context.core_backend_mode() == CoreBackendMode::Rpc;

        Frame::group(ui.style())
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(16, 16))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.heading(RichText::new(&alias).strong().color(text_color));
                    ui.add_space(10.0);

                    // Balance info
                    ui.label(RichText::new(format!("Balance: {:.8} DASH", balance_dash)));
                    ui.add_space(10.0);

                    // When the app is on its built-in SPV backend, surface a
                    // warning banner explaining that single-key wallet actions
                    // are unavailable. The actions themselves are greyed out
                    // below; this banner is the "why" users otherwise wouldn't
                    // see from a silent disable.
                    if !is_rpc_mode {
                        render_single_key_requires_core_banner(ui);
                        ui.add_space(10.0);
                    }

                    // Action buttons for SK wallet
                    ui.horizontal(|ui| {
                        // Only force the primary text color when the button is
                        // enabled; otherwise let egui apply its default disabled
                        // visuals so the button actually looks greyed out.
                        let send_label = RichText::new("Send").strong();
                        let send_label = if is_rpc_mode {
                            send_label.color(text_color)
                        } else {
                            send_label
                        };
                        let send_button = egui::Button::new(send_label);
                        let send_response = ui.add_enabled(is_rpc_mode, send_button);
                        let send_response = if is_rpc_mode {
                            send_response
                        } else {
                            send_response.on_disabled_hover_text(SINGLE_KEY_REQUIRES_CORE_MESSAGE)
                        };
                        if send_response.clicked() {
                            action = AppAction::AddScreen(
                                ScreenType::SingleKeyWalletSendScreen(wallet_arc.clone())
                                    .create_screen(&self.app_context),
                            );
                        }

                        // Receive only displays the local address — it does
                        // not touch Core or SPV, so it stays enabled in both
                        // modes.
                        if ui
                            .button(RichText::new("Receive").color(text_color))
                            .clicked()
                        {
                            self.receive_dialog.core_addresses =
                                vec![(address.clone(), balance_duffs)];
                            self.receive_dialog.selected_core_index = 0;
                            self.receive_dialog.is_open = true;
                        }
                    });
                    ui.add_space(15.0);

                    // UTXOs section
                    ui.separator();
                    ui.add_space(10.0);
                    ui.heading(RichText::new(format!("UTXOs ({})", utxo_count)).color(text_color));
                    ui.add_space(10.0);

                    if utxos.is_empty() {
                        ui.label("No UTXOs available. Click 'Refresh' to load UTXOs from Core.");
                    } else {
                        const UTXOS_PER_PAGE: usize = 50;
                        let total_pages = utxo_count.div_ceil(UTXOS_PER_PAGE);

                        // Ensure current page is valid
                        if self.utxo_page >= total_pages {
                            self.utxo_page = total_pages.saturating_sub(1);
                        }

                        let start_idx = self.utxo_page * UTXOS_PER_PAGE;
                        let utxos_page: Vec<_> =
                            utxos.iter().skip(start_idx).take(UTXOS_PER_PAGE).collect();

                        // Pagination controls
                        if total_pages > 1 {
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(self.utxo_page > 0, egui::Button::new("<< First"))
                                    .clicked()
                                {
                                    self.utxo_page = 0;
                                }
                                if ui
                                    .add_enabled(self.utxo_page > 0, egui::Button::new("< Prev"))
                                    .clicked()
                                {
                                    self.utxo_page = self.utxo_page.saturating_sub(1);
                                }

                                ui.label(format!(
                                    "Page {} of {} ({}-{} of {})",
                                    self.utxo_page + 1,
                                    total_pages,
                                    start_idx + 1,
                                    (start_idx + utxos_page.len()).min(utxo_count),
                                    utxo_count
                                ));

                                if ui
                                    .add_enabled(
                                        self.utxo_page < total_pages - 1,
                                        egui::Button::new("Next >"),
                                    )
                                    .clicked()
                                {
                                    self.utxo_page += 1;
                                }
                                if ui
                                    .add_enabled(
                                        self.utxo_page < total_pages - 1,
                                        egui::Button::new("Last >>"),
                                    )
                                    .clicked()
                                {
                                    self.utxo_page = total_pages - 1;
                                }
                            });
                            ui.add_space(10.0);
                        }

                        egui::ScrollArea::vertical()
                            .max_height(300.0)
                            .show(ui, |ui| {
                                for (outpoint, tx_out) in utxos_page {
                                    Frame::group(ui.style())
                                        .fill(DashColors::surface(dark_mode).gamma_multiply(0.9))
                                        .inner_margin(Margin::symmetric(10, 8))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.vertical(|ui| {
                                                    ui.horizontal(|ui| {
                                                        ui.label("TxID:");
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "{}:{}",
                                                                outpoint.txid, outpoint.vout
                                                            ))
                                                            .monospace()
                                                            .size(11.0)
                                                            .color(text_color),
                                                        );
                                                    });
                                                    ui.horizontal(|ui| {
                                                        ui.label("Amount:");
                                                        ui.label(
                                                            RichText::new(format!(
                                                                "{:.8} DASH",
                                                                tx_out.value as f64 * 1e-8
                                                            ))
                                                            .strong()
                                                            .color(text_color),
                                                        );
                                                    });
                                                });
                                            });
                                        });
                                    ui.add_space(5.0);
                                }
                            });
                    }
                });
            });

        action
    }
}
