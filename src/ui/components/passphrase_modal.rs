//! Shared modal chrome for passphrase entry.
//!
//! Both the wallet-unlock popup ([`WalletUnlockPopup`](super::wallet_unlock_popup))
//! and the just-in-time secret prompt
//! ([`EguiSecretPromptHost`](super::secret_prompt_host)) ask the user for a
//! passphrase through the same centered, overlay-dimmed modal built on the
//! shared [`modal_chrome`](super::modal_chrome). This module adds the passphrase
//! specifics: focus-once, the [`PasswordInput`] field, an inline error line, an
//! optional `extra` body (e.g. a "remember" checkbox), and the action row.
//!
//! Cancellable callers resolve Cancel / Escape / X / click-outside uniformly
//! to [`PassphraseModalOutcome::Cancel`]. Blocking callers omit every dismissal.
//!
//! ## State ownership
//!
//! Per-modal mutable state — the [`PasswordInput`] buffer, a focus-once flag,
//! and the opening-click guard — is stored in egui's data cache keyed by the
//! caller-provided `state_id`. Callers carry only domain state (`remember`,
//! `error`). On [`PassphraseModalOutcome::Submit`] the typed text is extracted
//! into a [`Zeroizing`] string and the cache entry is cleared; dismissal and
//! secondary actions clear the cache too.

use std::fmt;

use egui::Context;
use zeroize::Zeroizing;

use crate::ui::components::modal_chrome::{ModalChromeConfig, modal_chrome};
use crate::ui::components::password_input::PasswordInput;
use crate::ui::helpers::{ModalOpeningGuard, clicked_outside_window_after_open};
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
    /// The user chose the caller-provided secondary action.
    SecondaryAction,
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
            Self::SecondaryAction => f.write_str("SecondaryAction"),
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
    /// Stable identity for this specific secret, wallet, or boot prompt.
    pub state_id: egui::Id,
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
    /// Optional caller-owned secondary action rendered left of Submit.
    pub secondary_action_label: Option<&'a str>,
    /// Placeholder text shown inside the password field before the user types.
    /// Defaults to `"Enter passphrase"` when the callers' existing default is
    /// appropriate; use `"Enter password"` for wallet-unlock flows.
    pub input_placeholder: &'a str,
    /// Caller-specific label for the "keep unlocked" checkbox drawn in `extra`.
    /// `None` falls back to [`KEEP_UNLOCKED_LABEL`] (the wallet wording); set
    /// `Some(...)` for non-wallet prompts (e.g. an identity key) so the
    /// checkbox copy is not wallet-specific (Diziet D-2).
    pub remember_label: Option<&'a str>,
    /// Whether Cancel, Escape, click-outside, and the title-bar close button
    /// may dismiss the modal.
    pub cancellable: bool,
}

/// Per-modal mutable state stored in egui's data cache between frames.
///
/// Keyed by the caller-provided per-secret identity. Created on the first call
/// with `config.input_placeholder`; cleared whenever the prompt closes.
#[derive(Clone)]
struct PassphraseModalState {
    password_input: PasswordInput,
    focus_requested: bool,
    opening_guard: ModalOpeningGuard,
}

fn modal_state_id(config_id: egui::Id) -> egui::Id {
    egui::Id::new("passphrase_modal_state").with(config_id)
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
    let state_id = modal_state_id(config.state_id);

    // Load or initialise per-modal state from egui's data cache.  `get_temp`
    // returns a clone; we mutate the clone during rendering then write it back.
    let mut state: PassphraseModalState = ctx
        .data(|d| d.get_temp::<PassphraseModalState>(state_id))
        .unwrap_or_else(|| PassphraseModalState {
            password_input: PasswordInput::new().with_hint_text(config.input_placeholder),
            focus_requested: false,
            opening_guard: ModalOpeningGuard::armed(),
        });

    let mut should_submit = false;
    let mut should_cancel = false;
    let mut secondary_action = false;

    // The overlay layer id is salted with the window title so a wallet-unlock modal and a JIT
    // secret prompt drawn in the same frame get distinct overlays instead of fighting over one.
    // The window renders on Order::Foreground so the prompt stays above the blocking progress
    // overlay — that overlay must never cover a secret prompt it triggered.
    //
    // `blocks_input` is unconditional and independent of `cancellable`: the
    // progress overlay yields its own barrier to ANY passphrase prompt, so every
    // prompt must own the interaction surface beneath it. `cancellable` governs
    // only who may dismiss it (Cancel / X / Escape / click-outside), which the
    // sink does not affect — dismissal reads raw pointer input.
    let chrome = modal_chrome(
        ctx,
        ModalChromeConfig {
            title: config.window_title.into(),
            overlay_id: egui::Id::new("passphrase_modal_overlay").with(config.window_title),
            overlay_order: egui::Order::Background,
            window_order: egui::Order::Foreground,
            resizable: false,
            show_close_button: config.cancellable,
            blocks_input: true,
            inner_margin: 20,
        },
        |ui| {
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
                    if let Some(label) = config.secondary_action_label
                        && ComponentStyles::add_secondary_button(ui, label, dark_mode).clicked()
                    {
                        secondary_action = true;
                    }
                    if config.cancellable
                        && ComponentStyles::add_secondary_button(ui, "Cancel", dark_mode).clicked()
                    {
                        should_cancel = true;
                    }
                    ui.add_space(8.0);
                });
            });
        },
    );

    // X button on the window title bar.
    if config.cancellable && chrome.closed_via_x {
        should_cancel = true;
    }

    // Escape key. Consume it so a second passphrase modal in the same frame
    // does not also dismiss on the same keypress.
    if config.cancellable
        && !should_submit
        && !should_cancel
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        should_cancel = true;
    }

    // Click outside the window. Uses the opening-guard variant so the very
    // click that opened this modal (still "outside" on the frame the window
    // first appears) can never be misread as an immediate dismissal.
    if config.cancellable
        && let Some(ref wr) = chrome.window_response
        && !should_submit
        && !should_cancel
        && clicked_outside_window_after_open(ctx, wr.rect, &mut state.opening_guard)
    {
        should_cancel = true;
    }

    // Resolve outcome: Submit takes priority; save or clear the cache entry.
    if should_submit {
        let text = Zeroizing::new(state.password_input.text().to_string());
        state.password_input.clear();
        ctx.data_mut(|d| d.remove::<PassphraseModalState>(state_id));
        PassphraseModalOutcome::Submit(text)
    } else if secondary_action {
        state.password_input.clear();
        ctx.data_mut(|d| d.remove::<PassphraseModalState>(state_id));
        PassphraseModalOutcome::SecondaryAction
    } else if should_cancel {
        state.password_input.clear();
        ctx.data_mut(|d| d.remove::<PassphraseModalState>(state_id));
        PassphraseModalOutcome::Cancel
    } else {
        ctx.data_mut(|d| d.insert_temp(state_id, state));
        PassphraseModalOutcome::Pending
    }
}

/// Clear and remove one modal's typed passphrase buffer.
pub fn clear_passphrase_modal_state(ctx: &Context, config_id: egui::Id) {
    let state_id = modal_state_id(config_id);
    ctx.data_mut(|data| data.remove::<PassphraseModalState>(state_id));
}

/// Drop any pointer click still pending on the frame a passphrase prompt first
/// becomes active.
///
/// egui resolves a frame's click at `begin_pass` — against the *previous* frame's
/// widget geometry and modal layer — before that frame's `update` runs. On the
/// frame a prompt first renders, the previous frame had no prompt and no modal
/// sink, so a press-then-release completing now still resolves to the control
/// beneath, *before* [`passphrase_modal`] installs its sink later this frame. A
/// widget only reports the click if a `Released` event is still in
/// `input.pointer` when it interacts (see egui's `Context::create_widget`);
/// clearing the pointer state — called as the prompt is promoted, before the
/// screen beneath runs — drops that one frame's click so it cannot fall through.
/// The sink covers every later frame. Keyboard events are left intact so the
/// freshly focused password field still receives typing.
pub fn drop_activation_frame_pointer_click(ctx: &Context) {
    ctx.input_mut(|input| {
        input.pointer = Default::default();
        input
            .events
            .retain(|event| !matches!(event, egui::Event::PointerButton { .. }));
    });
}

#[cfg(test)]
pub(crate) fn passphrase_modal_state_exists(ctx: &Context, config_id: egui::Id) -> bool {
    ctx.data(|data| {
        data.get_temp::<PassphraseModalState>(modal_state_id(config_id))
            .is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_specific_state_never_crosses_to_another_prompt() {
        let ctx = egui::Context::default();
        let wallet_a = egui::Id::new("wallet").with([0xA1u8; 32]);
        let wallet_b = egui::Id::new("wallet").with([0xB2u8; 32]);
        let mut password_input = PasswordInput::new();
        password_input.set_text("wallet-a-password");
        ctx.data_mut(|data| {
            data.insert_temp(
                modal_state_id(wallet_a),
                PassphraseModalState {
                    password_input,
                    focus_requested: true,
                    opening_guard: ModalOpeningGuard::armed(),
                },
            );
        });

        assert!(
            ctx.data(|data| data.get_temp::<PassphraseModalState>(modal_state_id(wallet_b)))
                .is_none(),
            "wallet B must not see wallet A's typed password",
        );
        clear_passphrase_modal_state(&ctx, wallet_a);
        assert!(
            ctx.data(|data| data.get_temp::<PassphraseModalState>(modal_state_id(wallet_a)))
                .is_none(),
            "closing the prompt must remove its typed buffer",
        );
    }

    #[test]
    fn opening_click_does_not_immediately_dismiss() {
        // Guards the c0835efd regression: a fresh, still-`armed` ModalOpeningGuard
        // must not be readable as an already-consumed outside click by callers
        // that construct `PassphraseModalState` directly (e.g. via
        // `passphrase_modal_state_exists`/cache-priming helpers elsewhere).
        let ctx = egui::Context::default();
        let id = egui::Id::new("wallet").with([0x42u8; 32]);
        ctx.data_mut(|data| {
            data.insert_temp(
                modal_state_id(id),
                PassphraseModalState {
                    password_input: PasswordInput::new(),
                    focus_requested: true,
                    opening_guard: ModalOpeningGuard::armed(),
                },
            );
        });

        assert!(
            passphrase_modal_state_exists(&ctx, id),
            "state seeded with a freshly-armed guard must still be present \
             (i.e. constructing it does not itself count as a dismissal)",
        );
    }
}
