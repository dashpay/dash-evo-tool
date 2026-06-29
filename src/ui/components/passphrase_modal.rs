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
//!
//! ## State ownership
//!
//! Per-modal mutable state — the [`PasswordInput`] buffer and a focus-once flag
//! — is stored in egui's data cache keyed by `window_title`. Callers carry only
//! domain state (`remember`, `error`). On [`PassphraseModalOutcome::Submit`] the
//! typed text is extracted into a [`Zeroizing`] string and the cache entry is
//! cleared; on [`PassphraseModalOutcome::Cancel`] the cache entry is cleared too.

use std::fmt;

use egui::Context;
use zeroize::Zeroizing;

use crate::ui::components::password_input::PasswordInput;
use crate::ui::helpers::clicked_outside_window;
use crate::ui::theme::{ComponentStyles, DashColors};

/// The "keep unlocked" checkbox label, shared by every passphrase modal caller.
///
/// A complete, translatable sentence (i18n-ready, no placeholders). Defined
/// here so [`WalletUnlockPopup`](super::wallet_unlock_popup) and
/// [`ActivePrompt`](super::secret_prompt_host::ActivePrompt) reference exactly
/// one literal — no silent drift between the two UIs.
pub const KEEP_UNLOCKED_LABEL: &str = "Keep this wallet unlocked until I close the app.";

/// What the user did with a [`passphrase_modal`] this frame.
#[derive(Clone, PartialEq)]
pub enum PassphraseModalOutcome {
    /// The modal is still open; no decision yet.
    Pending,
    /// The user submitted (Enter or the submit button). The inner value is the
    /// typed passphrase; the internal buffer is zeroized immediately after
    /// extraction.
    Submit(Zeroizing<String>),
    /// The user dismissed (Cancel button, Escape, X, or click-outside).
    Cancel,
}

// Manual Debug to prevent the typed passphrase from leaking into logs.
// (`Zeroizing` derives its Debug from the inner type, which would expose the
// plaintext string.)
impl fmt::Debug for PassphraseModalOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => f.write_str("Pending"),
            Self::Submit(_) => f.write_str("Submit(<redacted>)"),
            Self::Cancel => f.write_str("Cancel"),
        }
    }
}

/// Static copy + layout knobs for one render of [`passphrase_modal`].
///
/// Borrowed for the call only; carries no secret. `window_title` and
/// `submit_label` are complete, translatable sentences/labels (i18n-ready)
/// supplied by the caller so the same chrome serves "Unlock Wallet" and the
/// JIT prompt.
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
    /// Placeholder text shown inside the password field before the user types.
    /// Defaults to `"Enter passphrase"` when the callers' existing default is
    /// appropriate; use `"Enter password"` for wallet-unlock flows.
    pub input_placeholder: &'a str,
    /// Caller-specific label for the "keep unlocked" checkbox drawn in `extra`.
    /// `None` falls back to [`KEEP_UNLOCKED_LABEL`] (the wallet wording); set
    /// `Some(...)` for non-wallet prompts (e.g. an identity key) so the
    /// checkbox copy is not wallet-specific (Diziet D-2).
    pub remember_label: Option<&'a str>,
}

/// Per-modal mutable state stored in egui's data cache between frames.
///
/// Keyed by `window_title` via [`egui::Id::new("passphrase_modal_state").with(title)`].
/// Created on the first call with `config.input_placeholder`; cleared on
/// Submit or Cancel.
#[derive(Clone)]
struct PassphraseModalState {
    password_input: PasswordInput,
    focus_requested: bool,
}

/// Render the shared passphrase modal and return what the user did.
///
/// All per-modal mutable state (the [`PasswordInput`], focus tracking) is
/// managed internally via egui's data cache so callers only need to carry
/// domain state (`remember`, `error`). The typed passphrase is returned inside
/// [`PassphraseModalOutcome::Submit`] and zeroized in the cache immediately
/// after extraction.
///
/// `extra` draws any caller-specific body (e.g. the "keep unlocked" checkbox)
/// between the error line and the button row.
pub fn passphrase_modal(
    ctx: &Context,
    config: &PassphraseModalConfig<'_>,
    extra: impl FnOnce(&mut egui::Ui),
) -> PassphraseModalOutcome {
    let state_id = egui::Id::new("passphrase_modal_state").with(config.window_title);

    // Load or initialise per-modal state from egui's data cache.  `get_temp`
    // returns a clone; we mutate the clone during rendering then write it back.
    let mut state: PassphraseModalState = ctx
        .data(|d| d.get_temp::<PassphraseModalState>(state_id))
        .unwrap_or_else(|| PassphraseModalState {
            password_input: PasswordInput::new().with_hint_text(config.input_placeholder),
            focus_requested: false,
        });

    // Dark overlay behind the modal. The layer id is salted with the window
    // title so a wallet-unlock modal and a JIT secret prompt drawn in the same
    // frame get distinct overlay layers instead of fighting over one.
    let screen_rect = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("passphrase_modal_overlay").with(config.window_title),
    ));
    painter.rect_filled(screen_rect, 0.0, DashColors::modal_overlay());

    let mut should_submit = false;
    let mut should_cancel = false;
    let mut window_is_open = true;

    let window_response = egui::Window::new(config.window_title)
        .collapsible(false)
        .resizable(false)
        // Render on Order::Foreground so the prompt stays above the blocking
        // progress overlay (also Foreground, but drawn earlier this frame) — the
        // overlay must never cover a secret prompt it triggered (R-1, SEC-002).
        // Created after the overlay and focus-raised, so it wins within Foreground.
        .order(egui::Order::Foreground)
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
            fill: ctx.global_style().visuals.window_fill,
            stroke: egui::Stroke::new(1.0, DashColors::popup_border_glow()),
        })
        .show(ctx, |ui| {
            ui.set_min_width(350.0);
            ui.set_max_width(400.0);

            let dark_mode = ui.style().visuals.dark_mode;

            ui.label(egui::RichText::new(config.body).color(DashColors::text_primary(dark_mode)));

            ui.add_space(12.0);

            let pw_response = state.password_input.show(ui);

            if !state.focus_requested {
                pw_response.response.request_focus();
                state.focus_requested = true;
            }

            if pw_response.response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                should_submit = true;
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
                        should_submit = true;
                    }
                    if ComponentStyles::add_secondary_button(ui, "Cancel", dark_mode).clicked() {
                        should_cancel = true;
                    }
                    ui.add_space(8.0);
                });
            });
        });

    // X button on the window title bar.
    if !window_is_open {
        should_cancel = true;
    }

    // Escape key. Consume it so a second passphrase modal in the same frame
    // does not also dismiss on the same keypress.
    if !should_submit
        && !should_cancel
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        should_cancel = true;
    }

    // Click outside the window.
    if let Some(ref wr) = window_response
        && !should_submit
        && !should_cancel
        && clicked_outside_window(ctx, wr.response.rect)
    {
        should_cancel = true;
    }

    // Resolve outcome: Submit takes priority; save or clear the cache entry.
    if should_submit {
        let text = Zeroizing::new(state.password_input.text().to_string());
        state.password_input.clear();
        ctx.data_mut(|d| d.remove::<PassphraseModalState>(state_id));
        PassphraseModalOutcome::Submit(text)
    } else if should_cancel {
        ctx.data_mut(|d| d.remove::<PassphraseModalState>(state_id));
        PassphraseModalOutcome::Cancel
    } else {
        ctx.data_mut(|d| d.insert_temp(state_id, state));
        PassphraseModalOutcome::Pending
    }
}
