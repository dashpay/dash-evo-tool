use crate::app::AppAction;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::get_selected_wallet;
use crate::ui::{BackendTaskSuccessResult, MessageType, ScreenLike};
use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
use dash_sdk::dpp::version::PlatformVersion;
use dash_sdk::platform::{DataContract, IdentityPublicKey};
use eframe::egui::{self, Color32, Context, ScrollArea, TextEdit};
use egui::RichText;
use serde_json::Value;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(PartialEq)]
enum BroadcastStatus {
    Idle,
    ParsingError(String),
    ValidContract(DataContract),
    Broadcasting(u64), // store "start time" so we can show how long
    BroadcastError(String),
    Done,
}

pub struct RegisterDataContractScreen {
    pub app_context: Arc<AppContext>,
    contract_json_input: String,
    broadcast_status: BroadcastStatus,

    pub show_identity_selector: bool,
    pub qualified_identities: Vec<(QualifiedIdentity, Vec<IdentityPublicKey>)>,
    pub selected_qualified_identity: Option<(QualifiedIdentity, Vec<IdentityPublicKey>)>,

    pub selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,
}

impl RegisterDataContractScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let security_level_requirements = SecurityLevel::CRITICAL..=SecurityLevel::MASTER; // is this right?

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
                        key.purpose() == Purpose::AUTHENTICATION // is this right?
                            && security_level_requirements.contains(&key.security_level())
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

        let show_identity_selector = qualified_identities.len() > 0;

        Self {
            app_context: app_context.clone(),
            contract_json_input: String::new(),
            broadcast_status: BroadcastStatus::Idle,

            show_identity_selector,
            qualified_identities,
            selected_qualified_identity,

            selected_wallet,
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
        }
    }

    fn parse_contract(&mut self) {
        // Clear previous parse/broadcast states
        self.broadcast_status = BroadcastStatus::Idle;

        if self.contract_json_input.trim().is_empty() {
            // No input yet
            return;
        }

        // Try to parse the user’s JSON -> DataContract
        let json_result: Result<Value, serde_json::Error> =
            serde_json::from_str(&self.contract_json_input);

        match json_result {
            Ok(json_val) => {
                // Convert to Dash Platform Value (e.g. from Platform’s JSON)
                // Then parse into a DataContract
                let platform_version = PlatformVersion::latest(); // or a pinned version

                match DataContract::from_json(json_val, true, platform_version) {
                    Ok(contract) => {
                        self.broadcast_status = BroadcastStatus::ValidContract(contract)
                    }
                    Err(e) => {
                        self.broadcast_status =
                            BroadcastStatus::ParsingError(format!("DataContract parse error: {e}"))
                    }
                }
            }
            Err(e) => {
                self.broadcast_status = BroadcastStatus::ParsingError(format!("Invalid JSON: {e}"));
            }
        }
    }

    fn ui_input_field(&mut self, ui: &mut egui::Ui) {
        ui.label("Paste your Data Contract JSON:");
        ui.add_space(5.0);

        let response = ui.add(
            TextEdit::multiline(&mut self.contract_json_input)
                .desired_rows(6)
                .desired_width(ui.available_width())
                .code_editor(),
        );

        if response.changed() {
            self.parse_contract();
        }
    }

    fn ui_parsed_contract(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut app_action = AppAction::None;

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        match &self.broadcast_status {
            BroadcastStatus::Idle => {
                ui.label("No contract parsed yet or empty input.");
            }
            BroadcastStatus::ParsingError(err) => {
                ui.colored_label(Color32::RED, format!("Parsing error: {err}"));
            }
            BroadcastStatus::ValidContract(contract) => {
                // Show a prettified JSON version
                if let Ok(json_str) = serde_json::to_string_pretty(contract) {
                    ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(5.0);
                        ui.add(
                            TextEdit::multiline(&mut json_str.clone())
                                .desired_rows(12)
                                .desired_width(ui.available_width())
                                .font(egui::TextStyle::Monospace),
                        );
                    });
                }

                // “Register” button
                ui.add_space(10.0);
                let button = egui::Button::new(
                    RichText::new("Register Data Contract").color(Color32::WHITE),
                )
                .fill(Color32::from_rgb(0, 128, 255))
                .rounding(3.0);

                if ui.add(button).clicked() {
                    // Fire off a backend task
                    app_action = AppAction::BackendTask(BackendTask::ContractTask(
                        ContractTask::RegisterDataContract(
                            contract.clone(),
                            self.selected_qualified_identity.clone().unwrap().0, // unwrap should be safe here
                        ),
                    ));
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
            BroadcastStatus::BroadcastError(msg) => {
                ui.colored_label(Color32::RED, format!("Broadcast error: {msg}"));
            }
            BroadcastStatus::Done => {
                ui.colored_label(Color32::GREEN, "Data Contract registered successfully!");
            }
        }

        match app_action {
            AppAction::BackendTask(BackendTask::ContractTask(
                ContractTask::RegisterDataContract(_, _),
            )) => {
                self.broadcast_status = BroadcastStatus::Broadcasting(
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
}

impl ScreenLike for RegisterDataContractScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                self.broadcast_status = BroadcastStatus::Done;
            }
            MessageType::Error => {
                self.broadcast_status = BroadcastStatus::BroadcastError(message.to_string());
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
                ("Register Data Contract", AppAction::None),
            ],
            vec![],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            self.ui_input_field(ui);
            action |= self.ui_parsed_contract(ui);
        });

        action
    }
}

// If you also need wallet unlocking, implement the trait
impl ScreenWithWalletUnlock for RegisterDataContractScreen {
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
