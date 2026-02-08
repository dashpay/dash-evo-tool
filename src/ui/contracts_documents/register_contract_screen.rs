use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::FeeResult;
use crate::backend_task::contract::ContractResult;
use crate::backend_task::contract::ContractTask;
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::error_display::ErrorDisplay;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::helpers::{TransactionType, add_key_chooser};
use crate::ui::identities::get_selected_wallet;
use crate::ui::theme::DashColors;
use crate::ui::{BackendTaskSuccessResult, MessageType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Setters;
use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use eframe::egui::{self, Color32, Context, Frame, Margin, TextEdit};
use egui::{RichText, ScrollArea, Ui};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(PartialEq)]
enum BroadcastStatus {
    Idle,
    ParsingError(String),
    ValidContract(Box<DataContract>),
    Broadcasting(u64),
    ProofError(u64),
    BroadcastError(String),
    Done,
}

pub struct RegisterDataContractScreen {
    pub app_context: Arc<AppContext>,
    contract_json_input: String,
    contract_alias_input: String,
    broadcast_status: BroadcastStatus,

    pub qualified_identities: Vec<QualifiedIdentity>,
    pub selected_qualified_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    pub selected_key: Option<IdentityPublicKey>,
    show_advanced_options: bool,

    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    error_message: Option<String>,
    error_details_expanded: bool,
    completed_fee_result: Option<FeeResult>,
    /// Set to true when the input JSON was auto-wrapped with contract metadata
    contract_was_wrapped: bool,
}

impl RegisterDataContractScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let qualified_identities: Vec<QualifiedIdentity> =
            app_context.load_local_user_identities().unwrap_or_default();

        let selected_qualified_identity = qualified_identities.first().cloned();

        let mut error_message: Option<String> = None;
        let selected_wallet = if let Some(ref identity) = selected_qualified_identity {
            get_selected_wallet(identity, Some(app_context), None, &mut error_message)
        } else {
            None
        };

        // Auto-select a suitable key for contract registration
        use dash_sdk::dpp::identity::KeyType;
        let selected_key = selected_qualified_identity.as_ref().and_then(|identity| {
            identity
                .identity
                .get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    [SecurityLevel::HIGH, SecurityLevel::CRITICAL].into(),
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

        Self {
            app_context: app_context.clone(),
            contract_json_input: String::new(),
            contract_alias_input: String::new(),
            broadcast_status: BroadcastStatus::Idle,

            qualified_identities,
            selected_qualified_identity,
            selected_identity_string,
            selected_key,
            show_advanced_options: false,

            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            error_message: None,
            error_details_expanded: false,
            completed_fee_result: None,
            contract_was_wrapped: false,
        }
    }

    /// Check if a JSON value looks like raw document schemas (e.g. output from dashpay.io)
    /// rather than a full contract definition. Raw schemas are an object where each value
    /// is a document schema (has "type", "properties", etc.) but the top-level object
    /// lacks contract metadata fields like "$format_version", "id", "version".
    fn looks_like_raw_document_schemas(json: &serde_json::Value) -> bool {
        let obj = match json.as_object() {
            Some(o) => o,
            None => return false,
        };

        // If it already has contract metadata fields, it's not raw schemas
        if obj.contains_key("$format_version")
            || obj.contains_key("id")
            || obj.contains_key("version")
            || obj.contains_key("documentSchemas")
        {
            return false;
        }

        // Check that at least one entry looks like a document schema
        obj.values().any(|v| {
            v.is_object()
                && (v.get("type").is_some()
                    || v.get("properties").is_some()
                    || v.get("indices").is_some())
        })
    }

    /// Wrap raw document schemas into a full contract JSON with required metadata.
    fn wrap_document_schemas(
        document_schemas: serde_json::Value,
        owner_id: &Identifier,
    ) -> serde_json::Value {
        let owner_id_str =
            owner_id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);
        // Generate a random contract ID (will be replaced by the platform on registration)
        let contract_id = Identifier::random();
        let contract_id_str =
            contract_id.to_string(dash_sdk::dpp::platform_value::string_encoding::Encoding::Base58);

        serde_json::json!({
            "$format_version": "0",
            "id": contract_id_str,
            "ownerId": owner_id_str,
            "version": 1,
            "documentSchemas": document_schemas,
            "config": {
                "$format_version": "0",
                "canBeDeleted": false,
                "readonly": false,
                "keepsHistory": false,
                "documentsKeepHistoryContractDefault": false,
                "documentsMutableContractDefault": true,
                "documentsCanBeDeletedContractDefault": false,
                "requiresIdentityEncryptionBoundedKey": null,
                "requiresIdentityDecryptionBoundedKey": null
            }
        })
    }

    fn parse_contract(&mut self) {
        // Clear any previous parse/broadcast states
        self.broadcast_status = BroadcastStatus::Idle;
        self.contract_was_wrapped = false;

        if self.contract_json_input.trim().is_empty() {
            // No input yet
            return;
        }

        // Try to parse the user's JSON -> serde_json::Value
        let json_result: Result<serde_json::Value, serde_json::Error> =
            serde_json::from_str(&self.contract_json_input);

        match json_result {
            Ok(mut json_val) => {
                let platform_version = self.app_context.platform_version();

                // If the JSON looks like raw document schemas (e.g. from dashpay.io),
                // auto-wrap it with the required contract metadata
                if Self::looks_like_raw_document_schemas(&json_val) {
                    if let Some(qualified_identity) = &self.selected_qualified_identity {
                        let owner_id = qualified_identity.identity.id();
                        json_val = Self::wrap_document_schemas(json_val, &owner_id);
                        self.contract_was_wrapped = true;
                    } else {
                        self.broadcast_status = BroadcastStatus::ParsingError(
                            "Please select an identity before pasting raw document schemas."
                                .to_string(),
                        );
                        return;
                    }
                }

                match DataContract::from_json(json_val, true, platform_version) {
                    Ok(mut contract) => {
                        // ------------------------------------------
                        // 1) Overwrite the contract's ownerId
                        // ------------------------------------------
                        if let Some(qualified_identity) = &self.selected_qualified_identity {
                            let new_owner_id = qualified_identity.identity.id();
                            contract.set_owner_id(new_owner_id);
                        }

                        // Mark it as a valid contract in our screen state
                        self.broadcast_status = BroadcastStatus::ValidContract(Box::new(contract));
                    }
                    Err(e) => {
                        self.broadcast_status =
                            BroadcastStatus::ParsingError(format!("DataContract parse error: {e}"));
                    }
                }
            }
            Err(e) => {
                self.broadcast_status = BroadcastStatus::ParsingError(format!("Invalid JSON: {e}"));
            }
        }
    }

    fn ui_input_field(&mut self, ui: &mut egui::Ui) {
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let response = ui.add(
            TextEdit::multiline(&mut self.contract_json_input)
                .desired_rows(12)
                .desired_width(ui.available_width())
                .text_color(crate::ui::theme::DashColors::text_primary(dark_mode))
                .background_color(crate::ui::theme::DashColors::input_background(dark_mode))
                .code_editor(),
        );
        if response.changed() {
            self.parse_contract();
        }
    }

    /// Renders an error message at the top of the screen using the shared ErrorDisplay component
    fn render_error_bubble(&mut self, ui: &mut egui::Ui) {
        let error_msg = match &self.broadcast_status {
            BroadcastStatus::ParsingError(err) => Some(format!("Parsing error: {err}")),
            BroadcastStatus::BroadcastError(msg) => Some(format!("Broadcast error: {msg}")),
            _ => None,
        };

        if let Some(msg) = error_msg {
            let dismissed = ErrorDisplay::new(&msg).show(ui, &mut self.error_details_expanded);
            if dismissed {
                self.broadcast_status = BroadcastStatus::Idle;
                self.error_details_expanded = false;
            }
            ui.add_space(10.0);
        }
    }

    fn ui_parsed_contract(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut app_action = AppAction::None;

        ui.add_space(5.0);

        match &self.broadcast_status {
            BroadcastStatus::Idle => {
                ui.label("No contract parsed yet or empty input.");
            }
            BroadcastStatus::ParsingError(_) | BroadcastStatus::BroadcastError(_) => {
                // Errors are now shown at the top via render_error_bubble
            }
            BroadcastStatus::ValidContract(contract) => {
                // Show notification if the contract was auto-wrapped
                if self.contract_was_wrapped {
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
                    Frame::new()
                        .fill(if dark_mode {
                            Color32::from_rgb(40, 60, 40)
                        } else {
                            Color32::from_rgb(220, 245, 220)
                        })
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(
                                    "Raw document schemas detected. Contract metadata (format version, ID, owner, config) was auto-populated.",
                                )
                                .color(if dark_mode {
                                    Color32::from_rgb(140, 220, 140)
                                } else {
                                    Color32::from_rgb(30, 100, 30)
                                }),
                            );
                        });
                    ui.add_space(5.0);
                }

                // Display estimated fee using SDK's registration_cost method
                // This accounts for document types, indexes, tokens, and keywords
                let platform_version = self.app_context.platform_version();
                let registration_fee = contract.registration_cost(platform_version).unwrap_or(0);
                // Add storage and processing fees for the contract data
                let contract_size = self.contract_json_input.len();
                let storage_fee = self
                    .app_context
                    .fee_estimator()
                    .estimate_storage_based_fee(contract_size, 20); // ~20 seeks for tree operations
                let estimated_fee = registration_fee.saturating_add(storage_fee);
                ui.add_space(10.0);
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                Frame::new()
                    .fill(crate::ui::theme::DashColors::surface(dark_mode))
                    .inner_margin(Margin::symmetric(10, 8))
                    .corner_radius(5.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Estimated Fee:")
                                    .color(crate::ui::theme::DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(
                                RichText::new(format_credits_as_dash(estimated_fee))
                                    .color(crate::ui::theme::DashColors::text_primary(dark_mode))
                                    .strong(),
                            );
                        });
                    });
                ui.add_space(10.0);

                // Register button
                let mut new_style = (**ui.style()).clone();
                new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                ui.set_style(new_style);
                let button =
                    egui::Button::new(RichText::new("Register Contract").color(Color32::WHITE))
                        .fill(DashColors::ACTION_BUTTON_BLUE)
                        .frame(true)
                        .corner_radius(3.0);
                if ui.add(button).clicked() {
                    // Fire off a backend task
                    app_action = AppAction::BackendTask(BackendTask::ContractTask(Box::new(
                        ContractTask::RegisterDataContract(
                            (**contract).clone(),
                            self.contract_alias_input.clone(),
                            self.selected_qualified_identity.clone().unwrap(), // unwrap should be safe here
                            self.selected_key.clone().unwrap(), // unwrap should be safe here
                        ),
                    )));
                }
            }
            BroadcastStatus::Broadcasting(start_time) => {
                // Show how long we've been broadcasting
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let elapsed = now - start_time;
                ui.label(format!(
                    "Broadcasting contract... {} seconds elapsed.",
                    elapsed
                ));
            }
            BroadcastStatus::ProofError(start_time) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let elapsed = now - start_time;
                ui.label("Broadcasted but received proof error. ⚠");
                ui.label(format!("Fetching contract from Platform and inserting into DET... {elapsed} seconds elapsed."));
            }
            BroadcastStatus::Done => {
                ui.colored_label(
                    Color32::DARK_GREEN,
                    "Data Contract registered successfully!",
                );
            }
        }

        if let AppAction::BackendTask(BackendTask::ContractTask(contract_task)) = &app_action
            && let ContractTask::RegisterDataContract(_, _, _, _) = **contract_task
        {
            self.broadcast_status = BroadcastStatus::Broadcasting(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
        }

        app_action
    }

    pub fn show_success(&mut self, ui: &mut Ui) -> AppAction {
        let action = crate::ui::helpers::show_success_screen_with_info(
            ui,
            "Data Contract Registered Successfully!".to_string(),
            vec![
                (
                    "Back to Contracts screen".to_string(),
                    AppAction::GoToMainScreen,
                ),
                (
                    "Register another contract".to_string(),
                    AppAction::Custom("register_another".to_string()),
                ),
            ],
            None,
        );

        // Handle the custom action to reset the form
        if let AppAction::Custom(ref s) = action
            && s == "register_another"
        {
            self.contract_json_input = String::new();
            self.contract_alias_input = String::new();
            self.broadcast_status = BroadcastStatus::Idle;
            self.completed_fee_result = None;
            self.contract_was_wrapped = false;
            return AppAction::None;
        }

        action
    }
}

impl ScreenLike for RegisterDataContractScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        if message_type == MessageType::Error {
            if message.contains("proof error logged, contract inserted into the database") {
                self.error_message = Some(message.to_string());
                self.broadcast_status = BroadcastStatus::Done;
            } else {
                self.broadcast_status = BroadcastStatus::BroadcastError(message.to_string());
            }
        }
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        match result {
            BackendTaskSuccessResult::Contract(ContractResult::FetchedNonce) => {
                self.broadcast_status = BroadcastStatus::Broadcasting(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                );
            }
            BackendTaskSuccessResult::Contract(ContractResult::Registered(fee_result)) => {
                self.completed_fee_result = Some(fee_result);
                self.broadcast_status = BroadcastStatus::Done;
            }
            BackendTaskSuccessResult::Contract(ContractResult::ProofErrorLogged) => {
                self.broadcast_status = BroadcastStatus::ProofError(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                );
            }
            _ => {}
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                ("Register Data Contract", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenDocumentQuery,
        );

        action |= island_central_panel(ctx, |ui| {
            if self.broadcast_status == BroadcastStatus::Done {
                return self.show_success(ui);
            }

            ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Register Data Contract");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
                    });
                });
                ui.add_space(10.0);

                // Show error message at the top if there's an error
                self.render_error_bubble(ui);

                // If no identities loaded, give message
                if self.qualified_identities.is_empty() {
                    ui.colored_label(
                        egui::Color32::DARK_RED,
                        "No identities loaded. Please load an identity first.",
                    );
                    return AppAction::None;
                }

                // Check if any identity has suitable private keys for contract registration
                let has_suitable_keys = self.qualified_identities.iter().any(|qi| {
                    qi.private_keys
                        .identity_public_keys()
                        .iter()
                        .any(|key_ref| {
                            let key = &key_ref.1.identity_public_key;
                            // Contract registration requires Authentication keys with High or Critical security level
                            key.purpose() == Purpose::AUTHENTICATION
                                && (key.security_level() == SecurityLevel::HIGH
                                    || key.security_level() == SecurityLevel::CRITICAL)
                        })
                });

                if !has_suitable_keys {
                    ui.colored_label(
                        egui::Color32::DARK_RED,
                        "No identities with high or critical authentication private keys loaded. Contract registration requires high or critical security level keys.",
                    );
                    return AppAction::None;
                }

                // Select the identity to register the contract for
                ui.heading("1. Select Identity");
                ui.add_space(5.0);

                // Identity selector
                let response = ui.add(
                    IdentitySelector::new(
                        "register_contract_identity_selector",
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
                        // Auto-select a suitable key for contract registration
                        use dash_sdk::dpp::identity::KeyType;
                        self.selected_key = identity
                            .identity
                            .get_first_public_key_matching(
                                Purpose::AUTHENTICATION,
                                [SecurityLevel::HIGH, SecurityLevel::CRITICAL].into(),
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

                        // Re-parse contract with new owner ID
                        self.parse_contract();
                    } else {
                        self.selected_key = None;
                        self.selected_wallet = None;
                    }
                }

                // Key selector (only shown in advanced mode)
                if self.show_advanced_options {
                    ui.add_space(10.0);
                    if let Some(identity) = &self.selected_qualified_identity {
                        add_key_chooser(
                            ui,
                            &self.app_context,
                            identity,
                            &mut self.selected_key,
                            TransactionType::RegisterContract,
                        );
                    }
                }

                ui.add_space(5.0);
                if let Some(identity) = &self.selected_qualified_identity {
                    ui.label(format!(
                        "Identity balance: {:.6}",
                        identity.identity.balance() as f64 * 1e-11
                    ));
                }

                if self.selected_key.is_none() {
                    return AppAction::None;
                }

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Render wallet unlock if needed
                if let Some(wallet) = &self.selected_wallet {
                    if let Err(e) = try_open_wallet_no_password(wallet) {
                        self.error_message = Some(e);
                    }
                    if wallet_needs_unlock(wallet) {
                        ui.add_space(10.0);
                        ui.colored_label(
                            DashColors::WARNING_ORANGE,
                            "Wallet is locked. Please unlock to continue.",
                        );
                        ui.add_space(8.0);
                        if ui.button("Unlock Wallet").clicked() {
                            self.wallet_unlock_popup.open();
                        }
                        return AppAction::None;
                    }
                }

                // Input for the alias
                ui.heading("2. Contract alias for DET (optional)");
                ui.add_space(5.0);
                ui.text_edit_singleline(&mut self.contract_alias_input);

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                // Input for the contract
                ui.heading("3. Paste the contract JSON below");
                ui.add_space(5.0);

                // Add link to dashpay.io
                ui.horizontal(|ui| {
                    ui.label("Easily create a contract JSON here:");
                    ui.add(egui::Hyperlink::from_label_and_url(
                        RichText::new("dashpay.io")
                            .underline()
                            .color(DashColors::ACTION_BUTTON_BLUE),
                        "https://dashpay.io",
                    ));
                });
                ui.add_space(5.0);

                self.ui_input_field(ui);

                // Parse the contract and show the result
                self.ui_parsed_contract(ui)
            }).inner
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
