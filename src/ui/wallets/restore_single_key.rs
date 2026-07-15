//! T-SK-03 — "Restore a password-protected imported key" dialog.
//!
//! After the platform-wallet migration, single-key rows that were saved
//! with a per-key password are preserved but invisible: they are still
//! encrypted under the user's OLD password, which the migration could not
//! supply. This dialog takes that password, hands it to
//! [`restore_protected_single_key`](crate::backend_task::migration::single_key_restore::restore_protected_single_key),
//! and re-protects the recovered key in the modern vault.
//!
//! The dialog is a self-contained component: the parent sets the target
//! key, flips `open = true`, and reads
//! [`RestoreSingleKeyResponse::confirmed`] on the frame the user submits.
//! The parent owns the actual restore call and the post-restore refresh.

use eframe::egui::{self, Context, RichText, Ui};
use zeroize::Zeroizing;

use crate::backend_task::migration::single_key_restore::PendingProtectedRestore;
use crate::model::wallet::passphrase::validate_single_key_passphrase;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::theme::DashColors;

/// Maximum hint length, matching the import dialog.
const HINT_MAX_CHARS: usize = 64;

/// Per-frame outcome of [`RestoreSingleKeyDialog::show`].
///
/// `Debug` is hand-written so the embedded secret material never leaks.
#[derive(Default, Clone)]
pub struct RestoreSingleKeyResponse {
    /// `Some` only on the frame the user submits valid inputs.
    pub confirmed: Option<RestoreSingleKeyRequest>,
    /// `true` if the user dismissed the dialog this frame.
    pub cancelled: bool,
}

/// Request emitted when the user submits the restore form. Holds the old
/// password and the new passphrase, both wrapped in [`Zeroizing`] so the
/// copies wipe on drop. `Debug` is hand-written to redact both — deriving
/// it would dump the secrets into logs or panic backtraces.
#[derive(Clone)]
pub struct RestoreSingleKeyRequest {
    /// Address of the protected key being restored.
    pub address: String,
    /// The OLD per-key password that decrypts the legacy blob.
    pub legacy_password: Zeroizing<String>,
    /// The NEW passphrase to re-protect the key in the modern vault.
    /// `None` ⇒ store without an extra passphrase layer (vault-scoped).
    pub new_passphrase: Option<Zeroizing<String>>,
    /// Optional hint for the new passphrase.
    pub new_hint: Option<String>,
}

impl std::fmt::Debug for RestoreSingleKeyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestoreSingleKeyRequest")
            .field("address", &self.address)
            .field(
                "legacy_password",
                &format_args!("[redacted; len {}]", self.legacy_password.len()),
            )
            .field(
                "new_passphrase",
                &self
                    .new_passphrase
                    .as_ref()
                    .map(|p| format!("[redacted; len {}]", p.len())),
            )
            .field("new_hint", &self.new_hint)
            .finish()
    }
}

impl std::fmt::Debug for RestoreSingleKeyResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RestoreSingleKeyResponse")
            .field("confirmed", &self.confirmed)
            .field("cancelled", &self.cancelled)
            .finish()
    }
}

/// Modal dialog state. Hold one per wallets screen.
pub struct RestoreSingleKeyDialog {
    /// Toggle this from outside to open/close the dialog.
    pub open: bool,
    /// The protected key the dialog is currently restoring. `None` until
    /// the parent points it at a key via [`Self::set_target`].
    target: Option<PendingProtectedRestore>,
    legacy_password_input: PasswordInput,
    protect_again: bool,
    new_passphrase_input: PasswordInput,
    confirm_passphrase_input: PasswordInput,
    hint_input: String,
    /// Inline validation message for the new-passphrase fields.
    passphrase_error: Option<String>,
}

impl Default for RestoreSingleKeyDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl RestoreSingleKeyDialog {
    pub fn new() -> Self {
        Self {
            open: false,
            target: None,
            legacy_password_input: PasswordInput::new().with_hint_text("Your old password"),
            protect_again: true,
            new_passphrase_input: PasswordInput::new().with_hint_text("New passphrase"),
            confirm_passphrase_input: PasswordInput::new().with_hint_text("Confirm new passphrase"),
            hint_input: String::new(),
            passphrase_error: None,
        }
    }

    /// Point the dialog at a specific protected key and open it.
    pub fn set_target(&mut self, target: PendingProtectedRestore) {
        self.target = Some(target);
        self.open = true;
    }

    /// Drop all transient state. Called by the parent after a successful
    /// restore or an explicit dismiss.
    pub fn reset(&mut self) {
        self.target = None;
        self.legacy_password_input.clear();
        self.protect_again = true;
        self.new_passphrase_input.clear();
        self.confirm_passphrase_input.clear();
        self.hint_input.clear();
        self.passphrase_error = None;
    }

    /// Render the dialog as an egui modal window. Returns the per-frame
    /// response describing whether the user confirmed or cancelled.
    pub fn show(&mut self, ctx: &Context) -> RestoreSingleKeyResponse {
        let mut response = RestoreSingleKeyResponse::default();
        if !self.open || self.target.is_none() {
            return response;
        }
        let mut open_flag = self.open;
        egui::Window::new("Restore a protected imported key")
            .collapsible(false)
            .resizable(false)
            .open(&mut open_flag)
            .show(ctx, |ui| {
                self.body(ui, &mut response);
            });
        if !open_flag && self.open {
            response.cancelled = true;
        }
        self.open = open_flag;
        response
    }

    /// Body of the dialog without the window wrapper, so the kittest
    /// harness can drive the layout directly. Returns the per-frame
    /// response.
    pub fn show_in_ui(&mut self, ui: &mut Ui) -> RestoreSingleKeyResponse {
        let mut response = RestoreSingleKeyResponse::default();
        self.body(ui, &mut response);
        response
    }

    fn body(&mut self, ui: &mut Ui, response: &mut RestoreSingleKeyResponse) {
        let dark_mode = ui.style().visuals.dark_mode;
        let text_color = DashColors::text_primary(dark_mode);

        let Some(target) = self.target.clone() else {
            return;
        };

        ui.label(
            RichText::new(
                "This imported key was protected with a password before the last update. \
                 Enter that password to restore it.",
            )
            .color(text_color),
        );
        ui.add_space(8.0);

        ui.horizontal_wrapped(|ui| {
            ui.label("Address:");
            ui.label(RichText::new(&target.address).monospace().color(text_color));
        });
        if let Some(alias) = &target.alias {
            ui.horizontal_wrapped(|ui| {
                ui.label("Name:");
                ui.label(RichText::new(alias).color(text_color));
            });
        }
        ui.add_space(8.0);

        ui.label("Old password");
        self.legacy_password_input.show(ui);
        ui.add_space(8.0);

        ui.checkbox(&mut self.protect_again, "Protect with a new passphrase");
        if self.protect_again {
            ui.add_space(4.0);
            ui.label("New passphrase");
            let pp = self.new_passphrase_input.show(ui);
            if pp.changed {
                self.passphrase_error = None;
            }
            ui.label("Confirm new passphrase");
            let cp = self.confirm_passphrase_input.show(ui);
            if cp.changed {
                self.passphrase_error = None;
            }
            ui.label("Hint (optional)");
            ui.add(
                egui::TextEdit::singleline(&mut self.hint_input)
                    .hint_text("Shown next to the unlock prompt")
                    .char_limit(HINT_MAX_CHARS),
            );
            if let Some(err) = &self.passphrase_error {
                ui.colored_label(DashColors::VALIDATION_WARNING, err);
            }
            ui.add_space(8.0);
        }

        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                response.cancelled = true;
            }
            let can_confirm = !self.legacy_password_input.is_empty();
            let confirm_btn = egui::Button::new(RichText::new("Restore key").strong());
            if ui.add_enabled(can_confirm, confirm_btn).clicked() {
                let new_passphrase = if self.protect_again {
                    let pp = Zeroizing::new(self.new_passphrase_input.text().to_string());
                    if let Err(err) =
                        validate_single_key_passphrase(&pp, self.confirm_passphrase_input.text())
                    {
                        self.passphrase_error = Some(err.to_string());
                        return;
                    }
                    Some(pp)
                } else {
                    None
                };
                let hint = {
                    let trimmed = self.hint_input.trim().to_string();
                    (self.protect_again && !trimmed.is_empty()).then_some(trimmed)
                };
                response.confirmed = Some(RestoreSingleKeyRequest {
                    address: target.address.clone(),
                    legacy_password: Zeroizing::new(self.legacy_password_input.text().to_string()),
                    new_passphrase,
                    new_hint: hint,
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::Network;

    fn sample_target() -> PendingProtectedRestore {
        PendingProtectedRestore {
            address: "yProtectedAddr".into(),
            alias: Some("savings".into()),
            network: Network::Testnet,
        }
    }

    /// The hand-written `Debug` must redact both secrets and never leak
    /// them in any representation.
    #[test]
    fn debug_redacts_old_password_and_new_passphrase() {
        let legacy = "old-legacy-password";
        let new_pp = "brand-new-passphrase";
        let request = RestoreSingleKeyRequest {
            address: "yProtectedAddr".into(),
            legacy_password: Zeroizing::new(legacy.to_string()),
            new_passphrase: Some(Zeroizing::new(new_pp.to_string())),
            new_hint: Some("the usual".into()),
        };
        let response = RestoreSingleKeyResponse {
            confirmed: Some(request.clone()),
            cancelled: false,
        };
        for dbg in [format!("{request:?}"), format!("{response:?}")] {
            assert!(!dbg.contains(legacy), "leaked old password: {dbg}");
            assert!(!dbg.contains(new_pp), "leaked new passphrase: {dbg}");
            assert!(dbg.contains("[redacted"), "must mark redaction: {dbg}");
        }
        // Non-secret fields stay visible for diagnostics.
        assert!(format!("{request:?}").contains("yProtectedAddr"));
    }

    /// `set_target` opens the dialog at the given key; `reset` clears it.
    #[test]
    fn set_target_opens_and_reset_clears() {
        let mut dialog = RestoreSingleKeyDialog::new();
        assert!(!dialog.open);
        dialog.set_target(sample_target());
        assert!(dialog.open);
        assert!(dialog.target.is_some());
        dialog.reset();
        assert!(dialog.target.is_none());
    }
}
