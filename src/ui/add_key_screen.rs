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
use dash_sdk::dpp::prelude::TimestampMillis;
use eframe::egui::{self, Context};
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub enum AddKeyStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct AddKeyScreen {
    pub identity: QualifiedIdentity,
    pub app_context: Arc<AppContext>,
    private_key_input: String,
    key_type: KeyType,
    purpose: Purpose,
    security_level: SecurityLevel,
    add_key_status: AddKeyStatus,
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
            add_key_status: AddKeyStatus::NotStarted,
        }
    }

    fn validate_and_add_key(&mut self) -> AppAction {
        let mut app_action = AppAction::None;
        // Convert the input string to bytes (hex decoding)
        match hex::decode(&self.private_key_input) {
            Ok(private_key_bytes_vec) if private_key_bytes_vec.len() == 32 => {
                let private_key_bytes = private_key_bytes_vec.try_into().unwrap();
                let public_key_data_result = self.key_type.public_key_data_from_private_key_data(
                    &private_key_bytes,
                    self.app_context.network,
                );
                if let Err(err) = public_key_data_result {
                    self.add_key_status =
                        AddKeyStatus::ErrorMessage(format!("Issue verifying private key: {}", err));
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
                        self.add_key_status = AddKeyStatus::ErrorMessage(format!(
                            "Issue verifying private key: {}",
                            err
                        ));
                    } else if validation_result.unwrap() {
                        app_action = AppAction::BackendTask(BackendTask::IdentityTask(
                            IdentityTask::AddKeyToIdentity(
                                self.identity.clone(),
                                new_key.into(),
                                private_key_bytes,
                            ),
                        ));
                    } else {
                        self.add_key_status = AddKeyStatus::ErrorMessage(
                            "Private key does not match the public key.".to_string(),
                        );
                    }
                }
            }
            Ok(_) => {
                self.add_key_status =
                    AddKeyStatus::ErrorMessage("Private key not 32 bytes".to_string());
            }
            Err(_) => {
                self.add_key_status =
                    AddKeyStatus::ErrorMessage("Invalid hex string for private key.".to_string());
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
            self.add_key_status =
                AddKeyStatus::ErrorMessage("Failed to generate a random private key.".to_string());
        }
    }
}

impl ScreenLike for AddKeyScreen {
    fn refresh(&mut self) {}

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if message_type == MessageType::Success && message == "Successfully added key to identity" {
            self.add_key_status = AddKeyStatus::Complete;
        } else {
            self.add_key_status = AddKeyStatus::ErrorMessage(message.to_string());
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
            vec![],
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
                // Set the status to waiting and capture the current time
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                self.add_key_status = AddKeyStatus::WaitingForResult(now);
                action |= self.validate_and_add_key();
            }

            match &self.add_key_status {
                AddKeyStatus::NotStarted => {
                    // Do nothing
                }
                AddKeyStatus::WaitingForResult(start_time) => {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs();
                    let elapsed_seconds = now - start_time;

                    let display_time = if elapsed_seconds < 60 {
                        format!(
                            "{} second{}",
                            elapsed_seconds,
                            if elapsed_seconds == 1 { "" } else { "s" }
                        )
                    } else {
                        let minutes = elapsed_seconds / 60;
                        let seconds = elapsed_seconds % 60;
                        format!(
                            "{} minute{} and {} second{}",
                            minutes,
                            if minutes == 1 { "" } else { "s" },
                            seconds,
                            if seconds == 1 { "" } else { "s" }
                        )
                    };

                    ui.label(format!("Loading... Time taken so far: {}", display_time));
                }
                AddKeyStatus::ErrorMessage(msg) => {
                    ui.colored_label(egui::Color32::RED, format!("Error: {}", msg));
                }
                AddKeyStatus::Complete => {
                    action = AppAction::PopScreenAndRefresh;
                }
            }
        });

        action
    }
}
