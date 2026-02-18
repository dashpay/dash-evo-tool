use crate::app::AppAction;
use crate::backend_task::identity::{IdentityTask, RegisterDpnsNameInput};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult, FeeResult};
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::helpers::{TransactionType, add_key_chooser_with_doc_type};
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Purpose, TimestampMillis};
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{Context, Frame, Margin};
use egui::{Color32, RichText, Ui};
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use super::get_selected_wallet;

/// Tracks where the user navigated from to reach this screen
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegisterDpnsNameSource {
    #[default]
    Dpns,
    Identities,
}

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
    selected_identity_string: String,
    pub selected_key: Option<IdentityPublicKey>,
    name_input: String,
    register_dpns_name_status: RegisterDpnsNameStatus,
    pub app_context: Arc<AppContext>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    error_message: Option<String>,
    show_advanced_options: bool,
    // Fee result from completed operation
    completed_fee_result: Option<FeeResult>,
    // Source of navigation to this screen
    pub source: RegisterDpnsNameSource,
}

#[cfg(feature = "e2e")]
impl RegisterDpnsNameScreen {
    pub fn set_name_input(&mut self, name: String) {
        self.name_input = name;
    }
}

impl RegisterDpnsNameScreen {
    pub fn new(app_context: &Arc<AppContext>, source: RegisterDpnsNameSource) -> Self {
        let qualified_identities: Vec<_> =
            app_context.load_local_user_identities().unwrap_or_default();
        let selected_qualified_identity = qualified_identities.first().cloned();

        let mut error_message: Option<String> = None;
        let selected_wallet = if let Some(ref identity) = selected_qualified_identity {
            get_selected_wallet(identity, Some(app_context), None, &mut error_message)
        } else {
            None
        };

        // Auto-select a suitable key for DPNS registration
        // Note: MASTER keys cannot be used for document operations,
        // only MEDIUM, HIGH, or CRITICAL security levels are allowed
        let selected_key = selected_qualified_identity.as_ref().and_then(|identity| {
            use dash_sdk::dpp::identity::{KeyType, SecurityLevel};
            identity
                .identity
                .get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    [
                        SecurityLevel::CRITICAL,
                        SecurityLevel::HIGH,
                        SecurityLevel::MEDIUM,
                    ]
                    .into(),
                    KeyType::all_key_types().into(),
                    false,
                )
                .cloned()
        });

        let selected_identity_string = selected_qualified_identity
            .as_ref()
            .map(|qi| {
                qi.identity
                    .id()
                    .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58)
            })
            .unwrap_or_default();

        let show_identity_selector = qualified_identities.len() > 1;
        Self {
            show_identity_selector,
            qualified_identities,
            selected_qualified_identity,
            selected_identity_string,
            selected_key,
            name_input: String::new(),
            register_dpns_name_status: RegisterDpnsNameStatus::NotStarted,
            app_context: app_context.clone(),
            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            error_message,
            show_advanced_options: false,
            completed_fee_result: None,
            source,
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
            self.selected_identity_string = qi
                .identity
                .id()
                .to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

            // Auto-select a suitable key for DPNS registration
            // Note: MASTER keys cannot be used for document operations,
            // only MEDIUM, HIGH, or CRITICAL security levels are allowed
            use dash_sdk::dpp::identity::{KeyType, SecurityLevel};
            self.selected_key = qi
                .identity
                .get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    [
                        SecurityLevel::CRITICAL,
                        SecurityLevel::HIGH,
                        SecurityLevel::MEDIUM,
                    ]
                    .into(),
                    KeyType::all_key_types().into(),
                    false,
                )
                .cloned();

            // Update the selected wallet
            self.selected_wallet =
                get_selected_wallet(qi, Some(&self.app_context), None, &mut self.error_message);
        } else {
            // If not found, you might want to handle this case
            // For now, we'll set selected_qualified_identity to None
            self.selected_qualified_identity = None;
            self.selected_identity_string = String::new();
            self.selected_key = None;
            self.selected_wallet = None;
        }
    }

    fn render_identity_id_selection(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;

        // Identity selector
        let response = ui.add(
            IdentitySelector::new(
                "dpns_register_identity_selector",
                &mut self.selected_identity_string,
                &self.qualified_identities,
            )
            .selected_identity(&mut self.selected_qualified_identity)
            .unwrap()
            .width(300.0)
            .label("Identity:")
            .other_option(false),
        );

        // Handle identity change - auto-select key and update wallet
        if response.changed() {
            if let Some(identity) = &self.selected_qualified_identity {
                // Auto-select a suitable key for DPNS registration
                // Note: MASTER keys cannot be used for document operations,
                // only MEDIUM, HIGH, or CRITICAL security levels are allowed
                use dash_sdk::dpp::identity::{KeyType, SecurityLevel};
                self.selected_key = identity
                    .identity
                    .get_first_public_key_matching(
                        Purpose::AUTHENTICATION,
                        [
                            SecurityLevel::CRITICAL,
                            SecurityLevel::HIGH,
                            SecurityLevel::MEDIUM,
                        ]
                        .into(),
                        KeyType::all_key_types().into(),
                        false,
                    )
                    .cloned();

                // Update wallet
                self.selected_wallet = get_selected_wallet(
                    identity,
                    Some(&self.app_context),
                    None,
                    &mut self.error_message,
                );
            } else {
                self.selected_key = None;
                self.selected_wallet = None;
            }
        }

        // Key selector (only shown in advanced mode)
        if self.show_advanced_options {
            ui.add_space(10.0);
            if let Some(identity) = &self.selected_qualified_identity {
                let key_action = add_key_chooser_with_doc_type(
                    ui,
                    &self.app_context,
                    identity,
                    &mut self.selected_key,
                    TransactionType::DocumentAction,
                    self.app_context
                        .dpns_contract
                        .document_type_cloned_for_name("domain")
                        .ok()
                        .as_ref(),
                );
                if !matches!(key_action, AppAction::None) {
                    action = key_action;
                }
            }
        }

        action
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
        let action = crate::ui::helpers::show_success_screen_with_info(
            ui,
            "DPNS Name Registered!".to_string(),
            vec![
                ("Back".to_string(), AppAction::PopScreenAndRefresh),
                (
                    "Register another name".to_string(),
                    AppAction::Custom("register_another".to_string()),
                ),
            ],
            None,
        );

        // Handle the custom action to reset the form
        if let AppAction::Custom(ref s) = action
            && s == "register_another"
        {
            self.name_input = String::new();
            self.register_dpns_name_status = RegisterDpnsNameStatus::NotStarted;
            self.completed_fee_result = None;
            return AppAction::None;
        }

        action
    }
}

impl ScreenLike for RegisterDpnsNameScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if let MessageType::Error = message_type {
            self.register_dpns_name_status =
                RegisterDpnsNameStatus::ErrorMessage(message.to_string());
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::RegisteredDpnsName(fee_result) =
            backend_task_success_result
        {
            self.completed_fee_result = Some(fee_result);
            self.register_dpns_name_status = RegisterDpnsNameStatus::Complete;
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        // Build breadcrumbs based on where we came from
        let breadcrumbs = match self.source {
            RegisterDpnsNameSource::Dpns => vec![
                (
                    "DPNS",
                    AppAction::SetMainScreen(
                        crate::ui::RootScreenType::RootScreenDPNSActiveContests,
                    ),
                ),
                ("Register Name", AppAction::None),
            ],
            RegisterDpnsNameSource::Identities => vec![
                (
                    "Identities",
                    AppAction::SetMainScreen(crate::ui::RootScreenType::RootScreenIdentities),
                ),
                ("Register Name", AppAction::None),
            ],
        };

        let mut action = add_top_panel(ctx, &self.app_context, breadcrumbs, vec![]);

        // Use the appropriate left panel highlight based on source
        let root_screen = match self.source {
            RegisterDpnsNameSource::Dpns => crate::ui::RootScreenType::RootScreenDPNSActiveContests,
            RegisterDpnsNameSource::Identities => crate::ui::RootScreenType::RootScreenIdentities,
        };
        action |= add_left_panel(ctx, &self.app_context, root_screen);

        // Don't show the tools/dpns subscreen chooser panels for this screen

        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    if self.register_dpns_name_status == RegisterDpnsNameStatus::Complete {
                        inner_action |= self.show_success(ui);
                        return;
                    }

                    ui.horizontal(|ui| {
                        ui.heading("Register DPNS Name");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                        });
                    });
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
            inner_action |= self.render_identity_id_selection(ui);
            ui.add_space(5.0);
            if let Some(identity) = &self.selected_qualified_identity {
                ui.label(format!("Identity balance: {:.6}", identity.identity.balance() as f64 * 1e-11));
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            if self.selected_wallet.is_some()
                && let Some(wallet) = &self.selected_wallet {
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
                        }
                    }
                    _ => {
                        if let Some(error_msg) = validation_result.error_message() {
                            ui.colored_label(
                                egui::Color32::RED,
                                error_msg,
                            );
                        }
                    }
                }
            }

            ui.add_space(10.0);

            // Fee estimation
            let fee_estimator = self.app_context.fee_estimator();
            let estimated_fee = fee_estimator.estimate_document_create();
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

            // Check if identity has enough balance
            let has_enough_balance = self
                .selected_qualified_identity
                .as_ref()
                .map(|id| id.identity.balance() > estimated_fee)
                .unwrap_or(false);

            // Register button
            let mut new_style = (**ui.style()).clone();
            new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
            ui.set_style(new_style);
            let name_is_valid = validate_dpns_name(self.name_input.trim()) == DpnsNameValidationResult::Valid;
            let button_enabled = self.selected_qualified_identity.is_some()
                && self.selected_key.is_some()
                && name_is_valid
                && has_enough_balance;

            let hover_text = if !has_enough_balance {
                format!(
                    "Insufficient identity balance for fee (need at least {})",
                    format_credits_as_dash(estimated_fee)
                )
            } else if !name_is_valid {
                "Please enter a valid name".to_string()
            } else if self.selected_key.is_none() {
                "Please select a signing key".to_string()
            } else {
                "Register DPNS name".to_string()
            };

            let button = egui::Button::new(RichText::new("Register Name").color(Color32::WHITE))
                .fill(if button_enabled {
                    DashColors::DASH_BLUE
                } else {
                    Color32::GRAY
                })
                .frame(true)
                .corner_radius(3.0);
            if ui
                .add_enabled(button_enabled, button)
                .on_hover_text(&hover_text)
                .on_disabled_hover_text(&hover_text)
                .clicked()
            {
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
                    let error_color = DashColors::ERROR;
                    let msg = msg.clone();
                    Frame::new()
                        .fill(error_color.gamma_multiply(0.1))
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .stroke(egui::Stroke::new(1.0, error_color))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("Error: {}", msg)).color(error_color));
                                ui.add_space(10.0);
                                if ui.small_button("Dismiss").clicked() {
                                    self.register_dpns_name_status = RegisterDpnsNameStatus::NotStarted;
                                }
                            });
                        });
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
            DpnsNameValidationResult::TooShort => {
                Some("Name must be at least 3 characters long".to_string())
            }
            DpnsNameValidationResult::TooLong => {
                Some("Name must be no more than 63 characters long".to_string())
            }
            DpnsNameValidationResult::InvalidCharacter(c) => Some(format!(
                "Invalid character '{}'. Only letters, numbers, and hyphens are allowed",
                c
            )),
            DpnsNameValidationResult::StartsWithHyphen => {
                Some("Name cannot start with a hyphen".to_string())
            }
            DpnsNameValidationResult::EndsWithHyphen => {
                Some("Name cannot end with a hyphen".to_string())
            }
        }
    }
}
