use crate::app::AppAction;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{BackendTaskSuccessResult, MessageType, ScreenLike};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::identifier::Identifier;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use eframe::egui::{self, Color32, Context, RichText, Ui};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_CONTRACTS: usize = 10;

#[derive(PartialEq)]
enum AddContractsStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    Complete(Vec<String>),
    ErrorMessage(String),
}

pub struct AddContractsScreen {
    pub app_context: Arc<AppContext>,
    contract_ids_input: Vec<String>,
    add_contracts_status: AddContractsStatus,
    maybe_found_contracts: Vec<String>,
    alias_inputs: Option<Vec<String>>,
    last_alias_result: Option<(usize, Result<String, String>)>,
}

impl AddContractsScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            contract_ids_input: vec!["".to_string()],
            add_contracts_status: AddContractsStatus::NotStarted,
            maybe_found_contracts: vec![],
            alias_inputs: None,
            last_alias_result: None,
        }
    }

    fn add_contract_field(&mut self) {
        if self.contract_ids_input.len() < MAX_CONTRACTS {
            self.contract_ids_input.push("".to_string());
        }
    }

    fn parse_identifiers(&self) -> Result<Vec<Identifier>, String> {
        let mut identifiers = Vec::new();
        for (i, input) in self.contract_ids_input.iter().enumerate() {
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

        for (i, contract_id) in self.contract_ids_input.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Contract {}:", i + 1));
                ui.text_edit_singleline(contract_id);
            });
            ui.add_space(5.0);
        }

        if self.contract_ids_input.len() < MAX_CONTRACTS && ui.button("Add Another Contract Field").clicked() {
            self.add_contract_field();
        }
    }

    fn show_success_screen(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.vertical_centered(|ui| {
            ui.add_space(50.0);

            ui.heading("🎉");
            ui.heading("Successfully queried contracts");
            ui.add_space(10.0);
            ui.label("Found and added the following contracts:");
            ui.add_space(10.0);
            let mut not_found = vec![];

            // Store alias input state for each contract ID
            if self.alias_inputs.is_none() {
                // Initialize alias_inputs with empty strings for each contract
                self.alias_inputs = Some(
                    self.contract_ids_input
                        .iter()
                        .map(|_| String::new())
                        .collect::<Vec<_>>(),
                );
            }
            let alias_inputs = self.alias_inputs.as_mut().unwrap();

            // Clone the options to avoid borrowing self.add_contracts_status during the UI closure
            let options = self.maybe_found_contracts.clone();

            use egui::{vec2, Grid};

            let mut clicked_idx: Option<usize> = None; // remember which row’s button was hit

            Grid::new("found_contracts_grid")
                .striped(false)
                .num_columns(3)
                .min_col_width(150.0)
                .spacing(vec2(12.0, 6.0)) // [horiz, vert] spacing between cells
                .show(ui, |ui| {
                    for (idx, id_string) in self.contract_ids_input.iter().enumerate() {
                        let trimmed = id_string.trim().to_string();

                        if options.contains(&trimmed) {
                            // ─ column 1: contract ID label ───────────────────────────────
                            ui.colored_label(Color32::DARK_GREEN, &trimmed);

                            // ─ column 2: editable alias field ───────────────────────────
                            ui.text_edit_singleline(&mut alias_inputs[idx]);

                            // ─ column 3: action button ──────────────────────────────────
                            if ui.button("Set Alias").clicked() {
                                clicked_idx = Some(idx);
                            }

                            ui.end_row(); // ← tells the grid we’ve finished this row
                        } else {
                            not_found.push(trimmed);
                        }
                    }
                });

            // ─ handle the button click AFTER the grid so we can borrow &mut self safely ──
            if let Some(idx) = clicked_idx {
                let trimmed_id_string = self.contract_ids_input[idx].trim();
                let alias = alias_inputs[idx].trim();
                if alias.is_empty() {
                    self.last_alias_result = Some((idx, Err("Alias cannot be empty.".into())));
                } else {
                    // Set the alias in the local db
                    let identifier_result =
                        Identifier::from_string(trimmed_id_string, Encoding::Base58).or_else(
                            |_| {
                                // Try hex if base58 fails
                                hex::decode(trimmed_id_string)
                                    .map_err(|e| e.to_string())
                                    .and_then(|bytes| {
                                        Identifier::from_bytes(&bytes).map_err(|e| e.to_string())
                                    })
                            },
                        );
                    match identifier_result {
                        Ok(identifier) => match self.app_context.get_contract_by_id(&identifier) {
                            Ok(Some(contract)) => {
                                match self
                                    .app_context
                                    .set_contract_alias(&contract.contract.id(), Some(alias))
                                {
                                    Ok(_) => {
                                        self.last_alias_result = Some((
                                            idx,
                                            Ok(format!("Alias set successfully ({})", alias)),
                                        ));
                                        alias_inputs[idx].clear();
                                    }
                                    Err(e) => {
                                        self.last_alias_result =
                                            Some((idx, Err(format!("Failed to set alias: {}", e))));
                                    }
                                }
                            }
                            _ => {
                                self.last_alias_result = Some((
                                    idx,
                                    Err(format!("Contract not found for ID {}", trimmed_id_string)),
                                ));
                            }
                        },
                        Err(e) => {
                            self.last_alias_result =
                                Some((idx, Err(format!("Invalid ID format: {}", e))));
                        }
                    }
                }
            }

            // Show alias set result message if any
            if let Some((_, ref result)) = self.last_alias_result {
                match result {
                    Ok(msg) => {
                        ui.colored_label(Color32::DARK_GREEN, msg);
                    }
                    Err(msg) => {
                        ui.colored_label(Color32::DARK_RED, msg);
                    }
                }
            }

            ui.add_space(20.0);

            if !not_found.is_empty() {
                ui.label("The following contracts were not found:");
                ui.add_space(10.0);
                for trimmed_id_string in not_found {
                    ui.colored_label(Color32::RED, trimmed_id_string);
                }
            }

            ui.add_space(20.0);
            let button =
                egui::Button::new(RichText::new("Back to Contracts").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .corner_radius(3.0);
            if ui.add(button).clicked() {
                // Return to previous screen
                action = AppAction::PopScreenAndRefresh;
                self.last_alias_result = None;
            }
        });

        action
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
            BackendTaskSuccessResult::FetchedContracts(maybe_found_contracts) => {
                let maybe_contracts: Vec<_> = self
                    .contract_ids_input
                    .iter()
                    .filter(|input_id| {
                        maybe_found_contracts.iter().flatten().any(|contract| {
                            let trimmed = input_id.trim();
                            contract.id().to_string(Encoding::Base58) == trimmed
                                || hex::encode(contract.id()) == trimmed
                        })
                    })
                    .cloned()
                    .collect();
                self.add_contracts_status = AddContractsStatus::Complete(maybe_contracts.clone());
                self.maybe_found_contracts = maybe_contracts;
            }
            _ => {
                // Nothing
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                ("Add Contracts", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenDocumentQuery,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Add Contracts");
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
                            .corner_radius(3.0);
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
                        "Fetching contracts... Time taken so far: {}",
                        display_time
                    ));
                }
                AddContractsStatus::Complete(_) => {
                    action |= self.show_success_screen(ui);
                }
            }
        });

        action
    }
}
