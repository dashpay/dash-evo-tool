use crate::app::AppAction;
use crate::backend_task::BackendTaskContext;
use crate::backend_task::BackendTaskSuccessResult;
use crate::backend_task::FeeResult;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, document::DocumentTask};
use crate::context::AppContext;
use crate::context::feature_gate::FeatureGate;
use crate::model::document::new_document_for_creation;
use crate::model::fee_estimation::format_credits_as_dash;
use crate::model::fee_estimation::{
    ACTION_FEE_MULTIPLIER_INCREASE_TOLERANCE_PERCENT, DocumentActionFeeQuote,
    document_action_fee_quote,
};
use crate::model::identity_key_usability::{KeyRequirements, SigningScope};
use crate::model::index_only_document::{
    IndexOnlyDeleteError, IndexOnlyDeleteValues, is_index_only, validate_index_only_delete,
};
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::ScreenLike;
use crate::ui::components::Component;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::{island_central_panel, styled_text_edit_singleline};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{
    WalletUnlockPopup, WalletUnlockResult, try_open_wallet_no_password, wallet_needs_unlock,
};
use crate::ui::components::{BannerHandle, MessageBanner, OptionBannerExt};
use crate::ui::helpers::{
    TransactionType, add_contract_doc_type_chooser_with_filtering, add_key_chooser_with_doc_type,
    show_success_screen_with_info,
};
use crate::ui::identity::wallet_for_signing_key;
use crate::ui::state::PendingRequest;
use crate::ui::theme::{ComponentStyles, DashColors};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bip39::rand::{Rng, SeedableRng, rngs::StdRng};
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::{
    DocumentTypeV0Getters, DocumentTypeV1Getters,
};
use dash_sdk::dpp::data_contract::document_type::action_fees::agreement::DocumentActionFeeAgreement;
use dash_sdk::dpp::data_contract::document_type::{DocumentPropertyType, DocumentType};
use dash_sdk::dpp::document::property_names::PRICE;
use dash_sdk::dpp::document::{Document, DocumentV0, DocumentV0Getters, DocumentV0Setters};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::platform_value::btreemap_extensions::BTreeValueMapHelper;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::tokens::gas_fees_paid_by::GasFeesPaidBy;
use dash_sdk::dpp::tokens::token_amount_on_contract_token::DocumentActionTokenCost;
use dash_sdk::dpp::tokens::token_amount_on_contract_token::DocumentActionTokenEffect;
use dash_sdk::dpp::tokens::token_payment_info::TokenPaymentInfo;
use dash_sdk::dpp::tokens::token_payment_info::v0::TokenPaymentInfoV0;
use dash_sdk::dpp::state_transition::batch_transition::batched_transition::document_transition_action_type::DocumentTransitionActionType;
use dash_sdk::drive::query::WhereClause;
use dash_sdk::platform::{DocumentQuery, Identifier, IdentityPublicKey};
use dash_sdk::query_types::IndexMap;
use eframe::epaint::Color32;
use egui::{Frame, Margin, RichText, Ui};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq)]
pub enum DocumentActionType {
    Create,
    Delete,
    Purchase,
    Replace,
    SetPrice,
    Transfer,
}

impl DocumentActionType {
    /// The document transition this action broadcasts, as action fees are
    /// declared per transition type.
    fn transition_action(&self) -> DocumentTransitionActionType {
        match self {
            DocumentActionType::Create => DocumentTransitionActionType::Create,
            DocumentActionType::Delete => DocumentTransitionActionType::Delete,
            DocumentActionType::Purchase => DocumentTransitionActionType::Purchase,
            DocumentActionType::Replace => DocumentTransitionActionType::Replace,
            DocumentActionType::SetPrice => DocumentTransitionActionType::UpdatePrice,
            DocumentActionType::Transfer => DocumentTransitionActionType::Transfer,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            DocumentActionType::Create => "Create Document",
            DocumentActionType::Delete => "Delete Document",
            DocumentActionType::Purchase => "Purchase Document",
            DocumentActionType::Replace => "Replace Document",
            DocumentActionType::SetPrice => "Set Document Price",
            DocumentActionType::Transfer => "Transfer Document",
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum BroadcastStatus {
    NotBroadcasted,
    Fetching,
    Fetched,
    Broadcasting,
    Broadcasted,
}

/// Shown in place of a field whose type the document editor cannot edit yet.
const UNEDITABLE_FIELD_NOTICE: &str = "The app cannot edit this field type yet.";

/// Why a document with a field type the editor cannot edit is not sent.
const UNEDITABLE_DOCUMENT_MESSAGE: &str = "This document has a field type the app cannot edit yet, so it cannot be created or changed here.";

/// Why a document type left over from another contract is not built.
const FOREIGN_DOCUMENT_TYPE_MESSAGE: &str = "This document type belongs to another contract. Select a document type from the selected contract and try again.";

/// Why a new document was not built.
#[derive(Debug, PartialEq)]
enum DocumentBuildError {
    /// The selected document type belongs to another contract.
    ForeignDocumentType,
    /// A selection or field is missing or invalid; the text is for details.
    Invalid(String),
}

impl From<String> for DocumentBuildError {
    fn from(detail: String) -> Self {
        Self::Invalid(detail)
    }
}

impl From<&str> for DocumentBuildError {
    fn from(detail: &str) -> Self {
        Self::Invalid(detail.to_string())
    }
}

pub struct DocumentActionScreen {
    pub app_context: Arc<AppContext>,
    pub action_type: DocumentActionType,

    // Common fields
    no_documents_found: bool,
    pub selected_identity: Option<QualifiedIdentity>,
    selected_identity_string: String,
    pub selected_key: Option<IdentityPublicKey>,
    show_advanced_options: bool,
    pub wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_open_attempted: bool,
    pub wallet_unlock_popup: WalletUnlockPopup,
    pub wallet_failure: Option<String>,
    pub broadcast_status: BroadcastStatus,
    pub selected_contract: Option<QualifiedContract>,
    pub selected_document_type: Option<DocumentType>,
    pub contract_search: String,

    // Action-specific fields
    pub document_id_input: String,

    // Create-specific
    pub field_inputs: HashMap<String, String>,

    // Purchase-specific
    pub fetched_price: Option<Credits>,

    // Replace-specific
    pub original_doc: Option<Document>,

    // SetPrice-specific
    pub price_input: String,

    // Transfer-specific
    pub identities_map: HashMap<Identifier, QualifiedIdentity>,
    pub recipient_id_input: String,

    // Delete-specific
    pub fetched_documents: IndexMap<Identifier, Option<Document>>,
    /// The fetched entry of an indexOnly type selected for deletion.
    index_only_entry: Option<Document>,
    /// The values the selected indexOnly entry is deleted by, as editable
    /// JSON (prefilled from the fetched entry).
    index_only_values_input: String,

    // Fee tracking
    pub completed_fee_result: Option<FeeResult>,

    // Banner for in-progress operations
    refresh_banner: Option<BannerHandle>,

    /// The confirmation of a document action fee or of an indexOnly delete,
    /// open while the user decides.
    action_fee_confirmation: Option<PendingActionFeeConfirmation>,
    /// The action fee agreement the next built action carries.
    agreed_action_fee: Option<DocumentActionFeeAgreement>,
    /// Whether to pay a document type's optional token cost. Off, the action
    /// carries no token payment and its gas is paid in credits instead.
    pay_optional_token_cost: bool,
    // The document fetch whose result the form waits for
    pending_fetch: PendingRequest,

    // The document action broadcast whose result or error alone ends
    // `BroadcastStatus::Broadcasting`
    pending_broadcast: PendingRequest,
}

/// Shown when the selected identity holds no key this device can sign the
/// chosen document action with.
const NO_SIGNING_KEY_NOTICE: &str = "This identity has no key on this device that can sign this action. Add or load a suitable key from the identity's keys, or select another identity.";

/// The token payment a document action carries for `cost`: none without a
/// cost, and none for an optional cost the user chose not to pay
/// (`pay_optional`).
fn token_payment(
    cost: Option<DocumentActionTokenCost>,
    pay_optional: bool,
) -> Option<TokenPaymentInfo> {
    cost.filter(|cost| !cost.optional || pay_optional)
        .map(|cost| {
            TokenPaymentInfo::V0(TokenPaymentInfoV0 {
                payment_token_contract_id: cost.contract_id,
                token_contract_position: cost.token_contract_position,
                gas_fees_paid_by: cost.gas_fees_paid_by,
                minimum_token_cost: None,
                maximum_token_cost: Some(cost.token_amount),
            })
        })
}

/// Shown for an action other than a delete on an indexOnly document type.
const INDEX_ONLY_READ_ONLY_NOTICE: &str = "Entries of this document type are rebuilt from the network's indexes and are read-only: they cannot be edited, replaced, transferred or sold. To remove an entry, use Delete Document.";

/// How an indexOnly entry is deleted, above its delete form.
const INDEX_ONLY_DELETE_INTRO: &str = "Entries of this document type are rebuilt from the network's indexes. They are read-only, and an entry is deleted by the values it was created with, not by its ID. Fetch your entries and select the one to delete.";

/// Shown instead of the indexOnly delete form on a network without it.
const INDEX_ONLY_DELETE_UNSUPPORTED: &str = "This network does not support deleting entries of this document type yet. Try again after the network upgrades.";

/// Above the editable values of the selected indexOnly entry.
const INDEX_ONLY_VALUES_HINT: &str = "These are the values the network returned for this entry. If some were not returned, add them exactly as the entry was created with.";

/// The confirmation of an indexOnly delete: Platform matches the entry by
/// its values and refuses a mismatch only after the fee is paid.
const INDEX_ONLY_DELETE_WARNING: &str = "This entry is deleted by its values, not by its ID. If any value differs from what the entry was created with, the network refuses the delete and the fee is still charged. Check the values before you continue.";

/// An open contract fee confirmation and the action it was opened for: the
/// action is built, fee agreement included, when the dialog opens, and that
/// exact action is sent on confirm.
struct PendingActionFeeConfirmation {
    dialog: ConfirmationDialog,
    task: BackendTask,
}

impl DocumentActionScreen {
    pub fn new(
        app_context: Arc<AppContext>,
        selected_identity: Option<QualifiedIdentity>,
        action_type: DocumentActionType,
    ) -> Self {
        let known_contracts = app_context.get_contracts().unwrap_or_default();

        let identities_map = if let Ok(identities) = app_context.load_local_user_identities() {
            identities
                .into_iter()
                .map(|identity| (identity.identity.id(), identity))
                .collect()
        } else {
            HashMap::new()
        };

        let selected_contract = known_contracts.into_iter().next();

        // Seed from the app-scoped selected identity when none was passed in (W2 SYNC).
        let selected_identity =
            selected_identity.or_else(|| app_context.resolve_selected_identity());

        let selected_identity_string = selected_identity
            .as_ref()
            .map(|qi| qi.identity.id().to_string(Encoding::Base58))
            .unwrap_or_default();

        Self {
            app_context,
            action_type,
            no_documents_found: false,
            selected_identity,
            selected_identity_string,
            selected_key: None,
            show_advanced_options: false,
            wallet: None,
            wallet_open_attempted: false,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            wallet_failure: None,
            broadcast_status: BroadcastStatus::NotBroadcasted,
            selected_contract,
            selected_document_type: None,
            contract_search: String::new(),
            document_id_input: String::new(),
            field_inputs: HashMap::new(),
            fetched_price: None,
            original_doc: None,
            price_input: String::new(),
            identities_map,
            recipient_id_input: String::new(),
            fetched_documents: IndexMap::new(),
            index_only_entry: None,
            index_only_values_input: String::new(),
            completed_fee_result: None,
            refresh_banner: None,
            action_fee_confirmation: None,
            agreed_action_fee: None,
            pay_optional_token_cost: true,
            pending_fetch: PendingRequest::default(),
            pending_broadcast: PendingRequest::default(),
        }
    }

    /// A fetch button, disabled while a broadcast runs: the fetch result would
    /// clear the broadcast's progress or replace its success.
    fn fetch_button(&self, ui: &mut Ui, label: &str) -> egui::Response {
        ui.add_enabled(
            self.broadcast_status != BroadcastStatus::Broadcasting,
            egui::Button::new(label),
        )
    }

    fn set_fetching_banner(&mut self, ctx: &egui::Context, text: &str) {
        self.refresh_banner.take_and_clear();
        let handle = MessageBanner::set_global(ctx, text, crate::ui::MessageType::Info);
        handle.with_elapsed();
        self.refresh_banner = Some(handle);
    }

    /// Fetch documents for the current selection and wait on that fetch
    /// alone; a result from any earlier fetch is then ignored, and nothing
    /// fetched before it can be sent while it runs or after it fails.
    fn dispatch_fetch(&mut self, query: DocumentQuery) -> AppAction {
        self.clear_fetched_document_state();
        self.broadcast_status = BroadcastStatus::Fetching;
        let task = BackendTask::DocumentTask(Box::new(DocumentTask::FetchDocuments(query)));
        self.pending_fetch.dispatch_with(task, ())
    }

    /// Send the document action and wait on that broadcast alone: only its
    /// own result or error ends it, so no other task can reopen the form for
    /// a second, paid send while it runs.
    fn start_broadcast(&mut self, ctx: &egui::Context, task: BackendTask) -> AppAction {
        self.broadcast_status = BroadcastStatus::Broadcasting;
        self.set_fetching_banner(ctx, "Broadcasting...");
        self.pending_broadcast.dispatch(task)
    }

    /// Stop waiting on an outstanding document fetch, so its result or error
    /// cannot reach the form. A broadcast started meanwhile owns the progress
    /// banner, so the banner goes only while the fetch is still shown.
    fn abandon_pending_fetch(&mut self) {
        self.pending_fetch.clear();
        if self.broadcast_status == BroadcastStatus::Fetching {
            self.refresh_banner.take_and_clear();
            self.broadcast_status = BroadcastStatus::NotBroadcasted;
        }
    }

    /// Drop the single document fetched for Purchase or Replace: its price,
    /// its content and the fields filled in from it. A running broadcast
    /// keeps its status; it was built before this call.
    fn clear_fetched_document_state(&mut self) {
        if self.broadcast_status == BroadcastStatus::Fetched {
            self.broadcast_status = BroadcastStatus::NotBroadcasted;
        }
        self.original_doc = None;
        self.field_inputs.clear();
        self.fetched_price = None;
    }

    /// The document ID was edited: the document fetched for the old ID, and
    /// any fetch still running for it, no longer match the form.
    fn document_id_changed(&mut self) {
        self.abandon_pending_fetch();
        self.clear_fetched_document_state();
    }

    /// The document ID input of Purchase and Replace, where the fetched
    /// document belongs to the ID shown.
    fn fetched_document_id_input(&mut self, ui: &mut Ui) {
        let dark_mode = ui.style().visuals.dark_mode;
        if ui
            .add(styled_text_edit_singleline(
                &mut self.document_id_input,
                dark_mode,
            ))
            .changed()
        {
            self.document_id_changed();
        }
    }

    /// Drop everything fetched or entered for the previous contract and
    /// document type, so no action can use it with the new selection.
    fn clear_selection_state(&mut self) {
        self.abandon_pending_fetch();
        self.clear_fetched_document_state();
        self.no_documents_found = false;
        self.fetched_documents.clear();
        self.clear_index_only_entry();
    }

    /// Start the form over on `app_context`'s network. Nothing selected,
    /// fetched or sent on the previous network stays, and no fetch or broadcast
    /// from it is still awaited.
    pub fn reset_for_network_switch(&mut self, app_context: Arc<AppContext>) {
        self.refresh_banner.take_and_clear();
        *self = Self::new(app_context, None, self.action_type.clone());
    }

    fn reset_screen(&mut self) {
        self.refresh_banner.take_and_clear();
        self.no_documents_found = false;
        self.selected_identity = None;
        self.selected_identity_string = String::new();
        self.selected_key = None;
        self.show_advanced_options = false;
        self.wallet = None;
        self.wallet_open_attempted = false;
        self.wallet_unlock_popup = WalletUnlockPopup::new();
        self.wallet_failure = None;
        self.broadcast_status = BroadcastStatus::NotBroadcasted;
        self.action_fee_confirmation = None;
        self.selected_contract = None;
        self.selected_document_type = None;
        self.contract_search.clear();
        self.document_id_input.clear();
        self.field_inputs.clear();
        self.fetched_price = None;
        self.original_doc = None;
        self.price_input.clear();
        self.recipient_id_input.clear();
        self.fetched_documents.clear();
        self.clear_index_only_entry();
        self.pending_fetch.clear();
        self.pending_broadcast.clear();
    }

    fn render_contract_and_type_selection(&mut self, ui: &mut Ui) {
        ui.heading("1. Select a contract and document type:");
        ui.add_space(10.0);

        let prev_contract_id = self.selected_contract.as_ref().map(|c| c.contract.id());
        let prev_doc_type = self
            .selected_document_type
            .as_ref()
            .map(|d| d.name().to_owned());

        add_contract_doc_type_chooser_with_filtering(
            ui,
            &mut self.contract_search,
            &self.app_context,
            &mut self.selected_contract,
            &mut self.selected_document_type,
        );

        let contract_changed =
            prev_contract_id != self.selected_contract.as_ref().map(|c| c.contract.id());
        let doc_type_changed = prev_doc_type
            != self
                .selected_document_type
                .as_ref()
                .map(|d| d.name().to_owned());
        if contract_changed || doc_type_changed {
            self.signing_scope_changed();
        }

        ui.add_space(10.0);
    }

    /// The wallet holding the key the action is signed with, for the unlock
    /// prompt.
    fn signing_wallet(&self) -> Option<Arc<RwLock<Wallet>>> {
        let identity = self.selected_identity.as_ref()?;
        wallet_for_signing_key(identity, self.selected_key.as_ref())
    }

    /// The contract or document type changed: what was fetched for the old
    /// one no longer applies, and the key is picked again for the new scope
    /// (a key bound to the old contract or type cannot sign in the new one).
    fn signing_scope_changed(&mut self) {
        self.clear_selection_state();
        if let Some(identity) = &self.selected_identity {
            self.selected_key = auto_selected_document_key(
                identity,
                self.selected_contract.as_ref(),
                self.selected_document_type.as_ref(),
            );
        }
    }

    fn render_identity_and_key_selection(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.heading("2. Select an identity:");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.show_advanced_options, "Advanced Options");
            });
        });
        ui.add_space(10.0);

        let identities_vec: Vec<_> = self.identities_map.values().cloned().collect();

        // Identity selector — SYNC: write-back via syncing_global on user pick.
        let response = ui.add(
            IdentitySelector::new(
                "document_action_identity_selector",
                &mut self.selected_identity_string,
                &identities_vec,
            )
            .selected_identity(&mut self.selected_identity)
            .unwrap()
            .width(300.0)
            .label("Identity:")
            .other_option(false)
            .syncing_global(self.app_context.clone()),
        );

        // Handle identity change - auto-select key and update wallet
        if response.changed() {
            self.abandon_pending_fetch();
            self.no_documents_found = false;
            self.fetched_documents.clear();
            if let Some(identity) = &self.selected_identity {
                self.selected_key = auto_selected_document_key(
                    identity,
                    self.selected_contract.as_ref(),
                    self.selected_document_type.as_ref(),
                );

                self.wallet = self.signing_wallet();
                self.wallet_open_attempted = false;
            } else {
                self.selected_key = None;
                self.wallet = None;
                self.wallet_open_attempted = false;
            }
        }

        // Key selector (only shown in advanced mode)
        if self.show_advanced_options {
            ui.add_space(10.0);
            if let Some(identity) = &self.selected_identity {
                add_key_chooser_with_doc_type(
                    ui,
                    &self.app_context,
                    identity,
                    &mut self.selected_key,
                    TransactionType::DocumentAction,
                    self.selected_document_type.as_ref(),
                    signing_scope(
                        self.selected_contract.as_ref(),
                        self.selected_document_type.as_ref(),
                    ),
                );
            }
        }

        ui.add_space(10.0);
    }

    fn render_action_specific_inputs(&mut self, ui: &mut Ui) -> AppAction {
        if self.selected_type_is_index_only() {
            return match self.action_type {
                DocumentActionType::Delete => self.render_index_only_delete_inputs(ui),
                _ => {
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.label(
                        RichText::new(INDEX_ONLY_READ_ONLY_NOTICE)
                            .color(DashColors::warning_color(dark_mode)),
                    );
                    AppAction::None
                }
            };
        }
        match self.action_type {
            DocumentActionType::Create => AppAction::None, // Handled separately
            DocumentActionType::Delete => self.render_delete_inputs(ui),
            DocumentActionType::Purchase => self.render_purchase_inputs(ui),
            DocumentActionType::Replace => self.render_replace_inputs(ui),
            DocumentActionType::SetPrice => self.render_set_price_inputs(ui),
            DocumentActionType::Transfer => self.render_transfer_inputs(ui),
        }
    }

    fn render_create_inputs(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.heading("3. Fill out the document type fields:");
        ui.add_space(10.0);

        if let (Some(contract), Some(doc_type)) =
            (&self.selected_contract, &self.selected_document_type)
        {
            let contract_id = contract.contract.id();
            let doc_type = doc_type.clone();

            self.ui_field_inputs(ui, &doc_type, contract_id);

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            self.render_token_cost_info(ui, &doc_type);
            action |= self.render_broadcast_button(ui);
        }
        action
    }

    fn render_delete_inputs(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.heading("3. Enter document ID to delete:");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Document ID:");
            let dark_mode = ui.style().visuals.dark_mode;
            ui.add(styled_text_edit_singleline(
                &mut self.document_id_input,
                dark_mode,
            ));
        });

        ui.add_space(10.0);

        // Check if we can fetch owned documents
        if self.fetched_documents.is_empty() {
            if self.selected_identity.is_some() {
                if let Some(query) = self.owned_documents_query() {
                    ui.label("This document type has an index on $ownerId, so you can fetch owned documents to view and select.");
                    ui.add_space(10.0);
                    if self.fetch_button(ui, "Fetch Owned Documents").clicked() {
                        self.set_fetching_banner(ui.ctx(), "Fetching owned documents...");
                        action = self.dispatch_fetch(query);
                    }
                } else {
                    ui.label("(Cannot use the Fetch Owned Documents feature as this document type does not have an index on $ownerId)");
                }
            }
        } else {
            // Show fetched documents
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            ui.heading("Fetched documents:");
            ui.add_space(10.0);
            for document in self.fetched_documents.values() {
                if let Some(doc) = document {
                    let doc_id_str = doc.id().to_string(Encoding::Base58);

                    ui.horizontal(|ui| {
                        ui.label(&doc_id_str);

                        let view_button_response = ui.button("View");

                        if ui.button("Select").clicked() {
                            self.document_id_input = doc_id_str.clone();
                        }

                        let popup_id = egui::Id::new(format!("popup_{}", doc_id_str));
                        egui::Popup::new(
                            popup_id,
                            ui.ctx().clone(),
                            &view_button_response,
                            view_button_response.layer_id,
                        )
                        .open_memory(
                            view_button_response
                                .clicked()
                                .then_some(egui::SetOpenCommand::Bool(true)),
                        )
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            ui.set_min_width(400.0);
                            ui.label("Document JSON:");
                            if let Ok(json) = serde_json::to_string_pretty(doc) {
                                ui.add(
                                    egui::TextEdit::multiline(&mut json.clone())
                                        .font(egui::TextStyle::Monospace)
                                        .desired_rows(10)
                                        .desired_width(380.0)
                                        .interactive(false),
                                );
                            } else {
                                ui.label("Failed to serialize document.");
                            }
                            if ui.button("Close").clicked() {
                                ui.close();
                            }
                        });
                    });
                } else {
                    ui.label("Document not found");
                }
            }
        }

        if self.no_documents_found {
            ui.add_space(10.0);
            ui.label("No owned documents found.");
        }

        // Show fetching status
        if self.broadcast_status == BroadcastStatus::Fetching {
            ui.add_space(10.0);
            ui.spinner();
        }

        ui.add_space(10.0);
        if let Some(doc_type) = &self.selected_document_type {
            self.render_token_cost_info(ui, &doc_type.clone());
        }
        action |= self.render_broadcast_button(ui);
        action
    }

    /// The query for the selected identity's documents of the selected type,
    /// when the type has an index starting with `$ownerId`.
    fn owned_documents_query(&self) -> Option<DocumentQuery> {
        let identity = self.selected_identity.as_ref()?;
        let doc_type = self.selected_document_type.as_ref()?;
        let contract = self.selected_contract.as_ref()?;
        let owner_indexed = doc_type.indexes().values().any(|index| {
            index
                .properties
                .first()
                .is_some_and(|property| property.name == "$ownerId")
        });
        if !owner_indexed {
            return None;
        }
        let query = DocumentQuery::new(contract.contract.clone(), doc_type.name()).ok()?;
        Some(query.with_where(WhereClause {
            field: "$ownerId".to_string(),
            operator: dash_sdk::drive::query::WhereOperator::Equal,
            value: Value::Identifier(identity.identity.id().into()),
        }))
    }

    fn selected_type_is_index_only(&self) -> bool {
        self.selected_document_type
            .as_ref()
            .is_some_and(|doc_type| is_index_only(doc_type.as_ref()))
    }

    /// Whether this screen deletes an entry of an indexOnly type.
    fn is_index_only_delete(&self) -> bool {
        self.action_type == DocumentActionType::Delete && self.selected_type_is_index_only()
    }

    fn clear_index_only_entry(&mut self) {
        self.index_only_entry = None;
        self.index_only_values_input.clear();
    }

    /// Selects a fetched indexOnly entry for deletion, prefilling the values
    /// it is deleted by.
    fn select_index_only_entry(&mut self, document: Document) {
        self.index_only_values_input = IndexOnlyDeleteValues::from_document(&document).to_json();
        self.index_only_entry = Some(document);
    }

    /// The values the selected indexOnly entry is deleted by, checked the
    /// way the backend checks them.
    fn index_only_delete_values(&self) -> Result<IndexOnlyDeleteValues, IndexOnlyDeleteError> {
        let values = IndexOnlyDeleteValues::from_json(&self.index_only_values_input)?;
        if let Some(doc_type) = &self.selected_document_type {
            validate_index_only_delete(
                doc_type.as_ref(),
                &values,
                self.app_context.platform_version(),
            )?;
        }
        Ok(values)
    }

    fn render_index_only_delete_inputs(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;
        ui.heading("3. Select the entry to delete:");
        ui.add_space(10.0);
        ui.label(INDEX_ONLY_DELETE_INTRO);
        ui.add_space(10.0);

        if !FeatureGate::IndexOnlyDocumentDelete.is_available(&self.app_context) {
            ui.label(
                RichText::new(INDEX_ONLY_DELETE_UNSUPPORTED)
                    .color(DashColors::warning_color(dark_mode)),
            );
            return action;
        }

        match self.owned_documents_query() {
            Some(query) => {
                if self.fetch_button(ui, "Fetch My Entries").clicked() {
                    self.clear_index_only_entry();
                    self.set_fetching_banner(ui.ctx(), "Fetching your entries...");
                    action = self.dispatch_fetch(query);
                }
            }
            None => {
                ui.label("Your entries cannot be fetched: this document type has no index that starts with the owner.");
            }
        }

        let mut selected = None;
        if !self.fetched_documents.is_empty() {
            ui.add_space(10.0);
            for document in self.fetched_documents.values().flatten() {
                let doc_id_str = document.id().to_string(Encoding::Base58);
                ui.horizontal(|ui| {
                    ui.label(&doc_id_str);
                    ui.label(
                        RichText::new("Read-only entry")
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                    if ui.button("Select").clicked() {
                        selected = Some(document.clone());
                    }
                });
            }
        }
        if let Some(document) = selected {
            self.select_index_only_entry(document);
        }

        if self.no_documents_found {
            ui.add_space(10.0);
            ui.label("No entries found.");
        }
        if self.broadcast_status == BroadcastStatus::Fetching {
            ui.add_space(10.0);
            ui.spinner();
        }

        if self.index_only_entry.is_some() {
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);
            ui.heading("4. Check the values of the entry:");
            ui.add_space(10.0);
            ui.label(INDEX_ONLY_VALUES_HINT);
            ui.add_space(10.0);
            ui.add(
                egui::TextEdit::multiline(&mut self.index_only_values_input)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(8)
                    .desired_width(f32::INFINITY),
            );
            if let Err(error) = self.index_only_delete_values() {
                ui.add_space(5.0);
                ui.label(
                    RichText::new(error.to_string()).color(DashColors::error_color(dark_mode)),
                );
            }
            ui.add_space(10.0);
            if let Some(doc_type) = &self.selected_document_type {
                self.render_token_cost_info(ui, &doc_type.clone());
            }
            action |= self.render_broadcast_button(ui);
        }
        action
    }

    fn render_purchase_inputs(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.heading("3. Enter document ID to purchase:");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Document ID:");
            self.fetched_document_id_input(ui);
        });

        // Add fetch button
        ui.add_space(10.0);
        if self.fetch_button(ui, "Fetch Document Price").clicked()
            && !self.document_id_input.is_empty()
        {
            if let Ok(doc_id) = Identifier::from_string(&self.document_id_input, Encoding::Base58) {
                if let (Some(contract), Some(doc_type)) =
                    (&self.selected_contract, &self.selected_document_type)
                {
                    let mut query = DocumentQuery::new(contract.contract.clone(), doc_type.name())
                        .expect("Failed to create document query");
                    query = query.with_document_id(&doc_id);

                    self.set_fetching_banner(ui.ctx(), "Fetching document price...");
                    action = self.dispatch_fetch(query);
                }
            } else {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Invalid Document ID format",
                    crate::ui::MessageType::Error,
                );
            }
        }

        // Show fetching status
        if self.broadcast_status == BroadcastStatus::Fetching {
            ui.add_space(10.0);
            ui.spinner();
        }

        if let Some(price) = self.fetched_price {
            ui.add_space(10.0);
            ui.label(format!("Document price: {} credits", price));
            ui.label("Note: This is the listed price at the time of fetching. If you press the broadcast button, you will attempt to purchase the document at this price.");
        }

        ui.add_space(10.0);
        if let Some(doc_type) = &self.selected_document_type {
            self.render_token_cost_info(ui, &doc_type.clone());
        }
        action |= self.render_broadcast_button(ui);
        action
    }

    fn render_replace_inputs(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.heading("3. Enter document ID and fetch existing document:");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Document ID:");
            self.fetched_document_id_input(ui);

            if self.fetch_button(ui, "Fetch").clicked() && !self.document_id_input.is_empty() {
                if let Ok(doc_id) =
                    Identifier::from_string(&self.document_id_input, Encoding::Base58)
                {
                    if let (Some(contract), Some(doc_type)) =
                        (&self.selected_contract, &self.selected_document_type)
                    {
                        let mut query =
                            DocumentQuery::new(contract.contract.clone(), doc_type.name())
                                .expect("Failed to create document query");
                        query = query.with_document_id(&doc_id);

                        self.set_fetching_banner(ui.ctx(), "Fetching document...");
                        action = self.dispatch_fetch(query);
                    }
                } else {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "Invalid Document ID format",
                        crate::ui::MessageType::Error,
                    );
                }
            }
        });

        // Show fetching status
        if self.broadcast_status == BroadcastStatus::Fetching {
            ui.add_space(10.0);
            ui.spinner();
        }

        if let Some(_original_doc) = &self.original_doc {
            ui.add_space(10.0);
            ui.label(RichText::new("Document fetched successfully.").color(Color32::DARK_GREEN));
            ui.add_space(10.0);
            ui.heading("4. Update document fields:");

            if let (Some(contract), Some(doc_type)) =
                (&self.selected_contract, &self.selected_document_type)
            {
                let contract_id = contract.contract.id();
                let doc_type = doc_type.clone();

                self.ui_field_inputs(ui, &doc_type, contract_id);

                ui.add_space(10.0);
                if let Some(doc_type) = &self.selected_document_type {
                    self.render_token_cost_info(ui, &doc_type.clone());
                }
                action |= self.render_broadcast_button(ui);
            }
        } else if self.broadcast_status == BroadcastStatus::Fetched {
            ui.add_space(10.0);
            let error_color = DashColors::error_color(ui.visuals().dark_mode);
            Frame::new()
                .fill(error_color.gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .stroke(egui::Stroke::new(1.0, error_color))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("No document found with the provided ID").color(error_color),
                    );
                });
        }
        action
    }

    fn render_set_price_inputs(&mut self, ui: &mut Ui) -> AppAction {
        ui.heading("3. Set document price:");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Document ID:");
            let dark_mode = ui.style().visuals.dark_mode;
            ui.add(styled_text_edit_singleline(
                &mut self.document_id_input,
                dark_mode,
            ));
        });

        ui.horizontal(|ui| {
            ui.label("Price (credits):");
            let dark_mode = ui.style().visuals.dark_mode;
            ui.add(styled_text_edit_singleline(
                &mut self.price_input,
                dark_mode,
            ));
        });

        ui.add_space(10.0);
        if let Some(doc_type) = &self.selected_document_type {
            self.render_token_cost_info(ui, &doc_type.clone());
        }
        self.render_broadcast_button(ui)
    }

    fn render_transfer_inputs(&mut self, ui: &mut Ui) -> AppAction {
        ui.heading("3. Transfer document:");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Document ID:");
            let dark_mode = ui.style().visuals.dark_mode;
            ui.add(styled_text_edit_singleline(
                &mut self.document_id_input,
                dark_mode,
            ));
        });

        ui.horizontal(|ui| {
            ui.label("Recipient Identity:");
            let dark_mode = ui.style().visuals.dark_mode;
            ui.add(styled_text_edit_singleline(
                &mut self.recipient_id_input,
                dark_mode,
            ));
        });

        ui.add_space(10.0);
        if let Some(doc_type) = &self.selected_document_type {
            self.render_token_cost_info(ui, &doc_type.clone());
        }
        self.render_broadcast_button(ui)
    }

    fn ui_field_inputs(
        &mut self,
        ui: &mut Ui,
        doc_type: &dash_sdk::dpp::data_contract::document_type::DocumentType,
        _contract_id: Identifier,
    ) {
        egui::Grid::new("property_input_grid")
            .num_columns(2)
            .spacing([10.0, 5.0])
            .striped(false)
            .show(ui, |ui| {
                for (prop_name, schema) in doc_type.properties() {
                    let val = self.field_inputs.entry(prop_name.clone()).or_default();
                    let label = if schema.required {
                        format!("{} *:", prop_name)
                    } else {
                        format!("{}:", prop_name)
                    };
                    ui.label(label);
                    match &schema.property_type {
                        DocumentPropertyType::U128
                        | DocumentPropertyType::I128
                        | DocumentPropertyType::U64
                        | DocumentPropertyType::I64
                        | DocumentPropertyType::U32
                        | DocumentPropertyType::KeyIdWithReference(_)
                        | DocumentPropertyType::I32
                        | DocumentPropertyType::U16
                        | DocumentPropertyType::I16
                        | DocumentPropertyType::U8
                        | DocumentPropertyType::I8 => {
                            let dark_mode = ui.style().visuals.dark_mode;
                            ui.add(
                                egui::TextEdit::singleline(val)
                                    .hint_text("integer")
                                    .text_color(DashColors::text_primary(dark_mode))
                                    .background_color(DashColors::input_background(dark_mode)),
                            );
                        }
                        DocumentPropertyType::F64 => {
                            let dark_mode = ui.style().visuals.dark_mode;
                            ui.add(
                                egui::TextEdit::singleline(val)
                                    .hint_text("floating-point")
                                    .text_color(DashColors::text_primary(dark_mode))
                                    .background_color(DashColors::input_background(dark_mode)),
                            );
                        }
                        DocumentPropertyType::String(size) => {
                            let dark_mode = ui.style().visuals.dark_mode;
                            ui.add({
                                let text_edit = egui::TextEdit::singleline(val)
                                    .text_color(DashColors::text_primary(dark_mode))
                                    .background_color(DashColors::input_background(dark_mode));
                                if let Some(max_length) = size.max_length {
                                    text_edit.hint_text(format!("max {}", max_length).as_str())
                                } else {
                                    text_edit
                                }
                            });
                        }
                        DocumentPropertyType::ByteArray(_size) => {
                            let dark_mode = ui.style().visuals.dark_mode;
                            ui.add(
                                egui::TextEdit::singleline(val)
                                    .hint_text("hex or base64")
                                    .text_color(DashColors::text_primary(dark_mode))
                                    .background_color(DashColors::input_background(dark_mode)),
                            );
                        }
                        DocumentPropertyType::Identifier
                        | DocumentPropertyType::IdentifierWithReference(_) => {
                            let dark_mode = ui.style().visuals.dark_mode;
                            ui.add(
                                egui::TextEdit::singleline(val)
                                    .hint_text("base58 identifier")
                                    .text_color(DashColors::text_primary(dark_mode))
                                    .background_color(DashColors::input_background(dark_mode)),
                            );
                        }
                        DocumentPropertyType::Boolean => {
                            let mut checked = matches!(
                                val.to_ascii_lowercase().as_str(),
                                "true" | "1" | "yes" | "on"
                            );
                            if ui.checkbox(&mut checked, "").changed() {
                                *val = checked.to_string();
                            }
                        }
                        DocumentPropertyType::Date => {
                            let dark_mode = ui.style().visuals.dark_mode;
                            ui.add(
                                egui::TextEdit::singleline(val)
                                    .hint_text("unix-ms")
                                    .text_color(DashColors::text_primary(dark_mode))
                                    .background_color(DashColors::input_background(dark_mode)),
                            );
                        }
                        DocumentPropertyType::TypedArray(_) => {
                            ui.label(UNEDITABLE_FIELD_NOTICE);
                        }
                        DocumentPropertyType::Object(_)
                        | DocumentPropertyType::Array(_)
                        | DocumentPropertyType::VariableTypeArray(_) => {
                            let dark_mode = ui.style().visuals.dark_mode;
                            ui.add(
                                egui::TextEdit::multiline(val)
                                    .hint_text("JSON value")
                                    .text_color(DashColors::text_primary(dark_mode))
                                    .background_color(DashColors::input_background(dark_mode)),
                            );
                        }
                    }
                    ui.end_row();
                }
            });
    }

    fn render_token_cost_info(
        &mut self,
        ui: &mut Ui,
        doc_type: &dash_sdk::dpp::data_contract::document_type::DocumentType,
    ) {
        let cost = match self.action_type {
            DocumentActionType::Create => doc_type.document_creation_token_cost(),
            DocumentActionType::Delete => doc_type.document_deletion_token_cost(),
            DocumentActionType::Purchase => doc_type.document_purchase_token_cost(),
            DocumentActionType::Replace => doc_type.document_replacement_token_cost(),
            DocumentActionType::SetPrice => doc_type.document_update_price_token_cost(),
            DocumentActionType::Transfer => doc_type.document_transfer_token_cost(),
        };
        if let Some(token_creation_cost) = cost {
            let token_amount = token_creation_cost.token_amount;
            let token_name = if let Some(contract) = &self.selected_contract {
                let contract_id = contract.contract.id();
                if let Ok(Some(contract)) = self
                    .app_context
                    .get_contract_by_id(&contract_id)
                    .map_err(|_| "Contract not found locally")
                {
                    contract
                        .contract
                        .tokens()
                        .get(&token_creation_cost.token_contract_position)
                        .map(|t| {
                            t.conventions()
                                .singular_form_by_language_code_or_default("en")
                                .to_string()
                        })
                        .unwrap_or_else(|| {
                            format!("Token {}", token_creation_cost.token_contract_position)
                        })
                } else {
                    "Unknown contract".to_string()
                }
            } else {
                "Unknown contract".to_string()
            };
            let token_effect_string = match token_creation_cost.effect {
                DocumentActionTokenEffect::TransferTokenToContractOwner => {
                    "transferred to the contract owner"
                }
                DocumentActionTokenEffect::BurnToken => "burned",
            };
            let gas_fees_paid_by_string = match token_creation_cost.gas_fees_paid_by {
                GasFeesPaidBy::DocumentOwner => "you",
                GasFeesPaidBy::ContractOwner => "the contract owner",
                GasFeesPaidBy::PreferContractOwner => {
                    "the contract owner unless their balance is insufficient, in which case you pay"
                }
            };
            ui.label(
                RichText::new(format!(
                    "Token cost: {} \"{}\" tokens.\nTokens will be {}.\nGas fees will be paid by {}.",
                    token_amount, token_name, token_effect_string, gas_fees_paid_by_string
                ))
                .color(Color32::DARK_RED),
            );
            // Protocol version 14: an optional token cost may be left out, the
            // action then paying its gas in credits with no sponsorship.
            if token_creation_cost.optional {
                ui.checkbox(&mut self.pay_optional_token_cost, "Pay the token cost")
                    .on_hover_text(
                        "Paying the token cost is optional for this action. Without it, you pay the network fee yourself in credits.",
                    );
            }
        } else {
            self.pay_optional_token_cost = true;
        }
    }

    /// The action fee the selected document type charges for this action at
    /// the current fee multiplier, `None` when it charges none.
    fn action_fee_quote(&self) -> Option<DocumentActionFeeQuote> {
        let doc_type = self.selected_document_type.as_ref()?;
        document_action_fee_quote(
            doc_type.as_ref(),
            self.action_type.transition_action(),
            self.app_context.fee_multiplier_permille(),
        )
    }

    /// The token payment for `cost`, or `None` when there is no cost or the
    /// user chose not to pay an optional one.
    fn token_payment_for(&self, cost: Option<DocumentActionTokenCost>) -> Option<TokenPaymentInfo> {
        token_payment(cost, self.pay_optional_token_cost)
    }

    fn render_broadcast_button(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Fee estimation display
        let fee_estimator = self.app_context.fee_estimator();
        let estimated_fee = match self.action_type {
            DocumentActionType::Create => fee_estimator.estimate_document_create(),
            DocumentActionType::Delete => fee_estimator.estimate_document_delete(),
            DocumentActionType::Replace => fee_estimator.estimate_document_replace(),
            DocumentActionType::Transfer => fee_estimator.estimate_document_transfer(),
            DocumentActionType::Purchase => fee_estimator.estimate_document_purchase(),
            DocumentActionType::SetPrice => fee_estimator.estimate_document_set_price(),
        };

        let action_fee = self.action_fee_quote();

        ui.add_space(10.0);
        let dark_mode = ui.style().visuals.dark_mode;
        Frame::new()
            .fill(DashColors::surface(dark_mode))
            .inner_margin(Margin::symmetric(10, 8))
            .corner_radius(5.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Estimated fee:")
                            .color(DashColors::text_secondary(dark_mode))
                            .size(14.0),
                    );
                    ui.label(
                        RichText::new(format_credits_as_dash(estimated_fee))
                            .color(DashColors::text_primary(dark_mode))
                            .size(14.0),
                    );
                });
                if let Some(quote) = &action_fee {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("Contract fee:")
                                .color(DashColors::text_secondary(dark_mode))
                                .size(14.0),
                        );
                        ui.label(
                            RichText::new(action_fee_summary(quote))
                                .color(DashColors::text_primary(dark_mode))
                                .size(14.0),
                        );
                    });
                }
            });

        ui.add_space(10.0);
        let button_text = match self.action_type {
            DocumentActionType::Create => "Broadcast document",
            DocumentActionType::Delete => "Delete document",
            DocumentActionType::Purchase => "Purchase document",
            DocumentActionType::Replace => "Replace document",
            DocumentActionType::SetPrice => "Set document price",
            DocumentActionType::Transfer => "Transfer document",
        };

        // A second click while the first action is being sent would pay for it twice.
        let idle = self.broadcast_status != BroadcastStatus::Broadcasting;
        if ComponentStyles::add_primary_button_enabled(ui, idle, button_text).clicked()
            && self.can_broadcast()
        {
            // A contract fee is paid only with the user's explicit agreement
            // to the amount shown, and an indexOnly delete only once the user
            // was told a mismatched value is refused after the fee is paid.
            if action_fee.is_some() || self.is_index_only_delete() {
                self.open_action_fee_confirmation(action_fee);
            } else {
                self.agreed_action_fee = None;
                action = self.dispatch_document_action(ui.ctx());
            }
        }

        let fee_response = self
            .action_fee_confirmation
            .as_mut()
            .and_then(|pending| pending.dialog.show(ui).inner.dialog_response);
        if let Some(response) = fee_response {
            match response {
                ConfirmationStatus::Confirmed => {
                    action = self.confirm_action_fee(ui.ctx());
                }
                ConfirmationStatus::Canceled => {
                    self.action_fee_confirmation = None;
                }
            }
        }

        // Status display
        match &self.broadcast_status {
            BroadcastStatus::Broadcasting | BroadcastStatus::Fetching => {
                ui.add_space(10.0);
                ui.spinner();
            }
            _ => {}
        }

        action
    }

    /// Ask the user to agree to the contract fee `quote`, and to an indexOnly
    /// delete, before the action is sent.
    /// The action is built now, carrying the quoted agreement, and the dialog
    /// blocks the form behind it, so what the user confirms is what is sent.
    fn open_action_fee_confirmation(&mut self, quote: Option<DocumentActionFeeQuote>) {
        self.agreed_action_fee = quote.as_ref().map(|quote| quote.agreement);
        let task = self.create_document_action();
        self.agreed_action_fee = None;
        if task == BackendTask::None {
            return;
        }
        let (title, text) =
            confirmation_title_and_text(quote.as_ref(), self.is_index_only_delete());
        self.action_fee_confirmation = Some(PendingActionFeeConfirmation {
            dialog: ConfirmationDialog::new(title.to_string(), text).blocks_input(true),
            task,
        });
    }

    /// The user agreed to the contract fee: send the action the confirmation
    /// was opened for.
    fn confirm_action_fee(&mut self, ctx: &egui::Context) -> AppAction {
        match self.action_fee_confirmation.take() {
            Some(pending) => self.dispatch_task(ctx, pending.task),
            None => AppAction::None,
        }
    }

    /// Build and dispatch the document action, carrying the agreed action fee.
    fn dispatch_document_action(&mut self, ctx: &egui::Context) -> AppAction {
        let task = self.create_document_action();
        self.dispatch_task(ctx, task)
    }

    /// Send `task`, a built document action, showing its progress.
    fn dispatch_task(&mut self, ctx: &egui::Context, task: BackendTask) -> AppAction {
        if task == BackendTask::None {
            return AppAction::None;
        }
        self.start_broadcast(ctx, task)
    }

    /// Resolve the four selections every document action needs — document
    /// type, contract, identity, and signing key.
    ///
    /// The action buttons are only enabled once all four are chosen, so a
    /// missing one is a broken UI invariant rather than normal input. Instead
    /// of panicking, show an actionable banner and return `None` so the caller
    /// degrades to `BackendTask::None`.
    #[allow(clippy::type_complexity)]
    fn require_selections(
        &self,
    ) -> Option<(
        &DocumentType,
        &QualifiedContract,
        &QualifiedIdentity,
        &IdentityPublicKey,
    )> {
        match (
            self.selected_document_type.as_ref(),
            self.selected_contract.as_ref(),
            self.selected_identity.as_ref(),
            self.selected_key.as_ref(),
        ) {
            (Some(doc_type), Some(contract), Some(identity), Some(key)) => {
                Some((doc_type, contract, identity, key))
            }
            _ => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "Select a document type, contract, identity, and key before continuing.",
                    crate::ui::MessageType::Error,
                );
                None
            }
        }
    }

    fn create_document_action(&mut self) -> BackendTask {
        match self.action_type {
            DocumentActionType::Create => self.create_document_task(),
            DocumentActionType::Delete => self.create_delete_task(),
            DocumentActionType::Purchase => self.create_purchase_task(),
            DocumentActionType::Replace => self.create_replace_task(),
            DocumentActionType::SetPrice => self.create_set_price_task(),
            DocumentActionType::Transfer => self.create_transfer_task(),
        }
    }

    fn create_document_task(&mut self) -> BackendTask {
        match self.try_build_document() {
            Ok((doc, entropy)) => {
                let Some((doc_type, contract, identity, key)) = self.require_selections() else {
                    return BackendTask::None;
                };

                let token_payment_info =
                    self.token_payment_for(doc_type.document_creation_token_cost());

                BackendTask::DocumentTask(Box::new(DocumentTask::BroadcastDocument {
                    action_fee_agreement: self.agreed_action_fee,
                    document: doc,
                    token_payment_info,
                    entropy,
                    document_type: doc_type.clone(),
                    data_contract: Arc::new(contract.contract.clone()),
                    qualified_identity: identity.clone(),
                    identity_key: key.clone(),
                }))
            }
            Err(DocumentBuildError::ForeignDocumentType) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    FOREIGN_DOCUMENT_TYPE_MESSAGE,
                    crate::ui::MessageType::Error,
                );
                BackendTask::None
            }
            Err(DocumentBuildError::Invalid(detail)) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    "The document could not be built. Check its fields and try again.",
                    crate::ui::MessageType::Error,
                )
                .with_details(detail);
                BackendTask::None
            }
        }
    }

    fn create_delete_task(&self) -> BackendTask {
        if self.is_index_only_delete() {
            return self.create_index_only_delete_task();
        }
        let document_id =
            Identifier::from_string(&self.document_id_input, Encoding::Base58).unwrap_or_default();

        let Some((doc_type, contract, identity, key)) = self.require_selections() else {
            return BackendTask::None;
        };

        let token_payment_info = self.token_payment_for(doc_type.document_deletion_token_cost());

        BackendTask::DocumentTask(Box::new(DocumentTask::DeleteDocument {
            action_fee_agreement: self.agreed_action_fee,
            document_id,
            document_type: doc_type.clone(),
            data_contract: Arc::new(contract.contract.clone()),
            qualified_identity: identity.clone(),
            identity_key: key.clone(),
            token_payment_info,
        }))
    }

    fn create_index_only_delete_task(&self) -> BackendTask {
        let Some(entry) = &self.index_only_entry else {
            return BackendTask::None;
        };
        let values = match self.index_only_delete_values() {
            Ok(values) => values,
            Err(error) => {
                MessageBanner::set_global(
                    self.app_context.egui_ctx(),
                    error.to_string(),
                    crate::ui::MessageType::Error,
                );
                return BackendTask::None;
            }
        };
        let Some((doc_type, contract, identity, key)) = self.require_selections() else {
            return BackendTask::None;
        };
        // Charged as the delete by id is, optional cost choice included.
        let token_payment_info = self.token_payment_for(doc_type.document_deletion_token_cost());

        BackendTask::DocumentTask(Box::new(DocumentTask::DeleteIndexOnlyDocument {
            action_fee_agreement: self.agreed_action_fee,
            document_id: entry.id(),
            values,
            document_type: doc_type.clone(),
            data_contract: Arc::new(contract.contract.clone()),
            qualified_identity: identity.clone(),
            identity_key: key.clone(),
            token_payment_info,
        }))
    }

    fn create_purchase_task(&self) -> BackendTask {
        let document_id =
            Identifier::from_string(&self.document_id_input, Encoding::Base58).unwrap_or_default();

        let Some((doc_type, contract, identity, key)) = self.require_selections() else {
            return BackendTask::None;
        };

        let token_payment_info = self.token_payment_for(doc_type.document_purchase_token_cost());

        BackendTask::DocumentTask(Box::new(DocumentTask::PurchaseDocument {
            action_fee_agreement: self.agreed_action_fee,
            price: self.fetched_price.unwrap_or(0),
            document_id,
            document_type: doc_type.clone(),
            data_contract: Arc::new(contract.contract.clone()),
            qualified_identity: identity.clone(),
            identity_key: key.clone(),
            token_payment_info,
        }))
    }

    fn create_replace_task(&mut self) -> BackendTask {
        if let Some(_original_doc) = &self.original_doc {
            match self.try_build_document_from_original() {
                Ok((updated_doc, _entropy)) => {
                    let Some((doc_type, contract, identity, key)) = self.require_selections()
                    else {
                        return BackendTask::None;
                    };

                    let token_payment_info =
                        self.token_payment_for(doc_type.document_replacement_token_cost());

                    BackendTask::DocumentTask(Box::new(DocumentTask::ReplaceDocument {
                        action_fee_agreement: self.agreed_action_fee,
                        document: updated_doc,
                        document_type: doc_type.clone(),
                        data_contract: Arc::new(contract.contract.clone()),
                        qualified_identity: identity.clone(),
                        identity_key: key.clone(),
                        token_payment_info,
                    }))
                }
                Err(error) => {
                    MessageBanner::set_global(
                        self.app_context.egui_ctx(),
                        "The updated document could not be built. Check its fields and try again.",
                        crate::ui::MessageType::Error,
                    )
                    .with_details(error);
                    BackendTask::None
                }
            }
        } else {
            let Some((doc_type, contract, identity, key)) = self.require_selections() else {
                return BackendTask::None;
            };

            let token_payment_info =
                self.token_payment_for(doc_type.document_replacement_token_cost());

            BackendTask::DocumentTask(Box::new(DocumentTask::ReplaceDocument {
                action_fee_agreement: self.agreed_action_fee,
                document: DocumentV0::default().into(),
                document_type: doc_type.clone(),
                data_contract: Arc::new(contract.contract.clone()),
                qualified_identity: identity.clone(),
                identity_key: key.clone(),
                token_payment_info,
            }))
        }
    }

    fn create_set_price_task(&self) -> BackendTask {
        let document_id =
            Identifier::from_string(&self.document_id_input, Encoding::Base58).unwrap_or_default();
        let price = self.price_input.parse::<u64>().unwrap_or(0);

        let Some((doc_type, contract, identity, key)) = self.require_selections() else {
            return BackendTask::None;
        };

        let token_payment_info =
            self.token_payment_for(doc_type.document_update_price_token_cost());

        BackendTask::DocumentTask(Box::new(DocumentTask::SetDocumentPrice {
            action_fee_agreement: self.agreed_action_fee,
            price,
            document_id,
            document_type: doc_type.clone(),
            data_contract: Arc::new(contract.contract.clone()),
            qualified_identity: identity.clone(),
            identity_key: key.clone(),
            token_payment_info,
        }))
    }

    fn create_transfer_task(&self) -> BackendTask {
        let document_id =
            Identifier::from_string(&self.document_id_input, Encoding::Base58).unwrap_or_default();
        let recipient_id =
            Identifier::from_string(&self.recipient_id_input, Encoding::Base58).unwrap_or_default();

        let Some((doc_type, contract, identity, key)) = self.require_selections() else {
            return BackendTask::None;
        };

        let token_payment_info = self.token_payment_for(doc_type.document_transfer_token_cost());

        BackendTask::DocumentTask(Box::new(DocumentTask::TransferDocument {
            action_fee_agreement: self.agreed_action_fee,
            document_id,
            new_owner_id: recipient_id,
            document_type: doc_type.clone(),
            data_contract: Arc::new(contract.contract.clone()),
            qualified_identity: identity.clone(),
            identity_key: key.clone(),
            token_payment_info,
        }))
    }

    fn try_build_document(&self) -> Result<(Document, [u8; 32]), DocumentBuildError> {
        let contract = self
            .selected_contract
            .as_ref()
            .ok_or("No contract selected")?;
        let doc_type = self
            .selected_document_type
            .as_ref()
            .ok_or("No document-type selected")?;
        // The chooser keeps the type when the contract changes, and the
        // document id commits to the type's contract, not the selected one.
        if doc_type.data_contract_id() != contract.contract.id() {
            return Err(DocumentBuildError::ForeignDocumentType);
        }
        let qi = self
            .selected_identity
            .as_ref()
            .ok_or("No identity selected")?;

        if !Self::supports_field_editor(doc_type) {
            return Err(UNEDITABLE_DOCUMENT_MESSAGE.into());
        }

        for (prop, schema) in doc_type.properties() {
            let entry = self.field_inputs.get(prop).map(|s| s.trim());
            if schema.required && entry.unwrap_or("").is_empty() {
                return Err(format!("Field \"{}\" is required", prop).into());
            }
        }

        let mut properties = BTreeMap::new();

        for (name, input_str) in &self.field_inputs {
            let schema = doc_type
                .properties()
                .get(name)
                .ok_or_else(|| format!("Unknown property {}", name))?;

            // Skip empty optional fields
            if !schema.required && input_str.trim().is_empty() {
                continue;
            }

            let value = match &schema.property_type {
                DocumentPropertyType::U128 => {
                    let n = input_str
                        .parse::<u128>()
                        .map_err(|_| format!("{} must be an unsigned 128-bit integer", name))?;
                    Value::U128(n)
                }
                DocumentPropertyType::I128 => {
                    let n = input_str
                        .parse::<i128>()
                        .map_err(|_| format!("{} must be a signed 128-bit integer", name))?;
                    Value::I128(n)
                }
                DocumentPropertyType::U64 => {
                    let n = input_str
                        .parse::<u64>()
                        .map_err(|_| format!("{} must be an unsigned 64-bit integer", name))?;
                    Value::U64(n)
                }
                DocumentPropertyType::I64 => {
                    let n = input_str
                        .parse::<i64>()
                        .map_err(|_| format!("{} must be a signed 64-bit integer", name))?;
                    Value::I64(n)
                }
                DocumentPropertyType::U32 | DocumentPropertyType::KeyIdWithReference(_) => {
                    let n = input_str
                        .parse::<u32>()
                        .map_err(|_| format!("{} must be an unsigned 32-bit integer", name))?;
                    Value::U32(n)
                }
                DocumentPropertyType::I32 => {
                    let n = input_str
                        .parse::<i32>()
                        .map_err(|_| format!("{} must be a signed 32-bit integer", name))?;
                    Value::I32(n)
                }
                DocumentPropertyType::U16 => {
                    let n = input_str
                        .parse::<u16>()
                        .map_err(|_| format!("{} must be an unsigned 16-bit integer", name))?;
                    Value::U16(n)
                }
                DocumentPropertyType::I16 => {
                    let n = input_str
                        .parse::<i16>()
                        .map_err(|_| format!("{} must be a signed 16-bit integer", name))?;
                    Value::I16(n)
                }
                DocumentPropertyType::U8 => {
                    let n = input_str
                        .parse::<u8>()
                        .map_err(|_| format!("{} must be an unsigned 8-bit integer", name))?;
                    Value::U8(n)
                }
                DocumentPropertyType::I8 => {
                    let n = input_str
                        .parse::<i8>()
                        .map_err(|_| format!("{} must be a signed 8-bit integer", name))?;
                    Value::I8(n)
                }
                DocumentPropertyType::F64 => {
                    let f = input_str
                        .parse::<f64>()
                        .map_err(|_| format!("{} must be a floating-point number", name))?;
                    Value::Float(f)
                }
                DocumentPropertyType::String(_size) => Value::Text(input_str.clone()),
                DocumentPropertyType::ByteArray(_size) => {
                    let bytes = if let Ok(b) = hex::decode(input_str) {
                        b
                    } else {
                        STANDARD.decode(input_str).map_err(|_| {
                            format!("{} must be hex or base64 for a ByteArray field", name)
                        })?
                    };
                    Value::Bytes(bytes)
                }
                DocumentPropertyType::Identifier
                | DocumentPropertyType::IdentifierWithReference(_) => {
                    let id = Identifier::from_string(input_str, Encoding::Base58)
                        .map_err(|_| format!("{} is not a valid Identifier (base58)", name))?;
                    id.into()
                }
                DocumentPropertyType::Boolean => {
                    let b = matches!(
                        input_str.to_ascii_lowercase().as_str(),
                        "true" | "1" | "yes"
                    );
                    Value::Bool(b)
                }
                DocumentPropertyType::Date => {
                    let ts = input_str
                        .parse::<u64>()
                        .map_err(|_| format!("{} (Date) expects unix-ms integer", name))?;
                    Value::U64(ts)
                }
                DocumentPropertyType::TypedArray(_) => {
                    // Unreachable while `supports_field_editor` refuses the
                    // document first; kept so a typed array is never encoded.
                    return Err(UNEDITABLE_DOCUMENT_MESSAGE.into());
                }
                DocumentPropertyType::Object(_) => {
                    return Err(format!(
                        "Object field {} must be supplied via JSON textarea",
                        name
                    )
                    .into());
                }
                DocumentPropertyType::Array(_) | DocumentPropertyType::VariableTypeArray(_) => {
                    return Err(
                        format!("Array field {} must be supplied via JSON textarea", name).into(),
                    );
                }
            };

            properties.insert(name.clone(), value);
        }

        let mut rng = StdRng::from_entropy();
        let entropy: [u8; 32] = rng.r#gen();

        let document = new_document_for_creation(
            doc_type.as_ref(),
            qi.identity.id(),
            properties,
            &entropy,
            self.app_context.platform_version(),
        )
        .map_err(|e| e.to_string())?;

        Ok((document, entropy))
    }

    fn try_build_document_from_original(&self) -> Result<(Document, [u8; 32]), String> {
        let original_doc = self.original_doc.as_ref().ok_or("No original document")?;
        let doc_type = self
            .selected_document_type
            .as_ref()
            .ok_or("No document-type selected")?;

        if !Self::supports_field_editor(doc_type) {
            return Err(UNEDITABLE_DOCUMENT_MESSAGE.to_string());
        }

        for (prop, schema) in doc_type.properties() {
            let entry = self.field_inputs.get(prop).map(|s| s.trim());
            if schema.required && entry.unwrap_or("").is_empty() {
                return Err(format!("Field \"{}\" is required", prop));
            }
        }

        let mut properties = BTreeMap::new();

        for (name, input_str) in &self.field_inputs {
            let schema = doc_type
                .properties()
                .get(name)
                .ok_or_else(|| format!("Unknown property {}", name))?;

            // Skip empty optional fields
            if !schema.required && input_str.trim().is_empty() {
                continue;
            }

            let value = match &schema.property_type {
                DocumentPropertyType::U128 => {
                    let n = input_str
                        .parse::<u128>()
                        .map_err(|_| format!("{} must be an unsigned 128-bit integer", name))?;
                    Value::U128(n)
                }
                DocumentPropertyType::I128 => {
                    let n = input_str
                        .parse::<i128>()
                        .map_err(|_| format!("{} must be a signed 128-bit integer", name))?;
                    Value::I128(n)
                }
                DocumentPropertyType::U64 => {
                    let n = input_str
                        .parse::<u64>()
                        .map_err(|_| format!("{} must be an unsigned 64-bit integer", name))?;
                    Value::U64(n)
                }
                DocumentPropertyType::I64 => {
                    let n = input_str
                        .parse::<i64>()
                        .map_err(|_| format!("{} must be a signed 64-bit integer", name))?;
                    Value::I64(n)
                }
                DocumentPropertyType::U32 | DocumentPropertyType::KeyIdWithReference(_) => {
                    let n = input_str
                        .parse::<u32>()
                        .map_err(|_| format!("{} must be an unsigned 32-bit integer", name))?;
                    Value::U32(n)
                }
                DocumentPropertyType::I32 => {
                    let n = input_str
                        .parse::<i32>()
                        .map_err(|_| format!("{} must be a signed 32-bit integer", name))?;
                    Value::I32(n)
                }
                DocumentPropertyType::U16 => {
                    let n = input_str
                        .parse::<u16>()
                        .map_err(|_| format!("{} must be an unsigned 16-bit integer", name))?;
                    Value::U16(n)
                }
                DocumentPropertyType::I16 => {
                    let n = input_str
                        .parse::<i16>()
                        .map_err(|_| format!("{} must be a signed 16-bit integer", name))?;
                    Value::I16(n)
                }
                DocumentPropertyType::U8 => {
                    let n = input_str
                        .parse::<u8>()
                        .map_err(|_| format!("{} must be an unsigned 8-bit integer", name))?;
                    Value::U8(n)
                }
                DocumentPropertyType::I8 => {
                    let n = input_str
                        .parse::<i8>()
                        .map_err(|_| format!("{} must be a signed 8-bit integer", name))?;
                    Value::I8(n)
                }
                DocumentPropertyType::F64 => {
                    let f = input_str
                        .parse::<f64>()
                        .map_err(|_| format!("{} must be a floating-point number", name))?;
                    Value::Float(f)
                }
                DocumentPropertyType::String(_size) => Value::Text(input_str.clone()),
                DocumentPropertyType::ByteArray(_size) => {
                    let bytes = if let Ok(b) = hex::decode(input_str) {
                        b
                    } else {
                        STANDARD.decode(input_str).map_err(|_| {
                            format!("{} must be hex or base64 for a ByteArray field", name)
                        })?
                    };
                    Value::Bytes(bytes)
                }
                DocumentPropertyType::Identifier
                | DocumentPropertyType::IdentifierWithReference(_) => {
                    let id = Identifier::from_string(input_str, Encoding::Base58)
                        .map_err(|_| format!("{} is not a valid Identifier (base58)", name))?;
                    id.into()
                }
                DocumentPropertyType::Boolean => {
                    let b = matches!(
                        input_str.to_ascii_lowercase().as_str(),
                        "true" | "1" | "yes"
                    );
                    Value::Bool(b)
                }
                DocumentPropertyType::Date => {
                    let ts = input_str
                        .parse::<u64>()
                        .map_err(|_| format!("{} (Date) expects unix-ms integer", name))?;
                    Value::U64(ts)
                }
                DocumentPropertyType::TypedArray(_) => {
                    // Unreachable while `supports_field_editor` refuses the
                    // document first; kept so a typed array is never encoded.
                    return Err(UNEDITABLE_DOCUMENT_MESSAGE.to_string());
                }
                DocumentPropertyType::Object(_) => {
                    return Err(format!(
                        "Object field {} must be supplied via JSON textarea",
                        name
                    ));
                }
                DocumentPropertyType::Array(_) | DocumentPropertyType::VariableTypeArray(_) => {
                    return Err(format!(
                        "Array field {} must be supplied via JSON textarea",
                        name
                    ));
                }
            };

            properties.insert(name.clone(), value);
        }

        let mut rng = StdRng::from_entropy();
        let entropy: [u8; 32] = rng.r#gen();

        let new_revision = if let Some(current_revision) = original_doc.revision() {
            Some(current_revision + 1)
        } else {
            Some(1)
        };

        // The replacement keeps the original's id, owner, creator and
        // moderation, and drops the timestamps and heights Platform sets.
        let mut updated_doc = original_doc.clone();
        updated_doc.set_properties(properties);
        updated_doc.set_revision(new_revision);
        updated_doc.set_created_at(None);
        updated_doc.set_updated_at(None);
        updated_doc.set_transferred_at(None);
        updated_doc.set_created_at_block_height(None);
        updated_doc.set_updated_at_block_height(None);
        updated_doc.set_transferred_at_block_height(None);
        updated_doc.set_created_at_core_block_height(None);
        updated_doc.set_updated_at_core_block_height(None);
        updated_doc.set_transferred_at_core_block_height(None);
        updated_doc.set_contract_version(None);

        Ok((updated_doc, entropy))
    }

    fn supports_field_editor(
        doc_type: &dash_sdk::dpp::data_contract::document_type::DocumentType,
    ) -> bool {
        !doc_type
            .properties()
            .values()
            .any(|schema| matches!(schema.property_type, DocumentPropertyType::TypedArray(_)))
    }

    fn can_broadcast(&self) -> bool {
        // A running fetch's result would overwrite the broadcast's progress or
        // success, and a running broadcast would be paid for twice.
        if self.pending_fetch.is_pending() || self.broadcast_status == BroadcastStatus::Broadcasting
        {
            return false;
        }
        if self.selected_type_is_index_only() {
            return self.is_index_only_delete()
                && self.index_only_entry.is_some()
                && FeatureGate::IndexOnlyDocumentDelete.is_available(&self.app_context)
                && self.index_only_delete_values().is_ok();
        }
        if matches!(
            self.action_type,
            DocumentActionType::Create | DocumentActionType::Replace
        ) && self
            .selected_document_type
            .as_ref()
            .is_some_and(|doc_type| !Self::supports_field_editor(doc_type))
        {
            return false;
        }
        match self.action_type {
            DocumentActionType::Create => !self.field_inputs.is_empty(),
            DocumentActionType::Delete => !self.document_id_input.is_empty(),
            DocumentActionType::Purchase => {
                !self.document_id_input.is_empty()
                    && self.fetched_price.is_some()
                    && (self.broadcast_status == BroadcastStatus::Fetched
                        || self.broadcast_status == BroadcastStatus::NotBroadcasted)
            }
            DocumentActionType::Replace => {
                self.original_doc.is_some()
                    && (self.broadcast_status == BroadcastStatus::Fetched
                        || self.broadcast_status == BroadcastStatus::NotBroadcasted)
            }
            DocumentActionType::SetPrice => {
                !self.document_id_input.is_empty() && !self.price_input.is_empty()
            }
            DocumentActionType::Transfer => {
                !self.document_id_input.is_empty() && !self.recipient_id_input.is_empty()
            }
        }
    }
}

impl ScreenLike for DocumentActionScreen {
    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = add_top_panel(
            ui,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                (self.action_type.display_name(), AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ui,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenDocumentQuery,
        );

        action |= island_central_panel(ui, |ui| match &self.broadcast_status {
            BroadcastStatus::Broadcasted => {
                let success_message = format!("{} successful!", self.action_type.display_name());
                let back_button = ("Back to Contracts".to_string(), AppAction::GoToMainScreen);
                let reset_button = (
                    format!("Back to {}", self.action_type.display_name()),
                    AppAction::Custom("Reset".to_string()),
                );

                let inner_action = show_success_screen_with_info(
                    ui,
                    success_message,
                    vec![back_button, reset_button],
                    None,
                );

                if inner_action == AppAction::Custom("Reset".to_string()) {
                    self.reset_screen();
                    self.completed_fee_result = None;
                }

                inner_action
            }
            _ => self.render_main_content(ui),
        });

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully
            }
        }

        action
    }

    fn refresh(&mut self) {
        // Task outcomes arrive in display_backend_task_result/_error.
    }

    // No `display_message` override: a message may come from any task, such
    // as a background scheduled-vote sweep, so it must not end a fetch or a
    // broadcast. Their own errors arrive in `display_backend_task_error`.

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        result: BackendTaskSuccessResult,
    ) {
        // Only the fetch and the broadcast this screen waits on change the
        // form: fetched documents belong to the selection that requested
        // them, and another task's result must not end a running broadcast.
        let own = if matches!(result, BackendTaskSuccessResult::Documents(_)) {
            self.pending_fetch.settle(context).is_some()
        } else {
            self.pending_broadcast.settle(context).is_some()
        };
        if own {
            self.display_task_result(result);
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        let own_fetch = self.pending_fetch.settle(context).is_some()
            && self.broadcast_status == BroadcastStatus::Fetching;
        let own_broadcast = self.pending_broadcast.settle(context).is_some()
            && self.broadcast_status == BroadcastStatus::Broadcasting;
        if own_fetch || own_broadcast {
            self.refresh_banner.take_and_clear();
            self.broadcast_status = BroadcastStatus::NotBroadcasted;
        }
    }

    fn should_suppress_backend_task_error(
        &self,
        context: &BackendTaskContext,
        _error: &TaskError,
    ) -> bool {
        context.dispatched_document_fetch() && !self.pending_fetch.owns(context)
    }

    fn display_task_result(&mut self, result: crate::ui::BackendTaskSuccessResult) {
        // Clear the progress banner on any completed task
        self.refresh_banner.take_and_clear();
        match result {
            BackendTaskSuccessResult::BroadcastedDocument(_) => {
                self.broadcast_status = BroadcastStatus::Broadcasted;
            }
            BackendTaskSuccessResult::DeletedDocument(_, fee_result)
            | BackendTaskSuccessResult::ReplacedDocument(_, fee_result)
            | BackendTaskSuccessResult::TransferredDocument(_, fee_result)
            | BackendTaskSuccessResult::PurchasedDocument(_, fee_result)
            | BackendTaskSuccessResult::SetDocumentPrice(_, fee_result) => {
                self.completed_fee_result = Some(fee_result);
                self.broadcast_status = BroadcastStatus::Broadcasted;
            }
            BackendTaskSuccessResult::Documents(documents) => {
                self.broadcast_status = BroadcastStatus::Fetched;

                match self.action_type {
                    DocumentActionType::Replace => {
                        // For replace, we need to fetch the original document and populate fields
                        if let Some((_id, Some(doc))) = documents.first() {
                            self.original_doc = Some(doc.clone());
                            // Populate field inputs with existing values
                            // Only include properties that are defined in the document type schema
                            let Some(doc_type) = self.selected_document_type.as_ref() else {
                                tracing::warn!(
                                    "Replace results arrived without a selected document type; skipping field population"
                                );
                                return;
                            };
                            let doc_type_properties = doc_type.properties();
                            self.field_inputs = doc
                                .properties()
                                .iter()
                                .filter(|(k, _)| doc_type_properties.contains_key(k.as_str()))
                                .map(|(k, v)| {
                                    let s = match v {
                                        Value::Text(text) => text.clone(),
                                        Value::Bytes(bytes) => hex::encode(bytes),
                                        Value::Bool(b) => b.to_string(),
                                        Value::U8(n) => n.to_string(),
                                        Value::U16(n) => n.to_string(),
                                        Value::U32(n) => n.to_string(),
                                        Value::U64(n) => n.to_string(),
                                        Value::U128(n) => n.to_string(),
                                        Value::I8(n) => n.to_string(),
                                        Value::I16(n) => n.to_string(),
                                        Value::I32(n) => n.to_string(),
                                        Value::I64(n) => n.to_string(),
                                        Value::I128(n) => n.to_string(),
                                        Value::Float(f) => f.to_string(),
                                        Value::Identifier(id) => match Identifier::from_bytes(id) {
                                            Ok(identifier) => {
                                                identifier.to_string(Encoding::Base58)
                                            }
                                            Err(_) => "<invalid identifier>".to_string(),
                                        },
                                        _ => v.to_string(), // fallback for arrays/objects
                                    };
                                    (k.clone(), s)
                                })
                                .collect();
                        } else {
                            self.original_doc = None;
                            self.field_inputs.clear();
                        }
                    }
                    DocumentActionType::Purchase => {
                        // For purchase, we need to extract the price
                        if let Some((_id, Some(doc))) = documents.first() {
                            match doc.properties().get_optional_integer::<Credits>(PRICE) {
                                Ok(Some(price)) => {
                                    self.fetched_price = Some(price);
                                }
                                Ok(None) => {
                                    MessageBanner::set_global(
                                        self.app_context.egui_ctx(),
                                        "Document has no price set",
                                        crate::ui::MessageType::Error,
                                    );
                                    self.fetched_price = None;
                                }
                                Err(_) => {
                                    MessageBanner::set_global(
                                        self.app_context.egui_ctx(),
                                        "Failed to get document price",
                                        crate::ui::MessageType::Error,
                                    );
                                    self.fetched_price = None;
                                }
                            }
                        } else {
                            MessageBanner::set_global(
                                self.app_context.egui_ctx(),
                                "No document found",
                                crate::ui::MessageType::Error,
                            );
                            self.fetched_price = None;
                        }
                    }
                    DocumentActionType::Delete => {
                        // For delete, store the fetched documents
                        self.no_documents_found = documents.is_empty();
                        self.fetched_documents = documents;
                    }
                    _ => {}
                }
            }
            _ => {
                // Nothing
            }
        }
    }
}

impl DocumentActionScreen {
    fn render_main_content(&mut self, ui: &mut Ui) -> AppAction {
        egui::ScrollArea::vertical()
            .show(ui, |ui| {
                let mut action = AppAction::None;

                // Step 1: Contract and Document Type Selection
                self.render_contract_and_type_selection(ui);

                if self.selected_contract.is_none() || self.selected_document_type.is_none() {
                    return action;
                }

                ui.separator();
                ui.add_space(10.0);

                // Step 2: Identity and Key Selection
                self.render_identity_and_key_selection(ui);

                if self.selected_identity.is_none() {
                    return action;
                }
                if self.selected_key.is_none() {
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.label(
                        RichText::new(NO_SIGNING_KEY_NOTICE)
                            .color(DashColors::warning_color(dark_mode)),
                    );
                    return action;
                }

                ui.separator();
                ui.add_space(10.0);

                // Wallet unlock
                if self.selected_identity.is_some() {
                    let new_wallet = self.signing_wallet();
                    let wallet_changed = match (&self.wallet, &new_wallet) {
                        (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
                        (None, None) => false,
                        _ => true,
                    };
                    if wallet_changed {
                        self.wallet_open_attempted = false;
                    }
                    self.wallet = new_wallet;
                }
                if let Some(wallet) = &self.wallet {
                    if !self.wallet_open_attempted {
                        if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                            let handle = MessageBanner::set_global(
                                self.app_context.egui_ctx(),
                                "Unable to open wallet. Please unlock it and try again.",
                                crate::ui::MessageType::Error,
                            );
                            handle.disable_auto_dismiss();
                            handle.with_details(e);
                        }
                        self.wallet_open_attempted = true;
                    }
                    if wallet_needs_unlock(wallet) {
                        ui.add_space(10.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(200, 150, 50),
                            "Wallet is locked. Please unlock to continue.",
                        );
                        ui.add_space(8.0);
                        if ui.button("Unlock Wallet").clicked() {
                            self.wallet_unlock_popup.open();
                        }
                        return action;
                    }
                }

                // Step 3: Action-specific inputs and broadcast
                action |= match self.action_type {
                    DocumentActionType::Create => self.render_create_inputs(ui),
                    _ => self.render_action_specific_inputs(ui),
                };

                action
            })
            .inner
    }
}

/// The key to sign a document action with when the user has not picked one:
/// a live AUTHENTICATION key this device can sign with, at a security level
/// the document type accepts, within the contract and document type's scope.
/// Keys are ranked in upstream platform-wallet's order, as for every document
/// key ([`QualifiedIdentity::signing_key_now`]).
fn auto_selected_document_key(
    identity: &QualifiedIdentity,
    contract: Option<&QualifiedContract>,
    document_type: Option<&DocumentType>,
) -> Option<IdentityPublicKey> {
    // The levels the document type accepts, so the preference
    // for unlimited keys only ever chooses among keys Platform accepts.
    let levels = crate::ui::helpers::compute_allowed_security_levels(
        TransactionType::DocumentAction,
        document_type,
    );
    identity
        .signing_key_now(KeyRequirements::new(
            dash_sdk::dpp::identity::Purpose::AUTHENTICATION,
            &levels,
            signing_scope(contract, document_type),
        ))
        .cloned()
}

/// The contract-bounds scope of a document action on `document_type` of
/// `contract`. Before a contract is chosen nothing is known about the target,
/// so only keys that can sign anywhere (unbound keys) qualify.
fn signing_scope<'a>(
    contract: Option<&QualifiedContract>,
    document_type: Option<&'a DocumentType>,
) -> SigningScope<'a> {
    match (contract, document_type) {
        (Some(contract), Some(document_type)) => SigningScope::Document {
            contract_id: contract.contract.id(),
            document_type_name: document_type.name().as_str(),
        },
        (Some(contract), None) => SigningScope::ContractWide {
            contract_id: contract.contract.id(),
        },
        (None, _) => SigningScope::NonBatch,
    }
}

/// The contract fee as a label value: the total, and its split when both
/// the owner and the moderators receive a part.
fn action_fee_summary(quote: &DocumentActionFeeQuote) -> String {
    let total = format_credits_as_dash(quote.total);
    match (quote.owner, quote.moderators) {
        (_, 0) => format!("{total} to the contract owner"),
        (0, _) => format!("{total} to the contract's moderators"),
        (owner, moderators) => format!(
            "{total} ({owner} to the contract owner, {moderators} to its moderators)",
            owner = format_credits_as_dash(owner),
            moderators = format_credits_as_dash(moderators)
        ),
    }
}

/// The confirmation text for a contract fee, in complete sentences: who
/// receives what, whether the fee follows network fees (and the most it can
/// then reach), and that it comes on top of the network fee. The pricing is
/// read from the agreement itself, so a fee too small to show the tolerance
/// after rounding is still described as following network fees.
/// The title and text of the confirmation shown before a document action
/// that charges a contract fee (`quote`) or deletes an indexOnly entry.
fn confirmation_title_and_text(
    quote: Option<&DocumentActionFeeQuote>,
    index_only_delete: bool,
) -> (&'static str, String) {
    match (quote, index_only_delete) {
        (Some(quote), true) => (
            "Confirm contract fee",
            format!(
                "{}\n\n{INDEX_ONLY_DELETE_WARNING}",
                action_fee_confirmation_text(quote)
            ),
        ),
        (Some(quote), false) => ("Confirm contract fee", action_fee_confirmation_text(quote)),
        (None, _) => ("Delete this entry?", INDEX_ONLY_DELETE_WARNING.to_string()),
    }
}

fn action_fee_confirmation_text(quote: &DocumentActionFeeQuote) -> String {
    let fee = format_credits_as_dash(quote.total);
    let recipients = match (quote.owner, quote.moderators) {
        (_, 0) => format!(
            "This contract charges a fee of {fee} for this action, paid to the contract owner."
        ),
        (0, _) => format!(
            "This contract charges a fee of {fee} for this action, paid to the contract's moderators."
        ),
        (owner, moderators) => format!(
            "This contract charges a fee of {fee} for this action: {owner} to the contract owner and {moderators} to its moderators.",
            owner = format_credits_as_dash(owner),
            moderators = format_credits_as_dash(moderators)
        ),
    };
    let pricing = if quote.agreement.fee_multiplier().is_some() {
        format!(
            "The fee follows network fees. If they rise before the action is processed, up to {max} can be charged. If they rise by more than {percent}%, the network refuses the action.",
            max = format_credits_as_dash(quote.max_total),
            percent = ACTION_FEE_MULTIPLIER_INCREASE_TOLERANCE_PERCENT
        )
    } else {
        "The fee is fixed and does not change with network fees.".to_string()
    };
    format!(
        "{recipients} {pricing} It is charged on top of the network fee. Do you want to continue?"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::data_contract::document_type::action_fees::agreement::AgreedFeeMultiplier;
    use dash_sdk::dpp::data_contract::document_type::action_fees::{
        ActionFeePricing, DocumentActionFee,
    };

    fn editor_document_type(array: bool) -> DocumentType {
        use dash_sdk::dpp::data_contract::config::DataContractConfig;
        use dash_sdk::dpp::platform_value::platform_value;
        use dash_sdk::dpp::version::PlatformVersion;

        let version = PlatformVersion::get(14).expect("protocol 14");
        let config = DataContractConfig::default_for_version(version).unwrap();
        let property = if array {
            platform_value!({"type": "array", "items": {"type": "string", "maxLength": 20_u32}, "maxItems": 4_u32, "position": 0_u32})
        } else {
            platform_value!({"type": "string", "maxLength": 20_u32, "position": 0_u32})
        };
        DocumentType::try_from_schema(
            Identifier::new([1; 32]), 1, config.version(), "entry",
            platform_value!({"type": "object", "properties": {"value": property}, "additionalProperties": false}),
            None, &BTreeMap::new(), &config, false, &mut Vec::new(), version,
        ).unwrap()
    }

    use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;

    use crate::context::test_support::identity_holding;

    /// An AUTHENTICATION key at `level`, optionally bound to `bounds`.
    fn auth_key(
        id: u32,
        level: dash_sdk::dpp::identity::SecurityLevel,
        bounds: Option<
            dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds,
        >,
    ) -> IdentityPublicKey {
        let key = crate::context::test_support::auth_key(id, level);
        match bounds {
            Some(bounds) => crate::context::test_support::bound_to(key, bounds),
            None => key,
        }
    }

    /// The DPNS contract, whose document types require a HIGH key.
    fn dpns() -> QualifiedContract {
        use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
        use dash_sdk::dpp::version::PlatformVersion;
        QualifiedContract {
            contract: load_system_data_contract(
                SystemDataContract::DPNS,
                PlatformVersion::latest(),
            )
            .expect("the DPNS contract"),
            alias: None,
        }
    }

    fn owned_document_type(contract: &QualifiedContract, name: &str) -> DocumentType {
        contract
            .contract
            .document_type_for_name(name)
            .expect("document type")
            .to_owned_document_type()
    }

    /// Regression: automatic selection admitted every non-master level and
    /// preferred unlimited keys, so an unlimited MEDIUM key beat a limited key
    /// at the level the document type requires, and Platform refused it.
    #[test]
    fn auto_selection_honours_the_document_types_security_level() {
        use dash_sdk::dpp::identity::SecurityLevel;
        let contract = dpns();
        let domain = owned_document_type(&contract, "domain");
        let required = domain.security_level_requirement();
        assert_ne!(
            required,
            SecurityLevel::MEDIUM,
            "the test needs a stricter type"
        );

        let limited = auth_key(1, required, None).with_limits(Some(5_000), None);
        let weaker_unlimited = auth_key(2, SecurityLevel::MEDIUM, None);
        let identity = identity_holding(&[limited, weaker_unlimited]);

        let chosen = auto_selected_document_key(&identity, Some(&contract), Some(&domain));
        assert_eq!(chosen.map(|key| key.id()), Some(1));
    }

    /// Automatic selection follows upstream platform-wallet's order: an
    /// unlimited key at any accepted level beats a limited key, so a budgeted
    /// HIGH key is not spent while an unlimited CRITICAL key is held.
    #[test]
    fn auto_selection_prefers_an_unlimited_key_at_any_accepted_level() {
        use dash_sdk::dpp::identity::SecurityLevel;
        let contract = dpns();
        let domain = owned_document_type(&contract, "domain");
        let required = domain.security_level_requirement();
        assert_eq!(required, SecurityLevel::HIGH, "precondition: a HIGH type");
        let identity = identity_holding(&[
            auth_key(1, SecurityLevel::CRITICAL, None),
            auth_key(2, SecurityLevel::HIGH, None).with_limits(Some(5_000), None),
        ]);

        let chosen = auto_selected_document_key(&identity, Some(&contract), Some(&domain));
        assert_eq!(chosen.map(|key| key.id()), Some(1));
    }

    /// Regression: the unlock prompt looked for a DPNS signing key, so an
    /// identity whose only key is bound to the action's contract got no
    /// wallet to unlock although that key signs the action.
    #[test]
    fn the_unlock_wallet_follows_the_key_that_signs() {
        use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let contract = QualifiedContract {
            contract: crate::context::test_support::dashpay_contract(),
            alias: None,
        };
        let (identity, wallet) = crate::context::test_support::identity_with_bound_wallet_key(
            ContractBounds::SingleContract {
                id: contract.contract.id(),
            },
        );

        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Create);
        screen.selected_document_type = Some(owned_document_type(&contract, "contactInfo"));
        screen.selected_contract = Some(contract);
        screen.selected_identity = Some(identity);
        screen.signing_scope_changed();
        assert_eq!(screen.selected_key.as_ref().map(|key| key.id()), Some(1));

        let resolved = screen.signing_wallet();
        assert!(resolved.is_some_and(|resolved| Arc::ptr_eq(&resolved, &wallet)));
    }

    /// Regression: a key picked for one document type survived a switch to a
    /// type it is not bound to, and the action was signed with it anyway.
    #[test]
    fn changing_the_document_type_picks_the_key_again() {
        use dash_sdk::dpp::identity::identity_public_key::contract_bounds::ContractBounds;
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let contract = dpns();
        let domain = owned_document_type(&contract, "domain");
        let preorder = owned_document_type(&contract, "preorder");
        let domain_only = auth_key(
            1,
            domain.security_level_requirement(),
            Some(ContractBounds::SingleContractDocumentType {
                id: contract.contract.id(),
                document_type_name: "domain".to_string(),
            }),
        );
        let identity = identity_holding(&[domain_only]);

        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Create);
        screen.selected_contract = Some(contract.clone());
        screen.selected_document_type = Some(domain.clone());
        screen.selected_key = auto_selected_document_key(&identity, Some(&contract), Some(&domain));
        screen.selected_identity = Some(identity);
        assert_eq!(screen.selected_key.as_ref().map(|key| key.id()), Some(1));

        screen.selected_document_type = Some(preorder);
        screen.signing_scope_changed();
        assert_eq!(
            screen.selected_key, None,
            "a key bound to domain cannot sign a preorder"
        );
    }

    /// The chooser keeps the document type when the contract changes. A type
    /// from another contract must not be built: the document id would commit
    /// to that contract while the transition targets the selected one.
    #[test]
    fn creating_refuses_a_document_type_from_another_contract() {
        use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
        use dash_sdk::dpp::version::PlatformVersion;

        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let network = context.network;
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Create);
        let dpns = load_system_data_contract(SystemDataContract::DPNS, PlatformVersion::latest())
            .expect("dpns contract");
        screen.selected_contract = Some(QualifiedContract {
            contract: dpns,
            alias: None,
        });
        // A type of contract [1; 32], not of the selected DPNS contract.
        screen.selected_document_type = Some(editor_document_type(false));
        screen.selected_identity = Some(
            crate::context::test_support::user_identity_with_signing_key(
                Identifier::new([9; 32]),
                network,
                false,
            ),
        );
        screen.field_inputs.insert("value".into(), "text".into());

        assert_eq!(
            screen.try_build_document().unwrap_err(),
            DocumentBuildError::ForeignDocumentType
        );
    }

    /// The create banner names the leftover document type as the problem,
    /// not the fields.
    #[test]
    fn the_create_banner_explains_a_document_type_from_another_contract() {
        use crate::ui::components::message_banner::global_banner_texts;
        use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
        use dash_sdk::dpp::version::PlatformVersion;

        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let network = context.network;
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Create);
        screen.selected_contract = Some(QualifiedContract {
            contract: load_system_data_contract(
                SystemDataContract::DPNS,
                PlatformVersion::latest(),
            )
            .expect("dpns contract"),
            alias: None,
        });
        screen.selected_document_type = Some(editor_document_type(false));
        screen.selected_identity = Some(
            crate::context::test_support::user_identity_with_signing_key(
                Identifier::new([9; 32]),
                network,
                false,
            ),
        );

        assert!(matches!(screen.create_document_task(), BackendTask::None));
        assert_eq!(
            global_banner_texts(screen.app_context.egui_ctx()),
            vec![FOREIGN_DOCUMENT_TYPE_MESSAGE.to_string()]
        );
    }

    #[test]
    fn replacing_an_optional_typed_array_does_not_drop_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Replace);
        screen.selected_document_type = Some(editor_document_type(true));
        screen.original_doc = Some(
            DocumentV0 {
                properties: BTreeMap::from([(
                    "value".to_string(),
                    Value::Array(vec![Value::Text("keep".into())]),
                )]),
                ..Default::default()
            }
            .into(),
        );
        screen.broadcast_status = BroadcastStatus::Fetched;
        assert!(!screen.can_broadcast());
        assert!(screen.try_build_document_from_original().is_err());
        assert!(matches!(screen.create_replace_task(), BackendTask::None));
    }

    fn fetch_query() -> DocumentQuery {
        use dash_sdk::dpp::data_contracts::SystemDataContract;
        use dash_sdk::dpp::system_data_contracts::load_system_data_contract;
        use dash_sdk::dpp::version::PlatformVersion;

        let contract =
            load_system_data_contract(SystemDataContract::TokenHistory, PlatformVersion::latest())
                .expect("token history contract");
        DocumentQuery::new(Arc::new(contract), "claim").expect("claim query")
    }

    fn fetched(price: Credits) -> BackendTaskSuccessResult {
        let document: Document = DocumentV0 {
            id: Identifier::new([9; 32]),
            properties: BTreeMap::from([(PRICE.to_string(), Value::U64(price))]),
            ..Default::default()
        }
        .into();
        BackendTaskSuccessResult::Documents(IndexMap::from([(document.id(), Some(document))]))
    }

    fn dispatched_context(action: AppAction) -> BackendTaskContext {
        let AppAction::BackendTaskWithContext { context, .. } = action else {
            panic!("fetch must be dispatched with its own context");
        };
        context
    }

    /// Changing the contract or document type drops everything fetched for
    /// the previous selection, so nothing from it can be sent.
    #[test]
    fn a_selection_change_discards_fetched_state() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Purchase);
        screen.document_id_input = Identifier::new([9; 32]).to_string(Encoding::Base58);
        screen.fetched_price = Some(5);
        screen.original_doc = Some(DocumentV0::default().into());
        screen.field_inputs.insert("value".into(), "old".into());
        screen
            .fetched_documents
            .insert(Identifier::new([9; 32]), None);
        screen.no_documents_found = true;
        screen.broadcast_status = BroadcastStatus::Fetched;
        assert!(screen.can_broadcast());

        screen.clear_selection_state();

        assert_eq!(screen.fetched_price, None);
        assert!(screen.original_doc.is_none());
        assert!(screen.field_inputs.is_empty());
        assert!(screen.fetched_documents.is_empty());
        assert!(!screen.no_documents_found);
        assert!(screen.broadcast_status == BroadcastStatus::NotBroadcasted);
        assert!(!screen.can_broadcast());
    }

    /// A fetch result that this screen is not waiting for never fills in
    /// the form.
    #[test]
    fn an_uncorrelated_fetch_result_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Purchase);

        screen.display_backend_task_result(&BackendTaskContext::Unknown, fetched(7));

        assert_eq!(screen.fetched_price, None);
        assert!(screen.broadcast_status == BroadcastStatus::NotBroadcasted);
    }

    /// A fetch that finishes after the selection changed is dropped; the
    /// fetch for the current selection is applied.
    #[test]
    fn a_late_fetch_result_for_an_old_selection_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Purchase);

        let old = dispatched_context(screen.dispatch_fetch(fetch_query()));
        assert!(screen.broadcast_status == BroadcastStatus::Fetching);
        screen.clear_selection_state();
        assert!(screen.broadcast_status == BroadcastStatus::NotBroadcasted);
        let current = dispatched_context(screen.dispatch_fetch(fetch_query()));

        screen.display_backend_task_result(&old, fetched(7));
        assert_eq!(screen.fetched_price, None);
        assert!(screen.broadcast_status == BroadcastStatus::Fetching);

        screen.display_backend_task_result(&current, fetched(11));
        assert_eq!(screen.fetched_price, Some(11));
        assert!(screen.broadcast_status == BroadcastStatus::Fetched);
    }

    /// An error from a fetch the screen gave up on stays off the banner; the
    /// current fetch's error still shows and ends the wait.
    #[test]
    fn only_the_current_fetch_error_reaches_the_banner() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Replace);
        let error = TaskError::WalletStorageNotReady;

        let old = dispatched_context(screen.dispatch_fetch(fetch_query()));
        screen.clear_selection_state();
        let current = dispatched_context(screen.dispatch_fetch(fetch_query()));

        assert!(screen.should_suppress_backend_task_error(&old, &error));
        assert!(!screen.should_suppress_backend_task_error(&current, &error));

        screen.display_backend_task_error(&current, &error);
        assert!(screen.broadcast_status == BroadcastStatus::NotBroadcasted);
        assert!(!screen.pending_fetch.is_pending());
    }

    /// A broadcast started while a fetch was still running keeps its status
    /// and its progress banner when the selection changes.
    #[test]
    fn a_selection_change_leaves_a_running_broadcast_alone() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let ctx = context.egui_ctx().clone();
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Create);

        let _ = screen.dispatch_fetch(fetch_query());
        screen.set_fetching_banner(&ctx, "Broadcasting...");
        screen.broadcast_status = BroadcastStatus::Broadcasting;

        screen.clear_selection_state();

        assert!(screen.broadcast_status == BroadcastStatus::Broadcasting);
        assert!(screen.refresh_banner.is_some());
        assert!(!screen.pending_fetch.is_pending());
    }

    /// Nothing is sent while a fetch is running, so its result cannot
    /// overwrite a broadcast's progress or success; once it lands, the action
    /// can be sent.
    #[test]
    fn a_running_fetch_blocks_broadcast() {
        for action_type in [
            DocumentActionType::Delete,
            DocumentActionType::SetPrice,
            DocumentActionType::Transfer,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let context = crate::context::test_support::test_app_context(dir.path());
            let mut screen = DocumentActionScreen::new(context, None, action_type.clone());
            screen.document_id_input = Identifier::new([9; 32]).to_string(Encoding::Base58);
            screen.price_input = "1".into();
            screen.recipient_id_input = Identifier::new([8; 32]).to_string(Encoding::Base58);
            assert!(screen.can_broadcast(), "{action_type:?} is ready to send");

            let fetch = dispatched_context(screen.dispatch_fetch(fetch_query()));
            assert!(
                !screen.can_broadcast(),
                "{action_type:?} waits for the fetch"
            );

            screen.display_backend_task_result(&fetch, fetched(7));
            assert!(
                screen.can_broadcast(),
                "{action_type:?} can be sent after the fetch"
            );
        }
    }

    /// Nothing more is sent while a broadcast is running, so one click cannot
    /// pay for the same document action twice.
    #[test]
    fn a_running_broadcast_blocks_another_broadcast() {
        for action_type in [
            DocumentActionType::Create,
            DocumentActionType::Delete,
            DocumentActionType::SetPrice,
            DocumentActionType::Transfer,
            DocumentActionType::Purchase,
            DocumentActionType::Replace,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let context = crate::context::test_support::test_app_context(dir.path());
            let mut screen = DocumentActionScreen::new(context, None, action_type.clone());
            screen.field_inputs.insert("value".into(), "text".into());
            screen.document_id_input = Identifier::new([9; 32]).to_string(Encoding::Base58);
            screen.price_input = "1".into();
            screen.recipient_id_input = Identifier::new([8; 32]).to_string(Encoding::Base58);
            screen.fetched_price = Some(7);
            screen.original_doc = Some(DocumentV0::default().into());
            assert!(screen.can_broadcast(), "{action_type:?} is ready to send");

            screen.broadcast_status = BroadcastStatus::Broadcasting;
            assert!(
                !screen.can_broadcast(),
                "{action_type:?} waits for the running broadcast"
            );
        }
    }

    fn deleted() -> BackendTaskSuccessResult {
        BackendTaskSuccessResult::DeletedDocument(
            Identifier::new([9; 32]),
            FeeResult {
                estimated_fee: 1,
                actual_fee: Some(1),
            },
        )
    }

    /// A Delete form ready to send, with its broadcast running; returns the
    /// screen and the broadcast's own dispatch context.
    fn screen_broadcasting(dir: &std::path::Path) -> (DocumentActionScreen, BackendTaskContext) {
        let context = crate::context::test_support::test_app_context(dir);
        let ctx = context.egui_ctx().clone();
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Delete);
        screen.document_id_input = Identifier::new([9; 32]).to_string(Encoding::Base58);
        assert!(screen.can_broadcast());
        let own = dispatched_context(screen.start_broadcast(&ctx, BackendTask::None));
        assert!(!screen.can_broadcast());
        (screen, own)
    }

    /// Another task's error or message, such as a background scheduled-vote
    /// failure, leaves a running broadcast in place, so the paid action cannot
    /// be sent again; only the broadcast's own error ends it.
    #[test]
    fn only_its_own_error_ends_a_running_broadcast() {
        let dir = tempfile::tempdir().unwrap();
        let (mut screen, own) = screen_broadcasting(dir.path());
        let foreign = BackendTaskContext::for_dispatch(&BackendTask::None);
        let error = TaskError::WalletStorageNotReady;

        screen.display_backend_task_error(&foreign, &error);
        screen.display_backend_task_error(&BackendTaskContext::Unknown, &error);
        screen.display_message("A scheduled vote failed.", crate::ui::MessageType::Error);
        screen.display_message("Theme updated.", crate::ui::MessageType::Success);
        screen.display_backend_task_result(&foreign, deleted());
        screen.display_backend_task_result(&BackendTaskContext::Unknown, deleted());

        assert!(screen.broadcast_status == BroadcastStatus::Broadcasting);
        assert!(screen.refresh_banner.is_some(), "progress stays shown");
        assert!(screen.completed_fee_result.is_none());
        assert!(!screen.can_broadcast(), "the running broadcast blocks");

        screen.display_backend_task_error(&own, &error);

        assert!(screen.broadcast_status == BroadcastStatus::NotBroadcasted);
        assert!(screen.refresh_banner.is_none());
        assert!(screen.can_broadcast(), "a failed broadcast can be retried");
    }

    /// The broadcast's own result ends it as sent; a later error of the same
    /// dispatch, or another task's message, does not reopen the form.
    #[test]
    fn its_own_result_ends_a_running_broadcast() {
        let dir = tempfile::tempdir().unwrap();
        let (mut screen, own) = screen_broadcasting(dir.path());

        screen.display_backend_task_result(&own, deleted());

        assert!(screen.broadcast_status == BroadcastStatus::Broadcasted);
        assert!(screen.refresh_banner.is_none());
        assert!(screen.completed_fee_result.is_some());

        screen.display_backend_task_error(&own, &TaskError::WalletStorageNotReady);
        screen.display_message("A scheduled vote failed.", crate::ui::MessageType::Error);
        assert!(screen.broadcast_status == BroadcastStatus::Broadcasted);
    }

    /// A new fetch drops the document fetched before it. When the new fetch
    /// fails, the previous document's price or content cannot be sent.
    #[test]
    fn a_new_fetch_drops_the_previously_fetched_document() {
        for action_type in [DocumentActionType::Purchase, DocumentActionType::Replace] {
            let dir = tempfile::tempdir().unwrap();
            let context = crate::context::test_support::test_app_context(dir.path());
            let mut screen = DocumentActionScreen::new(context, None, action_type.clone());
            screen.document_id_input = Identifier::new([9; 32]).to_string(Encoding::Base58);
            let first = dispatched_context(screen.dispatch_fetch(fetch_query()));
            screen.display_backend_task_result(&first, fetched(7));
            assert!(screen.can_broadcast(), "{action_type:?} sends the fetch");

            screen.document_id_input = Identifier::new([10; 32]).to_string(Encoding::Base58);
            let second = dispatched_context(screen.dispatch_fetch(fetch_query()));
            assert_eq!(screen.fetched_price, None, "{action_type:?}");
            assert!(screen.original_doc.is_none(), "{action_type:?}");
            assert!(screen.field_inputs.is_empty(), "{action_type:?}");

            screen.display_backend_task_error(&second, &TaskError::WalletStorageNotReady);
            screen.display_message("failed", crate::ui::MessageType::Error);
            assert!(
                !screen.can_broadcast(),
                "{action_type:?} cannot send the previous document"
            );
        }
    }

    /// Editing the document ID drops the document fetched for the old ID and
    /// any fetch still running for it, so the old document's price or content
    /// cannot be sent under the new ID.
    #[test]
    fn editing_the_document_id_drops_the_fetched_document() {
        use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
        use dash_sdk::dpp::version::PlatformVersion;
        use egui_kittest::{Harness, kittest::Queryable};

        let withdrawals =
            load_system_data_contract(SystemDataContract::Withdrawals, PlatformVersion::latest())
                .expect("withdrawals contract");
        let fetched_id = Identifier::new([9; 32]).to_string(Encoding::Base58);
        for action_type in [DocumentActionType::Purchase, DocumentActionType::Replace] {
            for fetch_running in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let context = crate::context::test_support::test_app_context(dir.path());
                let mut screen = DocumentActionScreen::new(context, None, action_type.clone());
                screen.selected_document_type =
                    withdrawals.document_type_cloned_for_name("withdrawal").ok();
                screen.selected_contract = Some(QualifiedContract {
                    contract: withdrawals.clone(),
                    alias: None,
                });
                screen.selected_identity = Some(named_identity(1, "alice"));
                screen.document_id_input = fetched_id.clone();
                screen.fetched_price = Some(7);
                screen.original_doc = Some(DocumentV0::default().into());
                screen.field_inputs.insert("amount".into(), "5".into());
                screen.broadcast_status = BroadcastStatus::Fetched;
                if fetch_running {
                    let task = BackendTask::DocumentTask(Box::new(DocumentTask::FetchDocuments(
                        fetch_query(),
                    )));
                    let _ = screen.pending_fetch.dispatch_with(task, ());
                    screen.broadcast_status = BroadcastStatus::Fetching;
                }

                let mut harness = Harness::builder()
                    .with_size(egui::vec2(1200.0, 800.0))
                    .build_ui_state(
                        |ui, screen: &mut DocumentActionScreen| {
                            let _ = screen.render_action_specific_inputs(ui);
                        },
                        screen,
                    );
                // A running fetch shows a spinner, which repaints forever.
                harness.run_steps(3);
                let id = fetched_id.clone();
                let id_input = move |node: &egui_kittest::kittest::AccessKitNode<'_>| {
                    node.role() == egui::accesskit::Role::TextInput
                        && node.value().as_deref() == Some(id.as_str())
                };
                harness.get_by(id_input.clone()).focus();
                harness.run_steps(3);
                harness.get_by(id_input).type_text("x");
                harness.run_steps(3);

                let screen = harness.state();
                let case = format!("{action_type:?}, fetch running: {fetch_running}");
                assert_eq!(screen.document_id_input, format!("{fetched_id}x"), "{case}");
                assert_eq!(screen.fetched_price, None, "{case}");
                assert!(screen.original_doc.is_none(), "{case}");
                assert!(screen.field_inputs.is_empty(), "{case}");
                assert!(!screen.pending_fetch.is_pending(), "{case}");
                assert!(
                    screen.broadcast_status == BroadcastStatus::NotBroadcasted,
                    "{case}"
                );
                assert!(!screen.can_broadcast(), "{case}");
            }
        }
    }

    /// No fetch can start while a broadcast is running, so a fetch result
    /// cannot clear the broadcast's progress or replace its success.
    #[test]
    fn a_running_broadcast_disables_fetching() {
        use dash_sdk::dpp::system_data_contracts::{SystemDataContract, load_system_data_contract};
        use dash_sdk::dpp::version::PlatformVersion;
        use egui_kittest::{
            Harness,
            kittest::{NodeT, Queryable},
        };

        // Its "withdrawal" type has an index on $ownerId, which Delete needs.
        let withdrawals =
            load_system_data_contract(SystemDataContract::Withdrawals, PlatformVersion::latest())
                .expect("withdrawals contract");
        for (action_type, label) in [
            (DocumentActionType::Delete, "Fetch Owned Documents"),
            (DocumentActionType::Purchase, "Fetch Document Price"),
            (DocumentActionType::Replace, "Fetch"),
        ] {
            for (status, disabled) in [
                (BroadcastStatus::NotBroadcasted, false),
                (BroadcastStatus::Broadcasting, true),
            ] {
                let dir = tempfile::tempdir().unwrap();
                let context = crate::context::test_support::test_app_context(dir.path());
                let mut screen = DocumentActionScreen::new(context, None, action_type.clone());
                screen.selected_document_type =
                    withdrawals.document_type_cloned_for_name("withdrawal").ok();
                screen.selected_contract = Some(QualifiedContract {
                    contract: withdrawals.clone(),
                    alias: None,
                });
                screen.selected_identity = Some(named_identity(1, "alice"));
                screen.document_id_input = Identifier::new([9; 32]).to_string(Encoding::Base58);
                screen.broadcast_status = status;

                let mut harness = Harness::builder()
                    .with_size(egui::vec2(1200.0, 800.0))
                    .build_ui_state(
                        |ui, screen: &mut DocumentActionScreen| {
                            let _ = screen.render_action_specific_inputs(ui);
                        },
                        screen,
                    );
                // A running broadcast shows a spinner, which repaints forever.
                harness.run_steps(3);

                assert_eq!(
                    harness.get_by_label(label).accesskit_node().is_disabled(),
                    disabled,
                    "{action_type:?}: \"{label}\" disabled while broadcasting"
                );
            }
        }
    }

    /// A network switch starts the form over on the new network: no fetch or
    /// broadcast from the old one is still awaited, so neither stays disabled.
    #[test]
    fn a_network_switch_drops_fetches_and_broadcasts_in_flight() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let ctx = context.egui_ctx().clone();
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Delete);
        let _ = screen.dispatch_fetch(fetch_query());
        screen.set_fetching_banner(&ctx, "Broadcasting...");
        screen.broadcast_status = BroadcastStatus::Broadcasting;
        screen.document_id_input = Identifier::new([9; 32]).to_string(Encoding::Base58);

        let other_dir = tempfile::tempdir().unwrap();
        let other = crate::context::test_support::test_app_context_for_network(
            other_dir.path(),
            dash_sdk::dpp::dashcore::Network::Devnet,
        );
        let mut wrapped = crate::ui::Screen::DocumentActionScreen(screen);
        wrapped.change_context(other.clone());
        let crate::ui::Screen::DocumentActionScreen(screen) = wrapped else {
            unreachable!("change_context keeps the screen variant");
        };

        assert!(Arc::ptr_eq(&screen.app_context, &other));
        assert!(screen.action_type == DocumentActionType::Delete);
        assert!(!screen.pending_fetch.is_pending());
        assert!(screen.broadcast_status == BroadcastStatus::NotBroadcasted);
        assert!(screen.refresh_banner.is_none());
        assert!(screen.document_id_input.is_empty());
    }

    fn named_identity(byte: u8, name: &str) -> QualifiedIdentity {
        use crate::model::qualified_identity::{IdentityStatus, IdentityType};
        use dash_sdk::dpp::identity::Identity;
        use dash_sdk::dpp::version::PlatformVersion;

        QualifiedIdentity {
            identity: Identity::create_basic_identity(
                Identifier::new([byte; 32]),
                PlatformVersion::latest(),
            )
            .expect("basic identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: vec![crate::model::qualified_identity::DPNSNameInfo {
                name: name.to_string(),
                acquired_at: 0,
            }],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: dash_sdk::dpp::dashcore::Network::Testnet,
        }
    }

    /// Picking another document type in the chooser drops the running fetch
    /// and everything fetched for the old type.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn picking_another_document_type_abandons_the_fetch() {
        use egui_kittest::{Harness, kittest::Queryable};

        let dir = tempfile::tempdir().unwrap();
        // The contract list needs the wallet backend.
        let context = crate::context::test_support::wired_test_app_context(dir.path()).await;
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Replace);
        let dpns = screen
            .selected_contract
            .clone()
            .expect("DPNS is listed first");
        screen.selected_document_type = dpns.contract.document_type_cloned_for_name("domain").ok();
        let _ = screen.dispatch_fetch(fetch_query());
        screen.original_doc = Some(DocumentV0::default().into());
        screen.field_inputs.insert("label".into(), "old".into());

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_ui_state(
                |ui, screen: &mut DocumentActionScreen| {
                    screen.render_contract_and_type_selection(ui)
                },
                screen,
            );
        harness.run();
        assert!(harness.state().pending_fetch.is_pending());

        harness.get_by_value("domain").click();
        harness.run();
        harness.get_by_label("preorder").click();
        harness.run();

        let screen = harness.state();
        assert_eq!(
            screen
                .selected_document_type
                .as_ref()
                .map(|t| t.name().as_str()),
            Some("preorder")
        );
        assert!(!screen.pending_fetch.is_pending());
        assert!(screen.original_doc.is_none());
        assert!(screen.field_inputs.is_empty());
        assert!(screen.broadcast_status == BroadcastStatus::NotBroadcasted);
    }

    /// Picking another identity drops the running fetch, so the previous
    /// identity's owned documents cannot fill in the list.
    #[test]
    fn picking_another_identity_abandons_the_fetch() {
        use egui_kittest::{Harness, kittest::Queryable};

        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Delete);
        let alice = named_identity(1, "alice");
        let bob = named_identity(2, "bob");
        screen.identities_map =
            HashMap::from([(alice.identity.id(), alice), (bob.identity.id(), bob)]);

        let mut harness = Harness::builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_ui_state(
                |ui, screen: &mut DocumentActionScreen| {
                    screen.render_identity_and_key_selection(ui)
                },
                screen,
            );
        harness.run();
        let alice_id = Identifier::new([1; 32]);
        let bob_id = Identifier::new([2; 32]);
        let (current, other, other_id) = match harness.state().selected_identity.as_ref() {
            Some(selected) if selected.identity.id() == alice_id => ("alice", "bob", bob_id),
            Some(_) => ("bob", "alice", alice_id),
            None => panic!("the selector picks an identity on the first frame"),
        };
        let _ = harness.state_mut().dispatch_fetch(fetch_query());
        assert!(harness.state().pending_fetch.is_pending());

        harness.get_by_value(current).click();
        harness.run();
        harness.get_by_label_contains(other).click();
        harness.run();

        let screen = harness.state();
        assert_eq!(
            screen.selected_identity.as_ref().map(|qi| qi.identity.id()),
            Some(other_id)
        );
        assert!(!screen.pending_fetch.is_pending());
        assert!(screen.broadcast_status == BroadcastStatus::NotBroadcasted);
    }

    /// DOC-REPLACE-001: the replacement keeps the original's id, owner,
    /// creator and moderation, takes the edited fields and the next revision,
    /// and drops every timestamp, height and the contract version.
    #[test]
    fn replacing_document_keeps_identity_and_drops_platform_set_fields() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Replace);
        screen.selected_document_type = Some(editor_document_type(false));
        let original = DocumentV0 {
            id: Identifier::new([1; 32]),
            owner_id: Identifier::new([2; 32]),
            creator_id: Some(Identifier::new([3; 32])),
            properties: BTreeMap::from([("value".to_string(), Value::Text("old".into()))]),
            revision: Some(4),
            created_at: Some(10),
            updated_at: Some(11),
            transferred_at: Some(12),
            created_at_block_height: Some(13),
            updated_at_block_height: Some(14),
            transferred_at_block_height: Some(15),
            created_at_core_block_height: Some(16),
            updated_at_core_block_height: Some(17),
            transferred_at_core_block_height: Some(18),
            contract_version: Some(2),
            moderated_at: Some(19),
            moderated_by: Some(Identifier::new([5; 32])),
        };
        screen.original_doc = Some(original.clone().into());
        screen.field_inputs.insert("value".into(), "changed".into());

        let (document, _) = screen.try_build_document_from_original().unwrap();

        let expected: Document = DocumentV0 {
            id: original.id,
            owner_id: original.owner_id,
            creator_id: original.creator_id,
            properties: BTreeMap::from([("value".to_string(), Value::Text("changed".into()))]),
            revision: Some(5),
            moderated_at: original.moderated_at,
            moderated_by: original.moderated_by,
            ..Default::default()
        }
        .into();
        assert_eq!(document, expected);
    }

    #[test]
    fn replacing_document_fields_preserves_moderation_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Replace);
        screen.selected_document_type = Some(editor_document_type(false));
        screen.original_doc = Some(
            DocumentV0 {
                moderated_at: Some(1234),
                moderated_by: Some(Identifier::new([7; 32])),
                ..Default::default()
            }
            .into(),
        );
        screen.field_inputs.insert("value".into(), "changed".into());
        let (document, _) = screen.try_build_document_from_original().unwrap();
        assert_eq!(document.moderated_at(), Some(1234));
        assert_eq!(document.moderated_by(), Some(Identifier::new([7; 32])));
        assert_eq!(
            document.properties().get("value"),
            Some(&Value::Text("changed".into()))
        );
    }

    fn quote(
        pricing: ActionFeePricing,
        owner: Credits,
        moderators: Credits,
    ) -> DocumentActionFeeQuote {
        let total = owner + moderators;
        let max_total = match pricing {
            ActionFeePricing::FeeMultiplier => {
                total * (100 + ACTION_FEE_MULTIPLIER_INCREASE_TOLERANCE_PERCENT as u64) / 100
            }
            ActionFeePricing::Fixed => total,
        };
        DocumentActionFeeQuote {
            agreement: DocumentActionFeeAgreement::for_declared_fee(
                pricing,
                DocumentActionFee { owner, moderators },
                AgreedFeeMultiplier {
                    known_permille: 1000,
                    increase_tolerance_percent: ACTION_FEE_MULTIPLIER_INCREASE_TOLERANCE_PERCENT,
                },
            ),
            owner,
            moderators,
            total,
            max_total,
        }
    }

    /// Regression: with an identity selected but no key that can sign the
    /// action, the screen stopped below the identity selector with no word
    /// of why (the key chooser is behind Advanced Options).
    #[test]
    fn an_identity_without_a_usable_key_is_told_why_nothing_follows() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;

        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let contract = dpns();
        let domain = owned_document_type(&contract, "domain");
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Delete);
        screen.selected_contract = Some(contract);
        screen.selected_document_type = Some(domain);
        screen.selected_identity = Some(identity_holding(&[]));
        assert!(screen.selected_key.is_none());

        let mut harness = Harness::builder().build_ui_state(
            |ui, screen: &mut DocumentActionScreen| {
                screen.render_main_content(ui);
            },
            screen,
        );
        harness.run();
        assert!(
            harness.query_by_label(NO_SIGNING_KEY_NOTICE).is_some(),
            "the missing key is explained"
        );
    }

    /// Regression: Confirm rebuilt the action from the live form, so an input
    /// changed while the fee confirmation was open was signed with the fee
    /// agreed for the action shown. What is confirmed is what is sent.
    #[test]
    fn confirming_a_contract_fee_sends_the_action_it_was_shown_for() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        let contract = dpns();
        let domain = owned_document_type(&contract, "domain");
        let key = auth_key(1, domain.security_level_requirement(), None);
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Delete);
        screen.selected_contract = Some(contract);
        screen.selected_document_type = Some(domain);
        screen.selected_identity = Some(identity_holding(std::slice::from_ref(&key)));
        screen.selected_key = Some(key);
        let shown = Identifier::new([0x11; 32]);
        screen.document_id_input = shown.to_string(Encoding::Base58);

        let quote = quote(ActionFeePricing::Fixed, 1_000, 0);
        screen.open_action_fee_confirmation(Some(quote));
        // The form behind the dialog changes before the user confirms.
        screen.document_id_input = Identifier::new([0x22; 32]).to_string(Encoding::Base58);

        let sent = screen.confirm_action_fee(&egui::Context::default());
        let AppAction::BackendTaskWithContext {
            task: BackendTask::DocumentTask(task),
            ..
        } = sent
        else {
            panic!("the confirmed action is sent, got {sent:?}");
        };
        let DocumentTask::DeleteDocument {
            document_id,
            action_fee_agreement,
            ..
        } = *task
        else {
            panic!("a delete was confirmed");
        };
        assert_eq!(document_id, shown);
        assert_eq!(action_fee_agreement, Some(quote.agreement));
    }

    /// The user agrees to the most that can be charged, so the confirmation
    /// names that ceiling and the tolerance behind it, not only today's price.
    #[test]
    fn a_multiplier_priced_fee_confirmation_names_the_ceiling_and_the_tolerance() {
        let quote = quote(ActionFeePricing::FeeMultiplier, 100_000_000_000, 0);
        let text = action_fee_confirmation_text(&quote);
        assert!(
            text.contains(&format!(
                "up to {}",
                format_credits_as_dash(quote.max_total)
            )),
            "the ceiling must be shown, got: {text}"
        );
        assert!(
            text.contains(&format!(
                "more than {ACTION_FEE_MULTIPLIER_INCREASE_TOLERANCE_PERCENT}%"
            )),
            "the tolerance must be shown, got: {text}"
        );
        assert!(text.contains(&format_credits_as_dash(quote.total)));
    }

    #[test]
    fn a_fixed_fee_confirmation_promises_no_increase() {
        let quote = quote(ActionFeePricing::Fixed, 50_000_000_000, 0);
        let text = action_fee_confirmation_text(&quote);
        assert!(text.contains("The fee is fixed"), "got: {text}");
        assert!(
            !text.contains("up to"),
            "a fixed fee cannot rise, got: {text}"
        );
    }

    /// Rounding can hide the tolerance of a tiny fee (4 credits * 1.2 floors
    /// to 4): it still follows network fees and must say so.
    #[test]
    fn a_tiny_multiplier_priced_fee_is_not_called_fixed() {
        let mut quote = quote(ActionFeePricing::FeeMultiplier, 4, 0);
        quote.max_total = quote.total;
        let text = action_fee_confirmation_text(&quote);
        assert!(text.contains("follows network fees"), "got: {text}");
        assert!(!text.contains("fixed"), "got: {text}");
    }

    #[test]
    fn a_fee_for_moderators_only_names_them() {
        let quote = quote(ActionFeePricing::Fixed, 0, 7_000);
        assert!(action_fee_confirmation_text(&quote).contains("paid to the contract's moderators"));
        assert!(action_fee_summary(&quote).contains("to the contract's moderators"));
    }

    #[test]
    fn a_fee_split_with_moderators_names_both_parts() {
        let quote = quote(
            ActionFeePricing::FeeMultiplier,
            60_000_000_000,
            40_000_000_000,
        );
        let summary = action_fee_summary(&quote);
        assert!(summary.contains(&format!(
            "{} to the contract owner",
            format_credits_as_dash(quote.owner)
        )));
        assert!(summary.contains(&format!(
            "{} to its moderators",
            format_credits_as_dash(quote.moderators)
        )));
    }

    /// A screen for `action` on the indexOnly `like` type, with an identity
    /// and a key that can sign it, on a network at `protocol_version`.
    fn index_only_screen(
        action: DocumentActionType,
        protocol_version: u32,
    ) -> (tempfile::TempDir, DocumentActionScreen) {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        crate::context::test_support::prove_protocol_version(&context, protocol_version);
        let contract = QualifiedContract {
            contract: crate::model::index_only_document::test_fixtures::index_only_contract(),
            alias: None,
        };
        let like = owned_document_type(&contract, "like");
        let key = auth_key(1, like.security_level_requirement(), None);
        let mut screen = DocumentActionScreen::new(context, None, action);
        screen.selected_contract = Some(contract);
        screen.selected_document_type = Some(like);
        screen.selected_identity = Some(identity_holding(std::slice::from_ref(&key)));
        screen.selected_key = Some(key);
        (dir, screen)
    }

    /// A fetched `like` entry, rebuilt with the id the network computes.
    fn like_entry() -> Document {
        DocumentV0 {
            id: Identifier::new([0x33; 32]),
            owner_id: Identifier::new([0xD0; 32]),
            properties: BTreeMap::from([("postId".to_string(), Value::Identifier([7; 32]))]),
            ..Default::default()
        }
        .into()
    }

    /// Replace, transfer, purchase and set price are never offered for an
    /// indexOnly type, even with a document at hand.
    #[test]
    fn index_only_entries_are_read_only() {
        for action in [
            DocumentActionType::Replace,
            DocumentActionType::Transfer,
            DocumentActionType::Purchase,
            DocumentActionType::SetPrice,
        ] {
            let (_dir, mut screen) = index_only_screen(action.clone(), 14);
            screen.original_doc = Some(like_entry());
            screen.document_id_input = like_entry().id().to_string(Encoding::Base58);
            screen.recipient_id_input = Identifier::new([9; 32]).to_string(Encoding::Base58);
            screen.price_input = "10".to_string();
            screen.fetched_price = Some(10);
            assert!(!screen.can_broadcast(), "{action:?} must not be offered");
        }
    }

    /// Selecting a fetched entry prefills its values, and the delete sent
    /// names the entry by them.
    #[test]
    fn an_index_only_entry_is_deleted_by_its_values() {
        let (_dir, mut screen) = index_only_screen(DocumentActionType::Delete, 14);
        assert!(!screen.can_broadcast(), "nothing selected yet");
        screen.select_index_only_entry(like_entry());
        assert!(screen.can_broadcast());

        let task = screen.create_document_action();
        let BackendTask::DocumentTask(task) = task else {
            panic!("a document task, got {task:?}");
        };
        let DocumentTask::DeleteIndexOnlyDocument {
            document_id,
            values,
            ..
        } = *task
        else {
            panic!("a delete by values");
        };
        assert_eq!(document_id, Identifier::new([0x33; 32]));
        use dash_sdk::dpp::data_contract::document_type::methods::DocumentTypeV0Methods;
        let mut carried = values.values;
        owned_document_type(screen.selected_contract.as_ref().unwrap(), "like")
            .as_ref()
            .sanitize_document_properties(&mut carried);
        assert_eq!(
            carried.get("postId").and_then(|v| v.to_identifier().ok()),
            Some(Identifier::new([7; 32]))
        );
    }

    /// An indexOnly delete honours "Pay the token cost" like every other
    /// action: an optional deletion cost the user chose not to pay is not
    /// carried, and a kept one is.
    #[test]
    fn an_index_only_delete_skips_an_optional_token_cost_the_user_declined() {
        use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV1Setters;
        let (_dir, mut screen) = index_only_screen(DocumentActionType::Delete, 14);
        let cost = DocumentActionTokenCost {
            token_contract_position: 0,
            token_amount: 5,
            optional: true,
            ..Default::default()
        };
        screen
            .selected_document_type
            .as_mut()
            .expect("a selected type")
            .set_document_deletion_token_cost(Some(cost));
        screen.select_index_only_entry(like_entry());

        let token_payment = |screen: &mut DocumentActionScreen| {
            let BackendTask::DocumentTask(task) = screen.create_document_action() else {
                panic!("a document task");
            };
            let DocumentTask::DeleteIndexOnlyDocument {
                token_payment_info, ..
            } = *task
            else {
                panic!("a delete by values");
            };
            token_payment_info
        };

        screen.pay_optional_token_cost = false;
        assert_eq!(
            token_payment(&mut screen),
            None,
            "the declined cost is not paid"
        );
        screen.pay_optional_token_cost = true;
        assert_eq!(
            token_payment(&mut screen),
            Some(TokenPaymentInfo::V0(TokenPaymentInfoV0 {
                payment_token_contract_id: None,
                token_contract_position: 0,
                gas_fees_paid_by: cost.gas_fees_paid_by,
                minimum_token_cost: None,
                maximum_token_cost: Some(5),
            })),
            "the kept cost is paid, capped at its amount"
        );
    }

    /// Values edited into a delete Platform would refuse are not sent.
    #[test]
    fn edited_values_are_checked_before_the_delete_is_offered() {
        let (_dir, mut screen) = index_only_screen(DocumentActionType::Delete, 14);
        screen.select_index_only_entry(like_entry());
        screen.index_only_values_input = "{}".to_string();
        assert!(!screen.can_broadcast(), "postId is required");
        screen.index_only_values_input = "not json".to_string();
        assert!(!screen.can_broadcast());
    }

    #[test]
    fn an_index_only_delete_is_not_offered_below_protocol_14() {
        let (_dir, mut screen) = index_only_screen(DocumentActionType::Delete, 13);
        screen.select_index_only_entry(like_entry());
        assert!(!screen.can_broadcast());
    }

    /// The confirmation says in plain words that a value differing from the
    /// original entry is refused after the fee is paid; other actions keep
    /// their own confirmation text.
    #[test]
    fn the_index_only_delete_confirmation_warns_about_mismatched_values() {
        let (_, text) = confirmation_title_and_text(None, true);
        assert!(text.contains("differs from what the entry was created with"));
        assert!(text.contains("the fee is still charged"));

        let quote = quote(ActionFeePricing::Fixed, 1_000, 0);
        let (_, text) = confirmation_title_and_text(Some(&quote), true);
        assert!(text.contains(&action_fee_confirmation_text(&quote)));
        assert!(text.contains(INDEX_ONLY_DELETE_WARNING));

        let (_, text) = confirmation_title_and_text(Some(&quote), false);
        assert!(!text.contains(INDEX_ONLY_DELETE_WARNING));
    }

    /// A stored document type keeps its delete by id.
    #[test]
    fn a_stored_document_type_is_still_deleted_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        crate::context::test_support::prove_protocol_version(&context, 14);
        let contract = dpns();
        let domain = owned_document_type(&contract, "domain");
        let key = auth_key(1, domain.security_level_requirement(), None);
        let mut screen = DocumentActionScreen::new(context, None, DocumentActionType::Delete);
        screen.selected_contract = Some(contract);
        screen.selected_document_type = Some(domain);
        screen.selected_identity = Some(identity_holding(std::slice::from_ref(&key)));
        screen.selected_key = Some(key);
        screen.document_id_input = Identifier::new([0x11; 32]).to_string(Encoding::Base58);
        assert!(screen.can_broadcast());
        let BackendTask::DocumentTask(task) = screen.create_document_action() else {
            panic!("a document task");
        };
        assert!(matches!(*task, DocumentTask::DeleteDocument { .. }));
    }

    /// Action fees are declared per transition, so every
    /// screen action must name the transition it broadcasts.
    #[test]
    fn every_action_maps_to_its_document_transition() {
        use DocumentTransitionActionType as T;
        for (action, expected) in [
            (DocumentActionType::Create, T::Create),
            (DocumentActionType::Delete, T::Delete),
            (DocumentActionType::Purchase, T::Purchase),
            (DocumentActionType::Replace, T::Replace),
            (DocumentActionType::SetPrice, T::UpdatePrice),
            (DocumentActionType::Transfer, T::Transfer),
        ] {
            assert_eq!(action.transition_action(), expected);
        }
    }

    /// A required token cost is always paid; an optional one only while the
    /// user keeps "Pay the token cost" on, and otherwise the action carries
    /// no token payment at all.
    #[test]
    fn an_optional_token_cost_is_paid_only_when_the_user_chooses_to() {
        let required = DocumentActionTokenCost {
            token_contract_position: 2,
            token_amount: 40,
            ..Default::default()
        };
        let optional = DocumentActionTokenCost {
            optional: true,
            ..required
        };
        let paid = |cost: DocumentActionTokenCost| {
            Some(TokenPaymentInfo::V0(TokenPaymentInfoV0 {
                payment_token_contract_id: cost.contract_id,
                token_contract_position: 2,
                gas_fees_paid_by: cost.gas_fees_paid_by,
                minimum_token_cost: None,
                maximum_token_cost: Some(40),
            }))
        };

        assert_eq!(token_payment(None, true), None);
        assert_eq!(token_payment(Some(required), false), paid(required));
        assert_eq!(token_payment(Some(optional), true), paid(optional));
        assert_eq!(token_payment(Some(optional), false), None);
    }
}
