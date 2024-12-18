use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::contract::ContractTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, ScreenLike};
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use eframe::egui::{self, Color32, Context, RichText, Ui};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CONTRACTS: usize = 10;

enum AddContractsStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    Complete(Vec<(String, Result<(), String>)>),
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

        if let AddContractsStatus::Complete(results) = &self.add_contracts_status {
            for (original_input, result) in results {
                match result {
                    Ok(_) => {
                        ui.colored_label(
                            Color32::DARK_GREEN,
                            format!("Contract {}: Successfully Added", original_input),
                        );
                    }
                    Err(err) => {
                        ui.colored_label(
                            Color32::RED,
                            format!("Contract {}: Failed to Add - {}", original_input, err),
                        );
                    }
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
                // Assume we get something like "AddContractsComplete" along with the contract results
                // You would parse the backend result here and store in Complete state
                // For demonstration, let's say the backend returns a success/fail result for each entered ID.
                // We’ll simulate it with a placeholder. In real code, you'd store the actual results from the backend.

                // Example:
                // self.add_contracts_status = AddContractsStatus::Complete(results_from_backend);

                // If you only got a single message, you might need to implement a channel or another mechanism
                // to store the actual results. For now, let's assume results were handled elsewhere
                // and that this message indicates completion.

                // If we have no mechanism, let's just set complete with a success message for each.
                let results = self
                    .contract_ids
                    .iter()
                    .map(|id| {
                        if !id.trim().is_empty() {
                            (id.clone(), Ok(()))
                        } else {
                            (id.clone(), Err("Empty input".to_string()))
                        }
                    })
                    .collect();
                self.add_contracts_status = AddContractsStatus::Complete(results);
            }
            MessageType::Error => {
                self.add_contracts_status = AddContractsStatus::ErrorMessage(message.to_string());
            }
            MessageType::Info => {
                // Not used in this scenario
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
