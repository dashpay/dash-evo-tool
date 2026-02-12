use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::FeeResult;
use crate::backend_task::contract::ContractTask;
use crate::context::AppContext;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
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
use dash_sdk::dpp::data_contract::accessors::v0::{DataContractV0Getters, DataContractV0Setters};
use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use eframe::egui::{self, Color32, Context, Frame, Margin, TextEdit};
use egui::{RichText, ScrollArea, Ui};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(PartialEq)]
enum BroadcastStatus {
    Idle,
    ParsingError(String),
    ValidContract(Box<DataContract>),
    FetchingNonce(u64),
    Broadcasting(u64),
    ProofError(u64),
    BroadcastError(String),
    Done,
}

pub struct UpdateDataContractScreen {
    pub app_context: Arc<AppContext>,
    contract_json_input: String,
    broadcast_status: BroadcastStatus,
    known_contracts: Vec<QualifiedContract>,
    selected_contract: Option<String>,

    pub qualified_identities: Vec<QualifiedIdentity>,
    pub selected_qualified_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    pub selected_key: Option<IdentityPublicKey>,
    show_advanced_options: bool,

    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    error_message: Option<String>,
    completed_fee_result: Option<FeeResult>,
}

impl UpdateDataContractScreen {
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

        let excluded_aliases = ["dpns", "keyword_search", "token_history", "withdrawals"];
        let known_contracts = app_context
            .get_contracts(None, None)
            .expect("Failed to load contracts")
            .into_iter()
            .filter(|c| match &c.alias {
                Some(alias) => !excluded_aliases.contains(&alias.as_str()),
                None => true,
            })
            .collect::<Vec<_>>();

        let selected_key = selected_qualified_identity.as_ref().and_then(|identity| {
            identity
                .identity
                .get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([SecurityLevel::CRITICAL]),
                    KeyType::all_key_types().into(),
                    false,
                )
                .cloned()
        });

        let selected_identity_string = selected_qualified_identity
            .as_ref()
            .map(|qi| qi.identity.id().to_string(Encoding::Base58))
            .unwrap_or_default();

        Self {
            app_context: app_context.clone(),
            contract_json_input: String::new(),
            broadcast_status: BroadcastStatus::Idle,
            known_contracts,
            selected_contract: None,

            qualified_identities,
            selected_qualified_identity,
            selected_identity_string,
            selected_key,
            show_advanced_options: false,

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
        ScrollArea::vertical()
            .max_height(ui.available_height() - 100.0)
            .show(ui, |ui| {
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                let response = ui.add(
                    TextEdit::multiline(&mut self.contract_json_input)
                        .desired_rows(6)
                        .desired_width(ui.available_width())
                        .text_color(DashColors::text_primary(dark_mode))
                        .background_color(DashColors::input_background(dark_mode))
                        .code_editor(),
                );
                if response.changed() {
                    self.parse_contract();
                }
            });
    }

    /// Renders an error message at the top of the screen with a styled bubble
    fn render_error_bubble(&mut self, ui: &mut egui::Ui) {
        let error_msg = match &self.broadcast_status {
            BroadcastStatus::ParsingError(err) => Some(format!("Parsing error: {err}")),
            BroadcastStatus::BroadcastError(msg) => Some(format!("Broadcast error: {msg}")),
            _ => None,
        };

        if let Some(msg) = error_msg {
            let error_color = DashColors::ERROR;
            Frame::new()
                .fill(error_color.gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .stroke(egui::Stroke::new(1.0, error_color))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(RichText::new(&msg).color(error_color)).wrap());
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

        ui.add_space(10.0);

        match &self.broadcast_status {
            BroadcastStatus::Idle => {}
            BroadcastStatus::ParsingError(_) | BroadcastStatus::BroadcastError(_) => {
                // Errors are now shown at the top via render_error_bubble
            }
            BroadcastStatus::ValidContract(contract) => {
                // Fee estimation display - contract updates charge registration fees for the new contract
                ui.add_space(10.0);
                let platform_version = self.app_context.platform_version();
                let registration_fee = contract.registration_cost(platform_version).unwrap_or(0);
                let base_fee = platform_version
                    .fee_version
                    .state_transition_min_fees
                    .contract_update;
                let estimated_fee = base_fee.saturating_add(registration_fee);

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

                // Update button
                ui.add_space(10.0);
                let mut new_style = (**ui.style()).clone();
                new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                ui.set_style(new_style);
                let button =
                    egui::Button::new(RichText::new("Update Contract").color(Color32::WHITE))
                        .fill(DashColors::ACTION_BUTTON_BLUE)
                        .frame(true)
                        .corner_radius(3.0);
                if ui.add(button).clicked() {
                    // Fire off a backend task
                    app_action = AppAction::BackendTask(BackendTask::ContractTask(Box::new(
                        ContractTask::UpdateDataContract(
                            (**contract).clone(),
                            self.selected_qualified_identity.clone().unwrap(), // unwrap should be safe here
                            self.selected_key.clone().unwrap(), // unwrap should be safe here
                        ),
                    )));
                }
            }
            BroadcastStatus::FetchingNonce(start_time) => {
                // Show how long we've been fetching nonce
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let elapsed = now - start_time;
                ui.label(format!(
                    "Fetching identity contract nonce... {} seconds elapsed.",
                    elapsed
                ));
            }
            BroadcastStatus::Broadcasting(start_time) => {
                // Show how long we've been broadcasting
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let elapsed = now - start_time;
                ui.label("Fetched nonce successfully. ✅ ");
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
                ui.label("Fetched nonce successfully. ✅ ");
                ui.label("Broadcasted but received proof error. ⚠");
                ui.label(format!(
                    "Fetching contract from Platform... {elapsed} seconds elapsed."
                ));
            }
            BroadcastStatus::Done => {
                ui.colored_label(Color32::DARK_GREEN, "Data Contract updated successfully!");
            }
        }

        if let AppAction::BackendTask(BackendTask::ContractTask(contract_task)) = &app_action
            && let ContractTask::UpdateDataContract(_, _, _) = **contract_task
        {
            self.broadcast_status = BroadcastStatus::FetchingNonce(
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
            "Data Contract Updated Successfully!".to_string(),
            vec![
                (
                    "Back to Contracts screen".to_string(),
                    AppAction::GoToMainScreen,
                ),
                (
                    "Update another contract".to_string(),
                    AppAction::Custom("update_another".to_string()),
                ),
            ],
            None,
        );

        // Handle the custom action to reset the form
        if let AppAction::Custom(ref s) = action
            && s == "update_another"
        {
            self.contract_json_input = String::new();
            self.broadcast_status = BroadcastStatus::Idle;
            self.completed_fee_result = None;
            return AppAction::None;
        }

        action
    }
}

impl ScreenLike for UpdateDataContractScreen {
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
                        .unwrap_or_default()
                        .as_secs(),
                );
            }
            BackendTaskSuccessResult::UpdatedContract(fee_result) => {
                self.completed_fee_result = Some(fee_result);
                self.broadcast_status = BroadcastStatus::Done;
            }
            BackendTaskSuccessResult::ProofErrorLogged => {
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
                ("Update Data Contract", AppAction::None),
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

            ui.horizontal(|ui| {
                ui.heading("Update Data Contract");
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

            // Check if any identity has suitable private keys for contract updates
            let has_suitable_keys = self.qualified_identities.iter().any(|qi| {
                qi.private_keys
                    .identity_public_keys()
                    .iter()
                    .any(|key_ref| {
                        let key = &key_ref.1.identity_public_key;
                        // Contract updates require Authentication keys with Critical security level
                        key.purpose() == Purpose::AUTHENTICATION
                            && key.security_level() == SecurityLevel::CRITICAL
                    })
            });

            if !has_suitable_keys {
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    "No identities with critical authentication private keys loaded. Contract updates require critical security level keys.",
                );
                return AppAction::None;
            }

            // Select the identity to update the contract for
            ui.heading("1. Select Identity");
            ui.add_space(5.0);

            // Identity selector
            let response = ui.add(
                IdentitySelector::new(
                    "update_contract_identity_selector",
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
                    // Auto-select a suitable key for contract updates
                    self.selected_key = identity
                        .identity
                        .get_first_public_key_matching(
                            Purpose::AUTHENTICATION,
                            HashSet::from([SecurityLevel::CRITICAL]),
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
                        TransactionType::UpdateContract,
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

            // Render the wallet unlock if needed
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

            // Select the contract to update
            ui.heading("2. Select contract to update");
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.label("Contract:");
                egui::ComboBox::from_id_salt("contract_selector")
                    .selected_text(
                        self.selected_contract
                            .as_ref()
                            .unwrap_or(&"Select a contract".to_string())
                            .clone(),
                    )
                    .show_ui(ui, |ui| {
                        for contract in &self.known_contracts {
                            let contract_id_str =
                                contract.contract.id().to_string(Encoding::Base58);
                            let display_text =
                                contract.alias.as_deref().unwrap_or(&contract_id_str);

                            if ui
                                .selectable_value(
                                    &mut self.selected_contract,
                                    Some(display_text.to_string()), // 👈 always Some(String)
                                    display_text,
                                )
                                .clicked()
                            {
                                let platform_version = self.app_context.platform_version();
                                self.selected_contract = Some(display_text.to_string());
                                self.contract_json_input =
                                    match contract.contract.to_json(platform_version) {
                                        Ok(json) => serde_json::to_string_pretty(&json)
                                            .expect("Expected to get string pretty"),
                                        Err(e) => format!("Error serialising contract: {e}"),
                                    };
                            }
                        }
                    });
            });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            // Input for the contract
            ui.heading("2. Edit the contract JSON below or paste a new one");
            ui.add_space(10.0);
            self.ui_input_field(ui);

            // Parse the contract and show the result
            self.ui_parsed_contract(ui)
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
