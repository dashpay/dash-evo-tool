use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::contract::ContractTask;
use crate::backend_task::document::DocumentTask::{self, FetchDocumentsPage}; // Updated import
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
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillis;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::transition::purchase_document;
use dash_sdk::platform::{Document, DocumentQuery, Identifier};
use egui::{Color32, Context, Frame, Margin, ScrollArea, Ui};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// A list of Dash-specific fields that do not appear in the
/// normal document_type properties.
pub const DOCUMENT_PRIVATE_FIELDS: &[&str] = &[
    "$id",
    "$ownerId",
    "$version",
    "$revision",
    "$createdAt",
    "$updatedAt",
    "$transferredAt",
    "$createdAtBlockHeight",
    "$updatedAtBlockHeight",
    "$transferredAtBlockHeight",
    "$createdAtCoreBlockHeight",
    "$updatedAtCoreBlockHeight",
];

pub struct DocumentQueryScreen {
    pub app_context: Arc<AppContext>,
    error_message: Option<(String, MessageType, DateTime<Utc>)>,
    contract_search_term: String,
    document_search_term: String,
    document_query: String,
    document_display_mode: DocumentDisplayMode,
    document_fields_selection: HashMap<String, bool>,
    show_fields_dropdown: bool,
    selected_data_contract: QualifiedContract,
    selected_document_type: DocumentType,
    selected_index: Option<Index>,
    pub matching_documents: Vec<Document>,
    document_query_status: DocumentQueryStatus,
    confirm_remove_contract_popup: bool,
    contract_to_remove: Option<Identifier>,
    pending_document_type: DocumentType,
    pending_fields_selection: HashMap<String, bool>,
    // Pagination fields
    current_page: usize,
    pub next_cursors: Vec<Start>,
    has_next_page: bool,
    previous_cursors: Vec<Start>,
}

#[derive(PartialEq, Eq, Clone)]
pub enum DocumentQueryStatus {
    NotStarted,
    WaitingForResult(TimestampMillis),
    Complete,
    ErrorMessage(String),
}

#[derive(PartialEq, Eq, Clone)]
pub enum DocumentDisplayMode {
    Json,
    Yaml,
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

        let mut document_fields_selection = HashMap::new();
        for (field_name, _schema) in selected_document_type.properties().iter() {
            document_fields_selection.insert(field_name.clone(), true);
        }
        for dash_field in DOCUMENT_PRIVATE_FIELDS {
            document_fields_selection.insert((*dash_field).to_string(), false);
        }

        let pending_document_type = selected_document_type.clone();
        let pending_fields_selection = document_fields_selection.clone();

        Self {
            app_context: app_context.clone(),
            error_message: None,
            contract_search_term: String::new(),
            document_search_term: String::new(),
            document_query: format!("SELECT * FROM {}", selected_document_type.name()),
            document_display_mode: DocumentDisplayMode::Yaml,
            document_fields_selection,
            show_fields_dropdown: false,
            selected_data_contract: dpns_contract,
            selected_document_type,
            selected_index: None,
            matching_documents: vec![],
            document_query_status: DocumentQueryStatus::NotStarted,
            confirm_remove_contract_popup: false,
            contract_to_remove: None,
            pending_document_type,
            pending_fields_selection,
            // Initialize pagination fields
            current_page: 1,
            next_cursors: vec![],
            has_next_page: false,
            previous_cursors: Vec::new(),
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

    fn build_document_query_with_cursor(&self, cursor: &Start) -> DocumentQuery {
        let mut query = DocumentQuery::new(
            self.selected_data_contract.contract.clone(),
            self.selected_document_type.name(),
        )
        .expect("Expected to create a new DocumentQuery");
        if self.current_page == 1 {
            query.start = None;
        } else {
            query.start = Some(cursor.clone());
        }
        query
    }

    fn get_previous_cursor(&mut self) -> Option<Start> {
        self.previous_cursors.pop()
    }

    fn get_next_cursor(&mut self) -> Option<Start> {
        self.next_cursors.last().cloned()
    }

    fn show_input_field(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.horizontal(|ui| {
            let button_width = 120.0;
            let text_width = ui.available_width() - button_width;

            ui.add(egui::TextEdit::singleline(&mut self.document_query).desired_width(text_width));

            let button_fetch =
                egui::Button::new(egui::RichText::new("Fetch Documents").color(Color32::WHITE))
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .corner_radius(3.0);

            if ui.add(button_fetch).clicked() {
                self.selected_document_type = self.pending_document_type.clone();
                self.document_fields_selection = self.pending_fields_selection.clone();

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
                        self.current_page = 1; // Reset to first page
                        self.next_cursors = vec![]; // Reset cursor
                        self.previous_cursors.clear(); // Clear previous cursors
                        action = AppAction::BackendTask(BackendTask::DocumentTask(
                            FetchDocumentsPage(parsed_query),
                        ));
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

    fn show_output(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.separator();
        ui.add_space(10.0);

        if !self.matching_documents.is_empty() {
            ui.horizontal(|ui| {
                ui.label("Filter documents:");
                ui.text_edit_singleline(&mut self.document_search_term);

                if ui.button("Select Properties").clicked() {
                    self.show_fields_dropdown = !self.show_fields_dropdown;
                }

                // Display mode toggle
                ui.label("Display as:");
                if ui
                    .selectable_label(
                        self.document_display_mode == DocumentDisplayMode::Yaml,
                        "YAML",
                    )
                    .clicked()
                {
                    self.document_display_mode = DocumentDisplayMode::Yaml;
                }
                if ui
                    .selectable_label(
                        self.document_display_mode == DocumentDisplayMode::Json,
                        "JSON",
                    )
                    .clicked()
                {
                    self.document_display_mode = DocumentDisplayMode::Json;
                }
            });

            if self.show_fields_dropdown {
                // 1) Partition fields into doc-type vs. dash
                let dash_field_set: std::collections::HashSet<&str> =
                    DOCUMENT_PRIVATE_FIELDS.iter().cloned().collect();

                let mut doc_type_fields = Vec::new();
                let mut dash_fields = Vec::new();

                for (field_name, is_checked) in &mut self.document_fields_selection {
                    if dash_field_set.contains(field_name.as_str()) {
                        dash_fields.push((field_name, is_checked));
                    } else {
                        doc_type_fields.push((field_name, is_checked));
                    }
                }

                egui::Window::new("Select Properties")
                    .collapsible(false)
                    .resizable(true)
                    .min_width(400.0)
                    .title_bar(false)
                    .show(ui.ctx(), |ui| {
                        ui.label("Check the properties to display:");
                        ui.add_space(10.0);

                        ui.columns(2, |columns| {
                            columns[0].heading("Document Properties");
                            columns[0].add_space(5.0);
                            for (field_name, is_checked) in &mut doc_type_fields {
                                columns[0].checkbox(is_checked, field_name.clone());
                            }

                            columns[1].heading("Universal Properties");
                            columns[1].add_space(5.0);
                            for (field_name, is_checked) in &mut dash_fields {
                                columns[1].checkbox(is_checked, field_name.clone());
                            }
                        });

                        ui.separator();
                        if ui.button("Close").clicked() {
                            self.show_fields_dropdown = false;
                        }
                    });
            }
        }

        ui.add_space(5.0);

        let pagination_height = 30.0;
        let max_scroll_height = ui.available_height() - pagination_height;

        ScrollArea::vertical()
            .max_height(max_scroll_height)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                match self.document_query_status {
                    DocumentQueryStatus::WaitingForResult(start_time) => {
                        let time_elapsed = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs()
                            - start_time;
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "Fetching documents... Time taken so far: {} seconds",
                                time_elapsed
                            ));
                            ui.add(
                                egui::widgets::Spinner::default()
                                    .color(Color32::from_rgb(0, 128, 255)),
                            );
                        });
                    }
                    DocumentQueryStatus::Complete => match self.document_display_mode {
                        DocumentDisplayMode::Json => {
                            self.show_filtered_docs(ui, DocumentDisplayMode::Json);
                        }
                        DocumentDisplayMode::Yaml => {
                            self.show_filtered_docs(ui, DocumentDisplayMode::Yaml);
                        }
                    },

                    DocumentQueryStatus::ErrorMessage(ref message) => {
                        self.error_message =
                            Some((message.to_string(), MessageType::Error, Utc::now()));
                        ui.colored_label(Color32::DARK_RED, message);
                    }
                    _ => {
                        // Nothing
                    }
                }
            });

        ui.add_space(10.0);

        if self.document_query_status == DocumentQueryStatus::Complete {
            ui.horizontal(|ui| {
                if self.current_page > 1 {
                    if ui.button("Previous Page").clicked() {
                        // Handle Previous Page
                        if let Some(prev_cursor) = self.get_previous_cursor() {
                            self.document_query_status = DocumentQueryStatus::WaitingForResult(
                                SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .expect("Time went backwards")
                                    .as_secs(),
                            );
                            self.current_page -= 1;
                            self.next_cursors.pop();
                            let parsed_query = self.build_document_query_with_cursor(&prev_cursor);
                            action = AppAction::BackendTask(BackendTask::DocumentTask(
                                DocumentTask::FetchDocumentsPage(parsed_query),
                            ));
                        } else {
                            self.document_query_status = DocumentQueryStatus::WaitingForResult(
                                SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .expect("Time went backwards")
                                    .as_secs(),
                            );
                            self.current_page = 1;
                            let next_cursor =
                                self.get_next_cursor().unwrap_or(Start::StartAfter(vec![])); // Doesn't matter what the value is
                            let parsed_query = self.build_document_query_with_cursor(&next_cursor);
                            action = AppAction::BackendTask(BackendTask::DocumentTask(
                                DocumentTask::FetchDocumentsPage(parsed_query),
                            ));
                        }
                    }
                }

                ui.label(format!("Page {}", self.current_page));

                if self.has_next_page {
                    if ui.button("Next Page").clicked() {
                        // Handle Next Page
                        if let Some(next_cursor) = &self.get_next_cursor() {
                            self.document_query_status = DocumentQueryStatus::WaitingForResult(
                                SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .expect("Time went backwards")
                                    .as_secs(),
                            );
                            if self.current_page > 1 {
                                self.previous_cursors.push(
                                    self.next_cursors
                                        .get(self.next_cursors.len() - 2)
                                        .expect("Expected a previous cursor")
                                        .clone(),
                                );
                            }
                            self.current_page += 1;
                            let parsed_query = self.build_document_query_with_cursor(next_cursor);
                            action = AppAction::BackendTask(BackendTask::DocumentTask(
                                DocumentTask::FetchDocumentsPage(parsed_query),
                            ));
                        }
                    }
                }
            });
        }

        action
    }

    fn show_filtered_docs(&mut self, ui: &mut egui::Ui, display_mode: DocumentDisplayMode) {
        // 1) Convert each Document to a filtered string
        let mut doc_strings = Vec::new();

        for doc in &self.matching_documents {
            if let Some(stringed) = doc_to_filtered_string(
                doc,
                &self.document_fields_selection, // or the user’s selected fields
                display_mode.clone(),
            ) {
                // Optionally also filter by `document_search_term` here
                if self.document_search_term.is_empty()
                    || stringed
                        .to_lowercase()
                        .contains(&self.document_search_term.to_lowercase())
                {
                    doc_strings.push(stringed);
                }
            }
        }

        // 2) Concatenate them all with spacing
        let mut combined_string = doc_strings.join("\n\n");

        // 3) Display in multiline text
        ui.add(
            egui::TextEdit::multiline(&mut combined_string)
                .desired_rows(10)
                .desired_width(ui.available_width())
                .font(egui::TextStyle::Monospace),
        );
    }

    fn show_remove_contract_popup(&mut self, ui: &mut egui::Ui) -> AppAction {
        // If no contract is set, nothing to confirm
        let contract_to_remove = match &self.contract_to_remove {
            Some(contract) => contract.clone(),
            None => {
                self.confirm_remove_contract_popup = false;
                return AppAction::None;
            }
        };

        let mut app_action = AppAction::None;
        let mut is_open = true;

        egui::Window::new("Confirm Remove Contract")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                let contract_alias_or_id =
                    match self.app_context.get_contract_by_id(&contract_to_remove) {
                        Ok(Some(contract)) => contract
                            .alias
                            .unwrap_or_else(|| contract.contract.id().to_string(Encoding::Base58)),
                        Ok(None) | Err(_) => contract_to_remove.to_string(Encoding::Base58),
                    };

                ui.label(format!(
                    "Are you sure you want to remove contract \"{}\"?",
                    contract_alias_or_id
                ));

                // Confirm button
                if ui.button("Confirm").clicked() {
                    app_action = AppAction::BackendTask(BackendTask::ContractTask(
                        ContractTask::RemoveContract(contract_to_remove),
                    ));
                    self.confirm_remove_contract_popup = false;
                    self.contract_to_remove = None;
                }

                // Cancel button
                if ui.button("Cancel").clicked() {
                    self.confirm_remove_contract_popup = false;
                    self.contract_to_remove = None;
                }
            });

        // If user closes the popup window (the [x] button), also reset state
        if !is_open {
            self.confirm_remove_contract_popup = false;
            self.contract_to_remove = None;
        }
        app_action
    }
}

impl ScreenLike for DocumentQueryScreen {
    fn refresh(&mut self) {
        // Reset the screen state
        self.error_message = None;
        self.contract_search_term.clear();
        self.document_search_term.clear();
        self.document_query.clear();
        self.document_query_status = DocumentQueryStatus::NotStarted;
        self.matching_documents.clear();
        self.current_page = 1;
        self.next_cursors.clear();
        self.has_next_page = false;
        self.previous_cursors.clear();

        // Reset the selected contract and document type
        let dpns_contract = QualifiedContract {
            contract: Arc::clone(&self.app_context.dpns_contract).as_ref().clone(),
            alias: Some("dpns".to_string()),
        };
        self.selected_data_contract = dpns_contract.clone();
        self.selected_document_type = dpns_contract
            .contract
            .document_type_cloned_for_name("domain")
            .expect("Expected to find domain document type in DPNS contract");
    }

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
            BackendTaskSuccessResult::PageDocuments(page_docs, next_cursor) => {
                self.matching_documents = page_docs
                    .iter()
                    .filter_map(|(_, doc)| doc.clone())
                    .collect();
                self.has_next_page = next_cursor.is_some();
                if let Some(cursor) = next_cursor {
                    self.next_cursors.push(cursor.clone());
                }
                self.document_query_status = DocumentQueryStatus::Complete;
            }
            _ => {
                // Handle other variants
            }
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        self.check_error_expiration();
        let load_contract_button = (
            "Load Contracts",
            DesiredAppAction::AddScreenType(ScreenType::AddContracts),
        );
        let register_contract_button = (
            "Register Contract",
            DesiredAppAction::AddScreenType(ScreenType::RegisterContract),
        );
        let update_contract_button = (
            "Update Contract",
            DesiredAppAction::AddScreenType(ScreenType::UpdateContract),
        );
        let add_document_button = (
            "Add Document",
            DesiredAppAction::AddScreenType(ScreenType::CreateDocument),
        );
        let delete_document_button = (
            "Delete Document",
            DesiredAppAction::AddScreenType(ScreenType::DeleteDocument),
        );
        let replace_document_button = (
            "Replace Document",
            DesiredAppAction::AddScreenType(ScreenType::ReplaceDocument),
        );
        let purchase_document_button = (
            "Purchase Document",
            DesiredAppAction::AddScreenType(ScreenType::PurchaseDocument),
        );
        let set_document_price_button = (
            "Set Document Price",
            DesiredAppAction::AddScreenType(ScreenType::SetDocumentPrice),
        );
        let group_actions_button = (
            "Group Actions",
            DesiredAppAction::AddScreenType(ScreenType::GroupActions),
        );
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Contracts", AppAction::None)],
            vec![
                load_contract_button,
                register_contract_button,
                update_contract_button,
                add_document_button,
                delete_document_button,
                replace_document_button,
                purchase_document_button,
                set_document_price_button,
                group_actions_button,
            ],
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
            &mut self.pending_document_type,
            &mut self.pending_fields_selection,
        );

        if let AppAction::BackendTask(BackendTask::ContractTask(ContractTask::RemoveContract(
            contract_id,
        ))) = action
        {
            action = AppAction::None;
            self.confirm_remove_contract_popup = true;
            self.contract_to_remove = Some(contract_id);
        }

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(Margin::same(10)),
            )
            .show(ctx, |ui| {
                action |= self.show_input_field(ui);
                action |= self.show_output(ui);

                if self.confirm_remove_contract_popup {
                    action |= self.show_remove_contract_popup(ui);
                }
            });

        action
    }
}

/// Convert a `Document` to a `serde_json::Value`, then filter out unselected fields,
/// then serialize the result to JSON/YAML.
fn doc_to_filtered_string(
    doc: &Document,
    selected_fields: &std::collections::HashMap<String, bool>,
    display_mode: DocumentDisplayMode,
) -> Option<String> {
    // 1) Convert doc to a serde_json Value
    let value = serde_json::to_value(doc).ok()?;
    let obj = value.as_object()?;

    // 2) Build a new JSON object containing only the selected fields
    let mut filtered_map = serde_json::Map::new();

    for (field_name, &is_checked) in selected_fields {
        if is_checked {
            if let Some(field_value) = obj.get(field_name) {
                filtered_map.insert(field_name.clone(), field_value.clone());
            }
        }
    }

    let filtered_value = serde_json::Value::Object(filtered_map);

    // 3) Convert filtered_value to the chosen format
    let final_string = match display_mode {
        DocumentDisplayMode::Json => serde_json::to_string_pretty(&filtered_value).ok()?,
        DocumentDisplayMode::Yaml => serde_yaml::to_string(&filtered_value).ok()?,
    };

    Some(final_string)
}
