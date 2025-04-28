use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::BackendTaskSuccessResult;

use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::document::serialization_traits::DocumentPlatformConversionMethodsV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
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

/// Extremely compact chooser: just two combo-boxes (Contract ▸ Doc-Type)
///
/// * No collapsible tree.
/// * Optional search box on top.
/// * Emits `ContractTask::RemoveContract` via a small “🗑” button next to the contract picker.
#[allow(clippy::too_many_arguments)]
pub fn add_simple_contract_doc_type_chooser(
    ctx: &Context,
    search_term: &mut String,

    app_context: &Arc<AppContext>,

    selected_contract: &mut Option<QualifiedContract>,
    selected_doc_type: &mut Option<DocumentType>,
) -> AppAction {
    let mut action = AppAction::None;

    /* ----------------------- 1.  Load + filter contracts ---------------------- */
    let contracts = app_context.get_contracts(None, None).unwrap_or_default();

    // filter by alias or id
    let filtered: Vec<_> = contracts
        .iter()
        .filter(|qc| {
            let key = qc
                .alias
                .clone()
                .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58));
            key.to_lowercase().contains(&search_term.to_lowercase())
        })
        .cloned()
        .collect();

    /* ----------------------- 2.  Paint UI  ----------------------------------- */
    egui::Window::new("Document Source")
        .default_pos([10.0, 90.0])
        .resizable(true)
        .show(ctx, |ui| {
            // search bar
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(search_term);
            });

            ui.add_space(6.0);

            // --- contract picker ------------------------------------------------
            egui::ComboBox::from_id_salt("contract_combo")
                .width(220.0)
                .selected_text(match selected_contract {
                    Some(qc) => qc
                        .alias
                        .clone()
                        .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58)),
                    None => "Select Contract…".into(),
                })
                .show_ui(ui, |cui| {
                    for qc in &filtered {
                        let label = qc
                            .alias
                            .clone()
                            .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58));
                        if cui
                            .selectable_label(selected_contract.as_ref() == Some(qc), label.clone())
                            .clicked()
                        {
                            *selected_contract = Some(qc.clone());

                            // default doc-type: first one
                            let doc_type = qc.contract.document_types().values().next().cloned();
                            *selected_doc_type = doc_type;
                        }
                    }
                });

            ui.add_space(8.0);

            // --- document-type picker -----------------------------------------
            egui::ComboBox::from_id_salt("doctype_combo")
                .width(220.0)
                .selected_text(
                    selected_doc_type
                        .as_ref()
                        .map(|d| d.name().to_owned())
                        .unwrap_or_else(|| "Select Doc-Type…".into()),
                )
                .show_ui(ui, |dui| {
                    if let Some(qc) = selected_contract {
                        for (name, dt) in qc.contract.document_types() {
                            if dui
                                .selectable_label(
                                    selected_doc_type
                                        .as_ref()
                                        .map(|cur| cur.name() == name)
                                        .unwrap_or(false),
                                    name,
                                )
                                .clicked()
                            {
                                *selected_doc_type =
                                    qc.contract.document_type_cloned_for_name(name).ok();
                            }
                        }
                    } else {
                        dui.label("Pick a contract first");
                    }
                });
        });

    action
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
        let (contract, doc_type) = match (&self.selected_contract, &self.selected_document_type) {
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
        match Document::from_bytes(&bytes, doc_type.as_ref(), self.app_context.platform_version) {
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
            crate::ui::RootScreenType::RootScreenToolsTransitionVisualizerScreen,
        );
        action |= add_tools_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        /* ---------- simple dual-combo chooser ---------- */
        action |= add_simple_contract_doc_type_chooser(
            ctx,
            &mut self.contract_search_term,
            &self.app_context,
            &mut self.selected_contract,
            &mut self.selected_document_type,
        );

        /* ---------- central panel ---------- */
        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_input(ui);
            self.show_output(ui);
        });

        action
    }
}
