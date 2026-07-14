//! Masternode/evonode load form (FR-4) — ProTxHash, type toggle, alias, the
//! three optional V/O/P keys, and an optional at-load encryption password
//! (FR-8). No auto-derive: masternode keys never live in a wallet's HD tree.

use bip39::rand::prelude::IteratorRandom;
use bip39::rand::thread_rng;

use crate::backend_task::identity::{IdentityInputToLoad, IdentityLoadMode};
use crate::model::masternode_input::is_valid_pro_tx_hash;
use crate::model::qualified_identity::IdentityType;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::masternodes::testnet_fixture::TestnetNodes;
use crate::ui::masternodes::{TIP_OWNER_KEY, TIP_PAYOUT_KEY, TIP_VOTING_KEY};
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
    /// Testnet-only Fill-Random fixture (FR-12). `None` off Testnet or when the
    /// `.testnet_nodes.yml` file is missing/malformed — the button never shows.
    testnet_nodes: Option<TestnetNodes>,
}

impl Default for MasternodeLoadForm {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl MasternodeLoadForm {
    /// Seed the ProTxHash field so a test can point the form at a specific node.
    pub(crate) fn set_pro_tx_hash_for_test(&mut self, value: impl Into<String>) {
        self.pro_tx_hash_input = value.into();
    }

    /// The outcome a Load click produces for the current field state, so a test
    /// can drive the real submit path without an egui frame.
    pub(crate) fn submit_for_test(&self) -> LoadFormOutcome {
        LoadFormOutcome::Submit(Box::new(self.build_input()))
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
            testnet_nodes: None,
        }
    }

    /// Attach the Testnet Fill-Random fixture (FR-12). Pass `None` off Testnet.
    pub fn with_testnet_fixture(mut self, testnet_nodes: Option<TestnetNodes>) -> Self {
        self.testnet_nodes = testnet_nodes;
        self
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

    /// The Fill-Random button label, following the node-type toggle (FR-12).
    fn fill_random_label(&self) -> &'static str {
        match self.node_type {
            IdentityType::Evonode => "🎲 Fill Random Evonode",
            _ => "🎲 Fill Random Masternode",
        }
    }

    /// Populate the form from a random fixture entry matching the current node
    /// type (TC-FR12-07/08): Masternode pulls from `masternodes` (Voting + Owner
    /// only — the fixture has no payout key); Evonode pulls from `hp_masternodes`
    /// (all three keys). A no-op when the fixture is absent or the list is empty.
    fn fill_random(&mut self) {
        let Some(nodes) = self.testnet_nodes.as_ref() else {
            return;
        };
        match self.node_type {
            IdentityType::Evonode => {
                if let Some((name, node)) = nodes.hp_masternodes.iter().choose(&mut thread_rng()) {
                    self.pro_tx_hash_input = node.protx_tx_hash.clone();
                    self.alias_input = name.clone();
                    self.voting_key
                        .set_text(node.voter.private_key.expose_secret());
                    self.owner_key
                        .set_text(node.owner.private_key.expose_secret());
                    self.payout_key
                        .set_text(node.payout.private_key.expose_secret());
                    self.pro_tx_hash_touched = false;
                }
            }
            _ => {
                if let Some((name, node)) = nodes.masternodes.iter().choose(&mut thread_rng()) {
                    self.pro_tx_hash_input = node.pro_tx_hash.clone();
                    self.alias_input = name.clone();
                    self.voting_key
                        .set_text(node.voter.private_key.expose_secret());
                    self.owner_key
                        .set_text(node.owner.private_key.expose_secret());
                    // MasternodeInfo has no payout key — leave Payout blank.
                    self.pro_tx_hash_touched = false;
                }
            }
        }
    }

    /// Whether the Load button is enabled: a non-empty ProTxHash. Shape and
    /// existence are enforced inline / by the backend respectively — an empty
    /// field is the only hard gate on submission (TC-FR4-05/07).
    fn can_submit(&self) -> bool {
        !self.pro_tx_hash_input.trim().is_empty()
    }

    /// Build the backend load input from the current field state. Clones the
    /// secret fields rather than draining them, so the form keeps every value
    /// if the load fails and the user needs to fix one field and resubmit. The
    /// form's own copies are zeroized when it is dropped on a successful load.
    fn build_input(&self) -> IdentityInputToLoad {
        let password = self.encryption_password.secret().clone();
        let encryption_password = (!password.is_blank()).then_some(password);
        IdentityInputToLoad {
            identity_id_input: self.pro_tx_hash_input.trim().to_string(),
            identity_type: self.node_type,
            alias_input: self.alias_input.trim().to_string(),
            voting_private_key_input: self.voting_key.secret().clone(),
            owner_private_key_input: self.owner_key.secret().clone(),
            payout_address_private_key_input: self.payout_key.secret().clone(),
            keys_input: vec![],
            // No auto-derive: masternode keys are never wallet-derived (§Locked-#4).
            derive_keys_from_wallets: false,
            selected_wallet_seed_hash: None,
            encryption_password,
            // A fresh load rejects an already-loaded ProTxHash (§10.9) rather
            // than silently overwriting the existing node.
            load_mode: IdentityLoadMode::RejectIfExists,
            // Stamped by the screen at dispatch: only it knows the record this
            // submission opened, and only a parseable ProTxHash gets one.
            load_token: None,
        }
    }

    /// Render the form. `dev_mode` gates the Testnet-only Fill-Random button as
    /// a defense-in-depth check (FR-12 / TC-FR12-09): the whole tab is already
    /// Expert-gated, but a plaintext-key-reading dev tool stays inside the
    /// Expert-Mode envelope regardless of any future non-nav entry point.
    ///
    /// `submitting` is `true` while a load dispatched from this form is in
    /// flight: the submit button locks and shows a spinner so the load cannot be
    /// re-submitted, and the form emits no further `Submit` until the result
    /// arrives. On error the caller re-enables the form with fields intact.
    pub fn show(&mut self, ui: &mut Ui, dev_mode: bool, submitting: bool) -> LoadFormOutcome {
        let dark_mode = ui.style().visuals.dark_mode;
        let mut outcome = LoadFormOutcome::None;

        // Back row (content-panel, not the global header) — matches the detail
        // view's `‹ All masternodes` link so both views return to the card list
        // the same way. Emits `Cancel`, the existing "return to list" outcome.
        if ui
            .selectable_label(false, RichText::new("‹ All masternodes"))
            .clicked()
        {
            outcome = LoadFormOutcome::Cancel;
        }
        ui.separator();

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

            // Fill-Random dev convenience (FR-12): Testnet + fixture present +
            // Expert Mode. Entire row is absent otherwise — never shown-disabled.
            if dev_mode && self.testnet_nodes.is_some() {
                if ui.button(self.fill_random_label()).clicked() {
                    self.fill_random();
                }
                ui.label(
                    RichText::new(
                        "Testnet-only dev convenience — visible only when a local test-node \
                         fixture is found.",
                    )
                    .size(12.0)
                    .color(DashColors::text_secondary(dark_mode)),
                );
                ui.add_space(12.0);
            }

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

            // Optional V/O/P key inputs (WIF or hex, hold-to-reveal). Labels and
            // tooltips follow the Dash Core DIP-3 ProRegTx key roles.
            ui.label(
                RichText::new("Voting private key")
                    .color(DashColors::text_primary(dark_mode)),
            )
            .info_tooltip(TIP_VOTING_KEY);
            self.voting_key.show(ui);
            ui.add_space(8.0);
            ui.label(
                RichText::new("Owner private key")
                    .color(DashColors::text_primary(dark_mode)),
            )
            .info_tooltip(TIP_OWNER_KEY);
            self.owner_key.show(ui);
            ui.add_space(8.0);
            ui.label(
                RichText::new("Payout address private key")
                    .color(DashColors::text_primary(dark_mode)),
            )
            .info_tooltip(TIP_PAYOUT_KEY);
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

                if submitting {
                    // A load is in flight: lock the button and show progress so
                    // the user cannot re-submit and knows the load is running.
                    ui.add_enabled(false, egui::Button::new("Loading…"));
                    ui.spinner();
                } else {
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
                }
            });
        });

        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::secret::Secret;
    use crate::ui::masternodes::testnet_fixture::{HpMasternodeInfo, KeyInfo, MasternodeInfo};
    use std::collections::HashMap;

    fn fixture() -> TestnetNodes {
        let mut masternodes = HashMap::new();
        masternodes.insert(
            "mn-fixture".to_string(),
            MasternodeInfo {
                pro_tx_hash: "aa".repeat(32),
                owner: KeyInfo {
                    private_key: Secret::new("owner-wif"),
                },
                voter: KeyInfo {
                    private_key: Secret::new("voter-wif"),
                },
            },
        );
        let mut hp_masternodes = HashMap::new();
        hp_masternodes.insert(
            "evo-fixture".to_string(),
            HpMasternodeInfo {
                protx_tx_hash: "bb".repeat(32),
                owner: KeyInfo {
                    private_key: Secret::new("hp-owner-wif"),
                },
                voter: KeyInfo {
                    private_key: Secret::new("hp-voter-wif"),
                },
                payout: KeyInfo {
                    private_key: Secret::new("hp-payout-wif"),
                },
            },
        );
        TestnetNodes {
            masternodes,
            hp_masternodes,
        }
    }

    #[test]
    fn tc_fr12_01_02_label_follows_node_type() {
        let mut form = MasternodeLoadForm::new();
        assert_eq!(form.fill_random_label(), "🎲 Fill Random Masternode");
        form.set_node_type(IdentityType::Evonode);
        assert_eq!(form.fill_random_label(), "🎲 Fill Random Evonode");
    }

    #[test]
    fn tc_fr12_07_masternode_fill_populates_voting_and_owner_only() {
        let mut form = MasternodeLoadForm::new().with_testnet_fixture(Some(fixture()));
        form.fill_random();
        assert_eq!(form.pro_tx_hash_input, "aa".repeat(32));
        assert_eq!(form.alias_input, "mn-fixture");
        assert_eq!(form.voting_key.text(), "voter-wif");
        assert_eq!(form.owner_key.text(), "owner-wif");
        // MasternodeInfo has no payout key — Payout stays blank.
        assert!(form.payout_key.is_empty());
    }

    #[test]
    fn tc_fr12_08_evonode_fill_populates_all_three_keys() {
        let mut form = MasternodeLoadForm::new().with_testnet_fixture(Some(fixture()));
        form.set_node_type(IdentityType::Evonode);
        form.fill_random();
        assert_eq!(form.pro_tx_hash_input, "bb".repeat(32));
        assert_eq!(form.alias_input, "evo-fixture");
        assert_eq!(form.voting_key.text(), "hp-voter-wif");
        assert_eq!(form.owner_key.text(), "hp-owner-wif");
        assert_eq!(form.payout_key.text(), "hp-payout-wif");
    }

    #[test]
    fn fill_random_without_fixture_is_a_noop() {
        let mut form = MasternodeLoadForm::new();
        form.fill_random();
        assert!(form.pro_tx_hash_input.is_empty());
    }

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

    /// Building the load input must NOT drain the form's fields: if the backend
    /// load fails, the form stays open and the user corrects one field and
    /// resubmits — every value (ProTxHash, alias, keys, password) must survive.
    #[test]
    fn build_input_preserves_fields_for_error_retry() {
        let mut form = MasternodeLoadForm::new();
        form.pro_tx_hash_input = "deadbeef".to_string();
        form.alias_input = "mn-east-01".to_string();
        form.voting_key.set_text("voter-wif");
        form.owner_key.set_text("owner-wif");
        form.payout_key.set_text("payout-wif");
        form.encryption_password.set_text("hunter2");

        let input = form.build_input();

        // The built input carries the values through to the backend task.
        assert_eq!(input.voting_private_key_input.expose_secret(), "voter-wif");
        assert_eq!(input.owner_private_key_input.expose_secret(), "owner-wif");
        assert_eq!(
            input.payout_address_private_key_input.expose_secret(),
            "payout-wif"
        );

        // The form retains every field so a failed load is corrected in place.
        assert_eq!(form.pro_tx_hash_input, "deadbeef");
        assert_eq!(form.alias_input, "mn-east-01");
        assert_eq!(form.voting_key.text(), "voter-wif");
        assert_eq!(form.owner_key.text(), "owner-wif");
        assert_eq!(form.payout_key.text(), "payout-wif");
        assert_eq!(form.encryption_password.text(), "hunter2");
    }
}
