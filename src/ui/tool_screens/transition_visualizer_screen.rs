use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{RootScreenType, ScreenLike};
use base64::{engine::general_purpose::STANDARD, Engine};
use dash_sdk::dpp::serialization::PlatformDeserializable;
use dash_sdk::dpp::state_transition::StateTransition;
use eframe::egui::{self, Context, ScrollArea, TextEdit, Ui};
use std::sync::Arc;

pub struct TransitionVisualizerScreen {
    pub app_context: Arc<AppContext>,
    input_data: String,
    parsed_json: Option<String>,
    error_message: Option<String>,
}

impl TransitionVisualizerScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            input_data: String::new(),
            parsed_json: None,
            error_message: None,
        }
    }

    fn parse_input(&mut self) {
        // Clear previous messages
        self.parsed_json = None;
        self.error_message = None;

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
                        // Convert state transition to JSON
                        match serde_json::to_string_pretty(&state_transition) {
                            Ok(json) => self.parsed_json = Some(json),
                            Err(e) => {
                                self.error_message =
                                    Some(format!("Failed to serialize to JSON: {}", e))
                            }
                        }
                    }
                    Err(e) => {
                        self.error_message =
                            Some(format!("Failed to parse state transition: {}", e))
                    }
                }
            }
            Err(e) => self.error_message = Some(e),
        }
    }

    fn show_input_field(&mut self, ui: &mut Ui) {
        ui.label("Enter hex or base64 encoded state transition:");
        let response = ui.add(
            TextEdit::multiline(&mut self.input_data)
                .desired_rows(6)
                .desired_width(ui.available_width())
                .code_editor(),
        );

        // If the user changes the input, parse it again
        if response.changed() {
            self.parse_input();
        }
    }

    fn show_output(&self, ui: &mut Ui) {
        ui.separator();
        ui.label("Parsed State Transition:");

        ScrollArea::vertical().show(ui, |ui| {
            ui.set_width(ui.available_width()); // Make the scroll area take the entire width

            if let Some(ref json) = self.parsed_json {
                ui.add(
                    TextEdit::multiline(&mut json.clone())
                        .desired_rows(10)
                        .desired_width(ui.available_width()) // Make the output take the entire width
                        .font(egui::TextStyle::Monospace), // Use a monospace font for JSON
                );
            } else if let Some(ref error) = self.error_message {
                ui.colored_label(egui::Color32::RED, error);
            } else {
                ui.label("No valid state transition parsed yet.");
            }
        });
    }
}

impl ScreenLike for TransitionVisualizerScreen {
    fn refresh(&mut self) {
        // No-op for this screen
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Dash Evo Tool", AppAction::None)],
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
            self.show_output(ui);
        });

        action
    }
}
