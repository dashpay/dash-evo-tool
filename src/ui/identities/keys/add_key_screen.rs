use crate::app::AppAction;
use crate::backend_task::identity::IdentityTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::fee_estimation::{PlatformFeeEstimator, format_credits_as_dash};
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::identities::get_selected_wallet;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike};
use bip39::rand::{SeedableRng, rngs::StdRng};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::hash::IdentityPublicKeyHashMethodsV0;
use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
use dash_sdk::dpp::identity::identity_public_key::v0::IdentityPublicKeyV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::Identifier;
use dash_sdk::dpp::prelude::TimestampMillis;
use eframe::egui::{self, Context, Frame, Margin};
use egui::{Color32, RichText, Ui};
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
    wallet_unlock_popup: WalletUnlockPopup,
    error_message: Option<String>,
    contract_id_input: String,
    document_type_input: String,
    enable_contract_bounds: bool,
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
            wallet_unlock_popup: WalletUnlockPopup::new(),
            error_message,
            contract_id_input: String::new(),
            document_type_input: String::new(),
            enable_contract_bounds: false,
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
                    // Handle contract bounds if enabled
                    let contract_bounds = if self.enable_contract_bounds
                        && !self.contract_id_input.is_empty()
                    {
                        match Identifier::from_string(&self.contract_id_input, Encoding::Base58) {
                            Ok(contract_id) => {
                                if self.document_type_input.is_empty() {
                                    Some(ContractBounds::SingleContract { id: contract_id })
                                } else {
                                    Some(ContractBounds::SingleContractDocumentType {
                                        id: contract_id,
                                        document_type_name: self.document_type_input.clone(),
                                    })
                                }
                            }
                            Err(e) => {
                                self.add_key_status = AddKeyStatus::ErrorMessage(format!(
                                    "Invalid contract ID: {}",
                                    e
                                ));
                                return app_action;
                            }
                        }
                    } else {
                        None
                    };

                    let new_key = IdentityPublicKeyV0 {
                        id: self.identity.identity.get_public_key_max_id() + 1,
                        key_type: self.key_type,
                        purpose: self.purpose,
                        security_level: self.security_level,
                        data: public_key_data_result.unwrap().into(),
                        read_only: false,
                        disabled_at: None,
                        contract_bounds,
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
                                new_qualified_key,
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
        let action = crate::ui::helpers::show_success_screen(
            ui,
            "Successfully added key.".to_string(),
            vec![
                (
                    "Back to Identities Screen".to_string(),
                    AppAction::PopScreenAndRefresh,
                ),
                (
                    "Add another key".to_string(),
                    AppAction::Custom("add_another".to_string()),
                ),
            ],
        );

        // Handle the custom action to reset the form and refresh identity
        if let AppAction::Custom(ref s) = action
            && s == "add_another"
        {
            self.private_key_input = String::new();
            self.contract_id_input = String::new();
            self.document_type_input = String::new();
            self.enable_contract_bounds = false;
            self.add_key_status = AddKeyStatus::NotStarted;
            return AppAction::BackendTask(BackendTask::IdentityTask(
                IdentityTask::RefreshIdentity(self.identity.clone()),
            ));
        }

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
        if let MessageType::Error = message_type {
            self.add_key_status = AddKeyStatus::ErrorMessage(message.to_string());
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::AddedKeyToIdentity => {
                self.add_key_status = AddKeyStatus::Complete;
            }
            BackendTaskSuccessResult::RefreshedIdentity(_) => {
                self.refresh();
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
                ("Add Key", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenIdentities,
        );

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            // Show the success screen if the key was added successfully
            if self.add_key_status == AddKeyStatus::Complete {
                inner_action |= self.show_success(ui);
                return inner_action;
            }

            ui.heading("Add New Key");
            ui.add_space(10.0);

            if self.add_key_status == AddKeyStatus::Complete {
                inner_action |= self.show_success(ui);
                return inner_action;
            }

            if self.selected_wallet.is_some()
                && let Some(wallet) = &self.selected_wallet
            {
                if let Err(e) = try_open_wallet_no_password(wallet) {
                    self.error_message = Some(e);
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
                    return inner_action;
                }
            }

            egui::Grid::new("add_key_grid")
                .num_columns(2)
                .spacing([10.0, 10.0])
                .striped(false)
                .show(ui, |ui| {
                    // Purpose
                    ui.label("Purpose:");
                    egui::ComboBox::from_id_salt("purpose_selector")
                        .selected_text(format!("{:?}", self.purpose))
                        .show_ui(ui, |ui| {
                            if self.enable_contract_bounds {
                                // When contract bounds are enabled, only allow ENCRYPTION and DECRYPTION
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::ENCRYPTION,
                                    "ENCRYPTION",
                                );
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::DECRYPTION,
                                    "DECRYPTION",
                                );
                            } else {
                                // When contract bounds are disabled, show all purpose options
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::AUTHENTICATION,
                                    "AUTHENTICATION",
                                );
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::TRANSFER,
                                    "TRANSFER",
                                );
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::ENCRYPTION,
                                    "ENCRYPTION",
                                );
                                ui.selectable_value(
                                    &mut self.purpose,
                                    Purpose::DECRYPTION,
                                    "DECRYPTION",
                                );
                            }
                        });
                    ui.end_row();

                    // Security Level
                    ui.label("Security Level:");
                    egui::ComboBox::from_id_salt("security_level_selector")
                        .selected_text(format!("{:?}", self.security_level))
                        .show_ui(ui, |ui| {
                            if self.enable_contract_bounds {
                                // When contract bounds are enabled, only allow MEDIUM
                                ui.selectable_value(
                                    &mut self.security_level,
                                    SecurityLevel::MEDIUM,
                                    "MEDIUM",
                                );
                            } else if self.purpose == Purpose::AUTHENTICATION {
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

                    // Contract Bounds Toggle
                    ui.label("Enable Contract Bounds:");
                    let prev_contract_bounds = self.enable_contract_bounds;
                    ui.checkbox(&mut self.enable_contract_bounds, "");

                    // If contract bounds was just enabled, set required values
                    if self.enable_contract_bounds && !prev_contract_bounds {
                        self.purpose = Purpose::ENCRYPTION;
                        self.security_level = SecurityLevel::MEDIUM;
                    }
                    ui.end_row();

                    // Contract ID Input (only shown if contract bounds are enabled)
                    if self.enable_contract_bounds {
                        ui.label("Contract ID:");
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut self.contract_id_input);
                            ui.label(RichText::new("(required)").size(10.0).color(Color32::GRAY));
                        });
                        ui.end_row();

                        // Document Type Input
                        ui.label("Document Type Name:");
                        ui.horizontal(|ui| {
                            ui.text_edit_singleline(&mut self.document_type_input);
                            ui.label(RichText::new("(optional)").size(10.0).color(Color32::GRAY));
                        });
                        ui.end_row();
                    }
                });
            ui.add_space(20.0);

            // Fee estimation display
            let fee_estimator = PlatformFeeEstimator::new();
            let estimated_fee = fee_estimator.estimate_identity_update();

            let dark_mode = ui.ctx().style().visuals.dark_mode;
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Estimated fee:")
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.label(
                            RichText::new(format_credits_as_dash(estimated_fee))
                                .color(DashColors::text_primary(dark_mode))
                                .size(14.0),
                        );
                    });
                });

            ui.add_space(10.0);

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
                inner_action |= self.validate_and_add_key();
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
                    let error_color = Color32::from_rgb(255, 100, 100);
                    let msg = msg.clone();
                    Frame::new()
                        .fill(error_color.gamma_multiply(0.1))
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .stroke(egui::Stroke::new(1.0, error_color))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!("Error: {}", msg)).color(error_color),
                                );
                                ui.add_space(10.0);
                                if ui.small_button("Dismiss").clicked() {
                                    self.add_key_status = AddKeyStatus::NotStarted;
                                }
                            });
                        });
                }
                AddKeyStatus::Complete => {
                    // handled above
                }
            }

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

        action
    }
}
