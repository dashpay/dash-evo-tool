use crate::app::AppAction;
use crate::backend_task::error::TaskError;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::legacy_recovery::RecoveryItem;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::{PrivateKeyTarget, QualifiedIdentity};
use crate::model::secret::Secret;
use crate::model::wallet::Wallet;
use crate::model::wallet::passphrase::validate_single_key_passphrase;
use crate::ui::components::MessageBanner;
use crate::ui::components::component_trait::Component;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::legacy_recovery_section::host_offer;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::components::styled::{ConfirmationDialog, ConfirmationStatus, island_central_panel};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::masternodes::{KeyVocabulary, key_role_label};
use crate::ui::state::legacy_recovery::LegacyRecoveryState;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike};
use crate::wallet_backend::IdentityKeyView;
use crate::wallet_backend::poison::RwLockRecover;
use crate::wallet_backend::secret_seam::SecretScheme;
use dash_sdk::dashcore_rpc::dashcore::PrivateKey as RPCPrivateKey;
use dash_sdk::dpp::dashcore::address::Payload;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1, SecretKey};
use dash_sdk::dpp::dashcore::sign_message::{MessageSignature, signed_msg_hash};
use dash_sdk::dpp::dashcore::{Address, PrivateKey, PubkeyHash, ScriptHash};
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::identity::KeyType::BIP13_SCRIPT_HASH;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::IdentityPublicKey;
use eframe::egui::{self};
use egui::{Color32, RichText, ScrollArea};
use std::sync::{Arc, RwLock};
use zxcvbn::zxcvbn;

pub struct KeyInfoScreen {
    pub identity: QualifiedIdentity,
    pub key: IdentityPublicKey,
    pub private_key_data: Option<(PrivateKeyData, Option<WalletDerivationPath>)>,
    pub decrypted_private_key: Option<RPCPrivateKey>,
    pub app_context: Arc<AppContext>,
    private_key_input: PasswordInput,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    wallet_open_attempted: bool,
    message_input: String,
    signed_message: Option<String>,
    view_wallet_unlock: bool,
    wallet_open: bool,
    view_private_key_even_if_encrypted_or_in_wallet: bool,
    show_pop_up_info: Option<String>,
    remove_private_key_dialog: Option<ConfirmationDialog>,
    /// A queued "derive private key for display" request for a wallet-derived
    /// key. Drained at the end of `ui()` into a `WalletTask::DeriveKeyForDisplay`
    /// backend task — the seed is fetched just-in-time and only the WIF returns.
    pending_key_display_request: Option<DerivationPath>,
    /// `true` once a display derivation has been dispatched, so the same
    /// request is not re-queued every frame while the result is in flight.
    key_display_requested: bool,
    /// A queued "sign message" request for a wallet-derived key. Drained at the
    /// end of `ui()` into a `WalletTask::SignMessageWithKey` backend task — the
    /// seed is fetched just-in-time and only the public signature returns.
    pending_sign_request: Option<DerivationPath>,
    /// A queued "derive for display" request for a vault-backed (`InVault`)
    /// identity key. Drained into `WalletTask::DeriveIdentityKeyForDisplay`.
    pending_identity_key_display: bool,
    /// A queued "sign message" request for a vault-backed identity key. Drained
    /// into `WalletTask::SignMessageWithIdentityKey`.
    pending_identity_sign: bool,
    /// Identity key password protection: cached at-rest protection status of
    /// this identity's vault keys. `None` until first probed; invalidated
    /// after a migration so the status line re-reads the vault.
    protection_status: Option<IdentityProtectionStatus>,
    /// Which step of the opt-in / opt-out flow is active.
    protection_stage: ProtectionStage,
    /// The danger confirmation dialog gating the active flow.
    protection_confirm: Option<ConfirmationDialog>,
    /// Opt-in password entry (new password + confirmation + hint).
    protection_new_password: PasswordInput,
    protection_confirm_password: PasswordInput,
    protection_hint: String,
    /// Opt-out password entry (verify the current password).
    protection_verify_password: PasswordInput,
    /// Inline validation error for the protection password form.
    protection_form_error: Option<String>,
    /// True while a Protect/Unprotect task is in flight (disables the
    /// action button so the same migration is not dispatched twice).
    protection_in_flight: bool,
    /// A queued opt-in dispatch (password + hint), drained in `ui()`.
    pending_protect: Option<(Secret, Option<String>)>,
    /// A queued opt-out dispatch (current password), drained in `ui()`.
    pending_unprotect: Option<Secret>,
    /// The offer to restore keys this identity left behind in the previous
    /// version's saved data (issue #889). Scoped to the identity, not to the
    /// key on screen, so it appears wherever a key of a partially-restored
    /// identity is opened.
    recovery: LegacyRecoveryState,
    /// A queued restore (the approved items), drained in `ui()`.
    pending_recovery_restore: Option<Vec<RecoveryItem>>,
    /// The screen this key was opened from, for the breadcrumb back to it.
    parent: Option<&'static str>,
}

/// At-rest protection posture of an identity's vault-stored keys.
#[derive(Clone, Copy, PartialEq, Eq)]
enum IdentityProtectionStatus {
    /// No keys live in the identity vault (e.g. only wallet-derived keys); the
    /// per-identity protection control does not apply.
    NoVaultKeys,
    /// Every vault key is keyless (Tier-1) — signs prompt-free (the default).
    Unprotected,
    /// Every vault key is password-protected (Tier-2).
    Protected,
    /// A partial state (some protected, some not) — typically a crash mid
    /// migration. The UI offers "Finish protecting".
    Mixed,
}

/// Which step of the Key Protection opt-in / opt-out flow is on screen.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum ProtectionStage {
    /// Status line + action button only.
    #[default]
    Idle,
    /// The danger warning before opt-in is showing.
    ConfirmAdd,
    /// The new-password form (opt-in) is showing.
    EnterNewPassword,
    /// The danger warning before opt-out is showing.
    ConfirmRemove,
    /// The verify-password form (opt-out) is showing.
    EnterVerifyPassword,
}

impl ScreenLike for KeyInfoScreen {
    /// Re-read the record this screen persists, because another writer may have
    /// changed it while the screen sat in the stack.
    ///
    /// A restore run from the node page — or any backend task whose result
    /// reached whichever screen was visible at the time — writes this identity
    /// behind this screen's back. Its key add and remove paths persist the whole
    /// clone taken when it opened, so a clone that missed a write puts the
    /// pre-write record back on the next key edit. The masternode detail view
    /// re-reads on arrival for the same reason.
    ///
    /// This is the hook that has to carry it: `AppState` dispatches
    /// `refresh_on_arrival` only to root screens, and this screen is always
    /// pushed onto the screen stack. What reaches a pushed screen is `refresh` —
    /// from `TaskResult::Refresh`, from `AppAction::Refresh`, and from the
    /// `PopScreenAndRefresh` that reveals it. `refresh_on_arrival` defaults to
    /// delegating here, so both hooks run this.
    fn refresh(&mut self) {
        self.reload_identity();
        self.protection_status = None;
        self.recovery.completed();
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::WalletKeyForDisplay { wif, .. } => {
                // The backend derived the key just-in-time; reconstruct the
                // RPC private key from the WIF only to render WIF + hex. The
                // seed never crossed into the UI.
                match RPCPrivateKey::from_wif(wif.expose_secret()) {
                    Ok(private_key) => self.decrypted_private_key = Some(private_key),
                    Err(e) => {
                        self.key_display_requested = false;
                        let banner = MessageBanner::set_global(
                            self.app_context.egui_ctx(),
                            "Could not display the private key. Please retry.",
                            MessageType::Error,
                        );
                        banner.with_details(e);
                        banner.disable_auto_dismiss();
                    }
                }
            }
            BackendTaskSuccessResult::WalletMessageSigned { signature, .. } => {
                self.signed_message = Some(signature);
            }
            BackendTaskSuccessResult::IdentityKeyForDisplay { wif, .. } => {
                match RPCPrivateKey::from_wif(wif.expose_secret()) {
                    Ok(private_key) => self.decrypted_private_key = Some(private_key),
                    Err(e) => {
                        self.key_display_requested = false;
                        let banner = MessageBanner::set_global(
                            self.app_context.egui_ctx(),
                            "Could not display the private key. Please retry.",
                            MessageType::Error,
                        );
                        banner.with_details(e);
                        banner.disable_auto_dismiss();
                    }
                }
            }
            BackendTaskSuccessResult::IdentityMessageSigned { signature, .. } => {
                self.signed_message = Some(signature);
            }
            BackendTaskSuccessResult::IdentityKeysProtected { .. } => {
                self.protection_in_flight = false;
                self.protection_status = None; // re-probe the vault on next render
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "This identity's keys are now password-protected. You will be asked for the password each time they sign.",
                    MessageType::Success,
                );
            }
            BackendTaskSuccessResult::IdentityKeysUnprotected { .. } => {
                self.protection_in_flight = false;
                self.protection_status = None; // re-probe the vault on next render
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Password protection removed. This identity's keys will now sign automatically.",
                    MessageType::Success,
                );
            }
            ref result => {
                // The offer attributes the result, re-arms itself and reports
                // the outcome; what is left is this screen's own. The clone it
                // persists on every key edit is now stale — writing it back
                // would erase the keys just restored — and restored keys land
                // in the vault, so the protection line has to re-read it.
                if self
                    .recovery
                    .absorb_result(self.app_context.egui_ctx(), result)
                {
                    self.reload_identity();
                    self.protection_status = None;
                }
            }
        }
    }

    fn display_message(&mut self, _message: &str, message_type: MessageType) {
        // A migration that failed surfaces as an error banner (set centrally by
        // AppState); clear the in-flight gate so the user can retry, and
        // re-probe the vault in case a partial change landed.
        if self.protection_in_flight && matches!(message_type, MessageType::Error) {
            self.protection_in_flight = false;
            self.protection_status = None;
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        self.recovery.absorb_error(context);
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = add_top_panel(ui, &self.app_context, self.breadcrumb(), vec![]);

        action |= add_left_panel(
            ui,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentities,
        );

        action |= island_central_panel(ui, |ui| {
            let inner_action = AppAction::None;

            ScrollArea::vertical().show(ui, |ui| {
                let text_primary = DashColors::text_primary(ui.style().visuals.dark_mode);
                ui.heading(RichText::new("Key Information").color(text_primary));
                ui.add_space(10.0);

                egui::Grid::new("key_info_grid")
                    .num_columns(2)
                    .spacing([10.0, 10.0])
                    .striped(false)
                    .show(ui, |ui| {
                        // Key ID
                        ui.label(RichText::new("Key ID:").strong().color(text_primary));
                        ui.label(RichText::new(format!("{}", self.key.id())).color(text_primary));
                        ui.end_row();

                        // Purpose, in the same words the keys list and the
                        // restore offer use for this key — one key cannot be
                        // called three things across three screens.
                        ui.label(RichText::new("Purpose:").strong().color(text_primary));
                        let (role, role_tip) = key_role_label(
                            KeyVocabulary::from(self.identity.identity_type),
                            &self.naming_target(),
                            &self.key,
                        );
                        let purpose_label = ui.label(RichText::new(role).color(text_primary));
                        if let Some(tip) = role_tip {
                            purpose_label.on_hover_text(tip);
                        }
                        ui.end_row();

                        // The raw Platform purpose is Expert diagnostics, so it
                        // gets a labelled field of its own beside Security Level
                        // and Type rather than being spliced into the caption
                        // above, which has to stay one translatable phrase.
                        if self
                            .app_context
                            .user_role()
                            .at_least(crate::model::user_role::UserRole::Power)
                        {
                            ui.label(
                                RichText::new("Platform purpose:")
                                    .strong()
                                    .color(text_primary),
                            );
                            ui.label(
                                RichText::new(format!("{:?}", self.key.purpose()))
                                    .color(text_primary),
                            );
                            ui.end_row();
                        }

                        // Security Level
                        ui.label(
                            RichText::new("Security Level:")
                                .strong()
                                .color(text_primary),
                        );
                        ui.label(
                            RichText::new(format!("{:?}", self.key.security_level()))
                                .color(text_primary),
                        );
                        ui.end_row();

                        // Type
                        ui.label(RichText::new("Type:").strong().color(text_primary));
                        ui.label(
                            RichText::new(format!("{:?}", self.key.key_type())).color(text_primary),
                        );
                        ui.end_row();

                        // Read Only
                        ui.label(RichText::new("Read Only:").strong().color(text_primary));
                        ui.label(
                            RichText::new(format!("{}", self.key.read_only())).color(text_primary),
                        );
                        ui.end_row();

                        // Disabled
                        ui.label(
                            RichText::new("Active/Disabled:")
                                .strong()
                                .color(text_primary),
                        );
                        if !self.key.is_disabled() {
                            ui.label(RichText::new("Active").color(text_primary));
                        } else {
                            ui.label(RichText::new("Disabled").color(text_primary));
                        }
                        ui.end_row();

                        if let Some((_, Some(wallet_derivation_path))) =
                            self.private_key_data.as_ref()
                        {
                            // Disabled
                            ui.label(
                                RichText::new("In local Wallet")
                                    .strong()
                                    .color(text_primary),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "At derivation path {}",
                                    wallet_derivation_path.derivation_path
                                ))
                                .strong()
                                .color(text_primary),
                            );
                            ui.end_row();
                        }

                        // Contract Bounds
                        if let Some(contract_bounds) = self.key.contract_bounds() {
                            ui.label(
                                RichText::new("Contract Bounds:")
                                    .strong()
                                    .color(text_primary),
                            );
                            match contract_bounds {
                                ContractBounds::SingleContract { id } => {
                                    ui.label(
                                        RichText::new(format!("Contract ID: {}", id))
                                            .color(text_primary),
                                    );
                                }
                                ContractBounds::SingleContractDocumentType {
                                    id,
                                    document_type_name,
                                } => {
                                    ui.label(
                                        RichText::new(format!(
                                            "Contract ID: {}\nDocument Type: {}",
                                            id, document_type_name
                                        ))
                                        .color(text_primary),
                                    );
                                }
                            }
                            ui.end_row();
                        }

                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Display the public key information
                ui.heading(RichText::new("Public Key Information").color(text_primary));
                ui.add_space(10.0);

                egui::Grid::new("public_key_info_grid")
                    .num_columns(2)
                    .spacing([10.0, 10.0])
                    .striped(false)
                    .show(ui, |ui| {
                        match self.key.key_type() {
                            KeyType::ECDSA_SECP256K1 | KeyType::BLS12_381 => {
                                // Public Key Hex
                                ui.label(
                                    RichText::new("Public Key (Hex):")
                                        .strong()
                                        .color(text_primary),
                                );
                                ui.label(
                                    RichText::new(self.key.data().to_string(Encoding::Hex))
                                        .color(text_primary),
                                );
                                ui.end_row();

                                // Public Key Hex
                                ui.label(
                                    RichText::new("Public Key (Base64):")
                                        .strong()
                                        .color(text_primary),
                                );
                                ui.label(
                                    RichText::new(self.key.data().to_string(Encoding::Base64))
                                        .color(text_primary),
                                );
                                ui.end_row();
                            }
                            _ => {}
                        }

                        // Public Key Hash
                        ui.label(
                            RichText::new("Public Key Hash:")
                                .strong()
                                .color(text_primary),
                        );
                        match self.key.public_key_hash() {
                            Ok(hash) => {
                                let hash_hex = hex::encode(hash);
                                ui.label(RichText::new(hash_hex).color(text_primary));
                            }
                            Err(e) => {
                                ui.colored_label(egui::Color32::RED, format!("Error: {}", e));
                            }
                        }

                        if self.key.key_type().is_core_address_key_type() {
                            // Public Key Hash
                            ui.label(RichText::new("Address:").strong().color(text_primary));
                            match self.key.public_key_hash() {
                                Ok(hash) => {
                                    let address = if self.key.key_type() == BIP13_SCRIPT_HASH {
                                        Address::new(
                                            self.app_context.network,
                                            Payload::ScriptHash(ScriptHash::from_byte_array(hash)),
                                        )
                                    } else {
                                        Address::new(
                                            self.app_context.network,
                                            Payload::PubkeyHash(PubkeyHash::from_byte_array(hash)),
                                        )
                                    };
                                    ui.label(
                                        RichText::new(address.to_string()).color(text_primary),
                                    );
                                }
                                Err(e) => {
                                    ui.colored_label(egui::Color32::RED, format!("Error: {}", e));
                                }
                            }
                        }

                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Display the private key if available
                if let Some((private_key, _)) = self.private_key_data.as_mut() {
                    ui.heading(RichText::new("Private Key").color(text_primary));
                    ui.add_space(10.0);

                    match private_key {
                        PrivateKeyData::Clear(clear) | PrivateKeyData::AlwaysClear(clear) => {
                            egui::Grid::new("private_key_grid")
                                .num_columns(2)
                                .spacing([10.0, 10.0])
                                .show(ui, |ui| {
                                    if let Ok(secret_key) = SecretKey::from_slice(clear) {
                                        let private_key =
                                            PrivateKey::new(secret_key, self.app_context.network);
                                        ui.label(
                                            RichText::new("Private Key (WIF):")
                                                .strong()
                                                .color(ui.visuals().text_color()),
                                        );
                                        let wif = Secret::new(private_key.to_wif());
                                        // WIF displayed as plaintext label — user-initiated key view.
                                        // Secret wrapper provides zeroize-on-drop for the Rust-side variable.
                                        ui.label(
                                            RichText::new(wif.expose_secret())
                                                .color(ui.visuals().text_color()),
                                        );
                                        ui.end_row();
                                    }

                                    ui.label(
                                        RichText::new("Private Key (Hex):")
                                            .strong()
                                            .color(ui.visuals().text_color()),
                                    );
                                    let private_key_hex = Secret::new(hex::encode(clear));
                                    // WIF displayed as plaintext label — user-initiated key view.
                                    // Secret wrapper provides zeroize-on-drop for the Rust-side variable.
                                    ui.label(
                                        RichText::new(private_key_hex.expose_secret())
                                            .color(ui.visuals().text_color()),
                                    );
                                    ui.end_row();
                                });
                            ui.add_space(10.0);
                            if ui.button("Remove private key from DET").clicked() {
                                self.remove_private_key_dialog = Some(
                                    ConfirmationDialog::new(
                                        "Remove Private Key",
                                        "Are you sure you want to remove the private key?",
                                    )
                                    .confirm_text(Some("Remove"))
                                    .cancel_text(Some("Cancel"))
                                    .danger_mode(true),
                                );
                            }
                            self.render_sign_input(ui);
                        }
                        PrivateKeyData::Encrypted(_) => {
                            ui.label(RichText::new("Key is encrypted").color(text_primary));
                            ui.add_space(10.0);

                            //todo decrypt key
                        }
                        PrivateKeyData::AtWalletDerivationPath(derivation_path) => {
                            if self.wallet_open
                                && self.view_private_key_even_if_encrypted_or_in_wallet
                                && self.selected_wallet.is_some()
                            {
                                if let Some(private_key) = self.decrypted_private_key {
                                    Self::render_decrypted_key_grid(ui, &private_key);
                                } else {
                                    Self::queue_key_display(
                                        &mut self.pending_key_display_request,
                                        &mut self.key_display_requested,
                                        &derivation_path.derivation_path,
                                    );
                                    ui.label(
                                        RichText::new("Deriving private key…")
                                            .color(ui.visuals().text_color()),
                                    );
                                }
                                self.render_sign_input(ui);
                            } else if self.wallet_open {
                                ui.colored_label(Color32::DARK_RED, "Key is in encrypted wallet");
                                ui.add_space(10.0);

                                if ui.button("View Private Key").clicked() {
                                    self.view_private_key_even_if_encrypted_or_in_wallet = true;
                                    self.view_wallet_unlock = true;
                                }
                                if let Some(private_key) = self.decrypted_private_key {
                                    Self::render_decrypted_key_grid(ui, &private_key);
                                } else {
                                    Self::queue_key_display(
                                        &mut self.pending_key_display_request,
                                        &mut self.key_display_requested,
                                        &derivation_path.derivation_path,
                                    );
                                    ui.label(
                                        RichText::new("Deriving private key…")
                                            .color(ui.visuals().text_color()),
                                    );
                                }
                                self.render_sign_input(ui);
                            } else {
                                ui.colored_label(Color32::DARK_RED, "Key is in encrypted wallet");
                                ui.add_space(10.0);

                                if ui.button("View Private Key").clicked() {
                                    self.view_private_key_even_if_encrypted_or_in_wallet = true;
                                    self.view_wallet_unlock = true;
                                }

                                if ui.button("Sign Message").clicked() {
                                    self.view_wallet_unlock = true;
                                }
                            }
                        }
                        PrivateKeyData::InVault => {
                            // Vault-backed identity key: the raw bytes are
                            // fetched just-in-time by a backend task. The UI
                            // only ever sees the derived WIF for display.
                            ui.label(
                                RichText::new(
                                    "This signing key is stored securely on this device.",
                                )
                                .color(text_primary),
                            );
                            ui.add_space(10.0);
                            if let Some(private_key) = self.decrypted_private_key {
                                Self::render_decrypted_key_grid(ui, &private_key);
                            } else if ui.button("View Private Key").clicked() {
                                self.pending_identity_key_display = true;
                                self.key_display_requested = true;
                            }
                            self.render_sign_input(ui);
                            ui.add_space(10.0);
                            ui.separator();
                            self.render_key_protection_section(ui);
                        }
                    }
                } else {
                    ui.label(RichText::new("Enter Private Key:").color(text_primary));
                    self.private_key_input.show(ui);

                    if ui.button("Add Private Key").clicked() {
                        self.validate_and_store_private_key();
                    }
                    // Error display is handled by the global MessageBanner
                }

                if self.view_wallet_unlock
                    && let Some(wallet) = &self.selected_wallet
                {
                    if !self.wallet_open_attempted {
                        if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                            MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                                .disable_auto_dismiss();
                        }
                        self.wallet_open_attempted = true;
                    }
                    if wallet_needs_unlock(wallet) {
                        ui.add_space(10.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 150, 50),
                            "Wallet is locked. Please unlock to continue.",
                        );
                        ui.add_space(8.0);
                        if ui.button("Unlock Wallet").clicked() {
                            self.wallet_unlock_popup.open();
                        }
                    } else {
                        self.wallet_open = true;
                    }
                }

                // Show the remove private key confirmation popup
                if self.remove_private_key_dialog.is_some() {
                    self.show_remove_private_key_dialog(ui);
                }

                // Identity-scoped, so it renders whatever this key's own state
                // is: the keys it offers are precisely the ones this identity
                // does not hold.
                self.render_recovery_section(ui);

                ui.add_space(10.0);
            });

            inner_action
        });

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully
            }
        }

        // Show the popup window if `show_popup` is true
        if let Some(show_pop_up_info_text) = self.show_pop_up_info.clone() {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    let mut popup = InfoPopup::new(
                        egui::Id::new("identity_key_sign_message_info_popup"),
                        "Sign Message Info",
                        &show_pop_up_info_text,
                    );
                    if popup.show(ui).inner {
                        self.show_pop_up_info = None;
                    }
                });
        }

        // Drain queued wallet-key requests into backend tasks that fetch the
        // seed just-in-time and derive/sign off the UI thread. Only the public
        // result (WIF for display, signature) returns to the UI.
        if let Some(seed_hash) = self.wallet_seed_hash() {
            if let Some(derivation_path) = self.pending_key_display_request.take() {
                action |= AppAction::BackendTask(BackendTask::WalletTask(
                    WalletTask::DeriveKeyForDisplay {
                        seed_hash,
                        derivation_path,
                    },
                ));
            }
            if let Some(derivation_path) = self.pending_sign_request.take() {
                action |= AppAction::BackendTask(BackendTask::WalletTask(
                    WalletTask::SignMessageWithKey {
                        seed_hash,
                        derivation_path,
                        message: self.message_input.clone(),
                        key_type: self.key.key_type(),
                    },
                ));
            }
        }

        // Vault-backed (InVault) identity-key requests: the raw key is fetched
        // JIT in the backend and only the public WIF / signature returns.
        let identity_id = self.identity.identity.id();
        let key_id = self.key.id();
        let wants_display = std::mem::take(&mut self.pending_identity_key_display);
        let wants_sign = std::mem::take(&mut self.pending_identity_sign);
        if wants_display || wants_sign {
            // The vault stores each key under the store it is filed in, so the
            // request has to name the placement the material is actually at.
            match self.target() {
                Some(target) => {
                    if wants_display {
                        action |= AppAction::BackendTask(BackendTask::WalletTask(
                            WalletTask::DeriveIdentityKeyForDisplay {
                                identity_id,
                                target: target.clone(),
                                key_id,
                            },
                        ));
                    }
                    if wants_sign {
                        action |= AppAction::BackendTask(BackendTask::WalletTask(
                            WalletTask::SignMessageWithIdentityKey {
                                identity_id,
                                target,
                                key_id,
                                message: self.message_input.clone(),
                                key_type: self.key.key_type(),
                            },
                        ));
                    }
                }
                None => {
                    MessageBanner::set_global(
                        ctx,
                        "This key is not saved on this device, so it cannot be shown or used to sign.",
                        MessageType::Error,
                    );
                }
            }
        }

        // Drain a queued identity-key protection opt-in / opt-out.
        if let Some((password, hint)) = self.pending_protect.take() {
            MessageBanner::set_global(
                ctx,
                "Protecting this identity's keys. Please wait.",
                MessageType::Info,
            );
            action |= AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::ProtectIdentityKeys {
                    identity_id,
                    password,
                    hint,
                },
            ));
        }
        if let Some(password) = self.pending_unprotect.take() {
            MessageBanner::set_global(
                ctx,
                "Removing password protection. Please wait.",
                MessageType::Info,
            );
            action |= AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::UnprotectIdentityKeys {
                    identity_id,
                    password,
                },
            ));
        }

        // Legacy recovery: the passive check goes out once per opened screen,
        // and a restore only after the user pressed Restore — so the two can
        // never contend for `action`, which keeps only its most recent value.
        if let Some(task) = self.recovery.ensure_checked() {
            action |= AppAction::BackendTask(task);
        }
        if let Some(approved) = self.pending_recovery_restore.take()
            && let Some(task) = self.recovery.restore(approved)
        {
            action |= AppAction::BackendTask(task);
        }

        action
    }
}

impl KeyInfoScreen {
    pub fn new(
        identity: QualifiedIdentity,
        key: IdentityPublicKey,
        private_key_data: Option<(PrivateKeyData, Option<WalletDerivationPath>)>,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let selected_wallet =
            if let Some((_, Some(wallet_derivation_path))) = private_key_data.as_ref() {
                let wallets = app_context.wallets.read_recover();
                wallets
                    .get(&wallet_derivation_path.wallet_seed_hash)
                    .cloned()
            } else {
                None
            };
        let recovery = LegacyRecoveryState::new(app_context, identity.identity.id());
        Self {
            identity,
            key,
            private_key_data,
            decrypted_private_key: None,
            app_context: app_context.clone(),
            private_key_input: PasswordInput::new()
                .with_hint_text("Private key (WIF or hex)")
                .with_monospace(),
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_open_attempted: false,
            message_input: "".to_string(),
            signed_message: None,
            view_wallet_unlock: false,
            wallet_open: false,
            view_private_key_even_if_encrypted_or_in_wallet: false,
            show_pop_up_info: None,
            remove_private_key_dialog: None,
            pending_key_display_request: None,
            key_display_requested: false,
            pending_sign_request: None,
            pending_identity_key_display: false,
            pending_identity_sign: false,
            protection_status: None,
            protection_stage: ProtectionStage::Idle,
            protection_confirm: None,
            protection_new_password: PasswordInput::new().with_hint_text("New password"),
            protection_confirm_password: PasswordInput::new().with_hint_text("Confirm password"),
            protection_hint: String::new(),
            protection_verify_password: PasswordInput::new().with_hint_text("Current password"),
            protection_form_error: None,
            protection_in_flight: false,
            pending_protect: None,
            pending_unprotect: None,
            recovery,
            pending_recovery_restore: None,
            parent: None,
        }
    }

    /// The store this key is *published* under, for naming it.
    ///
    /// Deliberately the structural answer rather than [`Self::target`]'s: a key's
    /// name follows the identity list it belongs to — which is what
    /// `identity_keys` pairs it with on every list that shows it — not wherever
    /// its private half happens to be filed. Naming it from the material's
    /// location would let one key be called two things depending on which build
    /// saved it. `Unknown` names the main identity, which is where a key not yet
    /// on any list is being added.
    fn naming_target(&self) -> PrivateKeyTarget {
        self.identity
            .placement_of(&self.key)
            .resolved()
            .unwrap_or(PrivateKeyTarget::PrivateKeyOnMainIdentity)
    }

    /// Name the screen this key was opened from, so the breadcrumb can lead back
    /// to it.
    ///
    /// Unset leaves the two-level `Identities > Key Info` trail every other
    /// caller has: the screen has nine parents, and naming one of them for all
    /// of them would simply mislabel the other eight.
    pub fn with_parent(mut self, label: &'static str) -> Self {
        self.parent = Some(label);
        self
    }

    /// The breadcrumb trail, with the parent screen in it when the caller named
    /// one. The parent crumb pops back rather than clearing the stack, so the
    /// screen underneath is the one the user actually came from.
    fn breadcrumb(&self) -> Vec<(&'static str, AppAction)> {
        match self.parent {
            Some(parent) => vec![
                ("Identities", AppAction::GoToMainScreen),
                (parent, AppAction::PopScreenAndRefresh),
                ("Key Info", AppAction::None),
            ],
            None => vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Key Info", AppAction::None),
            ],
        }
    }

    /// The key store this screen's key is filed under.
    ///
    /// Resolved from the identity's own records every time it is asked, so no
    /// caller has to supply it and none can supply a wrong one. An existing
    /// placement wins — that is where the material actually is — and only when
    /// nothing is held does the identity's on-chain lists decide where a new
    /// private half would go.
    ///
    /// `None` when the key is on none of this identity's lists and nothing is
    /// held for it, which is the one case where no store can be named honestly.
    fn target(&self) -> Option<PrivateKeyTarget> {
        self.identity
            .private_keys
            .candidates(&self.key)
            .next()
            .map(|(target, _)| target)
            .or_else(|| self.identity.placement_of(&self.key).resolved())
    }

    /// Re-read this screen's identity from the store, after a backend task
    /// wrote it.
    ///
    /// The screen keeps a clone taken when it opened, and its own key add /
    /// remove paths persist that whole clone. Any change another writer makes
    /// while the screen is open therefore has to be picked up here, or the next
    /// key edit writes it away. The key on screen is refreshed from the same
    /// record. A read failure leaves the clone alone and says so — the change
    /// landed, this screen just cannot show it.
    fn reload_identity(&mut self) {
        let identity_id = self.identity.identity.id();
        match self.app_context.get_local_qualified_identity(&identity_id) {
            Ok(Some(fresh)) => {
                // Resolved against the record just read, not the stale clone:
                // the write being picked up here may be the one that filed this
                // key in the first place.
                self.private_key_data =
                    fresh
                        .private_keys
                        .candidates(&self.key)
                        .next()
                        .and_then(|placement| {
                            fresh
                                .private_keys
                                .get_cloned_private_key_data_and_wallet_info(&placement)
                        });
                self.identity = fresh;
            }
            Ok(None) => {}
            Err(error) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "This identity's keys could not be reloaded. Close this key and open it again to see them.",
                    MessageType::Error,
                )
                .with_details(error);
            }
        }
    }

    /// Render the offer, queueing the approved items for dispatch. The rule
    /// above it is this screen's own: the offer arrives after the key's details
    /// and has to be told apart from them.
    fn render_recovery_section(&mut self, ui: &mut egui::Ui) {
        if !self.recovery.has_offer() {
            return;
        }
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);
        self.pending_recovery_restore = host_offer(
            &self.recovery,
            KeyVocabulary::from(self.identity.identity_type),
            ui,
        );
    }

    /// Build a key-info screen with the add-protection confirmation already open
    /// when vault-backed protection is available, or show a warning when wallet
    /// setup has not made protection available yet.
    pub fn new_with_protection_prompt(
        identity: QualifiedIdentity,
        key: IdentityPublicKey,
        private_key_data: Option<(PrivateKeyData, Option<WalletDerivationPath>)>,
        app_context: &Arc<AppContext>,
    ) -> Self {
        let mut screen = Self::new(identity, key, private_key_data, app_context);
        let status = screen.compute_protection_status();
        if status == IdentityProtectionStatus::NoVaultKeys {
            screen.protection_stage = ProtectionStage::Idle;
            MessageBanner::set_global(
                app_context.egui_ctx(),
                "Password protection is not available yet. Wait for wallet setup to finish, then try again.",
                MessageType::Warning,
            );
        } else {
            screen.protection_status = Some(status);
            screen.open_add_confirm();
        }
        screen
    }

    fn validate_and_store_private_key(&mut self) {
        // Convert the input string to bytes (hex decoding)
        let private_key_bytes = match hex::decode(self.private_key_input.text()) {
            Ok(private_key_bytes_vec) if private_key_bytes_vec.len() == 32 => {
                let bytes: [u8; 32] = private_key_bytes_vec
                    .try_into()
                    .expect("invariant: length checked to be 32 in the match guard");
                bytes
            }
            Ok(_) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Private key not 32 bytes",
                    MessageType::Error,
                );
                return;
            }
            Err(_) => match PrivateKey::from_wif(self.private_key_input.text()) {
                Ok(key) => key.inner.secret_bytes(),
                Err(_) => {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Invalid hex string or WIF for private key.",
                        MessageType::Error,
                    );
                    return;
                }
            },
        };

        let validation_result = self
            .key
            .validate_private_key_bytes(&private_key_bytes, self.app_context.network);
        if let Err(error) = validation_result {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "The private key could not be verified. Check the key and try again.",
                MessageType::Error,
            )
            .with_details(error);
        } else if validation_result.expect("invariant: Err handled in the preceding branch") {
            // If valid, store the private key in the context and reset the input field
            self.private_key_data = Some((PrivateKeyData::Clear(private_key_bytes), None));
            // An existing placement is reused so a re-entered key overwrites
            // itself rather than growing a second copy under another store;
            // otherwise the identity's own lists say where it belongs. Both
            // agree with where the resolver will look for it.
            let Some(target) = self.target() else {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "This key does not belong to this identity, so it cannot be saved here.",
                    MessageType::Error,
                );
                return;
            };
            self.identity.private_keys.insert_non_encrypted(
                (target, self.key.id()),
                (self.key.clone().into(), private_key_bytes),
            );
            if let Err(error) = self
                .app_context
                .update_local_qualified_identity(&self.identity)
            {
                let handle = MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "The private key could not be saved. Check available disk space and try again.",
                    MessageType::Error,
                );
                handle.with_details(error);
                handle.disable_auto_dismiss();
            }
        } else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Private key does not match the public key.",
                MessageType::Error,
            );
        }
    }

    fn render_sign_input(&mut self, ui: &mut egui::Ui) {
        let text_primary = DashColors::text_primary(ui.style().visuals.dark_mode);
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.heading(RichText::new("Sign").color(text_primary));

            // Create an info icon button
            let response = crate::ui::helpers::info_icon_button(ui, "Enter a message and click Sign to encrypt it with your private key. You can send the encrypted message to someone and they can decrypt it using your public key. This is useful for proving you own the private key.");

            // Check if the label was clicked
            if response.clicked() {
                self.show_pop_up_info = Some("Enter a message click Sign to encrypt it with your private key. You can can send the encrypted message to someone and they can decrypt it using your public key. This is useful for proving you own the private key.".to_string());
            }
        });
        ui.add_space(5.0);

        ui.label(RichText::new("Enter message to sign:").color(text_primary));
        ui.add_space(5.0);
        ui.add(
            egui::TextEdit::multiline(&mut self.message_input)
                .desired_width(f32::INFINITY)
                .desired_rows(3),
        );
        ui.add_space(5.0);

        if ui.button("Sign Message").clicked() {
            // Attempt to sign the message
            self.sign_message();
        }

        // Sign error display is handled by the global MessageBanner

        if let Some(signed_message) = &self.signed_message {
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            ui.label(RichText::new("Signed Message (Base64):").color(text_primary));
            ui.add_space(5.0);
            ui.add(
                egui::TextEdit::multiline(&mut signed_message.as_str().to_owned())
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
        }
    }

    fn sign_message(&mut self) {
        let Some((private_key_data, _)) = &self.private_key_data else {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Private key is not available.",
                MessageType::Error,
            );
            return;
        };

        if !matches!(
            self.key.key_type(),
            KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160
        ) {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Unsupported key type for signing.",
                MessageType::Error,
            );
            return;
        }

        match private_key_data {
            // Keys that carry their own plaintext sign locally — no wallet seed
            // is involved, so there is nothing to fetch through the chokepoint.
            PrivateKeyData::Clear(bytes) | PrivateKeyData::AlwaysClear(bytes) => {
                self.signed_message = Some(Self::sign_ecdsa_local(bytes, &self.message_input));
            }
            // Wallet-derived keys sign in the backend: the seed is fetched
            // just-in-time through the JIT chokepoint and only the public
            // signature returns. Queue the request; `ui()` dispatches it.
            PrivateKeyData::AtWalletDerivationPath(wdp) => {
                self.pending_sign_request = Some(wdp.derivation_path.clone());
            }
            PrivateKeyData::Encrypted(_) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Private key is not available.",
                    MessageType::Error,
                );
            }
            // Vault-backed identity key: signs in the backend via the JIT
            // chokepoint (InVault route). Queue the request; `ui()` dispatches it.
            PrivateKeyData::InVault => {
                self.pending_identity_sign = true;
            }
        }
    }

    /// Sign `message` with a locally-held ECDSA secret, returning the
    /// Base64-encoded Dash signed-message envelope. Used only for keys that
    /// already carry their plaintext in the UI — never for wallet-derived keys.
    ///
    /// The envelope is a recoverable signature: a header byte (`27 + recId`,
    /// `+4` for a compressed key) followed by the 64-byte signature. These keys
    /// are compressed by convention, so a verifier can recover the signer's
    /// public key and address from the signature alone.
    fn sign_ecdsa_local(private_key_bytes: &[u8; 32], message: &str) -> String {
        let secp = Secp256k1::new();
        let message_hash = signed_msg_hash(message);
        let digest = Message::from_digest(*message_hash.as_byte_array());
        let secret_key = SecretKey::from_byte_array(private_key_bytes)
            .expect("clear private key is a valid 32-byte secret");
        let recoverable = secp.sign_ecdsa_recoverable(&digest, &secret_key);
        MessageSignature::new(recoverable, true).to_base64()
    }

    /// Render the WIF + hex of an already-derived private key. The key is
    /// derived in the backend via `WalletTask::DeriveKeyForDisplay` and the
    /// reconstructed [`RPCPrivateKey`] passed here only for rendering.
    fn render_decrypted_key_grid(ui: &mut egui::Ui, private_key: &RPCPrivateKey) {
        egui::Grid::new("private_key_grid_wallet")
            .num_columns(2)
            .spacing([10.0, 10.0])
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Private Key (WIF):")
                        .strong()
                        .color(ui.visuals().text_color()),
                );
                let wif = Secret::new(private_key.to_wif());
                ui.label(RichText::new(wif.expose_secret()).color(ui.visuals().text_color()));
                ui.end_row();

                ui.label(
                    RichText::new("Private Key (Hex):")
                        .strong()
                        .color(ui.visuals().text_color()),
                );
                let private_key_hex = Secret::new(hex::encode(private_key.inner.secret_bytes()));
                ui.label(
                    RichText::new(private_key_hex.expose_secret()).color(ui.visuals().text_color()),
                );
                ui.end_row();
            });
    }

    /// Queue a one-shot "derive private key for display" request the first time
    /// a wallet-derived key needs to be shown. Idempotent within a view session
    /// via `requested`, so the backend task is dispatched once, not every frame.
    fn queue_key_display(
        pending: &mut Option<DerivationPath>,
        requested: &mut bool,
        path: &DerivationPath,
    ) {
        if !*requested {
            *pending = Some(path.clone());
            *requested = true;
        }
    }

    /// The HD wallet this key derives from, or `None` for keys that carry their
    /// own plaintext. Used to scope the JIT chokepoint for display/sign tasks.
    fn wallet_seed_hash(&self) -> Option<crate::model::wallet::WalletSeedHash> {
        match self.private_key_data.as_ref()? {
            (PrivateKeyData::AtWalletDerivationPath(wdp), _) => Some(wdp.wallet_seed_hash),
            _ => None,
        }
    }

    fn show_remove_private_key_dialog(&mut self, ui: &mut egui::Ui) {
        if let Some(dialog) = self.remove_private_key_dialog.as_mut() {
            let response = dialog.show(ui);
            if let Some(result) = response.inner.dialog_response {
                self.remove_private_key_dialog = None;
                if result == ConfirmationStatus::Confirmed
                    && let Err(error) = self.remove_held_private_key()
                {
                    let handle = MessageBanner::set_global(
                        ui.ctx(),
                        "The private-key change could not be saved. Check available disk space and try again.",
                        MessageType::Error,
                    );
                    handle.with_details(error);
                    handle.disable_auto_dismiss();
                }
            }
        }
    }

    /// Drop this device's copy of the on-screen key's private half and persist
    /// the record.
    ///
    /// Removes **every** placement holding *this* key, so a duplicate written
    /// under another convention cannot survive the removal the user asked for.
    /// The placements come from
    /// [`candidates`](crate::model::qualified_identity::encrypted_key_storage::KeyStorage::candidates),
    /// which selects on the public half — a removal keyed on the id alone lands
    /// on whichever key happens to occupy the derived slot, and on a masternode
    /// the voter and main id spaces overlap, so that can be a different key
    /// entirely.
    fn remove_held_private_key(&mut self) -> Result<(), TaskError> {
        self.private_key_data = None;
        for placement in self
            .identity
            .private_keys
            .candidates(&self.key)
            .collect::<Vec<_>>()
        {
            self.identity.private_keys.remove_at(&placement);
        }
        self.app_context
            .update_local_qualified_identity(&self.identity)
    }

    // --- Identity key password protection (per-identity at-rest key encryption) ---

    /// At-rest protection posture of this identity's vault keys, by probing the
    /// vault scheme of each key. Cheap (a handful of local vault reads). Cached
    /// in `protection_status`; invalidated after a migration.
    fn compute_protection_status(&self) -> IdentityProtectionStatus {
        let Ok(backend) = self.app_context.wallet_backend() else {
            return IdentityProtectionStatus::NoVaultKeys;
        };
        let id = self.identity.identity.id().to_buffer();
        let view = IdentityKeyView::new(backend.secret_store(), id);
        let (mut protected, mut unprotected) = (0usize, 0usize);
        for (target, key_id) in self.identity.private_keys.keys_set() {
            match view.scheme(&target, key_id) {
                Ok(SecretScheme::Protected) => protected += 1,
                Ok(SecretScheme::Unprotected) => unprotected += 1,
                // Absent (wallet-derived / resident-plaintext) or a transient
                // vault error: not a protectable vault key — ignore it.
                _ => {}
            }
        }
        match (protected, unprotected) {
            (0, 0) => IdentityProtectionStatus::NoVaultKeys,
            (_, 0) => IdentityProtectionStatus::Protected,
            (0, _) => IdentityProtectionStatus::Unprotected,
            _ => IdentityProtectionStatus::Mixed,
        }
    }

    /// Render the collapsible "Key Protection" section (default closed). Hidden
    /// entirely when the identity has no vault-stored keys.
    fn render_key_protection_section(&mut self, ui: &mut egui::Ui) {
        if self.protection_status.is_none() {
            let status = self.compute_protection_status();
            self.protection_status = Some(status);
        }
        let status = self
            .protection_status
            .unwrap_or(IdentityProtectionStatus::NoVaultKeys);
        if status == IdentityProtectionStatus::NoVaultKeys {
            return;
        }
        let dark_mode = ui.style().visuals.dark_mode;

        egui::CollapsingHeader::new("Key Protection")
            .default_open(false)
            // Keep the header open so an active protection form or dialog remains visible.
            .open((self.protection_stage != ProtectionStage::Idle).then_some(true))
            .show(ui, |ui| {
                let status_text = match status {
                    IdentityProtectionStatus::Unprotected => {
                        "This identity's keys sign automatically. No password is required."
                    }
                    IdentityProtectionStatus::Protected => {
                        "This identity's keys require a password each time they sign."
                    }
                    IdentityProtectionStatus::Mixed => {
                        "Password protection for this identity's keys is incomplete. Finish protecting them with the same password you set."
                    }
                    IdentityProtectionStatus::NoVaultKeys => "",
                };
                ui.label(
                    RichText::new(status_text).color(DashColors::text_secondary(dark_mode)),
                );
                ui.add_space(8.0);

                match self.protection_stage {
                    ProtectionStage::Idle => self.render_protection_idle(ui, status),
                    ProtectionStage::EnterNewPassword => self.render_new_password_form(ui),
                    ProtectionStage::EnterVerifyPassword => self.render_verify_password_form(ui),
                    // The confirm dialogs draw as modals (below), not inline.
                    ProtectionStage::ConfirmAdd | ProtectionStage::ConfirmRemove => {}
                }
            });

        // The danger confirmation dialog (opt-in / opt-out) draws as a modal.
        self.handle_protection_confirm(ui);
    }

    /// The idle status row: the action button whose meaning depends on the
    /// current protection posture.
    fn render_protection_idle(&mut self, ui: &mut egui::Ui, status: IdentityProtectionStatus) {
        let (label, is_add) = match status {
            IdentityProtectionStatus::Protected => ("Remove password protection…", false),
            IdentityProtectionStatus::Mixed => ("Finish protecting…", true),
            _ => ("Add password protection…", true),
        };
        let resp = ui.add_enabled(!self.protection_in_flight, egui::Button::new(label));
        if resp.clicked() {
            if is_add {
                self.open_add_confirm();
            } else {
                self.open_remove_confirm();
            }
        }
        if self.protection_in_flight {
            ui.add_space(4.0);
            ui.label(
                RichText::new("Working…")
                    .color(DashColors::text_secondary(ui.style().visuals.dark_mode)),
            );
        }
    }

    /// Open the danger warning before opt-in.
    fn open_add_confirm(&mut self) {
        self.protection_form_error = None;
        self.protection_new_password.clear();
        self.protection_confirm_password.clear();
        self.protection_hint.clear();
        self.protection_stage = ProtectionStage::ConfirmAdd;
        self.protection_confirm = Some(
            ConfirmationDialog::new(
                "Protect this identity's keys with a password?",
                "Adding a password means this identity's keys will ask for the password each time they are used to sign. Keep this in mind:\n\n\
                 • If you forget the password, these keys cannot be recovered. There is no reset option.\n\n\
                 • Automatic tools (such as scripts or the command-line interface) will no longer be able to sign with this identity without the password.\n\n\
                 Are you sure you want to continue?",
            )
            .danger_mode(true)
            .confirm_text(Some("Yes, add protection"))
            .cancel_text(Some("Cancel"))
            .open(true),
        );
    }

    /// Open the danger warning before opt-out.
    fn open_remove_confirm(&mut self) {
        self.protection_form_error = None;
        self.protection_verify_password.clear();
        self.protection_stage = ProtectionStage::ConfirmRemove;
        self.protection_confirm = Some(
            ConfirmationDialog::new(
                "Remove password protection?",
                "Removing the password means this identity's keys will sign automatically without any password. Anyone with access to this device could use them to sign on behalf of this identity.\n\n\
                 You will need to enter the current password to confirm this change.",
            )
            .danger_mode(true)
            .confirm_text(Some("Yes, remove protection"))
            .cancel_text(Some("Cancel"))
            .open(true),
        );
    }

    /// Drive the danger confirmation dialog; on confirm, advance to the
    /// matching password form; on cancel, return to idle.
    fn handle_protection_confirm(&mut self, ui: &mut egui::Ui) {
        let Some(dialog) = self.protection_confirm.as_mut() else {
            return;
        };
        let response = dialog.show(ui);
        if let Some(result) = response.inner.dialog_response {
            self.protection_confirm = None;
            match (self.protection_stage, result) {
                (ProtectionStage::ConfirmAdd, ConfirmationStatus::Confirmed) => {
                    self.protection_stage = ProtectionStage::EnterNewPassword;
                }
                (ProtectionStage::ConfirmRemove, ConfirmationStatus::Confirmed) => {
                    self.protection_stage = ProtectionStage::EnterVerifyPassword;
                }
                _ => self.protection_stage = ProtectionStage::Idle,
            }
        }
    }

    /// The opt-in password form: new password + confirmation + strength + hint.
    fn render_new_password_form(&mut self, ui: &mut egui::Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        ui.label(
            RichText::new(format!(
                "This password protects the signing keys for {}.",
                self.identity
            ))
            .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(6.0);

        ui.label("New password:");
        self.protection_new_password.show(ui);
        ui.add_space(4.0);
        let pw = self.protection_new_password.text().to_string();
        render_password_strength(ui, &pw);

        ui.add_space(8.0);
        ui.label("Confirm password:");
        self.protection_confirm_password.show(ui);

        ui.add_space(8.0);
        ui.label(
            "Password hint (optional — visible in plain text. Do not use the password itself as a hint.):",
        );
        ui.add(egui::TextEdit::singleline(&mut self.protection_hint).hint_text("Password hint"));

        if let Some(err) = &self.protection_form_error {
            ui.add_space(6.0);
            ui.colored_label(DashColors::ERROR, err);
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Protect keys").clicked() {
                self.submit_new_password();
            }
            if ui.button("Cancel").clicked() {
                self.cancel_protection_flow();
            }
        });
    }

    /// The opt-out password form: verify the current password.
    fn render_verify_password_form(&mut self, ui: &mut egui::Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        ui.label(
            RichText::new(format!(
                "Enter the current password for the signing keys for {}.",
                self.identity
            ))
            .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(6.0);
        self.protection_verify_password.show(ui);

        if let Some(err) = &self.protection_form_error {
            ui.add_space(6.0);
            ui.colored_label(DashColors::ERROR, err);
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.button("Verify and remove").clicked() {
                self.submit_verify_password();
            }
            if ui.button("Cancel").clicked() {
                self.cancel_protection_flow();
            }
        });
    }

    /// Validate the opt-in form and queue the `ProtectIdentityKeys` dispatch.
    fn submit_new_password(&mut self) {
        let pw = self.protection_new_password.text().to_string();
        let confirm = self.protection_confirm_password.text().to_string();
        if let Err(e) = validate_single_key_passphrase(&pw, &confirm) {
            self.protection_form_error = Some(e.to_string());
            return;
        }
        let hint = {
            let h = self.protection_hint.trim();
            if h.is_empty() {
                None
            } else {
                Some(h.to_string())
            }
        };
        self.pending_protect = Some((Secret::new(pw), hint));
        self.finish_protection_flow();
    }

    /// Queue the `UnprotectIdentityKeys` dispatch (the backend verifies the
    /// password — a wrong one returns a typed error, no client-side oracle).
    fn submit_verify_password(&mut self) {
        let pw = self.protection_verify_password.text().to_string();
        if pw.is_empty() {
            self.protection_form_error =
                Some("Enter the current password to remove protection.".to_string());
            return;
        }
        self.pending_unprotect = Some(Secret::new(pw));
        self.finish_protection_flow();
    }

    /// Mark a migration as dispatched: clear the forms, flip to in-flight, and
    /// return to the idle status row.
    fn finish_protection_flow(&mut self) {
        self.protection_in_flight = true;
        self.protection_form_error = None;
        self.protection_new_password.clear();
        self.protection_confirm_password.clear();
        self.protection_verify_password.clear();
        self.protection_hint.clear();
        self.protection_stage = ProtectionStage::Idle;
    }

    /// Abandon the active flow with no change.
    fn cancel_protection_flow(&mut self) {
        self.protection_form_error = None;
        self.protection_new_password.clear();
        self.protection_confirm_password.clear();
        self.protection_verify_password.clear();
        self.protection_hint.clear();
        self.protection_stage = ProtectionStage::Idle;
    }
}

/// Render a zxcvbn-backed password-strength bar (0–4 score). Mirrors the
/// wallet-creation strength UI so the two surfaces feel identical.
fn render_password_strength(ui: &mut egui::Ui, password: &str) {
    let score = if password.is_empty() {
        0u8
    } else {
        u8::from(zxcvbn(password, &[]).score())
    };
    let fraction = f32::from(score) / 4.0;
    let (fill, label) = match score {
        0 => (DashColors::STRENGTH_WEAK, "None"),
        1 => (DashColors::STRENGTH_WEAK, "Very weak"),
        2 => (DashColors::STRENGTH_FAIR, "Weak"),
        3 => (DashColors::STRENGTH_GOOD, "Strong"),
        _ => (DashColors::STRENGTH_STRONG, "Very strong"),
    };
    ui.horizontal(|ui| {
        ui.label("Password strength:");
        ui.add(
            egui::ProgressBar::new(fraction)
                .desired_width(180.0)
                .text(label)
                .fill(fill),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TaskResult;
    use crate::app_dir::ensure_env_file;
    use crate::context::connection_status::ConnectionStatus;
    use crate::database::test_helpers::create_database_at_path;
    use crate::model::legacy_recovery::RecoveryPlan;
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
    use crate::model::qualified_identity::{IdentityStatus, IdentityType, PrivateKeyTarget};
    use crate::utils::egui_mpsc::SenderAsync;
    use crate::utils::tasks::TaskManager;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
    use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
    use dash_sdk::dpp::identity::{Identity, KeyID, KeyType, Purpose, SecurityLevel};
    use dash_sdk::dpp::platform_value::BinaryData;
    use dash_sdk::dpp::version::PlatformVersion;
    use dash_sdk::platform::Identifier;
    use std::collections::BTreeMap;

    const MAIN: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnMainIdentity;
    const VOTER: PrivateKeyTarget = PrivateKeyTarget::PrivateKeyOnVoterIdentity;

    /// An offline, wired context on a throwaway data dir — the identity store
    /// refuses writes until the wallet backend is up.
    async fn offline_ctx() -> (Arc<AppContext>, tempfile::TempDir) {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let data_dir = temp_dir.path().to_path_buf();
        ensure_env_file(&data_dir);
        let db = Arc::new(create_database_at_path(&data_dir.join("data.db")).expect("db"));
        let app_kv = AppContext::open_app_kv(&data_dir).expect("app kv");
        let secret_store = AppContext::open_secret_store(&data_dir).expect("secret store");
        let ctx = AppContext::new(
            data_dir,
            Network::Testnet,
            db,
            Arc::new(TaskManager::new()),
            Arc::new(ConnectionStatus::new()),
            egui::Context::default(),
            app_kv,
            secret_store,
            crate::model::user_role::UserRoleCell::default(),
        )
        .expect("offline testnet AppContext::new");
        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(32);
        let sender = SenderAsync::new(tx, ctx.egui_ctx().clone());
        ctx.ensure_wallet_backend(sender)
            .await
            .expect("wire wallet backend offline");
        (ctx, temp_dir)
    }

    fn public_key(id: KeyID, purpose: Purpose) -> IdentityPublicKey {
        IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id,
            purpose,
            security_level: SecurityLevel::HIGH,
            contract_bounds: None,
            key_type: KeyType::ECDSA_HASH160,
            read_only: false,
            data: BinaryData::new(vec![id as u8; 20]),
            disabled_at: None,
        })
    }

    fn identity_with(id: u8, keys: &[(IdentityPublicKey, [u8; 32])]) -> QualifiedIdentity {
        let mut private_keys = KeyStorage::default();
        for (key, secret) in keys {
            private_keys.insert_at(
                (MAIN, key.id()),
                (
                    QualifiedIdentityPublicKey::from(key.clone()),
                    PrivateKeyData::Clear(*secret),
                ),
            );
        }
        QualifiedIdentity {
            identity: Identity::create_basic_identity(
                Identifier::from([id; 32]),
                PlatformVersion::latest(),
            )
            .expect("basic identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        }
    }

    /// Removing this device's copy of one key must not touch a *different* key
    /// that happens to share its id.
    ///
    /// The shape this reaches needs two writers, which is how a real install gets
    /// it: the structural loader files a main-identity `VOTING` key under `Main`
    /// (`load_identity` files every main-identity key structurally, purpose
    /// included), while an older build's paste path filed an unrelated key under
    /// `Voter` at the same id — the voter and main id spaces overlap, so id 0
    /// names two keys on a masternode. A removal that derived its slot from the
    /// key's purpose would send this one to `Voter` and delete the wrong key's
    /// private half, leaving the key the user asked about still on the device.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn removing_one_key_leaves_a_different_key_sharing_its_id_alone() {
        let (app_context, _dir) = offline_ctx().await;

        // Same id, different keys: purpose is what tells them apart.
        let on_screen = public_key(0, Purpose::VOTING);
        let other = IdentityPublicKey::V0(IdentityPublicKeyV0 {
            id: 0,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::HIGH,
            contract_bounds: None,
            key_type: KeyType::ECDSA_HASH160,
            read_only: false,
            data: BinaryData::new(vec![0xEE; 20]),
            disabled_at: None,
        });

        let mut stored = identity_with(0x5A, &[(on_screen.clone(), [0x11; 32])]);
        stored.private_keys.insert_at(
            (VOTER, other.id()),
            (
                QualifiedIdentityPublicKey::from(other.clone()),
                PrivateKeyData::Clear([0x22; 32]),
            ),
        );
        app_context
            .insert_local_qualified_identity(&stored, &None)
            .expect("insert the record");

        let mut screen =
            KeyInfoScreen::new(stored, on_screen.clone(), None, &app_context).with_parent("Keys");
        screen
            .remove_held_private_key()
            .expect("the removal must persist");

        assert!(
            screen
                .identity
                .private_keys
                .candidates(&on_screen)
                .next()
                .is_none(),
            "the key the user asked to remove must be gone",
        );
        assert!(
            screen
                .identity
                .private_keys
                .candidates(&other)
                .next()
                .is_some(),
            "a different key sharing the id must survive the removal",
        );
    }

    /// Write `key` into `identity_id`'s stored record, the way a restore or any
    /// other backend writer does — behind whatever screen holds a clone of it.
    fn write_key_behind_the_screen(
        app_context: &Arc<AppContext>,
        identity_id: Identifier,
        key: &IdentityPublicKey,
        secret: [u8; 32],
    ) {
        let mut record = app_context
            .get_local_qualified_identity(&identity_id)
            .expect("read the record")
            .expect("record stored");
        record.private_keys.insert_at(
            (MAIN, key.id()),
            (
                QualifiedIdentityPublicKey::from(key.clone()),
                PrivateKeyData::Clear(secret),
            ),
        );
        app_context
            .update_local_qualified_identity(&record)
            .expect("the other writer's write");
    }

    /// This screen persists the identity clone it was opened with on every key
    /// add or remove. A restore writes that same record behind its back, so the
    /// completion has to refresh the clone — otherwise the next ordinary key
    /// edit writes the pre-restore copy back and the restored keys vanish,
    /// right after a banner said they were back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_finished_restore_refreshes_the_identity_this_screen_writes_back() {
        let (app_context, _dir) = offline_ctx().await;

        let on_screen_key = public_key(1, Purpose::AUTHENTICATION);
        let stored = identity_with(0x4E, &[(on_screen_key.clone(), [0x11; 32])]);
        let identity_id = stored.identity.id();
        app_context
            .insert_local_qualified_identity(&stored, &None)
            .expect("insert the record");

        let mut screen = KeyInfoScreen::new(stored, on_screen_key, None, &app_context);

        // What the restore writes: the record gains the stranded key.
        let restored_key = public_key(2, Purpose::TRANSFER);
        write_key_behind_the_screen(&app_context, identity_id, &restored_key, [0x22; 32]);

        screen.display_task_result(BackendTaskSuccessResult::LegacyRecoveryCompleted {
            identity_id,
            applied: vec![],
            skipped_stale: vec![],
            excluded: vec![],
        });

        assert!(
            screen.identity.private_keys.has(&(MAIN, restored_key.id())),
            "the screen must hold the restored record, not the clone it opened with",
        );

        app_context
            .wallet_backend()
            .expect("backend")
            .shutdown()
            .await;
    }

    /// A restore that lands while this screen is off-screen never reaches its
    /// `display_task_result` — results go only to the visible screen. Returning
    /// to it must re-read the record, or the clone it opened with is written
    /// back over the restored keys by the next ordinary key edit, silently and
    /// with no error to show for it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_restore_that_landed_off_screen_survives_the_next_key_edit() {
        let (app_context, _dir) = offline_ctx().await;

        let on_screen_key = public_key(1, Purpose::AUTHENTICATION);
        let stored = identity_with(0x4E, &[(on_screen_key.clone(), [0x11; 32])]);
        let identity_id = stored.identity.id();
        app_context
            .insert_local_qualified_identity(&stored, &None)
            .expect("insert the record");
        let mut screen = KeyInfoScreen::new(stored, on_screen_key, None, &app_context);

        // The restore lands while another screen is the visible one, so this
        // screen is never told about it.
        let restored_key = public_key(2, Purpose::TRANSFER);
        write_key_behind_the_screen(&app_context, identity_id, &restored_key, [0x22; 32]);

        screen.refresh_on_arrival();

        // What every key add and remove on this screen does with its clone.
        app_context
            .update_local_qualified_identity(&screen.identity)
            .expect("the next key edit's write");

        assert!(
            app_context
                .get_local_qualified_identity(&identity_id)
                .expect("read back")
                .expect("still stored")
                .private_keys
                .has(&(MAIN, restored_key.id())),
            "a key edit on this screen must not erase keys restored while it was away",
        );

        app_context
            .wallet_backend()
            .expect("backend")
            .shutdown()
            .await;
    }

    /// A restore dispatched from one identity's Key Info screen can complete
    /// after the user has opened another's, and results reach whichever screen
    /// is visible. The stray completion must touch nothing here: not the clone,
    /// not this identity's own recovery offer, not the banner — it says
    /// "restored to this identity" about an identity that is not on screen.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_completion_for_another_identity_is_ignored() {
        let (app_context, _dir) = offline_ctx().await;

        let on_screen_key = public_key(1, Purpose::AUTHENTICATION);
        let on_screen = identity_with(0xB0, &[(on_screen_key.clone(), [0x11; 32])]);
        let on_screen_id = on_screen.identity.id();
        app_context
            .insert_local_qualified_identity(&on_screen, &None)
            .expect("insert the record");
        let mut screen = KeyInfoScreen::new(on_screen, on_screen_key, None, &app_context);

        // This identity has a restore of its own running.
        screen
            .recovery
            .offered(on_screen_id, RecoveryPlan::default());
        screen.recovery.restore(vec![]).expect("dispatch a restore");

        // A change to this identity's record that only a reload would pick up,
        // so a wrongly-attributed reload is observable.
        let other_writer_key = public_key(2, Purpose::TRANSFER);
        write_key_behind_the_screen(&app_context, on_screen_id, &other_writer_key, [0x22; 32]);

        screen.display_task_result(BackendTaskSuccessResult::LegacyRecoveryCompleted {
            identity_id: Identifier::from([0xA0; 32]),
            applied: vec![],
            skipped_stale: vec![],
            excluded: vec![],
        });

        assert!(
            screen.recovery.is_restoring(),
            "another identity's completion must not end this identity's restore",
        );
        assert!(
            !screen
                .identity
                .private_keys
                .has(&(MAIN, other_writer_key.id())),
            "another identity's completion must not be acted on here at all",
        );

        app_context
            .wallet_backend()
            .expect("backend")
            .shutdown()
            .await;
    }

    /// Regression: every failing backend task routed to the visible screen used
    /// to end the restore, so an unrelated failure re-enabled the Restore button
    /// while the original task still held the identity — pressing it again only
    /// reported that a load was already in progress. Only this restore's own
    /// failure may return the offer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn only_this_restores_own_failure_returns_it_to_its_offer() {
        let (app_context, _dir) = offline_ctx().await;

        let on_screen_key = public_key(1, Purpose::AUTHENTICATION);
        let on_screen = identity_with(0xB4, &[(on_screen_key.clone(), [0x11; 32])]);
        let on_screen_id = on_screen.identity.id();
        app_context
            .insert_local_qualified_identity(&on_screen, &None)
            .expect("insert the record");
        let mut screen = KeyInfoScreen::new(on_screen, on_screen_key, None, &app_context);
        screen
            .recovery
            .offered(on_screen_id, RecoveryPlan::default());
        screen.recovery.restore(vec![]).expect("dispatch a restore");

        let error = TaskError::IdentityNotFoundLocally;
        for unrelated in [
            BackendTaskContext::Other,
            BackendTaskContext::TokenBalanceRefresh,
            BackendTaskContext::LegacyRecoveryRestore(Identifier::from([0xA0; 32])),
        ] {
            screen.display_backend_task_error(&unrelated, &error);
            screen.display_message("something else failed", MessageType::Error);
            assert!(
                screen.recovery.is_restoring(),
                "{unrelated:?} is not this restore, so it must stay in flight",
            );
        }

        screen.display_backend_task_error(
            &BackendTaskContext::LegacyRecoveryRestore(on_screen_id),
            &error,
        );
        assert!(
            !screen.recovery.is_restoring(),
            "this restore's own failure must return the offer so it can be retried",
        );

        app_context
            .wallet_backend()
            .expect("backend")
            .shutdown()
            .await;
    }
}
