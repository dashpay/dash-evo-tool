use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::BackendTaskSuccessResult;
use dash_sdk::dpp::serialization::PlatformDeserializableWithPotentialValidationFromVersionedStructure;
use dash_sdk::platform::DataContract;
use eframe::egui::{Color32, Context, ScrollArea, TextEdit, Ui};
use std::sync::Arc;
// ======================= 1.  Data & helpers =======================

#[derive(PartialEq)]
enum ContractParseStatus {
    NotStarted,
    Error(String),
    Complete,
}

/// Visualiser for hex-encoded `Contract`
pub struct ContractVisualizerScreen {
    pub app_context: Arc<AppContext>,

    // ---- raw input ----------
    input_data_hex: String,

    // ---- parsed output -------
    parsed_json: Option<String>,
    parse_status: ContractParseStatus,

    // ---- helper for chooser search ----
    // Allow dead_code: This field provides search functionality for contract selection,
    // useful for filtering contracts in the visualizer interface
    #[allow(dead_code)]
    contract_search_term: String,
}

impl ContractVisualizerScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: Arc::clone(app_context),

            input_data_hex: String::new(),

            parsed_json: None,
            parse_status: ContractParseStatus::NotStarted,

            contract_search_term: String::new(),
        }
    }

    // --------------- core parsing ---------------

    fn parse_input(&mut self) {
        // clear previous
        self.parsed_json = None;
        self.parse_status = ContractParseStatus::NotStarted;

        // hex decode
        let Ok(bytes) = hex::decode(&self.input_data_hex) else {
            self.parse_status = ContractParseStatus::Error("Invalid hex".to_owned());
            return;
        };

        match DataContract::versioned_deserialize(
            &bytes,
            false,
            self.app_context.platform_version(),
        ) {
            Ok(data_contract) => match serde_json::to_string_pretty(&data_contract) {
                Ok(json) => {
                    self.parsed_json = Some(json);
                    self.parse_status = ContractParseStatus::Complete;
                }
                Err(e) => {
                    self.parse_status = ContractParseStatus::Error(format!("JSON error: {e}"));
                }
            },
            Err(e) => {
                self.parse_status =
                    ContractParseStatus::Error(format!("Deserialisation error: {e}"));
            }
        }
    }

    // --------------- egui helpers ---------------

    fn show_input(&mut self, ui: &mut Ui) {
        ui.label("Hex-encoded Contract:");
        let resp = ui.add(
            TextEdit::multiline(&mut self.input_data_hex)
                .desired_rows(4)
                .desired_width(ui.available_width())
                .code_editor(),
        );
        if resp.changed() {
            self.parse_input();
        }
    }

    fn show_output(&mut self, ui: &mut Ui) {
        ui.separator();
        ui.add_space(6.0);
        ui.label("Result:");

        ScrollArea::vertical().show(ui, |ui| match &self.parse_status {
            ContractParseStatus::Complete => {
                ui.monospace(self.parsed_json.as_ref().unwrap());
            }
            ContractParseStatus::Error(msg) => {
                ui.colored_label(Color32::RED, format!("Error: {msg}"));
            }
            ContractParseStatus::NotStarted => {
                ui.colored_label(Color32::GRAY, "Awaiting input …");
            }
        });
    }
}

// ======================= 2.  ScreenLike impl =======================

impl crate::ui::ScreenLike for ContractVisualizerScreen {
    fn display_message(&mut self, msg: &str, t: crate::ui::MessageType) {
        if matches!(t, crate::ui::MessageType::Error) {
            self.parse_status = ContractParseStatus::Error(msg.to_owned());
        }
    }
    fn display_task_result(&mut self, _r: BackendTaskSuccessResult) {}
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
            crate::ui::RootScreenType::RootScreenToolsContractVisualizerScreen,
        );
        action |= add_tools_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        /* ---------- central panel ---------- */
        action |= island_central_panel(ctx, |ui| {
            self.show_input(ui);
            self.show_output(ui);
            AppAction::None
        });

        action
    }
}
