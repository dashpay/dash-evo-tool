use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::encrypted_key_storage::{
    PrivateKeyData, WalletDerivationPath,
};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::ScreenLike;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use dash_sdk::dashcore_rpc::dashcore::consensus::encode;
use dash_sdk::dashcore_rpc::dashcore::hashes::sha256d;
use dash_sdk::dpp::dashcore::address::Payload;
use dash_sdk::dpp::dashcore::consensus::Encodable;
use dash_sdk::dpp::dashcore::hashes::{Hash, HashEngine};
use dash_sdk::dpp::dashcore::secp256k1::{Message, Secp256k1, SecretKey};
use dash_sdk::dpp::dashcore::sign_message::signed_msg_hash;
use dash_sdk::dpp::dashcore::{Address, PrivateKey, PubkeyHash, ScriptHash};
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::identity::KeyType::BIP13_SCRIPT_HASH;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::IdentityPublicKey;
use eframe::egui::{self, Context};
use egui::{RichText, TextEdit};
use std::sync::{Arc, RwLock};

pub struct KeyInfoScreen {
    pub identity: QualifiedIdentity,
    pub key: IdentityPublicKey,
    pub private_key_data: Option<(PrivateKeyData, Option<WalletDerivationPath>)>,
    pub decrypted_private_key: Option<PrivateKey>,
    pub app_context: Arc<AppContext>,
    private_key_input: String,
    error_message: Option<String>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    message_input: String,
    signed_message: Option<String>,
    sign_error_message: Option<String>,
    view_wallet_unlock: bool,
    wallet_open: bool,
    view_private_key_even_if_encrypted_or_in_wallet: bool,
}

// /// The prefix for signed messages using Dash's message signing protocol.
// pub const DASH_SIGNED_MSG_PREFIX: &[u8] = b"\x19Dash Signed Message:\n";
//
// pub fn signed_msg_hash(msg: &str) -> sha256d::Hash {
//     let mut engine = sha256d::Hash::engine();
//     engine.input(DASH_SIGNED_MSG_PREFIX);
//     let msg_len = encode::VarInt(msg.len() as u64);
//     msg_len.consensus_encode(&mut engine).expect("engines don't error");
//     engine.input(msg.as_bytes());
//     sha256d::Hash::from_engine(engine)
// }

impl ScreenLike for KeyInfoScreen {
    fn refresh(&mut self) {}

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Key Info", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Key Information");

            egui::Grid::new("key_info_grid")
                .num_columns(2)
                .spacing([10.0, 10.0])
                .striped(true)
                .show(ui, |ui| {
                    // Key ID
                    ui.label(RichText::new("Key ID:").strong());
                    ui.label(format!("{}", self.key.id()));
                    ui.end_row();

                    // Purpose
                    ui.label(RichText::new("Purpose:").strong());
                    ui.label(format!("{:?}", self.key.purpose()));
                    ui.end_row();

                    // Security Level
                    ui.label(RichText::new("Security Level:").strong());
                    ui.label(format!("{:?}", self.key.security_level()));
                    ui.end_row();

                    // Type
                    ui.label(RichText::new("Type:").strong());
                    ui.label(format!("{:?}", self.key.key_type()));
                    ui.end_row();

                    // Read Only
                    ui.label(RichText::new("Read Only:").strong());
                    ui.label(format!("{}", self.key.read_only()));
                    ui.end_row();

                    // Disabled
                    ui.label(RichText::new("Active/Disabled:").strong());
                    if !self.key.is_disabled() {
                        ui.label("Active");
                    } else {
                        ui.label("Disabled");
                    }
                    ui.end_row();

                    if let Some((_, Some(wallet_derivation_path))) = self.private_key_data.as_ref()
                    {
                        // Disabled
                        ui.label(RichText::new("In local Wallet").strong());
                        ui.label(
                            RichText::new(format!(
                                "At derivation path {}",
                                wallet_derivation_path.derivation_path
                            ))
                            .strong(),
                        );
                        ui.end_row();
                    }

                    ui.end_row();
                });

            ui.separator();

            // Display the public key information
            ui.heading("Public Key Information");

            egui::Grid::new("public_key_info_grid")
                .num_columns(2)
                .spacing([10.0, 10.0])
                .striped(true)
                .show(ui, |ui| {
                    match self.key.key_type() {
                        KeyType::ECDSA_SECP256K1 | KeyType::BLS12_381 => {
                            // Public Key Hex
                            ui.label(RichText::new("Public Key (Hex):").strong());
                            ui.label(self.key.data().to_string(Encoding::Hex));
                            ui.end_row();

                            // Public Key Hex
                            ui.label(RichText::new("Public Key (Base64):").strong());
                            ui.label(self.key.data().to_string(Encoding::Base64));
                            ui.end_row();
                        }
                        _ => {}
                    }

                    // Public Key Hash
                    ui.label(RichText::new("Public Key Hash:").strong());
                    match self.key.public_key_hash() {
                        Ok(hash) => {
                            let hash_hex = hex::encode(hash);
                            ui.label(hash_hex);
                        }
                        Err(e) => {
                            ui.colored_label(egui::Color32::RED, format!("Error: {}", e));
                        }
                    }

                    if self.key.key_type().is_core_address_key_type() {
                        // Public Key Hash
                        ui.label(RichText::new("Address:").strong());
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
                                ui.label(address.to_string());
                            }
                            Err(e) => {
                                ui.colored_label(egui::Color32::RED, format!("Error: {}", e));
                            }
                        }
                    }

                    ui.end_row();
                });

            ui.separator();

            // Display the private key if available
            if let Some((private_key, _)) = self.private_key_data.as_mut() {
                ui.label("Private Key:");
                match private_key {
                    PrivateKeyData::Clear(clear) | PrivateKeyData::AlwaysClear(clear) => {
                        let private_key_hex = hex::encode(clear);
                        ui.add(
                            TextEdit::multiline(&mut private_key_hex.as_str().to_owned())
                                .desired_width(f32::INFINITY),
                        );
                        self.render_sign_input(ui);
                    }
                    PrivateKeyData::Encrypted(_) => {
                        ui.label("key is encrypted");
                        //todo decrypt key
                    }
                    PrivateKeyData::AtWalletDerivationPath(derivation_path) => {
                        if self.wallet_open
                            && self.view_private_key_even_if_encrypted_or_in_wallet
                            && self.selected_wallet.is_some()
                        {
                            if let Some(private_key) = self.decrypted_private_key {
                                let private_key_wif = private_key.to_wif();
                                ui.add(
                                    TextEdit::multiline(&mut private_key_wif.as_str().to_owned())
                                        .desired_width(f32::INFINITY),
                                );
                            } else {
                                let wallet = self.selected_wallet.as_ref().unwrap().read().unwrap();
                                match wallet.private_key_at_derivation_path(
                                    &derivation_path.derivation_path,
                                ) {
                                    Ok(private_key) => {
                                        let private_key_wif = private_key.to_wif();
                                        ui.add(
                                            TextEdit::multiline(
                                                &mut private_key_wif.as_str().to_owned(),
                                            )
                                            .desired_width(f32::INFINITY),
                                        );
                                        self.decrypted_private_key = Some(private_key);
                                    }
                                    Err(e) => {
                                        ui.label(format!("Error: {}", e));
                                        return;
                                    }
                                }
                            }
                            self.render_sign_input(ui);
                        } else if self.wallet_open {
                            ui.label("key is in encrypted wallet");
                            if ui.button("view private key").clicked() {
                                self.view_private_key_even_if_encrypted_or_in_wallet = true;
                                self.view_wallet_unlock = true;
                            }
                            if self.decrypted_private_key.is_none() {
                                let wallet = self.selected_wallet.as_ref().unwrap().read().unwrap();
                                match wallet.private_key_at_derivation_path(
                                    &derivation_path.derivation_path,
                                ) {
                                    Ok(private_key) => {
                                        let private_key_wif = private_key.to_wif();
                                        ui.add(
                                            TextEdit::multiline(
                                                &mut private_key_wif.as_str().to_owned(),
                                            )
                                            .desired_width(f32::INFINITY),
                                        );
                                        self.decrypted_private_key = Some(private_key);
                                    }
                                    Err(e) => {
                                        ui.label(format!("Error: {}", e));
                                        return;
                                    }
                                }
                            }
                            self.render_sign_input(ui);
                        } else {
                            ui.label("key is in encrypted wallet");
                            if ui.button("view private key").clicked() {
                                self.view_private_key_even_if_encrypted_or_in_wallet = true;
                                self.view_wallet_unlock = true;
                            }
                            if ui.button("sign message").clicked() {
                                self.view_wallet_unlock = true;
                            }
                        }
                    }
                }
            } else {
                ui.label("Enter Private Key:");
                ui.text_edit_singleline(&mut self.private_key_input);

                if ui.button("Add Private Key").clicked() {
                    self.validate_and_store_private_key();
                }

                // Display error message if validation fails
                if let Some(error_message) = &self.error_message {
                    ui.colored_label(egui::Color32::RED, error_message);
                }
            }

            if self.view_wallet_unlock {
                let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);
                if !needed_unlock || just_unlocked {
                    self.wallet_open = true;
                }
            }
        });
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
            private_key_input: String::new(),
            error_message: None,
            selected_wallet,
            wallet_password: "".to_string(),
            show_password: false,
            message_input: "".to_string(),
            signed_message: None,
            sign_error_message: None,
            view_wallet_unlock: false,
            wallet_open: false,
            view_private_key_even_if_encrypted_or_in_wallet: false,
        }
    }

    fn validate_and_store_private_key(&mut self) {
        // Convert the input string to bytes (hex decoding)
        let private_key_bytes = match hex::decode(&self.private_key_input) {
            Ok(private_key_bytes_vec) if private_key_bytes_vec.len() == 32 => {
                private_key_bytes_vec.try_into().unwrap()
            }
            Ok(_) => {
                self.error_message = Some("Private key not 32 bytes".to_string());
                return;
            }
            Err(_) => match PrivateKey::from_wif(&self.private_key_input) {
                Ok(key) => key.inner.secret_bytes(),
                Err(_) => {
                    self.error_message =
                        Some("Invalid hex string or WIF for private key.".to_string());
                    return;
                }
            },
        };

        let validation_result = self
            .key
            .validate_private_key_bytes(&private_key_bytes, self.app_context.network);
        if let Err(err) = validation_result {
            self.error_message = Some(format!("Issue verifying private key {}", err));
        } else if validation_result.unwrap() {
            // If valid, store the private key in the context and reset the input field
            self.private_key_data = Some((PrivateKeyData::Clear(private_key_bytes), None));
            self.identity.private_keys.insert_non_encrypted(
                (self.key.purpose().into(), self.key.id()),
                (self.key.clone().into(), private_key_bytes),
            );
            match self
                .app_context
                .insert_local_qualified_identity(&self.identity, None)
            {
                Ok(_) => {
                    self.error_message = None;
                }
                Err(e) => {
                    self.error_message = Some(format!("Issue saving: {}", e));
                }
            }
        } else {
            self.error_message = Some("Private key does not match the public key.".to_string());
        }
    }

    fn render_sign_input(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.heading("Sign Message");

        ui.label("Enter message to sign:");
        ui.add(
            egui::TextEdit::multiline(&mut self.message_input)
                .desired_width(f32::INFINITY)
                .desired_rows(3),
        );

        if ui.button("Sign Message").clicked() {
            // Attempt to sign the message
            self.sign_message();
        }

        if let Some(error_message) = &self.sign_error_message {
            ui.colored_label(egui::Color32::RED, error_message);
        }

        if let Some(signed_message) = &self.signed_message {
            ui.separator();
            ui.label("Signed Message (Base64):");
            ui.add(
                egui::TextEdit::multiline(&mut signed_message.as_str().to_owned())
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
        }
    }

    fn sign_message(&mut self) {
        // Check that we have a private key
        if let Some((private_key_data, _)) = &self.private_key_data {
            let private_key_bytes = match (private_key_data, self.decrypted_private_key.as_ref()) {
                (PrivateKeyData::Clear(bytes), _) | (PrivateKeyData::AlwaysClear(bytes), _) => {
                    *bytes
                }
                (_, Some(private_key)) => private_key.inner.secret_bytes(),
                // Other cases may not have the private key directly
                _ => {
                    self.sign_error_message = Some("Private key is not available.".to_string());
                    return;
                }
            };

            // Use the key type to determine how to sign
            match self.key.key_type() {
                KeyType::ECDSA_SECP256K1 | KeyType::ECDSA_HASH160 => {
                    // Sign the message using ECDSA
                    let secp = Secp256k1::new();

                    let message_hash = signed_msg_hash(self.message_input.as_str());
                    let message = Message::from_digest(*message_hash.as_byte_array());

                    let secret_key = SecretKey::from_byte_array(&private_key_bytes).unwrap();

                    let signature = secp.sign_ecdsa(&message, &secret_key);

                    // Serialize the signature
                    let mut serialized_signature = signature.serialize_compact().to_vec();
                    serialized_signature.insert(0, 32);

                    // Encode to Base64
                    let signature_base64 = STANDARD.encode(serialized_signature);

                    self.signed_message = Some(signature_base64);
                    self.sign_error_message = None;
                }
                _ => {
                    self.sign_error_message = Some("Unsupported key type for signing.".to_string());
                }
            }
        } else {
            self.sign_error_message = Some("Private key is not available.".to_string());
        }
    }
}

impl ScreenWithWalletUnlock for KeyInfoScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.selected_wallet
    }

    fn wallet_password_ref(&self) -> &String {
        &self.wallet_password
    }

    fn wallet_password_mut(&mut self) -> &mut String {
        &mut self.wallet_password
    }

    fn show_password(&self) -> bool {
        self.show_password
    }

    fn show_password_mut(&mut self) -> &mut bool {
        &mut self.show_password
    }

    fn set_error_message(&mut self, error_message: Option<String>) {
        self.error_message = error_message;
    }

    fn error_message(&self) -> Option<&String> {
        self.error_message.as_ref()
    }
}
