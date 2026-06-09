//! J-6 "Import private key (advanced)" dialog.
//!
//! Lets a power user paste a WIF-encoded single private key, preview the
//! derived P2PKH address, then route the import through
//! [`crate::wallet_backend::SingleKeyView::import_wif`]. The dialog is a
//! self-contained component: callers create one, set `open = true`, and
//! call [`ImportSingleKeyDialog::show`] each frame. Side effects on
//! confirm are returned via [`ImportSingleKeyResponse::confirmed`] so the
//! parent screen owns the `WalletBackend` lookup and the post-import
//! refresh.
//!
//! Backed by TC-SK-004 (valid WIF accepted), TC-SK-005 (invalid WIF
//! inline error), TC-SK-007 (mask + reveal toggle, ARIA), TC-A11Y-005.

use dash_sdk::dpp::dashcore::secp256k1::Secp256k1;
use dash_sdk::dpp::dashcore::{Address, Network, PrivateKey, PublicKey};
use eframe::egui::{self, Context, RichText, Ui};
use zeroize::Zeroizing;

use crate::backend_task::error::TaskError;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::theme::DashColors;

/// Maximum alias length accepted by the dialog. Matches the legacy
/// single-key alias limit so list rows stay readable.
const ALIAS_MAX_CHARS: usize = 64;

/// Outcome of a single frame of [`ImportSingleKeyDialog::show`].
///
/// `Debug` is hand-written so the embedded [`ImportSingleKeyRequest`]'s
/// secret material never leaks through a derived `{:?}`.
#[derive(Default, Clone)]
pub struct ImportSingleKeyResponse {
    /// `Some` only on the frame the user clicks **Add to wallets** with a
    /// valid WIF. Carries the WIF and the user-supplied alias (trimmed)
    /// so the parent can call
    /// [`crate::wallet_backend::SingleKeyView::import_wif`] without
    /// re-reading the input.
    pub confirmed: Option<ImportSingleKeyRequest>,
    /// `true` if the user dismissed the dialog this frame (Cancel button
    /// or window close). Parent should set `open = false` and clear any
    /// stale state.
    pub cancelled: bool,
}

/// Request emitted when the user confirms a valid WIF.
///
/// Holds raw secret material (`wif`, `passphrase`). The WIF is wrapped in
/// [`Zeroizing`] so the copy wipes on drop; `Debug` is hand-written to redact
/// both — deriving it would dump the private key and passphrase into logs or
/// panic backtraces.
#[derive(Clone)]
pub struct ImportSingleKeyRequest {
    /// WIF-encoded private key. Wrapped in [`Zeroizing`]; derefs to `&str`
    /// for callers that need the raw text.
    pub wif: Zeroizing<String>,
    pub alias: Option<String>,
    /// Address preview shown to the user — handed back so the parent can
    /// echo it in a success message without re-deriving.
    pub address_preview: String,
    /// SEC-002 — optional per-key passphrase. `None` means store the
    /// key without an additional encryption layer (still inside the
    /// vault); `Some` means encrypt under the user's passphrase.
    pub passphrase: Option<String>,
    /// Optional hint shown next to the unlock prompt for protected
    /// imports.
    pub passphrase_hint: Option<String>,
}

impl std::fmt::Debug for ImportSingleKeyRequest {
    /// Redacts `wif` and `passphrase` (presence + length only) so neither
    /// the private key nor the passphrase can leak through `{:?}`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportSingleKeyRequest")
            .field("wif", &format_args!("[redacted; len {}]", self.wif.len()))
            .field("alias", &self.alias)
            .field("address_preview", &self.address_preview)
            .field(
                "passphrase",
                &self
                    .passphrase
                    .as_ref()
                    .map(|p| format!("[redacted; len {}]", p.len())),
            )
            .field("passphrase_hint", &self.passphrase_hint)
            .finish()
    }
}

impl std::fmt::Debug for ImportSingleKeyResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportSingleKeyResponse")
            .field("confirmed", &self.confirmed)
            .field("cancelled", &self.cancelled)
            .finish()
    }
}

/// Modal dialog state. Hold one per wallets screen; reset between
/// invocations by calling [`Self::reset`].
pub struct ImportSingleKeyDialog {
    /// Toggle this from outside to open/close the dialog.
    pub open: bool,
    network: Network,
    wif_input: PasswordInput,
    alias_input: String,
    /// `Some` once the WIF parses cleanly — drives the preview rendering
    /// and the Add button enablement.
    derived_address: Option<String>,
    /// Set when the latest WIF input fails to parse. `None` while the
    /// field is empty so we don't yell at the user on first focus.
    error_message: Option<String>,
    /// SEC-002 Option C — "Protect with passphrase" toggle.
    passphrase_enabled: bool,
    passphrase_input: PasswordInput,
    confirm_passphrase_input: PasswordInput,
    hint_input: String,
    /// Inline validation message for the passphrase fields. Reset on
    /// every keystroke; populated on confirm click if validation fails.
    passphrase_error: Option<String>,
}

impl ImportSingleKeyDialog {
    /// Build a fresh dialog scoped to `network`. The dialog only derives
    /// the address preview locally; the canonical import path still uses
    /// the network configured on the active `WalletBackend`.
    pub fn new(network: Network) -> Self {
        Self {
            open: false,
            network,
            wif_input: PasswordInput::new()
                .with_hint_text("Paste your private key (WIF)")
                .with_monospace(),
            alias_input: String::new(),
            derived_address: None,
            error_message: None,
            passphrase_enabled: false,
            passphrase_input: PasswordInput::new().with_hint_text("Passphrase"),
            confirm_passphrase_input: PasswordInput::new().with_hint_text("Confirm passphrase"),
            hint_input: String::new(),
            passphrase_error: None,
        }
    }

    /// Switch the network the preview is derived against. Call this on
    /// network change so the dialog never previews a mainnet address for
    /// a testnet wallet (or vice versa).
    pub fn set_network(&mut self, network: Network) {
        if self.network != network {
            self.network = network;
            self.recompute_preview();
        }
    }

    /// Drop all transient state. Called by the parent after a successful
    /// import or an explicit dismiss so the next open starts clean.
    pub fn reset(&mut self) {
        self.wif_input.clear();
        self.alias_input.clear();
        self.derived_address = None;
        self.error_message = None;
        self.passphrase_enabled = false;
        self.passphrase_input.clear();
        self.confirm_passphrase_input.clear();
        self.hint_input.clear();
        self.passphrase_error = None;
    }

    /// Render the dialog as an egui modal window. Returns the per-frame
    /// response describing whether the user confirmed or cancelled.
    pub fn show(&mut self, ctx: &Context) -> ImportSingleKeyResponse {
        let mut response = ImportSingleKeyResponse::default();
        if !self.open {
            return response;
        }

        let mut open_flag = self.open;
        egui::Window::new("Import private key (advanced)")
            .collapsible(false)
            .resizable(false)
            .open(&mut open_flag)
            .show(ctx, |ui| {
                self.body(ui, &mut response);
            });

        // Window close icon ('x') flips `open_flag` to false — treat that
        // the same as the Cancel button so the parent learns to dismiss.
        if !open_flag && self.open {
            response.cancelled = true;
        }
        self.open = open_flag;
        response
    }

    /// Body of the dialog, factored out so the kittest harness can drive
    /// the layout directly without a window wrapper.
    pub fn show_in_ui(&mut self, ui: &mut Ui) -> ImportSingleKeyResponse {
        let mut response = ImportSingleKeyResponse::default();
        self.body(ui, &mut response);
        response
    }

    fn body(&mut self, ui: &mut Ui, response: &mut ImportSingleKeyResponse) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let text_color = DashColors::text_primary(dark_mode);

        ui.label(
            RichText::new(
                "Paste a WIF-encoded private key. The key is stored encrypted on this device.",
            )
            .color(text_color),
        );
        ui.add_space(8.0);

        ui.label("Private key (WIF)");
        let wif_response = self.wif_input.show(ui);
        if wif_response.changed {
            self.recompute_preview();
        }
        ui.add_space(8.0);

        if let Some(err) = &self.error_message {
            ui.colored_label(DashColors::VALIDATION_WARNING, err);
            ui.add_space(8.0);
        }

        if let Some(addr) = &self.derived_address {
            ui.horizontal_wrapped(|ui| {
                ui.label("Derived address:");
                ui.label(RichText::new(addr).monospace().color(text_color));
            });
            ui.label(
                RichText::new(format!(
                    "This is a {network} address.",
                    network = network_label(self.network)
                ))
                .small()
                .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(8.0);
        }

        ui.label("Nickname (optional)");
        ui.add(
            egui::TextEdit::singleline(&mut self.alias_input)
                .hint_text("Imported key")
                .char_limit(ALIAS_MAX_CHARS),
        );
        ui.add_space(12.0);

        // SEC-002 Option C — passphrase opt-in.
        ui.checkbox(&mut self.passphrase_enabled, "Protect with passphrase");
        if self.passphrase_enabled {
            ui.add_space(4.0);
            ui.label("Passphrase");
            let pp = self.passphrase_input.show(ui);
            if pp.changed {
                self.passphrase_error = None;
            }
            ui.label("Confirm passphrase");
            let cp = self.confirm_passphrase_input.show(ui);
            if cp.changed {
                self.passphrase_error = None;
            }
            ui.label("Hint (optional)");
            ui.add(
                egui::TextEdit::singleline(&mut self.hint_input)
                    .hint_text("Shown next to the unlock prompt")
                    .char_limit(ALIAS_MAX_CHARS),
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

            let can_confirm = self.derived_address.is_some();
            let confirm_btn = egui::Button::new(RichText::new("Add to wallets").strong());
            if ui.add_enabled(can_confirm, confirm_btn).clicked()
                && let Some(addr) = self.derived_address.clone()
            {
                let (passphrase, hint) = if self.passphrase_enabled {
                    let pp = self.passphrase_input.text().to_string();
                    let cp = self.confirm_passphrase_input.text().to_string();
                    if let Some(err) = validate_passphrase(&pp, &cp) {
                        self.passphrase_error = Some(err);
                        return;
                    }
                    let trimmed_hint = self.hint_input.trim().to_string();
                    let hint = (!trimmed_hint.is_empty()).then_some(trimmed_hint);
                    (Some(pp), hint)
                } else {
                    (None, None)
                };
                let alias = self.alias_input.trim().to_string();
                response.confirmed = Some(ImportSingleKeyRequest {
                    wif: Zeroizing::new(self.wif_input.text().to_string()),
                    alias: (!alias.is_empty()).then_some(alias),
                    address_preview: addr,
                    passphrase,
                    passphrase_hint: hint,
                });
            }
        });
    }

    /// Test-only seam: write the given string into the WIF input and
    /// recompute the preview so the kittest harness can drive the dialog
    /// without simulating keystrokes (which `password()`-flagged TextEdits
    /// are awkward to script). Not exposed for production callers.
    #[doc(hidden)]
    pub fn force_input_for_test(&mut self, wif: String) {
        self.wif_input.set_text(wif);
        self.recompute_preview();
    }

    fn recompute_preview(&mut self) {
        let raw = self.wif_input.text().trim();
        if raw.is_empty() {
            self.derived_address = None;
            self.error_message = None;
            return;
        }
        match PrivateKey::from_wif(raw) {
            Ok(priv_key) => {
                let secp = Secp256k1::new();
                let pub_key = PublicKey {
                    compressed: priv_key.compressed,
                    inner: priv_key.inner.public_key(&secp),
                };
                let address = Address::p2pkh(&pub_key, self.network);
                self.derived_address = Some(address.to_string());
                self.error_message = None;
            }
            Err(e) => {
                self.derived_address = None;
                self.error_message = Some(
                    TaskError::InvalidWif {
                        source: Box::new(e),
                    }
                    .to_string(),
                );
            }
        }
    }
}

/// Client-side passphrase validation mirroring the backend's typed
/// rules. Returns the user-facing message (from the corresponding
/// [`TaskError`] variant's `Display`) when the input is rejected,
/// `None` otherwise. The backend re-checks both rules so a callsite
/// that skips this still gets the typed error.
fn validate_passphrase(passphrase: &str, confirm: &str) -> Option<String> {
    if passphrase.chars().count() < crate::wallet_backend::single_key::MIN_SINGLE_KEY_PASSPHRASE_LEN
    {
        return Some(
            TaskError::SingleKeyPassphraseTooShort {
                min: crate::wallet_backend::single_key::MIN_SINGLE_KEY_PASSPHRASE_LEN as u32,
            }
            .to_string(),
        );
    }
    if passphrase != confirm {
        return Some(TaskError::SingleKeyPassphraseMismatch.to_string());
    }
    None
}

fn network_label(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "Mainnet",
        Network::Testnet => "Testnet",
        Network::Devnet => "Devnet",
        Network::Regtest => "Regtest",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Testnet WIF pinned to the same value the wallet-backend unit test
    /// uses (`src/wallet_backend/single_key.rs::known_wif`). Keeping the
    /// dialog test in sync with the backend avoids a third magic string.
    fn known_testnet_wif() -> &'static str {
        "cMahea7zqjxrtgAbB7LSGbcQUr1uX1ojuat9jZodMN8rFTv2sfUK"
    }

    /// F92 — the hand-written `Debug` must redact `wif` and `passphrase`,
    /// in every representation. We assert the secret text is absent and that
    /// neither its hex nor its decimal-byte-array form leaks.
    #[test]
    fn debug_redacts_wif_and_passphrase_no_leak() {
        let wif = known_testnet_wif();
        let passphrase = "correct-horse-battery-staple";
        let request = ImportSingleKeyRequest {
            wif: Zeroizing::new(wif.to_string()),
            alias: Some("primary".into()),
            address_preview: "yPreviewAddr".into(),
            passphrase: Some(passphrase.to_string()),
            passphrase_hint: Some("the usual".into()),
        };

        // Cover the request directly and nested in the response.
        let response = ImportSingleKeyResponse {
            confirmed: Some(request.clone()),
            cancelled: false,
        };
        let dbgs = [format!("{request:?}"), format!("{response:?}")];

        let wif_hex = hex::encode(wif.as_bytes());
        let pp_hex = hex::encode(passphrase.as_bytes());
        let wif_decimal = format!("{:?}", wif.as_bytes()); // decimal byte-array form
        let pp_decimal = format!("{:?}", passphrase.as_bytes());

        for dbg in &dbgs {
            assert!(!dbg.contains(wif), "debug leaked the WIF text: {dbg}");
            assert!(
                !dbg.contains(passphrase),
                "debug leaked the passphrase text: {dbg}"
            );
            assert!(!dbg.contains(&wif_hex), "debug leaked the WIF hex: {dbg}");
            assert!(
                !dbg.contains(&pp_hex),
                "debug leaked the passphrase hex: {dbg}"
            );
            assert!(
                !dbg.contains(&wif_decimal),
                "debug leaked the WIF byte array: {dbg}"
            );
            assert!(
                !dbg.contains(&pp_decimal),
                "debug leaked the passphrase byte array: {dbg}"
            );
            assert!(
                dbg.contains("[redacted"),
                "debug must mark redaction: {dbg}"
            );
        }
        // Non-secret fields are still visible for diagnostics.
        assert!(dbgs[0].contains("yPreviewAddr"));
        assert!(dbgs[0].contains("primary"));
    }

    /// TC-SK-005 (typed-error path): an invalid WIF surfaces the inline
    /// error string the dialog shows next to the input. No derived
    /// address; the confirm button reads "disabled" by virtue of
    /// `derived_address == None`.
    #[test]
    fn invalid_wif_sets_inline_error_no_address() {
        let mut dialog = ImportSingleKeyDialog::new(Network::Testnet);
        dialog.wif_input.set_text("not-a-valid-wif".to_string());
        dialog.recompute_preview();
        assert!(dialog.derived_address.is_none());
        assert_eq!(
            dialog.error_message.as_deref(),
            Some(
                "This does not look like a valid private key. Check the characters and try again."
            )
        );
    }

    /// TC-SK-004 (preview half): a valid testnet WIF yields a P2PKH
    /// address and clears any prior error. Dash testnet P2PKH addresses
    /// start with `y` (script-version 140) — defensive against future
    /// network mis-routing.
    #[test]
    fn valid_wif_produces_testnet_p2pkh_preview() {
        let mut dialog = ImportSingleKeyDialog::new(Network::Testnet);
        dialog.wif_input.set_text(known_testnet_wif().to_string());
        dialog.recompute_preview();
        let addr = dialog.derived_address.as_deref().expect("address derived");
        assert!(dialog.error_message.is_none());
        assert!(
            addr.starts_with('y'),
            "Dash testnet P2PKH addresses start with `y`; got {addr}"
        );
    }

    /// Switching networks recomputes the preview so a paste-and-switch
    /// flow can't show a mainnet address while the active wallet is on
    /// testnet.
    #[test]
    fn network_switch_recomputes_preview() {
        let mut dialog = ImportSingleKeyDialog::new(Network::Testnet);
        dialog.wif_input.set_text(known_testnet_wif().to_string());
        dialog.recompute_preview();
        let testnet_addr = dialog.derived_address.clone().expect("testnet addr");

        dialog.set_network(Network::Mainnet);
        let mainnet_addr = dialog.derived_address.clone().expect("mainnet addr");
        assert_ne!(testnet_addr, mainnet_addr);
        assert!(
            mainnet_addr.starts_with('X'),
            "mainnet P2PKH addresses use `X` prefix; got {mainnet_addr}"
        );
    }

    /// Clearing the input wipes the preview and the error so the next
    /// paste starts from a clean slate.
    #[test]
    fn empty_input_clears_preview_and_error() {
        let mut dialog = ImportSingleKeyDialog::new(Network::Testnet);
        dialog.wif_input.set_text("not-a-valid-wif".to_string());
        dialog.recompute_preview();
        assert!(dialog.error_message.is_some());

        dialog.wif_input.clear();
        dialog.recompute_preview();
        assert!(dialog.derived_address.is_none());
        assert!(dialog.error_message.is_none());
    }

    /// Reset blanks every transient field — used by the parent screen
    /// after a successful import so the next open starts clean.
    #[test]
    fn reset_clears_every_field() {
        let mut dialog = ImportSingleKeyDialog::new(Network::Testnet);
        dialog.wif_input.set_text(known_testnet_wif().to_string());
        dialog.alias_input = "primary".to_string();
        dialog.recompute_preview();
        assert!(dialog.derived_address.is_some());

        dialog.reset();
        assert_eq!(dialog.wif_input.text(), "");
        assert!(dialog.alias_input.is_empty());
        assert!(dialog.derived_address.is_none());
        assert!(dialog.error_message.is_none());
    }
}
