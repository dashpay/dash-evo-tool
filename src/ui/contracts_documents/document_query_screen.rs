use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::ui::components::contract_chooser_panel::add_contract_chooser_panel;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, ScreenLike, ScreenType};
use chrono::{DateTime, Utc};
use dash_sdk::dpp::prelude::DocumentType;
use egui::Context;
use std::sync::Arc;

pub struct DocumentQueryScreen {
    pub app_context: Arc<AppContext>,
    error_message: Option<(String, MessageType, DateTime<Utc>)>,
    contract_search_term: String,
    document_query: String,
    selected_data_contract: DataContract,
    selected_document_type: DocumentType,
    matching_documents: Vec<String>,
}

impl DocumentQueryScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let selected_document_type = app_context.dpns_contract.document_type_for_name("domain");
        Self {
            app_context: app_context.clone(),
            error_message: None,
            contract_search_term: String::new(),
            document_query: format!("SELECT * FROM {}", selected_document_type),
            selected_data_contract: app_context.dpns_contract,
            selected_document_type,
            matching_documents: vec![],
        }
    }

    fn dismiss_error(&mut self) {
        self.error_message = None;
    }

    fn check_error_expiration(&mut self) {
        if let Some((_, _, timestamp)) = &self.error_message {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(*timestamp);

            // Automatically dismiss the error message after 5 seconds
            if elapsed.num_seconds() > 5 {
                self.dismiss_error();
            }
        }
    }

    fn show_input_field(&mut self, ui: &mut Ui) {
        ui.label("Document SQL query:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(self.document_query);
            let button = egui::Button::new(RichText::new("Go").color(Color32::WHITE))
                .fill(Color32::from_rgb(0, 128, 255))
                .frame(true)
                .rounding(3.0);
            if ui.add(button).clicked() {
                // Set the status to waiting and capture the current time
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();
                self.register_dpns_name_status = DocumentQueryStatus::WaitingForResult(now);
                action = AppAction::BackendTask(BackendTask::DocumentTask(FetchDocuments(
                    self.document_query,
                )));
            }
        });
    }

    fn show_output(&self, ui: &mut Ui) {
        ui.separator();
        ui.label("Matching documents:");

        ScrollArea::vertical().show(ui, |ui| {
            ui.set_width(ui.available_width()); // Make the scroll area take the entire width

            if let Some(ref json) = self.matching_documents {
                ui.add(
                    TextEdit::multiline(&mut json.clone())
                        .desired_rows(10)
                        .desired_width(ui.available_width()) // Make the output take the entire width
                        .font(egui::TextStyle::Monospace), // Use a monospace font for JSON
                );
            } else if let Some(ref error) = self.error_message {
                ui.colored_label(egui::Color32::RED, error.0);
            } else {
                ui.label("No valid documents parsed yet.");
            }
        });
    }
}

impl ScreenLike for DocumentQueryScreen {
    fn refresh(&mut self) {}

    fn display_message(&mut self, message: &str, message_type: MessageType) {
        self.error_message = Some((message.to_string(), message_type, Utc::now()));
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::Documents(documents) => self.matching_documents = documents,
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

        action |=
            add_contract_chooser_panel(ctx, &mut self.contract_search_term, &self.app_context);

        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_input_field(ui);

            let parser = DocumentQueryTextInputParser::new(self.selected_data_contract);
            match parser.parse_input(&query) {
                Ok(drive_document_query) => {
                    // BackendTask to query documents
                }
                Err(e) => {
                    self.error_message = Some((
                        format!("Failed to parse query properly: {}", e),
                        MessageType::Error,
                        Utc::now(),
                    ));
                }
            }

            self.show_output(ui);
        });

        action
    }
}
