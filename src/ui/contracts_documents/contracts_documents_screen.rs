use crate::app::{AppAction, DesiredAppAction};
use crate::backend_task::contract::ContractTask;
use crate::backend_task::document::DocumentTask::{self, FetchDocumentsPage}; // Updated import
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskContext};
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::ui::components::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::contract_chooser_panel::{
    ContractChooserState, add_contract_chooser_panel,
};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::message_banner::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::components::top_panel::{add_top_panel_with_global_nav, subdued_everyday_spec};
use crate::ui::helpers::{ModalOpeningGuard, clicked_outside_window_after_open};
use crate::ui::theme::{ComponentStyles, DashColors, Shadow, Shape};
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, ScreenLike, ScreenType};
use crate::utils::parsers::{DocumentQueryTextInputParser, TextInputParser};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::document_type::{DocumentType, Index};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{Document, DocumentQuery, Identifier};
use egui::{CentralPanel, Frame, Margin, ScrollArea, Stroke, Ui};
use std::collections::HashMap;
use std::sync::Arc;

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
    contract_search_term: String,
    document_search_term: String,
    document_query: String,
    document_display_mode: DocumentDisplayMode,
    document_fields_selection: HashMap<String, bool>,
    show_fields_dropdown: bool,
    fields_dropdown_opening_guard: ModalOpeningGuard,
    selected_data_contract: QualifiedContract,
    selected_document_type: DocumentType,
    selected_index: Option<Index>,
    pub matching_documents: Vec<Document>,
    document_query_status: DocumentQueryStatus,
    confirmation_dialog: Option<ConfirmationDialog>,
    contract_to_remove: Option<Identifier>,
    pending_document_type: DocumentType,
    pending_fields_selection: HashMap<String, bool>,
    // Pagination fields
    current_page: usize,
    pub next_cursors: Vec<Start>,
    has_next_page: bool,
    previous_cursors: Vec<Start>,
    // Contract chooser state
    contract_chooser_state: ContractChooserState,
    query_banner: Option<BannerHandle>,
    pending_fetch_context: Option<BackendTaskContext>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum DocumentQueryStatus {
    NotStarted,
    WaitingForResult,
    Complete,
    Error,
}

impl DocumentQueryStatus {
    fn complete_if_fetch_in_flight(
        &mut self,
        expected: Option<&BackendTaskContext>,
        completed: &BackendTaskContext,
    ) -> bool {
        if *self == Self::WaitingForResult && expected == Some(completed) {
            *self = Self::Complete;
            true
        } else {
            false
        }
    }

    fn fail_if_fetch_in_flight(
        &mut self,
        expected: Option<&BackendTaskContext>,
        failed: &BackendTaskContext,
    ) -> bool {
        if *self == Self::WaitingForResult && expected == Some(failed) {
            *self = Self::Error;
            true
        } else {
            false
        }
    }
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
            let is_selected = *dash_field == "$ownerId" || *dash_field == "$id";
            document_fields_selection.insert((*dash_field).to_string(), is_selected);
        }

        let pending_document_type = selected_document_type.clone();
        let pending_fields_selection = document_fields_selection.clone();

        Self {
            app_context: app_context.clone(),
            contract_search_term: String::new(),
            document_search_term: String::new(),
            document_query: format!("SELECT * FROM {}", selected_document_type.name()),
            document_display_mode: DocumentDisplayMode::Yaml,
            document_fields_selection,
            show_fields_dropdown: false,
            fields_dropdown_opening_guard: ModalOpeningGuard::default(),
            selected_data_contract: dpns_contract,
            selected_document_type,
            selected_index: None,
            matching_documents: vec![],
            document_query_status: DocumentQueryStatus::NotStarted,
            confirmation_dialog: None,
            contract_to_remove: None,
            pending_document_type,
            pending_fields_selection,
            // Initialize pagination fields
            current_page: 1,
            next_cursors: vec![],
            has_next_page: false,
            previous_cursors: Vec::new(),
            contract_chooser_state: ContractChooserState::default(),
            query_banner: None,
            pending_fetch_context: None,
        }
    }

    fn correlate_fetch_action(&mut self, action: AppAction) -> AppAction {
        let AppAction::BackendTask(task) = action else {
            return action;
        };
        let operation = BackendTaskContext::from(&task);
        if !matches!(
            operation,
            BackendTaskContext::FetchDocuments(_) | BackendTaskContext::FetchDocumentsPage(_)
        ) {
            return AppAction::BackendTask(task);
        }

        let context = BackendTaskContext::for_dispatch(&task);
        self.pending_fetch_context = Some(context.clone());
        AppAction::BackendTaskWithContext { task, context }
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
            let button_width = 140.0; // Increased from 120.0
            let spacing = 10.0; // Add some spacing
            let available = ui.available_width();
            let text_width = (available - button_width - spacing).max(100.0); // Ensure minimum width

            let dark_mode = ui.style().visuals.dark_mode;
            ui.add(
                egui::TextEdit::singleline(&mut self.document_query)
                    .desired_width(text_width)
                    .text_color(DashColors::text_primary(dark_mode))
                    .background_color(DashColors::input_background(dark_mode)),
            );

            ui.add_space(spacing);

            if ComponentStyles::add_primary_button(ui, "Fetch Documents").clicked() {
                self.selected_document_type = self.pending_document_type.clone();
                self.document_fields_selection = self.pending_fields_selection.clone();

                let parser = DocumentQueryTextInputParser::new(
                    self.selected_data_contract.contract.clone(),
                    self.app_context.platform_version(),
                );
                match parser.parse_input(&self.document_query) {
                    Ok(parsed_query) => {
                        // Set the status to waiting
                        self.query_banner.take_and_clear();
                        let handle = MessageBanner::set_global(
                            ui.ctx(),
                            "Querying documents...",
                            MessageType::Info,
                        );
                        handle.with_elapsed();
                        self.query_banner = Some(handle);
                        self.document_query_status = DocumentQueryStatus::WaitingForResult;
                        self.current_page = 1; // Reset to first page
                        self.next_cursors = vec![]; // Reset cursor
                        self.previous_cursors.clear(); // Clear previous cursors
                        action = AppAction::BackendTask(BackendTask::DocumentTask(Box::new(
                            FetchDocumentsPage(parsed_query),
                        )));
                    }
                    Err(error) => {
                        self.query_banner.take_and_clear();
                        self.document_query_status = DocumentQueryStatus::Error;
                        MessageBanner::set_global(
                            ui.ctx(),
                            "The document query is not valid. Check its fields and try again.",
                            MessageType::Error,
                        )
                        .with_details(error);
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

        // Controls section
        if !self.matching_documents.is_empty() {
            ui.horizontal(|ui| {
                ui.label("Filter documents:");
                ui.text_edit_singleline(&mut self.document_search_term);

                if ui.button("Select Properties").clicked() {
                    self.show_fields_dropdown = !self.show_fields_dropdown;
                    if self.show_fields_dropdown {
                        self.fields_dropdown_opening_guard.arm();
                    }
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

                let window_response = egui::Window::new("Select Properties")
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
                        let dark_mode = ui.style().visuals.dark_mode;
                        if ComponentStyles::add_secondary_button(ui, "Close", dark_mode).clicked() {
                            self.show_fields_dropdown = false;
                        }
                    });

                if let Some(ref wr) = window_response
                    && clicked_outside_window_after_open(
                        ui.ctx(),
                        wr.response.rect,
                        &mut self.fields_dropdown_opening_guard,
                    )
                {
                    self.show_fields_dropdown = false;
                }
            }
        } else if matches!(self.document_query_status, DocumentQueryStatus::NotStarted) {
            ui.label("Select a contract and document type on the left and hit \"Fetch Documents\" to query documents.");
        } else if matches!(self.document_query_status, DocumentQueryStatus::Complete) {
            ui.label("No documents found.");
        }

        ui.add_space(5.0);

        // Calculate available height minus space for pagination controls and margins
        let available_height = ui.available_height();
        let pagination_height = 80.0; // Reserve more space for pagination buttons and margins
        let document_area_height = (available_height - pagination_height).max(200.0);

        // Document display area with constrained height
        ScrollArea::both()
            .max_height(document_area_height)
            .show(ui, |ui| {
                // Use full available width
                ui.set_width(ui.available_width());

                match self.document_query_status {
                    DocumentQueryStatus::WaitingForResult => {
                        // Elapsed time is shown in the global banner
                    }
                    DocumentQueryStatus::Complete => match self.document_display_mode {
                        DocumentDisplayMode::Json => {
                            self.show_filtered_docs(ui, DocumentDisplayMode::Json);
                        }
                        DocumentDisplayMode::Yaml => {
                            self.show_filtered_docs(ui, DocumentDisplayMode::Yaml);
                        }
                    },

                    DocumentQueryStatus::Error => {
                        // Error message is displayed globally via MessageBanner
                    }
                    _ => {
                        // Nothing
                    }
                }
            });

        // Pagination controls - always visible at the bottom
        if self.document_query_status == DocumentQueryStatus::Complete {
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(5.0);

            ui.horizontal(|ui| {
                if self.current_page > 1 && ui.button("Previous Page").clicked() {
                    // Handle Previous Page
                    if let Some(prev_cursor) = self.get_previous_cursor() {
                        self.query_banner.take_and_clear();
                        let handle = MessageBanner::set_global(
                            ui.ctx(),
                            "Querying documents...",
                            MessageType::Info,
                        );
                        handle.with_elapsed();
                        self.query_banner = Some(handle);
                        self.document_query_status = DocumentQueryStatus::WaitingForResult;
                        self.current_page -= 1;
                        self.next_cursors.pop();
                        let parsed_query = self.build_document_query_with_cursor(&prev_cursor);
                        action = AppAction::BackendTask(BackendTask::DocumentTask(Box::new(
                            DocumentTask::FetchDocumentsPage(parsed_query),
                        )));
                    } else {
                        self.query_banner.take_and_clear();
                        let handle = MessageBanner::set_global(
                            ui.ctx(),
                            "Querying documents...",
                            MessageType::Info,
                        );
                        handle.with_elapsed();
                        self.query_banner = Some(handle);
                        self.document_query_status = DocumentQueryStatus::WaitingForResult;
                        self.current_page = 1;
                        let next_cursor =
                            self.get_next_cursor().unwrap_or(Start::StartAfter(vec![])); // Doesn't matter what the value is
                        let parsed_query = self.build_document_query_with_cursor(&next_cursor);
                        action = AppAction::BackendTask(BackendTask::DocumentTask(Box::new(
                            DocumentTask::FetchDocumentsPage(parsed_query),
                        )));
                    }
                }

                ui.label(format!("Page {}", self.current_page));

                if self.has_next_page && ui.button("Next Page").clicked() {
                    // Handle Next Page
                    if let Some(next_cursor) = &self.get_next_cursor() {
                        self.query_banner.take_and_clear();
                        let handle = MessageBanner::set_global(
                            ui.ctx(),
                            "Querying documents...",
                            MessageType::Info,
                        );
                        handle.with_elapsed();
                        self.query_banner = Some(handle);
                        self.document_query_status = DocumentQueryStatus::WaitingForResult;
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
                        action = AppAction::BackendTask(BackendTask::DocumentTask(Box::new(
                            DocumentTask::FetchDocumentsPage(parsed_query),
                        )));
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
        let dark_mode = ui.style().visuals.dark_mode;
        ui.add(
            egui::TextEdit::multiline(&mut combined_string)
                .desired_rows(10)
                // Remove desired_width to respect parent container margins
                .text_color(DashColors::text_primary(dark_mode))
                .background_color(DashColors::input_background(dark_mode))
                .font(egui::TextStyle::Monospace),
        );
    }

    fn show_remove_contract_popup(&mut self, ui: &mut egui::Ui) -> AppAction {
        // If no contract is set, nothing to confirm
        let contract_to_remove = match &self.contract_to_remove {
            Some(contract) => *contract,
            None => {
                self.confirmation_dialog = None;
                return AppAction::None;
            }
        };

        let contract_alias_or_id = match self.app_context.get_contract_by_id(&contract_to_remove) {
            Ok(Some(contract)) => contract
                .alias
                .unwrap_or_else(|| contract.contract.id().to_string(Encoding::Base58)),
            Ok(None) | Err(_) => contract_to_remove.to_string(Encoding::Base58),
        };

        let dialog = self.confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new(
                "Confirm Remove Contract".to_string(),
                format!(
                    "Are you sure you want to remove contract \"{}\"?",
                    contract_alias_or_id
                ),
            )
        });

        match dialog.show(ui).inner.dialog_response {
            Some(ConfirmationStatus::Confirmed) => {
                let action = AppAction::BackendTask(BackendTask::ContractTask(Box::new(
                    ContractTask::RemoveContract(contract_to_remove),
                )));
                self.confirmation_dialog = None;
                self.contract_to_remove = None;
                action
            }
            Some(ConfirmationStatus::Canceled) => {
                self.confirmation_dialog = None;
                self.contract_to_remove = None;
                AppAction::None
            }
            None => AppAction::None,
        }
    }
}

impl ScreenLike for DocumentQueryScreen {
    fn refresh_on_arrival(&mut self) {
        // This will be called when navigating to this screen
        // Note: We can't easily control egui's collapsing headers from here
        // They maintain their own internal state
    }

    fn refresh(&mut self) {
        // Reset the screen state
        self.contract_search_term.clear();
        self.document_search_term.clear();
        self.document_query.clear();
        self.document_query_status = DocumentQueryStatus::NotStarted;
        self.matching_documents.clear();
        self.current_page = 1;
        self.next_cursors.clear();
        self.has_next_page = false;
        self.previous_cursors.clear();
        self.pending_fetch_context = None;

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

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        if self
            .document_query_status
            .fail_if_fetch_in_flight(self.pending_fetch_context.as_ref(), context)
        {
            self.pending_fetch_context = None;
            self.query_banner.take_and_clear();
        }
    }

    fn should_suppress_backend_task_error(
        &self,
        context: &BackendTaskContext,
        _error: &TaskError,
    ) -> bool {
        context.dispatched_document_fetch() && self.pending_fetch_context.as_ref() != Some(context)
    }

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        backend_task_success_result: BackendTaskSuccessResult,
    ) {
        match backend_task_success_result {
            BackendTaskSuccessResult::Documents(documents)
                if context.is_fetch_documents()
                    && self.document_query_status.complete_if_fetch_in_flight(
                        self.pending_fetch_context.as_ref(),
                        context,
                    ) =>
            {
                self.pending_fetch_context = None;
                self.query_banner.take_and_clear();
                self.matching_documents = documents
                    .iter()
                    .filter_map(|(_, doc)| doc.clone())
                    .collect();
            }
            BackendTaskSuccessResult::PageDocuments(page_docs, next_cursor)
                if context.is_fetch_documents_page()
                    && self.document_query_status.complete_if_fetch_in_flight(
                        self.pending_fetch_context.as_ref(),
                        context,
                    ) =>
            {
                self.pending_fetch_context = None;
                self.query_banner.take_and_clear();
                self.matching_documents = page_docs
                    .iter()
                    .filter_map(|(_, doc)| doc.clone())
                    .collect();
                self.has_next_page = next_cursor.is_some();
                if let Some(cursor) = next_cursor {
                    self.next_cursors.push(cursor.clone());
                }
            }
            _ => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let load_contract_button = (
            "Load Contracts",
            DesiredAppAction::AddScreenType(Box::new(ScreenType::AddContracts)),
        );
        let register_contract_button = (
            "Register Contract",
            DesiredAppAction::AddScreenType(Box::new(ScreenType::RegisterContract)),
        );
        let update_contract_button = (
            "Update Contract",
            DesiredAppAction::AddScreenType(Box::new(ScreenType::UpdateContract)),
        );
        let add_document_button = (
            "Create Document",
            DesiredAppAction::AddScreenType(Box::new(ScreenType::CreateDocument)),
        );
        let delete_document_button = (
            "Delete Document",
            DesiredAppAction::AddScreenType(Box::new(ScreenType::DeleteDocument)),
        );
        let replace_document_button = (
            "Replace Document",
            DesiredAppAction::AddScreenType(Box::new(ScreenType::ReplaceDocument)),
        );
        let transfer_document_button = (
            "Transfer Document",
            DesiredAppAction::AddScreenType(Box::new(ScreenType::TransferDocument)),
        );
        let purchase_document_button = (
            "Purchase Document",
            DesiredAppAction::AddScreenType(Box::new(ScreenType::PurchaseDocument)),
        );
        let set_document_price_button = (
            "Set Document Price",
            DesiredAppAction::AddScreenType(Box::new(ScreenType::SetDocumentPrice)),
        );
        let group_actions_button = (
            "Group Actions",
            DesiredAppAction::AddScreenType(Box::new(ScreenType::GroupActions)),
        );
        let mut action = AppAction::None;
        if self.app_context.network == Network::Mainnet {
            action |= add_top_panel_with_global_nav(
                ui,
                &self.app_context,
                subdued_everyday_spec("Contracts", RootScreenType::RootScreenDocumentQuery),
                vec![
                    load_contract_button,
                    register_contract_button,
                    update_contract_button,
                    add_document_button,
                    delete_document_button,
                    replace_document_button,
                    transfer_document_button,
                    purchase_document_button,
                    set_document_price_button,
                ],
            );
        } else {
            action |= add_top_panel_with_global_nav(
                ui,
                &self.app_context,
                subdued_everyday_spec("Contracts", RootScreenType::RootScreenDocumentQuery),
                vec![
                    load_contract_button,
                    register_contract_button,
                    update_contract_button,
                    add_document_button,
                    delete_document_button,
                    replace_document_button,
                    transfer_document_button,
                    purchase_document_button,
                    set_document_price_button,
                    group_actions_button,
                ],
            );
        }

        action |= add_left_panel(
            ui,
            &self.app_context,
            RootScreenType::RootScreenDocumentQuery,
        );

        action |= add_contract_chooser_panel(
            ui,
            &mut self.contract_search_term,
            &self.app_context,
            &mut self.selected_data_contract,
            &mut self.selected_document_type,
            &mut self.selected_index,
            &mut self.document_query,
            &mut self.pending_document_type,
            &mut self.pending_fields_selection,
            &mut self.contract_chooser_state,
        );

        if let AppAction::BackendTask(BackendTask::ContractTask(contract_task)) = &action
            && let ContractTask::RemoveContract(contract_id) = **contract_task
        {
            action = AppAction::None;
            self.contract_to_remove = Some(contract_id);
            // Clear any existing dialog to create a new one with updated content
            self.confirmation_dialog = None;
        }

        // Custom central panel with adjusted margins for Document Query screen
        let dark_mode = ctx.global_style().visuals.dark_mode;

        action |= {
            CentralPanel::default()
                .frame(
                    Frame::new()
                        .fill(DashColors::background(dark_mode))
                        .inner_margin(Margin {
                            left: 10,
                            right: 19, // More space on the right
                            top: 10,
                            bottom: 0, // Less space on the bottom
                        }),
                )
                .show(ui, |ui| {
                    // Create an island panel with rounded edges
                    Frame::new()
                        .fill(DashColors::surface(dark_mode))
                        .stroke(Stroke::new(1.0, DashColors::border_light(dark_mode)))
                        .inner_margin(Margin::same(20))
                        .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                        .shadow(Shadow::elevated())
                        .show(ui, |ui| {
                            MessageBanner::show_global(ui);
                            let mut inner_action = AppAction::None;

                            // Use a vertical layout that allocates space properly
                            ui.vertical(|ui| {
                                // Input field at the top
                                inner_action |= self.show_input_field(ui);

                                // Document display area that expands to fill available space
                                ui.with_layout(
                                    egui::Layout::top_down_justified(egui::Align::LEFT),
                                    |ui| {
                                        inner_action |= self.show_output(ui);
                                    },
                                );
                            });

                            if self.contract_to_remove.is_some() {
                                inner_action |= self.show_remove_contract_popup(ui);
                            }
                            inner_action
                        })
                        .inner
                })
                .inner
        };

        self.correlate_fetch_action(action)
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
        if is_checked && let Some(field_value) = obj.get(field_name) {
            filtered_map.insert(field_name.clone(), field_value.clone());
        }
    }

    let filtered_value = serde_json::Value::Object(filtered_map);

    // 3) Convert filtered_value to the chosen format
    let final_string = match display_mode {
        DocumentDisplayMode::Json => serde_json::to_string_pretty(&filtered_value).ok()?,
        DocumentDisplayMode::Yaml => serde_yaml_ng::to_string(&filtered_value).ok()?,
    };

    Some(final_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::BackendTaskContext;
    use dash_sdk::dpp::data_contracts::SystemDataContract;
    use dash_sdk::dpp::system_data_contracts::load_system_data_contract;
    use dash_sdk::dpp::version::PlatformVersion;

    fn query(limit: u32) -> DocumentQuery {
        let contract =
            load_system_data_contract(SystemDataContract::DPNS, PlatformVersion::latest())
                .expect("DPNS contract");
        let mut query = DocumentQuery::new(Arc::new(contract), "domain").expect("domain query");
        query.limit = limit;
        query
    }

    #[test]
    fn in_flight_fetch_failure_requires_the_matching_backend_task() {
        let expected = BackendTaskContext::FetchDocumentsPage(Box::new(query(10)));
        let different_query = BackendTaskContext::FetchDocumentsPage(Box::new(query(20)));
        let mut status = DocumentQueryStatus::WaitingForResult;
        assert!(!status.fail_if_fetch_in_flight(Some(&expected), &BackendTaskContext::Other));
        assert!(!status.fail_if_fetch_in_flight(Some(&expected), &BackendTaskContext::Unknown));
        assert!(!status.fail_if_fetch_in_flight(Some(&expected), &different_query));
        assert!(!status.fail_if_fetch_in_flight(
            Some(&expected),
            &BackendTaskContext::FetchDocuments(Box::new(query(10))),
        ));
        assert_eq!(status, DocumentQueryStatus::WaitingForResult);

        assert!(status.fail_if_fetch_in_flight(Some(&expected), &expected));
        assert_eq!(status, DocumentQueryStatus::Error);

        let mut status = DocumentQueryStatus::Complete;
        assert!(!status.fail_if_fetch_in_flight(Some(&expected), &expected));
        assert_eq!(status, DocumentQueryStatus::Complete);
    }

    #[test]
    fn document_success_requires_the_matching_in_flight_context() {
        let tmp = tempfile::tempdir().expect("temp data dir");
        let app_context = crate::context::test_support::test_app_context(tmp.path());
        let mut screen = DocumentQueryScreen::new(&app_context);
        let expected = BackendTaskContext::FetchDocumentsPage(Box::new(query(10)));
        let different_query = BackendTaskContext::FetchDocumentsPage(Box::new(query(20)));
        let existing_cursor = Start::StartAfter(vec![1]);

        screen.document_query_status = DocumentQueryStatus::WaitingForResult;
        screen.pending_fetch_context = Some(expected.clone());
        screen.current_page = 2;
        screen.next_cursors = vec![existing_cursor.clone()];
        screen.has_next_page = true;

        screen.display_backend_task_result(
            &different_query,
            BackendTaskSuccessResult::PageDocuments(Default::default(), None),
        );

        assert_eq!(
            screen.document_query_status,
            DocumentQueryStatus::WaitingForResult
        );
        assert_eq!(screen.pending_fetch_context.as_ref(), Some(&expected));
        assert_eq!(screen.current_page, 2);
        assert_eq!(screen.next_cursors, vec![existing_cursor]);
        assert!(screen.has_next_page);

        let next_cursor = Start::StartAfter(vec![2]);
        screen.display_backend_task_result(
            &expected,
            BackendTaskSuccessResult::PageDocuments(Default::default(), Some(next_cursor.clone())),
        );

        assert_eq!(screen.document_query_status, DocumentQueryStatus::Complete);
        assert_eq!(screen.pending_fetch_context, None);
        assert_eq!(
            screen.next_cursors,
            vec![Start::StartAfter(vec![1]), next_cursor]
        );
        assert!(screen.has_next_page);

        let accepted_cursors = screen.next_cursors.clone();
        screen.display_backend_task_result(
            &expected,
            BackendTaskSuccessResult::PageDocuments(Default::default(), None),
        );

        assert_eq!(screen.next_cursors, accepted_cursors);
        assert!(screen.has_next_page);
    }

    #[test]
    fn identical_document_redispatch_ignores_the_first_error_and_accepts_the_second_success() {
        let tmp = tempfile::tempdir().expect("temp data dir");
        let app_context = crate::context::test_support::test_app_context(tmp.path());
        let mut screen = DocumentQueryScreen::new(&app_context);
        let task = BackendTask::DocumentTask(Box::new(FetchDocumentsPage(query(10))));

        screen.document_query_status = DocumentQueryStatus::WaitingForResult;
        let first_action = screen.correlate_fetch_action(AppAction::BackendTask(task.clone()));
        let AppAction::BackendTaskWithContext {
            context: first_context,
            ..
        } = first_action
        else {
            panic!("document fetch must carry its dispatch context");
        };
        let second_action = screen.correlate_fetch_action(AppAction::BackendTask(task));
        let AppAction::BackendTaskWithContext {
            context: second_context,
            ..
        } = second_action
        else {
            panic!("document fetch must carry its dispatch context");
        };

        assert_ne!(first_context, second_context);
        assert!(
            screen
                .should_suppress_backend_task_error(&first_context, &TaskError::NoIdentitiesFound)
        );
        screen.display_backend_task_error(&first_context, &TaskError::NoIdentitiesFound);
        assert_eq!(
            screen.document_query_status,
            DocumentQueryStatus::WaitingForResult
        );
        assert_eq!(screen.pending_fetch_context.as_ref(), Some(&second_context));

        screen.display_backend_task_result(
            &second_context,
            BackendTaskSuccessResult::PageDocuments(Default::default(), None),
        );

        assert_eq!(screen.document_query_status, DocumentQueryStatus::Complete);
        assert_eq!(screen.pending_fetch_context, None);
    }
}
