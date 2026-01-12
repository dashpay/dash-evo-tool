use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::FeeResult;
use crate::backend_task::contract::ContractTask;
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::helpers::{TransactionType, add_identity_key_chooser};
use crate::ui::identities::get_selected_wallet;
use crate::ui::{BackendTaskSuccessResult, MessageType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Setters;
use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
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
    pub selected_key: Option<IdentityPublicKey>,

    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    error_message: Option<String>,
    completed_fee_result: Option<FeeResult>,
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

        Self {
            app_context: app_context.clone(),
            contract_json_input: String::new(),
            contract_alias_input: String::new(),
            broadcast_status: BroadcastStatus::Idle,

            qualified_identities,
            selected_qualified_identity,
            selected_key: None,

            selected_wallet,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            error_message: None,
            completed_fee_result: None,
        }
    }

    fn parse_contract(&mut self) {
        // Clear any previous parse/broadcast states
        self.broadcast_status = BroadcastStatus::Idle;

        if self.contract_json_input.trim().is_empty() {
            // No input yet
            return;
        }

        // Try to parse the user’s JSON -> serde_json::Value
        let json_result: Result<serde_json::Value, serde_json::Error> =
            serde_json::from_str(&self.contract_json_input);

        match json_result {
            Ok(json_val) => {
                let platform_version = self.app_context.platform_version();
                match DataContract::from_json(json_val, true, platform_version) {
                    Ok(mut contract) => {
                        // ------------------------------------------
                        // 1) Overwrite the contract’s ownerId
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

    /// Renders an error message at the top of the screen with a styled bubble
    fn render_error_bubble(&mut self, ui: &mut egui::Ui) {
        let error_msg = match &self.broadcast_status {
            BroadcastStatus::ParsingError(err) => Some(format!("Parsing error: {err}")),
            BroadcastStatus::BroadcastError(msg) => Some(format!("Broadcast error: {msg}")),
            _ => None,
        };

        if let Some(msg) = error_msg {
            let error_color = Color32::from_rgb(255, 100, 100);
            Frame::new()
                .fill(error_color.gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .stroke(egui::Stroke::new(1.0, error_color))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&msg).color(error_color)).wrap(),
                        );
                        ui.add_space(8.0);
                        if ui.small_button("Dismiss").clicked() {
                            self.broadcast_status = BroadcastStatus::Idle;
                        }
                    });
                });
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
                // Display estimated fee based on contract size
                let contract_size = self.contract_json_input.len();
                let estimated_fee =
                    crate::model::fee_estimation::PlatformFeeEstimator::new()
                        .estimate_contract_create_with_size(contract_size);
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
                        .fill(Color32::from_rgb(0, 128, 255))
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
                    .unwrap()
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
                    .unwrap()
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
                    .unwrap()
                    .as_secs(),
            );
        }

        app_action
    }

    pub fn show_success(&mut self, ui: &mut Ui) -> AppAction {
        // Prepare fee info for display
        let fee_info = self.completed_fee_result.as_ref().map(|fee_result| {
            let fee_str = format!(
                "Estimated: {}  •  Actual: {}",
                format_credits_as_dash(fee_result.estimated_fee),
                format_credits_as_dash(fee_result.actual_fee)
            );
            ("Transaction Fee".to_string(), fee_str)
        });
        let fee_ref = fee_info.as_ref().map(|(title, desc)| (title.as_str(), desc.as_str()));

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
            fee_ref,
        );

        // Handle the custom action to reset the form
        if let AppAction::Custom(ref s) = action
            && s == "register_another"
        {
            self.contract_json_input = String::new();
            self.contract_alias_input = String::new();
            self.broadcast_status = BroadcastStatus::Idle;
            self.completed_fee_result = None;
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
            BackendTaskSuccessResult::FetchedNonce => {
                self.broadcast_status = BroadcastStatus::Broadcasting(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                );
            }
            BackendTaskSuccessResult::RegisteredContract(fee_result) => {
                self.completed_fee_result = Some(fee_result);
                self.broadcast_status = BroadcastStatus::Done;
            }
            BackendTaskSuccessResult::ProofErrorLogged => {
                self.broadcast_status = BroadcastStatus::ProofError(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
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

        // Contracts sub-left panel
        action |= crate::ui::components::contracts_subscreen_chooser_panel::add_contracts_subscreen_chooser_panel(
            ctx,
            &self.app_context,
        );

        action |= island_central_panel(ctx, |ui| {
            if self.broadcast_status == BroadcastStatus::Done {
                return self.show_success(ui);
            }

            ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Register Data Contract");
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

                // Select the identity to register the name for
                ui.heading("1. Select Identity");
                ui.add_space(5.0);
                add_identity_key_chooser(
                    ui,
                    &self.app_context,
                    self.qualified_identities.iter(),
                    &mut self.selected_qualified_identity,
                    &mut self.selected_key,
                    TransactionType::RegisterContract,
                );
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
                            egui::Color32::from_rgb(200, 150, 50),
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
                            .color(Color32::from_rgb(0, 128, 255)),
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
