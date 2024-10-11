use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::ScreenLike;
use dash_sdk::dpp::dashcore::address::Payload;
use dash_sdk::dpp::dashcore::hashes::Hash;
use dash_sdk::dpp::dashcore::{Address, PubkeyHash, ScriptHash};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::KeyType::BIP13_SCRIPT_HASH;
use dash_sdk::dpp::identity::KeyType;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::IdentityPublicKey;
use eframe::egui::{self, Context};
use egui::{RichText, TextEdit};
use std::sync::Arc;

pub struct KeyInfoScreen {
    pub identity: QualifiedIdentity,
    pub key: IdentityPublicKey,
    pub private_key_bytes: Option<Vec<u8>>,
    pub app_context: Arc<AppContext>,
    private_key_input: String,
    error_message: Option<String>,
}

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
            None,
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
            if let Some(private_key) = &self.private_key_bytes {
                ui.label("Private Key:");
                let private_key_hex = hex::encode(private_key);
                ui.add(
                    TextEdit::multiline(&mut private_key_hex.as_str().to_owned())
                        .desired_width(f32::INFINITY),
                );
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
        });

        action
    }
}

impl KeyInfoScreen {
    pub fn new(
        identity: QualifiedIdentity,
        key: IdentityPublicKey,
        private_key_bytes: Option<Vec<u8>>,
        app_context: &Arc<AppContext>,
    ) -> Self {
        Self {
            identity,
            key,
            private_key_bytes,
            app_context: app_context.clone(),
            private_key_input: String::new(),
            error_message: None,
        }
    }

    fn validate_and_store_private_key(&mut self) {
        // Convert the input string to bytes (hex decoding)
        match hex::decode(&self.private_key_input) {
            Ok(private_key_bytes) => {
                let validation_result = self
                    .key
                    .validate_private_key_bytes(&private_key_bytes, self.app_context.network);
                if let Err(err) = validation_result {
                    self.error_message = Some(format!("Issue verifying private key {}", err));
                } else if validation_result.unwrap() {
                    // If valid, store the private key in the context and reset the input field
                    self.private_key_bytes = Some(private_key_bytes.clone());
                    self.identity.encrypted_private_keys.insert(
                        (self.key.purpose().into(), self.key.id()),
                        (self.key.clone(), private_key_bytes),
                    );
                    match self
                        .app_context
                        .insert_local_qualified_identity(&self.identity)
                    {
                        Ok(_) => {
                            self.error_message = None;
                        }
                        Err(e) => {
                            self.error_message = Some(format!("Issue saving: {}", e));
                        }
                    }
                } else {
                    self.error_message =
                        Some("Private key does not match the public key.".to_string());
                }
            }
            Err(_) => {
                self.error_message = Some("Invalid hex string for private key.".to_string());
            }
        }
    }
}
