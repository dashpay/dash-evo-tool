//! Shared modal chrome for passphrase entry.
//!
//! Both the wallet-unlock popup ([`WalletUnlockPopup`](super::wallet_unlock_popup))
//! and the just-in-time secret prompt
//! ([`EguiSecretPromptHost`](super::secret_prompt_host)) ask the user for a
//! passphrase through the same centered, overlay-dimmed modal. This module
//! owns that chrome once: the dark overlay, the bordered `Window`, focus-once,
//! the [`PasswordInput`] field, an inline error line, an optional `extra` body
//! (e.g. a "remember" checkbox), and the Cancel / Submit button row.
//!
//! It resolves Cancel / Escape / X / click-outside uniformly to
//! [`PassphraseModalOutcome::Cancel`] so callers never re-implement dismissal.
//! It holds no secret state of its own — the [`PasswordInput`] the caller
//! passes in owns (and zeroizes) the typed bytes.

use egui::Context;

use crate::ui::components::password_input::PasswordInput;
use crate::ui::helpers::clicked_outside_window;
use crate::ui::theme::{ComponentStyles, DashColors};

/// What the user did with a [`passphrase_modal`] this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassphraseModalOutcome {
    /// The modal is still open; no decision yet.
    Pending,
    /// The user submitted (Enter or the submit button). The caller reads the
    /// passphrase from its `PasswordInput`.
    Submit,
    /// The user dismissed (Cancel button, Escape, X, or click-outside).
    Cancel,
}

/// Static copy + layout knobs for one render of [`passphrase_modal`].
///
/// Borrowed for the call only; carries no secret. `title` and `submit_label`
/// are complete, translatable sentences/labels (i18n-ready) supplied by the
/// caller so the same chrome serves "Unlock Wallet" and the JIT prompt.
pub struct PassphraseModalConfig<'a> {
    /// `Window` title (top bar). Stable across re-asks.
    pub window_title: &'a str,
    /// Body prompt line above the field, e.g. the wallet/key label.
    pub body: &'a str,
    /// Optional user-set hint shown under the field.
    pub hint: Option<&'a str>,
    /// Optional inline error (e.g. wrong-passphrase), shown in error color.
    pub error: Option<&'a str>,
    /// Submit button label, e.g. "Unlock".
    pub submit_label: &'a str,
}

/// Render the shared passphrase modal and return what the user did.
///
/// `focus_requested` tracks whether the field was focused once already; the
/// modal sets it `true` after requesting focus so the cursor lands in the
/// field on open without stealing focus every frame. `extra` draws any caller-
/// specific body (the remember checkbox) between the error line and the button
/// row.
///
/// The caller owns dismissal side effects (clearing the `PasswordInput`,
/// sending a reply): this function only reports the outcome.
pub fn passphrase_modal(
    ctx: &Context,
    config: &PassphraseModalConfig<'_>,
    password_input: &mut PasswordInput,
    focus_requested: &mut bool,
    extra: impl FnOnce(&mut egui::Ui),
) -> PassphraseModalOutcome {
    // Dark overlay behind the modal. The layer id is salted with the window
    // title so a wallet-unlock modal and a JIT secret prompt drawn in the same
    // frame get distinct overlay layers instead of fighting over one.
    let screen_rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("passphrase_modal_overlay").with(config.window_title),
    ));
    painter.rect_filled(screen_rect, 0.0, DashColors::modal_overlay());

    let mut outcome = PassphraseModalOutcome::Pending;
    let mut window_is_open = true;

    let window_response = egui::Window::new(config.window_title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut window_is_open)
        .frame(egui::Frame {
            inner_margin: egui::Margin::same(20),
            outer_margin: egui::Margin::same(0),
            corner_radius: egui::CornerRadius::same(8),
            shadow: egui::epaint::Shadow {
                offset: [0, 8],
                blur: 16,
                spread: 0,
                color: DashColors::popup_shadow(),
            },
            fill: ctx.style().visuals.window_fill,
            stroke: egui::Stroke::new(1.0, DashColors::popup_border_glow()),
        })
        .show(ctx, |ui| {
            ui.set_min_width(350.0);
            ui.set_max_width(400.0);

            let dark_mode = ui.ctx().style().visuals.dark_mode;

            ui.label(egui::RichText::new(config.body).color(DashColors::text_primary(dark_mode)));

            ui.add_space(12.0);

            let mut submit = false;

            let pw_response = password_input.show(ui);

            if !*focus_requested {
                pw_response.response.request_focus();
                *focus_requested = true;
            }

            if pw_response.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                submit = true;
            }

            if let Some(hint) = config.hint {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(format!("Hint: {hint}"))
                        .color(DashColors::text_secondary(dark_mode)),
                );
            }

            if let Some(error) = config.error {
                ui.add_space(8.0);
                ui.colored_label(DashColors::ERROR, error);
            }

            ui.add_space(12.0);

            extra(ui);

            ui.add_space(16.0);

            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ComponentStyles::add_primary_button(ui, config.submit_label).clicked() {
                        submit = true;
                    }
                    if ComponentStyles::add_secondary_button(ui, "Cancel", dark_mode).clicked() {
                        outcome = PassphraseModalOutcome::Cancel;
                    }
                    ui.add_space(8.0);
                });
            });

            if submit && outcome == PassphraseModalOutcome::Pending {
                outcome = PassphraseModalOutcome::Submit;
            }
        });

    // X button on the window title bar.
    if !window_is_open && outcome == PassphraseModalOutcome::Pending {
        outcome = PassphraseModalOutcome::Cancel;
    }

    // Escape key. Consume it so a second passphrase modal in the same frame
    // does not also dismiss on the same keypress.
    if outcome == PassphraseModalOutcome::Pending
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        outcome = PassphraseModalOutcome::Cancel;
    }

    // Click outside the window.
    if let Some(ref wr) = window_response
        && outcome == PassphraseModalOutcome::Pending
        && clicked_outside_window(ctx, wr.response.rect)
    {
        outcome = PassphraseModalOutcome::Cancel;
    }

    outcome
}
