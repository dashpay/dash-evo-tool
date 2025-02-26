use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};

use base64::{engine::general_purpose::STANDARD, Engine};
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::dpp::serialization::PlatformDeserializable;
use dash_sdk::dpp::state_transition::StateTransition;
use eframe::egui::{self, Color32, Context, ScrollArea, TextEdit, Ui};
use egui::RichText;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(PartialEq)]
enum TransitionBroadcastStatus {
    NotStarted,
    Submitting(TimestampMillis),
    Error(String),
    Complete,
}

pub struct TransitionVisualizerScreen {
    pub app_context: Arc<AppContext>,
    input_data: String,
    parsed_json: Option<String>,
    broadcast_status: TransitionBroadcastStatus,
}

impl TransitionVisualizerScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            input_data: String::new(),
            parsed_json: None,
            broadcast_status: TransitionBroadcastStatus::NotStarted,
        }
    }

    fn parse_input(&mut self) {
        // Clear previous parse results...
        self.parsed_json = None;

        // Reset the broadcast status so we no longer show old errors
        // or "Submitting" states from a previous parse/broadcast.
        self.broadcast_status = TransitionBroadcastStatus::NotStarted;

        // Try to decode the input as hex first
        let decoded_bytes = hex::decode(&self.input_data).or_else(|_| {
            STANDARD
                .decode(&self.input_data)
                .map_err(|e| format!("Base64 decode error: {}", e))
        });

        match decoded_bytes {
            Ok(bytes) => {
                // Try to deserialize into a StateTransition
                match StateTransition::deserialize_from_bytes(&bytes) {
                    Ok(state_transition) => {
                        // Convert to JSON
                        match serde_json::to_string_pretty(&state_transition) {
                            Ok(json) => self.parsed_json = Some(json),
                            Err(e) => {
                                self.broadcast_status = TransitionBroadcastStatus::Error(format!(
                                    "Failed to serialize to JSON: {}",
                                    e
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        self.broadcast_status =
                            TransitionBroadcastStatus::Error(format!("Failed to parse: {}", e));
                    }
                }
            }
            Err(e) => {
                self.broadcast_status = TransitionBroadcastStatus::Error(e);
            }
        }
    }

    fn show_input_field(&mut self, ui: &mut Ui) {
        ui.label("Enter hex or base64 encoded state transition:");
        ui.add_space(5.0);
        let response = ui.add(
            TextEdit::multiline(&mut self.input_data)
                .desired_rows(6)
                .desired_width(ui.available_width())
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

        // Show the JSON if we have it
        ScrollArea::vertical().show(ui, |ui| {
            if let Some(ref json) = self.parsed_json {
                ui.add_space(5.0);
                ui.add(
                    TextEdit::multiline(&mut json.clone())
                        .desired_rows(10)
                        .desired_width(ui.available_width())
                        .font(egui::TextStyle::Monospace),
                );

                ui.add_space(10.0);

                // if we are NotStarted or in an Error state, show the button
                if matches!(
                    self.broadcast_status,
                    TransitionBroadcastStatus::NotStarted | TransitionBroadcastStatus::Error(_)
                ) {
                    let mut new_style = (**ui.style()).clone();
                    new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                    ui.set_style(new_style);

                    let button = egui::Button::new(
                        RichText::new("Broadcast Transition to Platform").color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .rounding(3.0);

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
                    ui.label("No state transition parsed yet.");
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
            TransitionBroadcastStatus::Error(msg) => {
                ui.colored_label(Color32::DARK_RED, format!("Error: {}", msg));
            }
            TransitionBroadcastStatus::Complete => {
                ui.colored_label(
                    Color32::DARK_GREEN,
                    "Successfully broadcasted state transition.",
                );
            }
        }

        app_action
    }
}

impl ScreenLike for TransitionVisualizerScreen {
    fn display_message(&mut self, message: &str, message_type: MessageType) {
        match message_type {
            MessageType::Success => {
                self.broadcast_status = TransitionBroadcastStatus::Complete;
            }
            MessageType::Error => {
                self.broadcast_status = TransitionBroadcastStatus::Error(message.to_string());
            }
            MessageType::Info => {
                // Could do nothing or handle info
            }
        }
    }

    fn display_task_result(
        &mut self,
        _backend_task_success_result: crate::ui::BackendTaskSuccessResult,
    ) {
        // Nothing
        // If we don't include this, messages from the ZMQ listener will keep popping up
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

        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_input_field(ui);
            action |= self.show_output(ui);
        });

        action
    }
}
