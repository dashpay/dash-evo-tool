use crate::app::AppAction;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
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
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(PartialEq)]
enum BroadcastStatus {
    Idle,
    ParsingError(String),
    ValidContract(DataContract),
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

    pub show_key_selector: bool,
    pub qualified_identities: Vec<(QualifiedIdentity, Vec<IdentityPublicKey>)>,
    pub selected_qualified_identity: Option<(QualifiedIdentity, Vec<IdentityPublicKey>)>,
    pub selected_key: Option<IdentityPublicKey>,

    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
}

impl UpdateDataContractScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let security_level_requirements = vec![SecurityLevel::HIGH, SecurityLevel::CRITICAL];

        let qualified_identities: Vec<_> = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| {
                let keys = e
                    .identity
                    .public_keys()
                    .values()
                    .filter(|key| {
                        key.purpose() == Purpose::AUTHENTICATION
                            && security_level_requirements.contains(&SecurityLevel::CRITICAL)
                            && !key.is_disabled()
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if keys.is_empty() {
                    None
                } else {
                    Some((e, keys))
                }
            })
            .collect();
        let selected_qualified_identity = qualified_identities.first().cloned();

        let mut error_message: Option<String> = None;
        let selected_wallet = if let Some(ref identity) = selected_qualified_identity {
            get_selected_wallet(&identity.0, Some(app_context), None, &mut error_message)
        } else {
            None
        };

        let show_key_selector = selected_qualified_identity.is_some();

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
                .0
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

            show_key_selector,
            qualified_identities,
            selected_qualified_identity,
            selected_key,

            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
        }
    }

    fn render_identity_id_and_key_selection(&mut self, ui: &mut egui::Ui) {
        if self.qualified_identities.len() == 1 {
            // Only one identity, display it directly
            let qualified_identity = &self.qualified_identities[0];
            ui.horizontal(|ui| {
                ui.label("Identity ID:");
                ui.label(
                    qualified_identity
                        .0
                        .alias
                        .as_ref()
                        .unwrap_or(
                            &qualified_identity
                                .0
                                .identity
                                .id()
                                .to_string(Encoding::Base58),
                        )
                        .clone(),
                );
            });
            self.selected_qualified_identity = Some(qualified_identity.clone());
        } else {
            // Multiple identities, display ComboBox
            ui.horizontal(|ui| {
                ui.label("Identity ID:");

                // Create a ComboBox for selecting a Qualified Identity
                egui::ComboBox::from_id_salt("identity")
                    .selected_text(
                        self.selected_qualified_identity
                            .as_ref()
                            .map(|qi| {
                                qi.0.alias
                                    .as_ref()
                                    .unwrap_or(&qi.0.identity.id().to_string(Encoding::Base58))
                                    .clone()
                            })
                            .unwrap_or_else(|| "Select an identity".to_string()),
                    )
                    .show_ui(ui, |ui| {
                        // Loop through the qualified identities and display each as selectable
                        for qualified_identity in &self.qualified_identities {
                            // Display each QualifiedIdentity as a selectable item
                            if ui
                                .selectable_value(
                                    &mut self.selected_qualified_identity,
                                    Some(qualified_identity.clone()),
                                    qualified_identity.0.alias.as_ref().unwrap_or(
                                        &qualified_identity
                                            .0
                                            .identity
                                            .id()
                                            .to_string(Encoding::Base58),
                                    ),
                                )
                                .clicked()
                            {
                                self.selected_qualified_identity = Some(qualified_identity.clone());
                                self.selected_wallet = get_selected_wallet(
                                    &qualified_identity.0,
                                    Some(&self.app_context),
                                    None,
                                    &mut self.error_message,
                                );
                                self.show_key_selector = true;
                            }
                        }
                    });
            });
        }

        // Key selection
        if let Some(ref qid) = self.selected_qualified_identity {
            // Attempt to list available keys (only auth keys in normal mode)
            let keys = if self.app_context.developer_mode.load(Ordering::Relaxed) {
                qid.0
                    .identity
                    .public_keys()
                    .values()
                    .cloned()
                    .collect::<Vec<_>>()
            } else {
                qid.0
                    .available_authentication_keys()
                    .into_iter()
                    .filter_map(|k| {
                        if k.identity_public_key.security_level() == SecurityLevel::CRITICAL {
                            Some(k.identity_public_key.clone())
                        } else {
                            None
                        }
                    })
                    .collect()
            };

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Key:");
                egui::ComboBox::from_id_salt("contract_creator_key_selector")
                    .selected_text(match &self.selected_key {
                        Some(k) => format!(
                            "Key {} (Purpose: {:?}, Security Level: {:?})",
                            k.id(),
                            k.purpose(),
                            k.security_level()
                        ),
                        None => "Select Key".to_owned(),
                    })
                    .show_ui(ui, |ui| {
                        for k in keys {
                            let label = format!(
                                "Key {} (Purpose: {:?}, Security Level: {:?})",
                                k.id(),
                                k.purpose(),
                                k.security_level()
                            );
                            if ui
                                .selectable_label(
                                    Some(k.id()) == self.selected_key.as_ref().map(|kk| kk.id()),
                                    label,
                                )
                                .clicked()
                            {
                                self.selected_key = Some(k.clone());

                                // If the key belongs to a wallet, set that wallet reference:
                                self.selected_wallet = crate::ui::identities::get_selected_wallet(
                                    &qid.0,
                                    None,
                                    Some(&k),
                                    &mut self.error_message,
                                );
                            }
                        }
                    });
            });
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
                        if let Some((qualified_identity, _keys)) = &self.selected_qualified_identity
                        {
                            let new_owner_id = qualified_identity.identity.id();
                            contract.set_owner_id(new_owner_id);
                        }

                        // Mark it as a valid contract in our screen state
                        self.broadcast_status = BroadcastStatus::ValidContract(contract);
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
                    app_action = AppAction::BackendTask(BackendTask::ContractTask(
                        ContractTask::UpdateDataContract(
                            contract.clone(),
                            self.selected_qualified_identity.clone().unwrap().0, // unwrap should be safe here
                            self.selected_key.clone().unwrap(), // unwrap should be safe here
                        ),
                    ));
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

        match app_action {
            AppAction::BackendTask(BackendTask::ContractTask(
                ContractTask::UpdateDataContract(_, _, _),
            )) => {
                self.broadcast_status = BroadcastStatus::FetchingNonce(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                );
            }
            _ => {}
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
        match result {
            BackendTaskSuccessResult::Message(_msg) => {
                self.broadcast_status = BroadcastStatus::Done;
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

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.broadcast_status == BroadcastStatus::Done {
                action |= self.show_success(ui);
                return;
            }

            ui.heading("Update Data Contract");
            ui.add_space(10.0);

            // If no identities loaded, give message
            if self.qualified_identities.is_empty() {
                ui.colored_label(
                    egui::Color32::DARK_RED,
                    "No qualified identities available to update a data contract.",
                );
                return;
            }

            // Select the identity to update the name for
            ui.heading("1. Select Identity");
            ui.add_space(5.0);
            self.render_identity_id_and_key_selection(ui);
            ui.add_space(5.0);
            if let Some(identity) = &self.selected_qualified_identity {
                ui.label(format!(
                    "Identity balance: {:.6}",
                    identity.0.identity.balance() as f64 * 1e-11
                ));
            }

            if self.selected_key.is_none() {
                action = AppAction::None;
                return;
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

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

            if self.selected_wallet.is_some() {
                let (needed_unlock, just_unlocked) = self.render_wallet_unlock_if_needed(ui);

                if needed_unlock && !just_unlocked {
                    return;
                }
            }

            // Input for the contract
            ui.heading("2. Edit the contract JSON below or paste a new one");
            ui.add_space(10.0);
            self.ui_input_field(ui);

            // Parse the contract and show the result
            action |= self.ui_parsed_contract(ui);
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
