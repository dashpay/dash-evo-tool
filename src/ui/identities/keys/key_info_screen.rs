use crate::app::AppAction;
use crate::backend_task::wallet::WalletTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::secret::Secret;
use crate::model::wallet::Wallet;
use crate::ui::components::MessageBanner;
use crate::ui::components::component_trait::Component;
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::password_input::PasswordInput;
use crate::ui::components::styled::{ConfirmationDialog, ConfirmationStatus, island_central_panel};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dashcore_rpc::dashcore::PrivateKey as RPCPrivateKey;
use dash_sdk::dpp::dashcore::address::Payload;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1, SecretKey};
use dash_sdk::dpp::dashcore::sign_message::{MessageSignature, signed_msg_hash};
use dash_sdk::dpp::dashcore::{Address, PrivateKey, PubkeyHash, ScriptHash};
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::identity::KeyType::BIP13_SCRIPT_HASH;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dash_sdk::dpp::key_wallet::bip32::DerivationPath;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::IdentityPublicKey;
use eframe::egui::{self, Context};
use egui::{Color32, RichText, ScrollArea};
use std::sync::{Arc, RwLock};

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
}

impl ScreenLike for KeyInfoScreen {
    fn refresh(&mut self) {}

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
                        MessageBanner::set_global(
                            self.app_context.egui_ctx(),
                            "Could not display the private key. Please retry.",
                            MessageType::Error,
                        )
                        .with_details(e);
                    }
                }
            }
            BackendTaskSuccessResult::WalletMessageSigned { signature, .. } => {
                self.signed_message = Some(signature);
            }
            _ => {}
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Key Info", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentities,
        );

        action |= island_central_panel(ctx, |ui| {
            let inner_action = AppAction::None;

            ScrollArea::vertical().show(ui, |ui| {
                let text_primary = DashColors::text_primary(ui.ctx().style().visuals.dark_mode);
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

                        // Purpose
                        ui.label(RichText::new("Purpose:").strong().color(text_primary));
                        ui.label(
                            RichText::new(format!("{:?}", self.key.purpose())).color(text_primary),
                        );
                        ui.end_row();

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
                                        // INTENTIONAL(CODE-003): WIF displayed as plaintext label — user-initiated key view.
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
                                    // INTENTIONAL(CODE-003): WIF displayed as plaintext label — user-initiated key view.
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
                            MessageBanner::set_global(ui.ctx(), &e, MessageType::Error);
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
                .show(ctx, |ui| {
                    let mut popup = InfoPopup::new("Sign Message Info", &show_pop_up_info_text);
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
                let wallets = app_context.wallets.read().unwrap();
                wallets
                    .get(&wallet_derivation_path.wallet_seed_hash)
                    .cloned()
            } else {
                None
            };
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
        }
    }

    fn validate_and_store_private_key(&mut self) {
        // Convert the input string to bytes (hex decoding)
        let private_key_bytes = match hex::decode(self.private_key_input.text()) {
            Ok(private_key_bytes_vec) if private_key_bytes_vec.len() == 32 => {
                private_key_bytes_vec.try_into().unwrap()
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
        if let Err(err) = validation_result {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                format!("Issue verifying private key {}", err),
                MessageType::Error,
            );
        } else if validation_result.unwrap() {
            // If valid, store the private key in the context and reset the input field
            self.private_key_data = Some((PrivateKeyData::Clear(private_key_bytes), None));
            self.identity.private_keys.insert_non_encrypted(
                (self.key.purpose().into(), self.key.id()),
                (self.key.clone().into(), private_key_bytes),
            );
            if let Err(e) = self
                .app_context
                .update_local_qualified_identity(&self.identity)
            {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    format!("Issue saving: {}", e),
                    MessageType::Error,
                );
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
        let text_primary = DashColors::text_primary(ui.ctx().style().visuals.dark_mode);
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
                if result == ConfirmationStatus::Confirmed {
                    self.private_key_data = None;
                    self.identity
                        .private_keys
                        .private_keys
                        .remove(&(self.key.purpose().into(), self.key.id()));
                    if let Err(e) = self
                        .app_context
                        .update_local_qualified_identity(&self.identity)
                    {
                        MessageBanner::set_global(
                            ui.ctx(),
                            format!("Issue saving: {}", e),
                            MessageType::Error,
                        );
                    }
                }
            }
        }
    }
}
