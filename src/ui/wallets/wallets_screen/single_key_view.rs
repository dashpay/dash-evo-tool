use crate::app::AppAction;
use crate::ui::MessageType;
use crate::ui::components::component_trait::Component;
use crate::ui::theme::DashColors;
use crate::wallet_backend::poison::RwLockRecover;
use eframe::egui;
use egui::{Frame, Margin, RichText, Ui};

use super::WalletsBalancesScreen;

/// Shown as a disabled-button tooltip and in the in-screen warning banner for
/// single-key-wallet send actions. Exported so the dedicated send screen and
/// the wallets action bar can reuse the same copy. Sending from a single-key
/// wallet is not available in this version; receiving still works.
pub(crate) const SINGLE_KEY_SEND_UNAVAILABLE: &str = "Sending from a single-key wallet is not available in this version. You can still receive funds at this address. To send these funds, import them into a recovery-phrase wallet.";

impl WalletsBalancesScreen {
    /// Render the detail view for a selected single key wallet
    pub(super) fn render_single_key_wallet_view(
        &mut self,
        ui: &mut Ui,
        dark_mode: bool,
    ) -> AppAction {
        let action = AppAction::None;

        let wallet_arc = match &self.selected_single_key_wallet {
            Some(w) => w.clone(),
            None => return action,
        };

        let wallet = wallet_arc.read_recover();
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

                    // Sending from a single-key wallet cannot work in this
                    // version, so the Send button below is permanently
                    // disabled. This banner carries the "why" plus the
                    // recovery-phrase workaround that a bare greyed-out
                    // button would leave unexplained.
                    //
                    // The banner lives on the screen struct so its state is
                    // constructed once and then re-rendered each frame. Setting
                    // the message via the struct field (instead of a fresh
                    // local) means `BannerState::logged` is preserved, so the
                    // underlying tracing log fires once — not 60 times a second
                    // while the screen is visible.
                    if !self.sk_spv_warning_banner.has_message() {
                        self.sk_spv_warning_banner
                            .set_message(SINGLE_KEY_SEND_UNAVAILABLE, MessageType::Warning)
                            .disable_auto_dismiss();
                    }
                    self.sk_spv_warning_banner.show(ui);
                    ui.add_space(10.0);

                    // Action buttons for SK wallet
                    ui.horizontal(|ui| {
                        // Left unstyled so egui's default disabled visuals apply
                        // and the button reads as genuinely greyed out.
                        let send_button = egui::Button::new(RichText::new("Send").strong());
                        ui.add_enabled(false, send_button)
                            .on_disabled_hover_text(SINGLE_KEY_SEND_UNAVAILABLE);

                        // Receive only displays the local address — it needs
                        // neither UTXO discovery nor signing, so it stays
                        // available.
                        if ui
                            .button(RichText::new("Receive").color(text_color))
                            .clicked()
                        {
                            self.receive_dialog.core_addresses =
                                vec![(address.clone(), balance_duffs)];
                            self.receive_dialog.selected_core_index = 0;
                            self.receive_dialog.open();
                        }
                    });
                    ui.add_space(15.0);

                    // UTXOs section
                    ui.separator();
                    ui.add_space(10.0);
                    ui.heading(RichText::new(format!("UTXOs ({})", utxo_count)).color(text_color));
                    ui.add_space(10.0);

                    if utxos.is_empty() {
                        ui.label("No funds at this address yet.");
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

#[cfg(test)]
mod tests {
    use super::SINGLE_KEY_SEND_UNAVAILABLE;
    use crate::backend_task::error::TaskError;

    /// Terms the Everyday User persona must never be shown (CLAUDE.md
    /// "Error messages" rule 1). "RPC" is listed even though this build is
    /// SPV-only: the concept is meaningless to the user either way.
    const JARGON: &[&str] = &[
        "SPV",
        "RPC",
        "UTXO",
        "backend",
        "consensus",
        "nonce",
        "SDK",
        "state transition",
    ];

    fn assert_everyday_user_copy(msg: &str) {
        let lower = msg.to_lowercase();
        for term in JARGON {
            assert!(
                !lower.contains(&term.to_lowercase()),
                "user-facing copy must not contain the jargon term {term:?}: {msg}"
            );
        }
        // Rule 2: users must be able to self-resolve — never redirected to a
        // human. Rule 3: calm, not apologetic/alarming.
        assert!(
            !lower.contains("contact support"),
            "user-facing copy must never redirect to support: {msg}"
        );
        assert!(
            !lower.contains("sorry") && !lower.contains("went wrong"),
            "user-facing copy must stay calm and non-apologetic: {msg}"
        );
    }

    /// The single-key send limitation is surfaced in-app, and the copy tells
    /// the user what happened AND the concrete step they can take themselves
    /// (move the funds into a recovery-phrase wallet). This is the whole
    /// user-visible contract of the disabled Send control: without the
    /// workaround the message would be a dead end.
    #[test]
    fn send_unavailable_copy_states_limitation_and_a_self_serve_action() {
        assert_everyday_user_copy(SINGLE_KEY_SEND_UNAVAILABLE);

        let lower = SINGLE_KEY_SEND_UNAVAILABLE.to_lowercase();
        assert!(
            lower.contains("not available"),
            "copy must state the limitation: {SINGLE_KEY_SEND_UNAVAILABLE}"
        );
        assert!(
            lower.contains("recovery-phrase"),
            "copy must name the recovery-phrase workaround so the user can act: \
             {SINGLE_KEY_SEND_UNAVAILABLE}"
        );
        assert!(
            lower.contains("receive"),
            "copy must say receiving still works, so the address is not read as dead: \
             {SINGLE_KEY_SEND_UNAVAILABLE}"
        );
    }

    /// The backend is the authoritative enforcement layer: a send that reaches
    /// it is refused with a typed variant whose `Display` is itself
    /// Everyday-User copy, since it is rendered straight into a `MessageBanner`.
    #[test]
    fn unsupported_task_error_display_is_everyday_user_copy() {
        let msg = TaskError::SingleKeyWalletsUnsupported.to_string();
        assert_everyday_user_copy(&msg);
        assert!(
            msg.to_lowercase().contains("recovery-phrase"),
            "the typed refusal must point at the same workaround as the UI copy: {msg}"
        );
    }
}
