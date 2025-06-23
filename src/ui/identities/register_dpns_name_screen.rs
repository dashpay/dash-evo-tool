use crate::app::AppAction;
use crate::backend_task::identity::{IdentityTask, RegisterDpnsNameInput};
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::helpers::{add_identity_key_chooser_with_doc_type, TransactionType};
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Purpose, TimestampMillis};
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::Context;
use egui::{Color32, RichText, Ui};
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use super::get_selected_wallet;

#[derive(PartialEq)]
pub enum RegisterDpnsNameStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    ErrorMessage(String),
    Complete,
}

pub struct RegisterDpnsNameScreen {
    pub show_identity_selector: bool,
    pub qualified_identities: Vec<QualifiedIdentity>,
    pub selected_qualified_identity: Option<QualifiedIdentity>,
    pub selected_key: Option<IdentityPublicKey>,
    name_input: String,
    register_dpns_name_status: RegisterDpnsNameStatus,
    pub app_context: Arc<AppContext>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
}

impl RegisterDpnsNameScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let qualified_identities: Vec<_> =
            app_context.load_local_user_identities().unwrap_or_default();
        let selected_qualified_identity = qualified_identities.first().cloned();

        let mut error_message: Option<String> = None;
        let selected_wallet = if let Some(ref identity) = selected_qualified_identity {
            get_selected_wallet(identity, Some(app_context), None, &mut error_message)
        } else {
            None
        };

        let show_identity_selector = qualified_identities.len() > 1;
        Self {
            show_identity_selector,
            qualified_identities,
            selected_qualified_identity,
            selected_key: None,
            name_input: String::new(),
            register_dpns_name_status: RegisterDpnsNameStatus::NotStarted,
            app_context: app_context.clone(),
            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
            error_message,
        }
    }

    pub fn select_identity(&mut self, identity_id: Identifier) {
        // Find the qualified identity with the matching identity_id
        if let Some(qi) = self
            .qualified_identities
            .iter()
            .find(|qi| qi.identity.id() == identity_id)
        {
            // Set the selected_qualified_identity to the found identity
            self.selected_qualified_identity = Some(qi.clone());
            self.selected_key = None; // Reset key selection
                                      // Update the selected wallet
            self.selected_wallet =
                get_selected_wallet(qi, Some(&self.app_context), None, &mut self.error_message);
        } else {
            // If not found, you might want to handle this case
            // For now, we'll set selected_qualified_identity to None
            self.selected_qualified_identity = None;
            self.selected_key = None;
            self.selected_wallet = None;
        }
    }

    fn render_identity_id_selection(&mut self, ui: &mut egui::Ui) {
        add_identity_key_chooser_with_doc_type(
            ui,
            &self.app_context,
            self.qualified_identities.iter(),
            &mut self.selected_qualified_identity,
            &mut self.selected_key,
            TransactionType::DocumentAction,
            self.app_context
                .dpns_contract
                .document_type_cloned_for_name("domain")
                .ok()
                .as_ref(),
        );
    }

    fn register_dpns_name_clicked(&mut self) -> AppAction {
        let Some(qualified_identity) = self.selected_qualified_identity.as_ref() else {
            return AppAction::None;
        };
        let Some(_selected_key) = self.selected_key.as_ref() else {
            return AppAction::None;
        };
        let dpns_name_input = RegisterDpnsNameInput {
            qualified_identity: qualified_identity.clone(),
            name_input: self.name_input.trim().to_string(),
        };

        AppAction::BackendTask(BackendTask::IdentityTask(IdentityTask::RegisterDpnsName(
            dpns_name_input,
        )))
    }

    pub fn show_success(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Center the content vertically and horizontally
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Successfully registered DPNS name.");

            ui.add_space(20.0);

            if ui.button("Back to DPNS screen").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
            ui.add_space(5.0);

            if ui.button("Register another name").clicked() {
                self.name_input = String::new();
                self.register_dpns_name_status = RegisterDpnsNameStatus::NotStarted;
            }
        });

        action
    }
}

impl ScreenLike for RegisterDpnsNameScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message == "Successfully registered dpns name" {
                    self.register_dpns_name_status = RegisterDpnsNameStatus::Complete;
                }
            }
            MessageType::Info => {}
            MessageType::Error => {
                // It's not great because the error message can be coming from somewhere else if there are other processes happening
                self.register_dpns_name_status =
                    RegisterDpnsNameStatus::ErrorMessage(message.to_string());
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("DPNS", AppAction::GoToMainScreen),
                ("Register Name", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenDPNSOwnedNames,
        );

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    if self.register_dpns_name_status == RegisterDpnsNameStatus::Complete {
                        inner_action |= self.show_success(ui);
                        return;
                    }

                    ui.heading("Register DPNS Name");
                    ui.add_space(10.0);

            // If no identities loaded, give message
            if self.qualified_identities.is_empty() {
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    "No identities loaded. Please load an identity first.",
                );
                return;
            }

            // Check if any identity has suitable private keys for DPNS registration
            let has_suitable_keys = self.qualified_identities.iter().any(|qi| {
                qi.private_keys.identity_public_keys().iter().any(|key_ref| {
                    let key = &key_ref.1.identity_public_key;
                    // DPNS registration requires Authentication keys
                    key.purpose() == Purpose::AUTHENTICATION
                })
            });

            if !has_suitable_keys {
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    "No identities with authentication private keys loaded. Please load identity keys to register a DPNS name.",
                );
                return;
            }

            // Select the identity to register the name for
            ui.heading("1. Select Identity");
            ui.add_space(5.0);
            self.render_identity_id_selection(ui);
            ui.add_space(5.0);
            if let Some(identity) = &self.selected_qualified_identity {
                ui.label(format!("Identity balance: {:.6}", identity.identity.balance() as f64 * 1e-11));
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            if self.selected_wallet.is_some() {
                let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                if needed_unlock && !just_unlocked {
                    return;
                }
            }

            // Input for the name
            ui.heading("2. Enter the Name to Register:");
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.label("Name (without \".dash\"):");
                ui.text_edit_singleline(&mut self.name_input);
            });

            // Display validation status and cost information
            let name = self.name_input.trim();
            if !name.is_empty() {
                ui.add_space(10.0);

                // Validate the name
                let validation_result = validate_dpns_name(name);

                match validation_result {
                    DpnsNameValidationResult::Valid => {
                        ui.colored_label(
                            egui::Color32::DARK_GREEN,
                            "Valid name format",
                        );

                        // Show contested status and cost if valid
                        if is_contested_name(&name.to_lowercase()) {
                            ui.colored_label(
                                egui::Color32::DARK_RED,
                                "This is a contested name.",
                            );
                            ui.colored_label(
                                egui::Color32::DARK_RED,
                                "Cost ≈ 0.2006 Dash",
                            );
                        } else {
                            ui.colored_label(
                                egui::Color32::DARK_GREEN,
                                "This is not a contested name.",
                            );
                            ui.colored_label(
                                egui::Color32::DARK_GREEN,
                                "Cost ≈ 0.0006 Dash",
                            );
                        }
                    }
                    _ => {
                        if let Some(error_msg) = validation_result.error_message() {
                            ui.colored_label(
                                egui::Color32::RED,
                                format!("{}", error_msg),
                            );
                        }
                    }
                }
            }

            ui.add_space(10.0);

            // Register button
            let mut new_style = (**ui.style()).clone();
            new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
            ui.set_style(new_style);
            let name_is_valid = validate_dpns_name(self.name_input.trim()) == DpnsNameValidationResult::Valid;
            let button_enabled = self.selected_qualified_identity.is_some() && self.selected_key.is_some() && name_is_valid;
            let button = egui::Button::new(RichText::new("Register Name").color(Color32::WHITE))
                .fill(Color32::from_rgb(0, 128, 255))
                .frame(true)
                .corner_radius(3.0);
            if ui.add_enabled(button_enabled, button).clicked() {
                // Set the status to waiting and capture the current time
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                self.register_dpns_name_status = RegisterDpnsNameStatus::WaitingForResult(now);
                inner_action = self.register_dpns_name_clicked();
            }

            ui.add_space(10.0);

            // Handle registration status messages
            match &self.register_dpns_name_status {
                RegisterDpnsNameStatus::NotStarted => {
                    // Do nothing
                }
                RegisterDpnsNameStatus::WaitingForResult(start_time) => {
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

                    ui.label(format!(
                        "Registering... Time taken so far: {}",
                        display_time
                    ));
                }
                RegisterDpnsNameStatus::ErrorMessage(msg) => {
                    ui.colored_label(egui::Color32::RED, format!("Error: {}", msg));
                }
                RegisterDpnsNameStatus::Complete => {}
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            // DPNS Name Constraints Explanation
            ui.heading("DPNS Name Constraints:");
            ui.add_space(5.0);
            ui.label("  • Minimum length: 3 characters");
            ui.label("  • Maximum length: 63 characters");
            ui.label("  • Allowed characters: letters (A-Z, case-insensitive), numbers (0-9), and hyphens (-)");
            ui.label("  • Cannot start or end with a hyphen (-)");
            ui.label("  • Names are case-sensitive");

            ui.add_space(20.0);

            // Contested Names Explanation
            ui.heading("Contested Names Info:");
            ui.add_space(5.0);
            ui.label("  • To prevent name front-running, some names are contested and require a higher fee to register.");
            ui.label("  • Masternodes vote whether or not to award contested names to contestants.");
            ui.label("  • Contests last two weeks and new contenders can only join during the first week.");
            ui.label("  • Contested names are those that are:");
            ui.label("  • Less than 20 characters long (i.e. “alice”, “quantumexplorer”)");
            ui.label("  • AND");
            ui.label("  • Contain no numbers or only contain the number(s) 0 and/or 1 (i.e. “bob”, “carol01”)");
                });
            inner_action
        });

        action
    }
}

impl ScreenWithWalletUnlock for RegisterDpnsNameScreen {
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

pub fn is_contested_name(name: &str) -> bool {
    let length = name.len();
    if length >= 20 {
        return false;
    }
    for c in name.chars() {
        if c.is_ascii_digit() && c != '0' && c != '1' {
            return false;
        }
    }
    true
}

#[derive(Debug, PartialEq)]
pub enum DpnsNameValidationResult {
    Valid,
    TooShort,
    TooLong,
    InvalidCharacter(char),
    StartsWithHyphen,
    EndsWithHyphen,
}

pub fn validate_dpns_name(name: &str) -> DpnsNameValidationResult {
    if name.len() < 3 {
        return DpnsNameValidationResult::TooShort;
    }
    
    if name.len() > 63 {
        return DpnsNameValidationResult::TooLong;
    }
    
    if name.starts_with('-') {
        return DpnsNameValidationResult::StartsWithHyphen;
    }
    
    if name.ends_with('-') {
        return DpnsNameValidationResult::EndsWithHyphen;
    }
    
    for c in name.chars() {
        if !c.is_ascii_alphanumeric() && c != '-' {
            return DpnsNameValidationResult::InvalidCharacter(c);
        }
    }
    
    DpnsNameValidationResult::Valid
}

impl DpnsNameValidationResult {
    pub fn error_message(&self) -> Option<String> {
        match self {
            DpnsNameValidationResult::Valid => None,
            DpnsNameValidationResult::TooShort => Some("Name must be at least 3 characters long".to_string()),
            DpnsNameValidationResult::TooLong => Some("Name must be no more than 63 characters long".to_string()),
            DpnsNameValidationResult::InvalidCharacter(c) => Some(format!("Invalid character '{}'. Only letters, numbers, and hyphens are allowed", c)),
            DpnsNameValidationResult::StartsWithHyphen => Some("Name cannot start with a hyphen".to_string()),
            DpnsNameValidationResult::EndsWithHyphen => Some("Name cannot end with a hyphen".to_string()),
        }
    }
}
