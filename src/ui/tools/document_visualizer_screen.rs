use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::helpers::add_contract_doc_type_chooser_with_filtering;
use crate::ui::BackendTaskSuccessResult;

use dash_sdk::dpp::document::serialization_traits::DocumentPlatformConversionMethodsV0;
use dash_sdk::dpp::{data_contract::document_type::DocumentType, document::Document};
use eframe::egui::{self, Color32, Context, ScrollArea, TextEdit, Ui};
use std::sync::Arc;
// ======================= 1.  Data & helpers =======================

#[derive(PartialEq)]
enum DocumentParseStatus {
    NotStarted,
    WaitingForSelection, // user still must choose contract & type
    Error(String),
    Complete,
}

/// Visualiser for hex-encoded `Document`
pub struct DocumentVisualizerScreen {
    pub app_context: Arc<AppContext>,

    // ---- user selections ----
    selected_contract: Option<QualifiedContract>,
    selected_document_type: Option<DocumentType>,

    // ---- raw input ----------
    input_data_hex: String,

    // ---- parsed output -------
    parsed_json: Option<String>,
    parse_status: DocumentParseStatus,

    // ---- helper for chooser search ----
    contract_search_term: String,
}

impl DocumentVisualizerScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: Arc::clone(app_context),

            selected_contract: None,
            selected_document_type: None,

            input_data_hex: String::new(),

            parsed_json: None,
            parse_status: DocumentParseStatus::WaitingForSelection,

            contract_search_term: String::new(),
        }
    }

    // --------------- core parsing ---------------

    fn parse_input(&mut self) {
        // need selections first
        let (_contract, doc_type) = match (&self.selected_contract, &self.selected_document_type) {
            (Some(c), Some(d)) => (&c.contract, d),
            _ => {
                self.parsed_json = None;
                self.parse_status = DocumentParseStatus::WaitingForSelection;
                return;
            }
        };

        // clear previous
        self.parsed_json = None;
        self.parse_status = DocumentParseStatus::NotStarted;

        // hex decode
        let Ok(bytes) = hex::decode(&self.input_data_hex) else {
            self.parse_status = DocumentParseStatus::Error("Invalid hex".to_owned());
            return;
        };

        // deserialise
        match Document::from_bytes(
            &bytes,
            doc_type.as_ref(),
            self.app_context.platform_version(),
        ) {
            Ok(doc) => match serde_json::to_string_pretty(&doc) {
                Ok(json) => {
                    self.parsed_json = Some(json);
                    self.parse_status = DocumentParseStatus::Complete;
                }
                Err(e) => {
                    self.parse_status = DocumentParseStatus::Error(format!("JSON error: {e}"));
                }
            },
            Err(e) => {
                self.parse_status =
                    DocumentParseStatus::Error(format!("Deserialisation error: {e}"));
            }
        }
    }

    // --------------- egui helpers ---------------

    fn show_input(&mut self, ui: &mut Ui) {
        ui.label("Hex-encoded Document:");
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
            DocumentParseStatus::Complete => {
                ui.monospace(self.parsed_json.as_ref().unwrap());
            }
            DocumentParseStatus::WaitingForSelection => {
                ui.colored_label(Color32::LIGHT_BLUE, "Select a contract and document type.");
            }
            DocumentParseStatus::Error(msg) => {
                ui.colored_label(Color32::RED, format!("Error: {msg}"));
            }
            DocumentParseStatus::NotStarted => {
                ui.label("Awaiting input …");
            }
        });
    }
}

// ======================= 2.  ScreenLike impl =======================

impl crate::ui::ScreenLike for DocumentVisualizerScreen {
    fn display_message(&mut self, msg: &str, t: crate::ui::MessageType) {
        if matches!(t, crate::ui::MessageType::Error) {
            self.parse_status = DocumentParseStatus::Error(msg.to_owned());
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
            crate::ui::RootScreenType::RootScreenToolsDocumentVisualizerScreen,
        );
        action |= add_tools_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        /* ---------- central panel ---------- */
        egui::CentralPanel::default().show(ctx, |ui| {
            /* ---------- simple dual-combo chooser ---------- */
            //todo cache the contracts
            add_contract_doc_type_chooser_with_filtering(
                ui,
                &mut self.contract_search_term,
                &self.app_context,
                &mut self.selected_contract,
                &mut self.selected_document_type,
            );

            ui.add_space(10.0);

            self.show_input(ui);
            self.show_output(ui);
        });

        action
    }
}
