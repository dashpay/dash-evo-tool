use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::contract::ContractTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{BackendTaskSuccessResult, MessageType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::DataContract;
use dash_sdk::dpp::prelude::TimestampMillis;
use eframe::egui::{self, Color32, Context, RichText, Ui};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CONTRACTS: usize = 10;

enum AddContractsStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    Complete(Vec<String>), // Vec of tuples: original input contract id and option if it was fetched from platform
    ErrorMessage(String),
}

pub struct AddContractsScreen {
    pub app_context: Arc<AppContext>,
    contract_ids: Vec<String>,
    add_contracts_status: AddContractsStatus,
}

impl AddContractsScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            contract_ids: vec!["".to_string()],
            add_contracts_status: AddContractsStatus::NotStarted,
        }
    }

    fn add_contract_field(&mut self) {
        if self.contract_ids.len() < MAX_CONTRACTS {
            self.contract_ids.push("".to_string());
        }
    }

    fn parse_identifiers(&self) -> Result<Vec<Identifier>, String> {
        let mut identifiers = Vec::new();
        for (i, input) in self.contract_ids.iter().enumerate() {
            let trimmed = input.trim();
            if trimmed.is_empty() {
                continue; // Empty fields are ignored
            }
            // Try hex first
            let identifier = if let Ok(bytes) = hex::decode(trimmed) {
                Identifier::from_bytes(&bytes)
                    .map_err(|e| format!("Invalid ID in field {}: {}", i + 1, e))?
            } else {
                // Try Base58
                Identifier::from_string(trimmed, Encoding::Base58)
                    .map_err(|e| format!("Invalid ID in field {}: {}", i + 1, e))?
            };
            identifiers.push(identifier);
        }
        if identifiers.is_empty() {
            return Err("No valid contract IDs entered.".to_string());
        }
        Ok(identifiers)
    }

    fn add_contracts_clicked(&mut self) -> AppAction {
        match self.parse_identifiers() {
            Ok(identifiers) => {
                self.add_contracts_status = AddContractsStatus::WaitingForResult(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs(),
                );
                AppAction::BackendTask(BackendTask::ContractTask(ContractTask::FetchContracts(
                    identifiers,
                )))
            }
            Err(e) => {
                self.add_contracts_status = AddContractsStatus::ErrorMessage(e);
                AppAction::None
            }
        }
    }

    fn show_input_fields(&mut self, ui: &mut Ui) {
        ui.heading("Enter Contract Identifiers:");
        ui.add_space(5.0);

        for (i, contract_id) in self.contract_ids.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Contract {}:", i + 1));
                ui.text_edit_singleline(contract_id);
            });
            ui.add_space(5.0);
        }

        if self.contract_ids.len() < MAX_CONTRACTS {
            if ui.button("Add Another Contract Field").clicked() {
                self.add_contract_field();
            }
        }
    }

    fn show_success_screen(&mut self, ui: &mut Ui) -> AppAction {
        ui.heading("Contracts Added");
        ui.add_space(10.0);

        if let AddContractsStatus::Complete(options) = &self.add_contracts_status {
            for id_string in self.contract_ids.clone() {
                let trimmed_id_string = id_string.trim();
                if options.contains(&trimmed_id_string.to_string()) {
                    ui.colored_label(
                        Color32::DARK_GREEN,
                        format!("Contract {}: Successfully Added", trimmed_id_string),
                    );
                } else {
                    ui.colored_label(
                        Color32::RED,
                        format!("Contract {}: Not Found", trimmed_id_string),
                    );
                }
                ui.add_space(5.0);
            }
        }

        ui.add_space(20.0);
        let button =
            egui::Button::new(RichText::new("Go back to Contracts Screen").color(Color32::WHITE))
                .fill(Color32::from_rgb(0, 128, 255))
                .frame(true)
                .rounding(3.0);
        if ui.add(button).clicked() {
            // Return to previous screen
            return AppAction::PopScreenAndRefresh;
        }

        AppAction::None
    }
}

impl ScreenLike for AddContractsScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                // Not used
            }
            MessageType::Error => {
                self.add_contracts_status = AddContractsStatus::ErrorMessage(message.to_string());
            }
            MessageType::Info => {
                // Not used
            }
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::FetchedContracts(contract_options) => {
                let options = self
                    .contract_ids
                    .iter()
                    .filter_map(|input_id| {
                        if contract_options.iter().any(|option| {
                            if let Some(contract) = option {
                                contract.id().to_string(Encoding::Base58) == input_id.trim()
                            } else {
                                false
                            }
                        }) {
                            Some(input_id)
                        } else {
                            None
                        }
                    })
                    .cloned()
                    .collect();
                self.add_contracts_status = AddContractsStatus::Complete(options);
            }
            _ => {
                // Nothing
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let add_contract_button = (
            "Add Contracts",
            DesiredAppAction::AddScreenType(crate::ui::ScreenType::AddContracts),
        );
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Document Queries", AppAction::GoToMainScreen),
                ("Add Contracts", AppAction::None),
            ],
            vec![add_contract_button],
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Add Contracts to Query");
            ui.add_space(10.0);

            match &self.add_contracts_status {
                AddContractsStatus::NotStarted | AddContractsStatus::ErrorMessage(_) => {
                    if let AddContractsStatus::ErrorMessage(msg) = &self.add_contracts_status {
                        ui.colored_label(Color32::RED, format!("Error: {}", msg));
                        ui.add_space(10.0);
                    }

                    // Show input fields
                    self.show_input_fields(ui);

                    ui.add_space(10.0);
                    // Add Contracts Button
                    let button =
                        egui::Button::new(RichText::new("Add Contracts").color(Color32::WHITE))
                            .fill(Color32::from_rgb(0, 128, 255))
                            .frame(true)
                            .rounding(3.0);
                    if ui.add(button).clicked() {
                        action = self.add_contracts_clicked();
                    }
                }
                AddContractsStatus::WaitingForResult(start_time) => {
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
                        "Adding contracts... Time taken so far: {}",
                        display_time
                    ));
                }
                AddContractsStatus::Complete(_) => {
                    action = self.show_success_screen(ui);
                }
            }
        });

        action
    }
}
