use crate::app::AppAction;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::get_selected_wallet;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::prelude::TimestampMillis;
use eframe::egui::{self, Context};
use egui::{Color32, RichText, Ui};
use rand::rngs::StdRng;
use rand::SeedableRng;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(PartialEq)]
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
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
}

impl AddKeyScreen {
    pub fn new(identity: QualifiedIdentity, app_context: &Arc<AppContext>) -> Self {
        let identity_clone = identity.clone();
        let selected_key = identity_clone.identity.get_first_public_key_matching(
            Purpose::AUTHENTICATION,
            HashSet::from([SecurityLevel::MASTER]),
            KeyType::all_key_types().into(),
            false,
        );
        let mut error_message = None;
        let selected_wallet =
            get_selected_wallet(&identity, None, selected_key, &mut error_message);

        Self {
            identity,
            app_context: app_context.clone(),
            private_key_input: String::new(),
            key_type: KeyType::ECDSA_SECP256K1,
            purpose: Purpose::AUTHENTICATION,
            security_level: SecurityLevel::HIGH,
            add_key_status: AddKeyStatus::NotStarted,
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
            error_message,
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
                        let new_qualified_key = QualifiedIdentityPublicKey {
                            identity_public_key: new_key.into(),
                            in_wallet_at_derivation_path: None,
                        };
                        app_action = AppAction::BackendTask(BackendTask::IdentityTask(
                            IdentityTask::AddKeyToIdentity(
                                self.identity.clone(),
                                new_qualified_key.into(),
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
            .random_public_and_private_key_data(&mut rng, self.app_context.platform_version())
        {
            self.private_key_input = hex::encode(private_key_bytes);
        } else {
            self.add_key_status =
                AddKeyStatus::ErrorMessage("Failed to generate a random private key.".to_string());
        }
    }

    pub fn show_success(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Center the content vertically and horizontally
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Successfully added key.");

            ui.add_space(20.0);

            if ui.button("Back to Identities Screen").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
            ui.add_space(5.0);

            if ui.button("Add another key").clicked() {
                action = AppAction::BackendTask(BackendTask::IdentityTask(
                    IdentityTask::RefreshIdentity(self.identity.clone()),
                ));
                self.private_key_input = String::new();
                self.add_key_status = AddKeyStatus::NotStarted;
            }
        });

        action
    }
}

impl ScreenLike for AddKeyScreen {
    fn refresh(&mut self) {
        if let Some(refreshed_identity) = self
            .app_context
            .load_local_user_identities()
            .expect("Expected to load local identities")
            .iter()
            .find(|identity| identity.identity.id() == self.identity.identity.id())
        {
            self.identity = refreshed_identity.clone();
        }
    }

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message == "Successfully added key to identity" {
                    self.add_key_status = AddKeyStatus::Complete;
                }
                if message == "Successfully refreshed identity" {
                    self.refresh();
                }
            }
            MessageType::Info => {}
            MessageType::Error => {
                // It's not great because the error message can be coming from somewhere else if there are other processes happening
                self.add_key_status = AddKeyStatus::ErrorMessage(message.to_string());
            }
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

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentities,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            // Show the success screen if the key was added successfully
            if self.add_key_status == AddKeyStatus::Complete {
                action |= self.show_success(ui);
                return;
            }

            ui.heading("Add New Key");
            ui.add_space(10.0);

            if self.add_key_status == AddKeyStatus::Complete {
                action |= self.show_success(ui);
                return;
            }

            if self.selected_wallet.is_some() {
                let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                if needed_unlock && !just_unlocked {
                    return;
                }
            }

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
                            // ui.selectable_value(
                            //     &mut self.key_type,
                            //     KeyType::BIP13_SCRIPT_HASH,
                            //     "BIP13_SCRIPT_HASH",
                            // );
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
            ui.add_space(20.0);

            // Add Key button
            let mut new_style = (**ui.style()).clone();
            new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
            ui.set_style(new_style);
            let button = egui::Button::new(RichText::new("Add Key").color(Color32::WHITE))
                .fill(Color32::from_rgb(0, 128, 255))
                .frame(true)
                .corner_radius(3.0);
            if ui.add(button).clicked() {
                // Set the status to waiting and capture the current time
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                self.add_key_status = AddKeyStatus::WaitingForResult(now);
                action |= self.validate_and_add_key();
            }
            ui.add_space(10.0);

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

                    ui.label(format!("Adding key... Time taken so far: {}", display_time));
                }
                AddKeyStatus::ErrorMessage(msg) => {
                    ui.colored_label(egui::Color32::DARK_RED, format!("Error: {}", msg));
                }
                AddKeyStatus::Complete => {
                    // handled above
                }
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for AddKeyScreen {
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
