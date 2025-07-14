use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::contract::ContractTask;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};

use base64::{Engine, engine::general_purpose::STANDARD};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::dpp::serialization::PlatformDeserializable;
use dash_sdk::dpp::state_transition::StateTransition;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Color32, Context, ScrollArea, TextEdit, Ui, Window};
use egui::RichText;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(PartialEq)]
enum TransitionBroadcastStatus {
    NotStarted,
    Submitting(TimestampMillis),
    Error(String, Instant),
    Complete(Instant),
}

pub struct TransitionVisualizerScreen {
    pub app_context: Arc<AppContext>,
    input_data: String,
    parsed_json: Option<String>,
    broadcast_status: TransitionBroadcastStatus,
    show_contract_dialog: bool,
    selected_contract_id: Option<String>,
    detected_contract_ids: Vec<String>,
    contract_fetch_message: Option<(String, Instant)>,
}

impl TransitionVisualizerScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            input_data: String::new(),
            parsed_json: None,
            broadcast_status: TransitionBroadcastStatus::NotStarted,
            show_contract_dialog: false,
            selected_contract_id: None,
            detected_contract_ids: Vec::new(),
            contract_fetch_message: None,
        }
    }

    fn extract_contract_ids(value: &Value, ids: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                // Check if this is a contractBounds object with an id
                if map.contains_key("type") && map.contains_key("id") {
                    if let (Some(Value::String(type_str)), Some(Value::String(id))) =
                        (map.get("type"), map.get("id"))
                    {
                        if type_str == "singleContract" {
                            ids.push(id.clone());
                        }
                    }
                }
                // Recursively check all values
                for val in map.values() {
                    Self::extract_contract_ids(val, ids);
                }
            }
            Value::Array(arr) => {
                for val in arr {
                    Self::extract_contract_ids(val, ids);
                }
            }
            _ => {}
        }
    }

    fn parse_input(&mut self) {
        // Clear previous parse results...
        self.parsed_json = None;
        self.detected_contract_ids.clear();

        // Reset the broadcast status so we no longer show old errors
        // or "Submitting" states from a previous parse/broadcast.
        self.broadcast_status = TransitionBroadcastStatus::NotStarted;

        // First, try to parse as comma-separated integers
        let decoded_bytes = if self.input_data.contains(',') {
            // Try parsing as comma-separated integers
            self.input_data
                .split(',')
                .filter(|s| !s.trim().is_empty()) // Skip empty segments
                .map(|s| s.trim().parse::<u8>())
                .collect::<Result<Vec<u8>, _>>()
                .map_err(|e| format!("Failed to parse comma-separated integers: {}", e))
        } else {
            // Try to decode the input as hex first
            hex::decode(self.input_data.trim()).or_else(|_| {
                STANDARD
                    .decode(self.input_data.trim())
                    .map_err(|e| format!("Base64 decode error: {}", e))
            })
        };

        match decoded_bytes {
            Ok(bytes) => {
                // Try to deserialize into a StateTransition
                match StateTransition::deserialize_from_bytes(&bytes) {
                    Ok(state_transition) => {
                        // Convert to JSON
                        match serde_json::to_string_pretty(&state_transition) {
                            Ok(json) => {
                                self.parsed_json = Some(json.clone());

                                // Extract contract IDs from the JSON
                                if let Ok(json_value) = serde_json::from_str::<Value>(&json) {
                                    Self::extract_contract_ids(
                                        &json_value,
                                        &mut self.detected_contract_ids,
                                    );
                                }
                            }
                            Err(e) => {
                                self.broadcast_status = TransitionBroadcastStatus::Error(
                                    format!("Failed to serialize to JSON: {}", e),
                                    Instant::now(),
                                );
                            }
                        }
                    }
                    Err(e) => {
                        self.broadcast_status = TransitionBroadcastStatus::Error(
                            format!("Failed to parse: {}", e),
                            Instant::now(),
                        );
                    }
                }
            }
            Err(e) => {
                self.broadcast_status = TransitionBroadcastStatus::Error(e, Instant::now());
            }
        }
    }

    fn show_input_field(&mut self, ui: &mut Ui) {
        ui.label("Enter hex, base64, or comma-separated integers for state transition:");
        ui.add_space(5.0);
        let dark_mode = ui.ctx().style().visuals.dark_mode;
        let response = ui.add(
            TextEdit::multiline(&mut self.input_data)
                .desired_rows(6)
                .desired_width(ui.available_width())
                .text_color(crate::ui::theme::DashColors::text_primary(dark_mode))
                .background_color(crate::ui::theme::DashColors::input_background(dark_mode))
                .code_editor(),
        );

        ui.add_space(10.0);

        if response.changed() {
            // Re-parse
            self.parse_input();
        }
    }

    fn show_output(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;

        ui.separator();
        ui.add_space(10.0);
        ui.label("Parsed State Transition:");

        // Show detected contract IDs if any
        if !self.detected_contract_ids.is_empty() {
            ui.add_space(5.0);
            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.label("📄 Contract references found:");
                    ui.add_space(10.0);
                    for (i, contract_id) in self.detected_contract_ids.iter().enumerate() {
                        if i > 0 {
                            ui.label("•");
                        }
                        if ui
                            .link(contract_id)
                            .on_hover_text("Click to view contract")
                            .clicked()
                        {
                            self.selected_contract_id = Some(contract_id.clone());
                            self.show_contract_dialog = true;
                        }
                    }
                });
            });
            ui.add_space(5.0);
        }

        // Show the JSON if we have it
        ScrollArea::vertical().show(ui, |ui| {
            if let Some(ref json) = self.parsed_json {
                ui.add_space(5.0);
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                ui.add(
                    TextEdit::multiline(&mut json.clone())
                        .desired_rows(10)
                        .desired_width(ui.available_width())
                        .text_color(crate::ui::theme::DashColors::text_primary(dark_mode))
                        .background_color(crate::ui::theme::DashColors::input_background(dark_mode))
                        .font(egui::TextStyle::Monospace),
                );

                ui.add_space(10.0);

                // if we are NotStarted or in an Error state, show the button
                if matches!(
                    self.broadcast_status,
                    TransitionBroadcastStatus::NotStarted | TransitionBroadcastStatus::Error(_, _)
                ) {
                    let mut new_style = (**ui.style()).clone();
                    new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                    ui.set_style(new_style);

                    let button = egui::Button::new(
                        RichText::new("Broadcast Transition to Platform").color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .corner_radius(3.0);

                    if ui.add(button).clicked() {
                        // Mark as submitting
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        self.broadcast_status = TransitionBroadcastStatus::Submitting(now);

                        if let Some(json) = &self.parsed_json {
                            if let Ok(state_transition) = serde_json::from_str(json) {
                                app_action = AppAction::BackendTask(
                                    BackendTask::BroadcastStateTransition(state_transition),
                                );
                            }
                        }
                    }
                }
            } else {
                // If parsed_json is None
                if matches!(self.broadcast_status, TransitionBroadcastStatus::NotStarted) {
                    ui.colored_label(Color32::GRAY, "No state transition parsed yet.");
                }
            }
        });

        // Show status
        ui.add_space(5.0);
        match &self.broadcast_status {
            TransitionBroadcastStatus::NotStarted => {}
            TransitionBroadcastStatus::Submitting(start_time) => {
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
                    "Broadcasting... Time taken so far: {}",
                    display_time
                ));
            }
            TransitionBroadcastStatus::Error(msg, timestamp) => {
                let elapsed = timestamp.elapsed();
                if elapsed < Duration::from_secs(8) {
                    // Calculate fade effect for last 2 seconds
                    let alpha = if elapsed > Duration::from_secs(6) {
                        let fade_progress = (8.0 - elapsed.as_secs_f32()) / 2.0;
                        (fade_progress * 255.0) as u8
                    } else {
                        255
                    };
                    ui.colored_label(
                        Color32::from_rgba_premultiplied(139, 0, 0, alpha), // Dark red
                        format!("Error: {}", msg),
                    );

                    // Request repaint to update the fade effect
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                } else {
                    // Clear the error after 8 seconds
                    self.broadcast_status = TransitionBroadcastStatus::NotStarted;
                }
            }
            TransitionBroadcastStatus::Complete(timestamp) => {
                let elapsed = timestamp.elapsed();
                if elapsed < Duration::from_secs(8) {
                    // Calculate fade effect for last 2 seconds
                    let alpha = if elapsed > Duration::from_secs(6) {
                        let fade_progress = (8.0 - elapsed.as_secs_f32()) / 2.0;
                        (fade_progress * 255.0) as u8
                    } else {
                        255
                    };
                    ui.colored_label(
                        Color32::from_rgba_premultiplied(0, 100, 0, alpha), // Dark green
                        "Successfully broadcasted state transition.",
                    );

                    // Request repaint to update the fade effect
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                } else {
                    // Clear the status after 8 seconds
                    self.broadcast_status = TransitionBroadcastStatus::NotStarted;
                }
            }
        }

        // Show contract fetch success message if any
        let mut clear_message = false;
        if let Some((message, timestamp)) = &self.contract_fetch_message {
            let elapsed = timestamp.elapsed();
            if elapsed < Duration::from_secs(8) {
                ui.add_space(10.0);
                let message_text = message.clone();
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        // Calculate fade effect for last 2 seconds
                        let alpha = if elapsed > Duration::from_secs(6) {
                            let fade_progress = (8.0 - elapsed.as_secs_f32()) / 2.0;
                            (fade_progress * 255.0) as u8
                        } else {
                            255
                        };

                        ui.colored_label(
                            Color32::from_rgba_premultiplied(0, 150, 0, alpha),
                            &message_text,
                        );

                        ui.add_space(20.0);

                        // Add button with same fade effect
                        let button_color = Color32::from_rgba_premultiplied(70, 130, 180, alpha); // Steel blue
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("View in Contracts").color(
                                        Color32::from_rgba_premultiplied(255, 255, 255, alpha),
                                    ),
                                )
                                .fill(button_color)
                                .frame(true)
                                .min_size(egui::vec2(140.0, 0.0)),
                            )
                            .clicked()
                        {
                            app_action |=
                                AppAction::SetMainScreen(RootScreenType::RootScreenDocumentQuery);
                            clear_message = true; // Mark for clearing after the UI block
                        }
                    });
                });

                // Request repaint to update the message timeout and fade effect
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            } else {
                // Clear the message after 8 seconds
                clear_message = true;
            }
        }

        if clear_message {
            self.contract_fetch_message = None;
        }

        app_action
    }
}

impl ScreenLike for TransitionVisualizerScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                // Only update broadcast status if we're actually broadcasting
                if matches!(
                    self.broadcast_status,
                    TransitionBroadcastStatus::Submitting(_)
                ) {
                    self.broadcast_status = TransitionBroadcastStatus::Complete(Instant::now());
                }
            }
            MessageType::Error => {
                self.broadcast_status =
                    TransitionBroadcastStatus::Error(message.to_string(), Instant::now());
            }
            MessageType::Info => {
                // Could do nothing or handle info
            }
        }
    }

    fn display_task_result(
        &mut self,
        backend_task_success_result: crate::ui::BackendTaskSuccessResult,
    ) {
        match backend_task_success_result {
            crate::ui::BackendTaskSuccessResult::FetchedContract(contract) => {
                let contract_id = contract.id().to_string(Encoding::Base58);
                self.contract_fetch_message = Some((
                    format!("✅ Contract {} fetched successfully", contract_id),
                    Instant::now(),
                ));
            }
            crate::ui::BackendTaskSuccessResult::FetchedContracts(contracts) => {
                let count = contracts.iter().filter(|c| c.is_some()).count();
                self.contract_fetch_message = Some((
                    format!("✅ {} contract(s) fetched successfully", count),
                    Instant::now(),
                ));
            }
            _ => {
                // Other results are handled globally
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Tools", AppAction::None)],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenToolsTransitionVisualizerScreen,
        );

        action |= add_tools_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        action |= island_central_panel(ctx, |ui| {
            self.show_input_field(ui);
            self.show_output(ui)
        });

        // Show contract fetch dialog if needed
        if self.show_contract_dialog {
            let mut dialog_action = AppAction::None;
            let mut close_dialog = false;

            Window::new("Fetch Contract")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(10.0);

                        if let Some(ref contract_id) = self.selected_contract_id {
                            ui.label(format!("Contract ID: {}", contract_id));
                            ui.add_space(10.0);

                            // Check if contract already exists
                            let contract_exists = self
                                .app_context
                                .get_contracts(None, None)
                                .unwrap_or_default()
                                .iter()
                                .any(|c| {
                                    c.contract.id().to_string(Encoding::Base58) == *contract_id
                                });

                            if contract_exists {
                                ui.label("This contract already exists locally.");
                                ui.add_space(10.0);

                                if ui.button("Go to Contract").clicked() {
                                    // Navigate to contract screen
                                    action |= AppAction::SetMainScreen(
                                        RootScreenType::RootScreenDocumentQuery,
                                    );
                                    close_dialog = true;
                                }
                            } else {
                                ui.label("Would you like to fetch this contract from Platform?");
                                ui.add_space(10.0);

                                ui.horizontal(|ui| {
                                    if ui.button("Yes, Fetch").clicked() {
                                        // Parse the contract ID string to Identifier
                                        if let Ok(identifier) =
                                            Identifier::from_string(contract_id, Encoding::Base58)
                                        {
                                            dialog_action = AppAction::BackendTask(
                                                BackendTask::ContractTask(Box::new(
                                                    ContractTask::FetchContracts(vec![identifier]),
                                                )),
                                            );
                                        }
                                        close_dialog = true;
                                    }

                                    if ui.button("Cancel").clicked() {
                                        close_dialog = true;
                                    }
                                });
                            }
                        }

                        ui.add_space(10.0);
                    });
                });

            if close_dialog {
                self.show_contract_dialog = false;
                self.selected_contract_id = None;
            }

            action |= dialog_action;
        }

        action
    }
}
