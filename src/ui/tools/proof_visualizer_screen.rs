use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{MessageType, RootScreenType, ScreenLike};

use base64::{Engine, engine::general_purpose::STANDARD};
use dash_sdk::drive::grovedb::operations::proof::GroveDBProof;
use eframe::egui::{self, Context, ScrollArea, TextEdit, Ui};
use egui::Color32;
use std::sync::Arc;

pub struct ProofVisualizerScreen {
    pub app_context: Arc<AppContext>,
    input_data: String,
    proof_string: Option<String>,
    error: Option<String>,
}

impl ProofVisualizerScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
            input_data: String::new(),
            proof_string: None,
            error: None,
        }
    }

    fn parse_input(&mut self) {
        // Clear previous parse results...
        self.proof_string = None;
        self.error = None;

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
                let config = bincode::config::standard()
                    .with_big_endian()
                    .with_no_limit();
                let grovedb_proof: Result<GroveDBProof, _> =
                    bincode::decode_from_slice(&bytes, config).map(|(a, _)| a);
                // Try to deserialize into a StateTransition
                match grovedb_proof {
                    Ok(proof) => {
                        self.proof_string = Some(proof.to_string());
                    }
                    Err(e) => {
                        self.error = Some(e.to_string());
                    }
                }
            }
            Err(e) => {
                self.error = Some(e);
            }
        }
    }

    fn show_input_field(&mut self, ui: &mut Ui) {
        ui.label("Enter hex, base64, or comma-separated integers for GroveDB proof:");
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

    fn show_output(&mut self, ui: &mut Ui) {
        ui.separator();
        ui.add_space(10.0);
        ui.label("Parsed Proof:");

        // Show the JSON if we have it
        ScrollArea::vertical().show(ui, |ui| {
            if let Some(ref json) = self.proof_string {
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
            } else if let Some(ref error) = self.error {
                ui.add_space(5.0);
                let dark_mode = ui.ctx().style().visuals.dark_mode;
                ui.add(
                    TextEdit::multiline(&mut error.clone())
                        .desired_rows(10)
                        .desired_width(ui.available_width())
                        .text_color(crate::ui::theme::DashColors::text_primary(dark_mode))
                        .background_color(crate::ui::theme::DashColors::input_background(dark_mode))
                        .font(egui::TextStyle::Monospace),
                );

                ui.add_space(10.0);
            } else {
                ui.colored_label(Color32::GRAY, "No proof parsed yet.");
            }
        });

        // Show status
        ui.add_space(5.0);
    }
}

impl ScreenLike for ProofVisualizerScreen {
    fn display_message(&mut self, _message: &str, _message_type: MessageType) {
        // Implement message display if needed
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
            RootScreenType::RootScreenToolsProofVisualizerScreen,
        );

        action |= add_tools_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        action |= island_central_panel(ctx, |ui| {
            self.show_input_field(ui);
            self.show_output(ui);
            AppAction::None
        });

        action
    }
}
