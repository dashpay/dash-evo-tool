use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::DashColors;
use crate::ui::{MessageType, RootScreenType, ScreenLike};

use base64::{Engine, engine::general_purpose::STANDARD};
use dash_sdk::dpp::platform_serialization::PLATFORM_BINCODE_CONFIG;
use dash_sdk::drive::grovedb::operations::proof::GroveDBProof;
use eframe::egui::{self, ScrollArea, TextEdit, Ui};
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
                // Pasted bytes are untrusted: decode them as upstream's
                // `prettify_proof` does, in Platform's shared configuration.
                let grovedb_proof: Result<GroveDBProof, _> =
                    bincode::decode_from_slice_untrusted(&bytes, PLATFORM_BINCODE_CONFIG)
                        .map(|(a, _)| a);
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
        let dark_mode = ui.style().visuals.dark_mode;
        let response = ui.add(
            TextEdit::multiline(&mut self.input_data)
                .desired_rows(6)
                .desired_width(ui.available_width())
                .text_color(DashColors::text_primary(dark_mode))
                .background_color(DashColors::input_background(dark_mode))
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
                let dark_mode = ui.style().visuals.dark_mode;
                ui.add(
                    TextEdit::multiline(&mut json.clone())
                        .desired_rows(10)
                        .desired_width(ui.available_width())
                        .text_color(DashColors::text_primary(dark_mode))
                        .background_color(DashColors::input_background(dark_mode))
                        .font(egui::TextStyle::Monospace),
                );

                ui.add_space(10.0);
            } else if let Some(ref error) = self.error {
                ui.add_space(5.0);
                let dark_mode = ui.style().visuals.dark_mode;
                ui.add(
                    TextEdit::multiline(&mut error.clone())
                        .desired_rows(10)
                        .desired_width(ui.available_width())
                        .text_color(DashColors::text_primary(dark_mode))
                        .background_color(DashColors::input_background(dark_mode))
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

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![("Tools", AppAction::None)],
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            RootScreenType::RootScreenToolsProofVisualizerScreen,
        );

        action |= add_tools_subscreen_chooser_panel(ui, self.app_context.as_ref());

        action |= island_central_panel(ui, |ui| {
            self.show_input_field(ui);
            self.show_output(ui);
            AppAction::None
        });

        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::drive::grovedb::operations::proof::{
        GroveDBProofV0, MerkOnlyLayerProof, ProveOptions,
    };

    fn screen_with_input(ctx: &Arc<AppContext>, input: String) -> ProofVisualizerScreen {
        let mut screen = ProofVisualizerScreen::new(ctx);
        screen.input_data = input;
        screen.parse_input();
        screen
    }

    /// A proof encoded the way Platform encodes it is shown.
    #[test]
    fn a_platform_encoded_proof_is_shown() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(temp_dir.path());
        let proof = GroveDBProof::V0(GroveDBProofV0 {
            root_layer: MerkOnlyLayerProof {
                merk_proof: Vec::new(),
                lower_layers: Default::default(),
            },
            prove_options: ProveOptions::default(),
        });
        let bytes = bincode::encode_to_vec(&proof, PLATFORM_BINCODE_CONFIG).expect("encode");

        let screen = screen_with_input(&ctx, hex::encode(bytes));

        assert_eq!(screen.error, None);
        assert!(screen.proof_string.is_some());
    }

    /// Pasted bytes are untrusted: a length prefix far beyond the input is
    /// refused as an error instead of being allocated.
    #[test]
    fn an_oversized_length_prefix_is_refused() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let ctx = crate::context::test_support::test_app_context(temp_dir.path());

        // `GroveDBProof::V0`, then a merk proof claiming 2^63 - 1 bytes.
        let screen = screen_with_input(&ctx, "00fd7fffffffffffffff".to_string());

        assert!(screen.proof_string.is_none());
        assert!(screen.error.is_some());
    }
}
