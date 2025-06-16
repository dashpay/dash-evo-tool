use crate::app::AppAction;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::helpers::{add_identity_key_chooser, TransactionType};
use crate::ui::identities::get_selected_wallet;
use crate::ui::{BackendTaskSuccessResult, MessageType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::{DataContractV0Getters, DataContractV0Setters};
use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{KeyType, Purpose, SecurityLevel};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use eframe::egui::{self, Color32, Context, TextEdit};
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
    pub selected_key: Option<IdentityPublicKey>,

    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
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

        let mut selected_key = None;
        if let Some(identity) = &selected_qualified_identity {
            selected_key = identity
                .identity
                .get_first_public_key_matching(
                    Purpose::AUTHENTICATION,
                    HashSet::from([SecurityLevel::CRITICAL]),
                    KeyType::all_key_types().into(),
                    false,
                )
                .cloned();
        }

        Self {
            app_context: app_context.clone(),
            contract_json_input: String::new(),
            broadcast_status: BroadcastStatus::Idle,
            known_contracts,
            selected_contract: None,

            qualified_identities,
            selected_qualified_identity,
            selected_key,

            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
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
                let response = ui.add(
                    TextEdit::multiline(&mut self.contract_json_input)
                        .desired_rows(6)
                        .desired_width(ui.available_width())
                        .code_editor(),
                );
                if response.changed() {
                    self.parse_contract();
                }
            });
    }

    fn ui_parsed_contract(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut app_action = AppAction::None;

        ui.add_space(10.0);

        match &self.broadcast_status {
            BroadcastStatus::Idle => {}
            BroadcastStatus::ParsingError(err) => {
                ui.colored_label(Color32::RED, format!("Parsing error: {err}"));
            }
            BroadcastStatus::ValidContract(contract) => {
                // “Update” button
                ui.add_space(10.0);
                // Update button
                let mut new_style = (**ui.style()).clone();
                new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                ui.set_style(new_style);
                let button =
                    egui::Button::new(RichText::new("Update Contract").color(Color32::WHITE))
                        .fill(Color32::from_rgb(0, 128, 255))
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
                    .unwrap()
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
                    .unwrap()
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
                    .unwrap()
                    .as_secs();
                let elapsed = now - start_time;
                ui.label("Fetched nonce successfully. ✅ ");
                ui.label("Broadcasted but received proof error. ⚠");
                ui.label(format!(
                    "Fetching contract from Platform... {elapsed} seconds elapsed."
                ));
            }
            BroadcastStatus::BroadcastError(msg) => {
                ui.label("Fetched nonce successfully. ✅ ");
                ui.colored_label(Color32::RED, format!("Broadcast error: {msg}"));
            }
            BroadcastStatus::Done => {
                ui.colored_label(Color32::DARK_GREEN, "Data Contract updated successfully!");
            }
        }

        if let AppAction::BackendTask(BackendTask::ContractTask(contract_task)) = &app_action {
            if let ContractTask::UpdateDataContract(_, _, _) = **contract_task {
                self.broadcast_status = BroadcastStatus::FetchingNonce(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                );
            }
        }

        app_action
    }

    pub fn show_success(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Center the content vertically and horizontally
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            if let Some(error_message) = &self.error_message {
                if error_message.contains("proof error logged, contract inserted into the database")
                {
                    ui.heading("⚠");
                    ui.heading("Transaction succeeded but received a proof error.");
                    ui.add_space(10.0);
                    ui.label("Please check if the contract was updated correctly.");
                    ui.label(
                        "If it was, this is just a Platform proofs bug and no need for concern.",
                    );
                    ui.label("Either way, please report to Dash Core Group.");
                }
            } else {
                ui.heading("🎉");
                ui.heading("Successfully updated data contract.");
            }

            ui.add_space(20.0);

            if ui.button("Back to Contracts screen").clicked() {
                action = AppAction::GoToMainScreen;
            }
            ui.add_space(5.0);

            if ui.button("Update another contract").clicked() {
                self.contract_json_input = String::new();
                self.broadcast_status = BroadcastStatus::Idle;
            }
        });

        action
    }
}

impl ScreenLike for UpdateDataContractScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                if message.contains("Nonce fetched successfully") {
                    self.broadcast_status = BroadcastStatus::Broadcasting(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                    );
                } else if message.contains("Transaction returned proof error") {
                    self.broadcast_status = BroadcastStatus::ProofError(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs(),
                    );
                } else {
                    self.broadcast_status = BroadcastStatus::Done;
                }
            }
            MessageType::Error => {
                if message.contains("proof error logged, contract inserted into the database") {
                    self.error_message = Some(message.to_string());
                    self.broadcast_status = BroadcastStatus::Done;
                } else {
                    self.broadcast_status = BroadcastStatus::BroadcastError(message.to_string());
                }
            }
            MessageType::Info => {
                // You could display an info label, or do nothing
            }
        }
    }

    fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        // If a separate result needs to be handled here, you can do so
        // For example, if success is a special message or we want to show it in the UI
        if let BackendTaskSuccessResult::Message(_msg) = result {
            self.broadcast_status = BroadcastStatus::Done;
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

            ui.heading("Update Data Contract");
            ui.add_space(10.0);

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

            // Select the identity to update the name for
            ui.heading("1. Select Identity");
            ui.add_space(5.0);
            add_identity_key_chooser(
                ui,
                &self.app_context,
                self.qualified_identities.iter(),
                &mut self.selected_qualified_identity,
                &mut self.selected_key,
                TransactionType::UpdateContract,
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

            // Render the wallet unlock if needed
            if self.selected_wallet.is_some() {
                let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);
                if needed_unlock && !just_unlocked {
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

        action
    }
}

// If you also need wallet unlocking, implement the trait
impl ScreenWithWalletUnlock for UpdateDataContractScreen {
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
