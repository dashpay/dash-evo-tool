use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::document::DocumentTask::FetchDocuments;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::ui::components::contract_chooser_panel::add_contract_chooser_panel;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, ScreenLike, ScreenType};
use crate::utils::parsers::{DocumentQueryTextInputParser, TextInputParser};
use chrono::{DateTime, Utc};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::document_type::{DocumentType, Index};
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::platform::Document;
use egui::{Color32, Context, Frame, Margin, RichText, ScrollArea, TextEdit, Ui};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct DocumentQueryScreen {
    pub app_context: Arc<AppContext>,
    error_message: Option<(String, MessageType, DateTime<Utc>)>,
    contract_search_term: String,
    document_query: String,
    selected_data_contract: QualifiedContract,
    selected_document_type: DocumentType,
    selected_index: Option<Index>,
    matching_documents: Vec<Document>,
    document_query_status: DocumentQueryStatus,
}

pub enum DocumentQueryStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    Complete,
    ErrorMessage(String),
}

impl DocumentQueryScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let dpns_contract = QualifiedContract {
            contract: Arc::clone(&app_context.dpns_contract).as_ref().clone(),
            alias: Some("dpns".to_string()),
        };

        let selected_document_type = dpns_contract
            .contract
            .document_type_cloned_for_name("domain")
            .expect("Expected to find domain document type in DPNS contract");

        Self {
            app_context: app_context.clone(),
            error_message: None,
            contract_search_term: String::new(),
            document_query: format!("SELECT * FROM {}", selected_document_type.name()),
            selected_data_contract: dpns_contract,
            selected_document_type,
            selected_index: None,
            matching_documents: vec![],
            document_query_status: DocumentQueryStatus::NotStarted,
        }
    }

    fn dismiss_error(&mut self) {
        self.error_message = None;
    }

    fn check_error_expiration(&mut self) {
        if let Some((_, _, timestamp)) = &self.error_message {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(*timestamp);

            // Automatically dismiss the error message after 10 seconds
            if elapsed.num_seconds() > 10 {
                self.dismiss_error();
            }
        }
    }

    fn show_input_field(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.horizontal(|ui| {
            let button_width = 120.0;
            let text_width = ui.available_width() - button_width;

            ui.add(egui::TextEdit::singleline(&mut self.document_query).desired_width(text_width));

            let button = egui::Button::new(
                egui::RichText::new("Fetch Documents").color(egui::Color32::WHITE),
            )
            .fill(egui::Color32::from_rgb(0, 128, 255))
            .frame(true)
            .rounding(3.0);
            if ui.add(button).clicked() {
                let parser =
                    DocumentQueryTextInputParser::new(self.selected_data_contract.contract.clone());
                match parser.parse_input(&self.document_query) {
                    Ok(parsed_query) => {
                        // Set the status to waiting and capture the current time
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs();
                        self.document_query_status = DocumentQueryStatus::WaitingForResult(now);
                        action = AppAction::BackendTask(BackendTask::DocumentTask(FetchDocuments(
                            parsed_query,
                        )));
                    }
                    Err(e) => {
                        self.document_query_status = DocumentQueryStatus::ErrorMessage(format!(
                            "Failed to parse query properly: {}",
                            e
                        ));
                        self.error_message = Some((
                            format!("Failed to parse query properly: {}", e),
                            MessageType::Error,
                            Utc::now(),
                        ));
                    }
                }
            }
        });

        action
    }

    fn show_output(&mut self, ui: &mut Ui) {
        ui.separator();
        ui.add_space(10.0);

        ScrollArea::vertical().show(ui, |ui| {
            ui.set_width(ui.available_width());

            match self.document_query_status {
                DocumentQueryStatus::WaitingForResult(start_time) => {
                    let time_elapsed = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("Time went backwards")
                        .as_secs()
                        - start_time;
                    ui.label(format!(
                        "Fetching documents... Time taken so far: {}",
                        time_elapsed
                    ));
                }
                DocumentQueryStatus::Complete => {
                    let docs: Vec<String> = self
                        .matching_documents
                        .iter()
                        .map(|doc| serde_json::to_string_pretty(doc).unwrap())
                        .collect();

                    let mut json_string_documents = docs.join("\n\n");

                    ui.add(
                        TextEdit::multiline(&mut json_string_documents)
                            .desired_rows(10)
                            .desired_width(ui.available_width())
                            .font(egui::TextStyle::Monospace),
                    );
                }

                DocumentQueryStatus::ErrorMessage(ref message) => {
                    self.error_message =
                        Some((message.to_string(), MessageType::Error, Utc::now()));
                    ui.colored_label(egui::Color32::DARK_RED, message);
                }
                _ => {
                    // Nothing
                }
            }
        });
    }
}

impl ScreenLike for DocumentQueryScreen {
    fn refresh(&mut self) {}

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        // Only display the error message resulting from FetchDocuments backend task
        if message.contains("Error fetching documents") {
            self.document_query_status = DocumentQueryStatus::ErrorMessage(message.to_string());
            self.error_message = Some((message.to_string(), message_type, Utc::now()));
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::Documents(documents) => {
                self.matching_documents = documents
                    .iter()
                    .filter_map(|(_, doc)| doc.clone())
                    .collect();
                self.document_query_status = DocumentQueryStatus::Complete;
            }
            _ => {
                // Nothing
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_error_expiration();
        let add_contract_button = (
            "Add Contracts",
            DesiredAppAction::AddScreenType(ScreenType::AddContracts),
        );
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Contracts", AppAction::None)],
            vec![add_contract_button],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDocumentQuery,
        );

        action |= add_contract_chooser_panel(
            ctx,
            &mut self.contract_search_term,
            &self.app_context,
            &mut self.selected_data_contract,
            &mut self.selected_document_type,
            &mut self.selected_index,
            &mut self.document_query,
        );

        egui::CentralPanel::default()
            .frame(
                Frame::none()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(Margin::same(10.0)),
            )
            .show(ctx, |ui| {
                action |= self.show_input_field(ui);
                self.show_output(ui);
            });

        action
    }
}
