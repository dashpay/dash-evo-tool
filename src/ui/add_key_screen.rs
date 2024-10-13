use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::platform::identity::IdentityTask;
use crate::platform::BackendTask;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use eframe::egui::{self, Context};
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::sync::Arc;
use dash_sdk::dpp::platform_value::Value;

pub struct AddKeyScreen {
    pub identity: QualifiedIdentity,
    pub app_context: Arc<AppContext>,
    private_key_input: String,
    key_type: KeyType,
    purpose: Purpose,
    security_level: SecurityLevel,
    error_message: Option<String>,
}

impl ScreenLike for AddKeyScreen {
    fn refresh(&mut self) {}

    fn display_message(&mut self, message: Value, message_type: MessageType) {
        if let Some(message) = message.as_str() {
            self.error_message = Some(message.to_string());
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Identities", AppAction::GoToMainScreen),
                ("Add Key", AppAction::None),
            ],
            None,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Add New Key");

            egui::Grid::new("add_key_grid")
                .num_columns(2)
                .spacing([10.0, 10.0])
                .striped(true)
                .show(ui, |ui| {
                    // Purpose
                    ui.label("Purpose:");
                    egui::ComboBox::from_id_salt("purpose_selector")
                        .selected_text(format!("{:?}", self.purpose))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.purpose,
                                Purpose::AUTHENTICATION,
                                "AUTHENTICATION",
                            );
                            ui.selectable_value(&mut self.purpose, Purpose::TRANSFER, "TRANSFER");
                        });
                    ui.end_row();

                    // Security Level
                    ui.label("Security Level:");
                    egui::ComboBox::from_id_salt("security_level_selector")
                        .selected_text(format!("{:?}", self.security_level))
                        .show_ui(ui, |ui| {
                            if self.purpose == Purpose::AUTHENTICATION {
                                ui.selectable_value(
                                    &mut self.security_level,
                                    SecurityLevel::CRITICAL,
                                    "CRITICAL",
                                );
                                ui.selectable_value(
                                    &mut self.security_level,
                                    SecurityLevel::HIGH,
                                    "HIGH",
                                );
                                ui.selectable_value(
                                    &mut self.security_level,
                                    SecurityLevel::MEDIUM,
                                    "MEDIUM",
                                );
                            } else {
                                ui.selectable_value(
                                    &mut self.security_level,
                                    SecurityLevel::CRITICAL,
                                    "CRITICAL",
                                );
                            }
                        });
                    ui.end_row();

                    // Key Type
                    ui.label("Key Type:");
                    egui::ComboBox::from_id_salt("key_type_selector")
                        .selected_text(format!("{:?}", self.key_type))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.key_type,
                                KeyType::ECDSA_SECP256K1,
                                "ECDSA_SECP256K1",
                            );
                            ui.selectable_value(
                                &mut self.key_type,
                                KeyType::BLS12_381,
                                "BLS12_381",
                            );
                            ui.selectable_value(
                                &mut self.key_type,
                                KeyType::ECDSA_HASH160,
                                "ECDSA_HASH160",
                            );
                            ui.selectable_value(
                                &mut self.key_type,
                                KeyType::EDDSA_25519_HASH160,
                                "EDDSA_25519_HASH160",
                            );
                            ui.selectable_value(
                                &mut self.key_type,
                                KeyType::BIP13_SCRIPT_HASH,
                                "BIP13_SCRIPT_HASH",
                            );
                        });
                    ui.end_row();

                    // Private Key Input
                    ui.label("Private Key:");
                    ui.text_edit_singleline(&mut self.private_key_input);
                    if ui.button("Generate Random").clicked() {
                        self.generate_random_private_key();
                    }
                    ui.end_row();
                });

            ui.separator();

            if ui.button("Add Key").clicked() {
                action |= self.validate_and_add_key();
            }

            // Display error message if validation fails
            if let Some(error_message) = &self.error_message {
                ui.colored_label(egui::Color32::RED, error_message);
            }
        });

        action
    }
}

impl AddKeyScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        Self {
            identity,
            app_context: app_context.clone(),
            private_key_input: String::new(),
            key_type: KeyType::ECDSA_SECP256K1,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::HIGH,
            error_message: None,
        }
    }

    fn validate_and_add_key(&mut self) -> AppAction {
        let mut app_action = AppAction::None;
        // Convert the input string to bytes (hex decoding)
        match hex::decode(&self.private_key_input) {
            Ok(private_key_bytes) => {
                let public_key_data_result = self.key_type.public_key_data_from_private_key_data(
                    &private_key_bytes,
                    self.app_context.network,
                );
                if let Err(err) = public_key_data_result {
                    self.error_message = Some(format!("Issue verifying private key: {}", err));
                } else {
                    let new_key = IdentityPublicKeyV0 {
                        id: self.identity.identity.get_public_key_max_id() + 1,
                        key_type: self.key_type,
                        purpose: self.purpose,
                        security_level: self.security_level,
                        data: public_key_data_result.unwrap().into(),
                        read_only: false,
                        disabled_at: None,
                        contract_bounds: None,
                    };

                    // Validate the private key against the public key
                    let validation_result = new_key
                        .validate_private_key_bytes(&private_key_bytes, self.app_context.network);
                    if let Err(err) = validation_result {
                        self.error_message = Some(format!("Issue verifying private key: {}", err));
                    } else if validation_result.unwrap() {
                        app_action = AppAction::BackendTask(BackendTask::IdentityTask(
                            IdentityTask::AddKeyToIdentity(
                                self.identity.clone(),
                                new_key.into(),
                                private_key_bytes,
                            ),
                        ));
                    } else {
                        self.error_message =
                            Some("Private key does not match the public key.".to_string());
                    }
                }
            }
            Err(_) => {
                self.error_message = Some("Invalid hex string for private key.".to_string());
            }
        }
        app_action
    }

    fn generate_random_private_key(&mut self) {
        // Create a new random number generator
        let mut rng = StdRng::from_entropy();

        // Generate a random private key based on the selected key type
        if let Ok((_, private_key_bytes)) = self
            .key_type
            .random_public_and_private_key_data(&mut rng, self.app_context.platform_version)
        {
            self.private_key_input = hex::encode(private_key_bytes);
        } else {
            self.error_message = Some("Failed to generate a random private key.".to_string());
        }
    }
}
