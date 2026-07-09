//! Masternode/evonode load form (FR-4).
//!
//! A dedicated, masternode-only load flow — carved out of the generic
//! `add_existing_identity_screen` Advanced-Options path (whose Masternode/
//! Evonode options are removed in the same change, §10.2). The form collects a
//! ProTxHash, a Masternode/Evonode type toggle, an optional alias, the three
//! optional V/O/P private keys, and an optional at-load encryption password
//! (FR-8). There is **no auto-derive affordance** — masternode keys are Core-
//! side keys never present in a wallet's HD tree (US-6 retired, §Locked-#4).
//!
//! Validation delegates to the model: [`is_valid_pro_tx_hash`] drives the inline
//! on-blur shape check; the backend load task performs the authoritative
//! existence and duplicate checks and surfaces a typed error.

use crate::backend_task::identity::IdentityInputToLoad;
use crate::model::masternode_input::is_valid_pro_tx_hash;
use crate::model::qualified_identity::IdentityType;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use eframe::egui::{self, RichText, Ui};

const KEY_PLACEHOLDER: &str = "Private key (WIF or hex)";
const WARNING_NOTE: &str = "Set an optional password to encrypt these keys on this device. \
     Without one, they are stored unencrypted and you can add protection later from the key \
     screen.";
const PRO_TX_HASH_FORMAT_ERROR: &str = "This doesn't look like a valid ProTxHash. Enter a hex or Base58 ProTxHash from your \
     masternode configuration.";
const LOAD_DISABLED_TOOLTIP: &str = "Enter a ProTxHash to continue.";

/// Outcome of rendering the load form for one frame.
pub enum LoadFormOutcome {
    /// No terminal interaction this frame.
    None,
    /// The user cancelled — discard the form, return to the list.
    Cancel,
    /// The user submitted a valid load request.
    Submit(Box<IdentityInputToLoad>),
}

/// Masternode/evonode load form state. Rendered by the Masternodes screen when
/// in the load view; holds its own field state and is dropped on cancel/submit
/// so reopening always yields a fresh form (TC-FR4-20).
pub struct MasternodeLoadForm {
    node_type: IdentityType,
    pro_tx_hash_input: String,
    /// Set once the ProTxHash field has lost focus, gating the inline error so
    /// it never flashes while the user is still typing (on-blur semantics).
    pro_tx_hash_touched: bool,
    alias_input: String,
    voting_key: PasswordInput,
    owner_key: PasswordInput,
    payout_key: PasswordInput,
    encryption_password: PasswordInput,
}

impl Default for MasternodeLoadForm {
    fn default() -> Self {
        Self::new()
    }
}

impl MasternodeLoadForm {
    pub fn new() -> Self {
        Self {
            node_type: IdentityType::Masternode,
            pro_tx_hash_input: String::new(),
            pro_tx_hash_touched: false,
            alias_input: String::new(),
            voting_key: PasswordInput::new()
                .with_hint_text(KEY_PLACEHOLDER)
                .with_monospace(),
            owner_key: PasswordInput::new()
                .with_hint_text(KEY_PLACEHOLDER)
                .with_monospace(),
            payout_key: PasswordInput::new()
                .with_hint_text(KEY_PLACEHOLDER)
                .with_monospace(),
            encryption_password: PasswordInput::new()
                .with_hint_text("Password to encrypt these keys"),
        }
    }

    /// The active node type — exposed for the B6 Fill-Random label + tests.
    pub fn node_type(&self) -> IdentityType {
        self.node_type
    }

    /// Switch the node type, clearing every field when it actually changes: a
    /// real node's identity is tied to one type, so autofilled or entered data
    /// for one is never valid for the other (§10.6).
    fn set_node_type(&mut self, node_type: IdentityType) {
        if node_type == self.node_type {
            return;
        }
        self.node_type = node_type;
        self.pro_tx_hash_input.clear();
        self.pro_tx_hash_touched = false;
        self.alias_input.clear();
        self.voting_key.clear();
        self.owner_key.clear();
        self.payout_key.clear();
    }

    /// Whether the Load button is enabled: a non-empty ProTxHash. Shape and
    /// existence are enforced inline / by the backend respectively — an empty
    /// field is the only hard gate on submission (TC-FR4-05/07).
    fn can_submit(&self) -> bool {
        !self.pro_tx_hash_input.trim().is_empty()
    }

    /// Build the backend load input from the current field state.
    fn build_input(&mut self) -> IdentityInputToLoad {
        let password = self.encryption_password.take_secret();
        let encryption_password = (!password.is_blank()).then_some(password);
        IdentityInputToLoad {
            identity_id_input: self.pro_tx_hash_input.trim().to_string(),
            identity_type: self.node_type,
            alias_input: self.alias_input.trim().to_string(),
            voting_private_key_input: self.voting_key.take_secret(),
            owner_private_key_input: self.owner_key.take_secret(),
            payout_address_private_key_input: self.payout_key.take_secret(),
            keys_input: vec![],
            // No auto-derive: masternode keys are never wallet-derived (§Locked-#4).
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password,
        }
    }

    pub fn show(&mut self, ui: &mut Ui) -> LoadFormOutcome {
        let dark_mode = ui.style().visuals.dark_mode;
        let mut outcome = LoadFormOutcome::None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.label(
                RichText::new("Load a masternode")
                    .size(20.0)
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Load a masternode or evonode that already exists on the Dash network.",
                )
                .size(13.0)
                .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(16.0);

            // Node-type toggle (Masternode / Evonode) — no User option.
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(self.node_type == IdentityType::Masternode, "Masternode")
                    .clicked()
                {
                    self.set_node_type(IdentityType::Masternode);
                }
                if ui
                    .selectable_label(self.node_type == IdentityType::Evonode, "Evonode")
                    .clicked()
                {
                    self.set_node_type(IdentityType::Evonode);
                }
            });
            ui.add_space(12.0);

            // ProTxHash (required) with inline on-blur shape validation.
            ui.label(
                RichText::new("ProTxHash")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.pro_tx_hash_input)
                    .hint_text("Enter the node's ProTxHash. You can find it in your masternode configuration.")
                    .desired_width(f32::INFINITY),
            );
            if response.lost_focus() {
                self.pro_tx_hash_touched = true;
            }
            if self.pro_tx_hash_touched
                && !self.pro_tx_hash_input.trim().is_empty()
                && !is_valid_pro_tx_hash(&self.pro_tx_hash_input)
            {
                ui.label(
                    RichText::new(PRO_TX_HASH_FORMAT_ERROR)
                        .size(12.0)
                        .color(DashColors::error_color(dark_mode)),
                );
            }
            ui.add_space(12.0);

            // Alias (optional, local-only).
            ui.label(
                RichText::new("Alias (optional)")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.alias_input)
                    .hint_text(
                        "An alias helps you recognize this node inside Dash Evo Tool. It is not \
                         saved to the Dash network.",
                    )
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(12.0);

            // Optional V/O/P key inputs (WIF or hex, hold-to-reveal).
            ui.label(
                RichText::new("Voting private key")
                    .color(DashColors::text_primary(dark_mode)),
            );
            self.voting_key.show(ui);
            ui.add_space(8.0);
            ui.label(
                RichText::new("Owner private key")
                    .color(DashColors::text_primary(dark_mode)),
            );
            self.owner_key.show(ui);
            ui.add_space(8.0);
            ui.label(
                RichText::new("Payout address private key")
                    .color(DashColors::text_primary(dark_mode)),
            );
            self.payout_key.show(ui);
            ui.add_space(12.0);

            // Optional at-load encryption password (FR-8).
            ui.label(
                RichText::new("Encryption password (optional)")
                    .color(DashColors::text_primary(dark_mode)),
            );
            self.encryption_password.show(ui);
            ui.label(
                RichText::new(
                    "Set a password to encrypt these keys on this device. Leave it blank to \
                     store them unencrypted and add protection later.",
                )
                .size(12.0)
                .color(DashColors::text_secondary(dark_mode)),
            );
            ui.add_space(12.0);

            // Always-visible Warning-tone key-storage note (NFR-4).
            ui.label(
                RichText::new(WARNING_NOTE)
                    .size(12.0)
                    .color(DashColors::warning_color(dark_mode)),
            );
            ui.add_space(16.0);

            // Actions: Cancel + Load. Load disabled until a ProTxHash is present.
            ui.horizontal(|ui| {
                if ComponentStyles::add_toolbar_button(
                    ui,
                    "Cancel",
                    DashColors::surface_elevated(dark_mode),
                )
                .clicked()
                {
                    outcome = LoadFormOutcome::Cancel;
                }

                let enabled = self.can_submit();
                let clicked = ComponentStyles::add_primary_button_enabled(
                    ui,
                    enabled,
                    "Load masternode",
                )
                .disabled_tooltip(LOAD_DISABLED_TOOLTIP)
                .clicked();
                if clicked && enabled {
                    outcome = LoadFormOutcome::Submit(Box::new(self.build_input()));
                }
            });
        });

        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tc_fr4_02_defaults_to_masternode() {
        assert_eq!(
            MasternodeLoadForm::new().node_type(),
            IdentityType::Masternode
        );
    }

    #[test]
    fn tc_fr4_05_07_submit_gated_on_non_empty_pro_tx_hash() {
        let mut form = MasternodeLoadForm::new();
        assert!(!form.can_submit(), "empty ProTxHash must disable submit");
        form.pro_tx_hash_input = "   ".to_string();
        assert!(!form.can_submit(), "whitespace-only must disable submit");
        form.pro_tx_hash_input = "abc".to_string();
        assert!(form.can_submit(), "any non-empty ProTxHash enables submit");
    }

    #[test]
    fn tc_edge_type_toggle_clears_fields() {
        let mut form = MasternodeLoadForm::new();
        form.pro_tx_hash_input = "deadbeef".to_string();
        form.pro_tx_hash_touched = true;
        form.alias_input = "mn-east-01".to_string();
        form.voting_key.set_text("wif");

        form.set_node_type(IdentityType::Evonode);

        assert_eq!(form.node_type(), IdentityType::Evonode);
        assert!(form.pro_tx_hash_input.is_empty());
        assert!(!form.pro_tx_hash_touched);
        assert!(form.alias_input.is_empty());
        assert!(form.voting_key.is_empty());
    }

    #[test]
    fn selecting_same_type_preserves_fields() {
        let mut form = MasternodeLoadForm::new();
        form.pro_tx_hash_input = "deadbeef".to_string();
        form.set_node_type(IdentityType::Masternode);
        assert_eq!(form.pro_tx_hash_input, "deadbeef");
    }

    #[test]
    fn tc_fr4_build_input_maps_fields_and_omits_blank_password() {
        let mut form = MasternodeLoadForm::new();
        form.pro_tx_hash_input = "  deadbeef  ".to_string();
        form.alias_input = " mn-east-01 ".to_string();
        form.set_node_type(IdentityType::Masternode);
        form.pro_tx_hash_input = "  deadbeef  ".to_string();
        form.alias_input = " mn-east-01 ".to_string();

        let input = form.build_input();
        assert_eq!(input.identity_id_input, "deadbeef");
        assert_eq!(input.alias_input, "mn-east-01");
        assert_eq!(input.identity_type, IdentityType::Masternode);
        assert!(
            !input.derive_keys_from_wallets,
            "never auto-derive (§Locked-#4)"
        );
        assert!(input.selected_wallet_seed_hash.is_none());
        assert!(input.keys_input.is_empty());
        assert!(
            input.encryption_password.is_none(),
            "a blank password must map to None (Tier-1 keyless)"
        );
    }

    #[test]
    fn build_input_keeps_non_blank_password() {
        let mut form = MasternodeLoadForm::new();
        form.pro_tx_hash_input = "deadbeef".to_string();
        form.encryption_password.set_text("hunter2");
        let input = form.build_input();
        assert!(input.encryption_password.is_some());
    }
}
