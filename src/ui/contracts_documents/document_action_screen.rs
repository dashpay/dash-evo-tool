use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

use crate::app::AppAction;
use crate::backend_task::{document::DocumentTask, BackendTask};
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::helpers::{
    add_contract_doc_type_chooser_with_filtering, render_identity_selector, render_key_selector,
};
use crate::ui::ScreenLike;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use dash_sdk::dpp::balances::credits::Credits;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::{
    DocumentTypeV0Getters, DocumentTypeV1Getters,
};
use dash_sdk::dpp::data_contract::document_type::methods::DocumentTypeBasicMethods;
use dash_sdk::dpp::data_contract::document_type::{DocumentPropertyType, DocumentType};
use dash_sdk::dpp::document::{Document, DocumentV0, DocumentV0Getters};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::tokens::gas_fees_paid_by::GasFeesPaidBy;
use dash_sdk::dpp::tokens::token_amount_on_contract_token::DocumentActionTokenEffect;
use dash_sdk::dpp::tokens::token_payment_info::v0::TokenPaymentInfoV0;
use dash_sdk::dpp::tokens::token_payment_info::TokenPaymentInfo;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::epaint::Color32;
use egui::{Context, RichText, Ui};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

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
    Broadcasting,
    Broadcasted,
}

pub struct DocumentActionScreen {
    pub app_context: Arc<AppContext>,
    pub action_type: DocumentActionType,

    // Common fields
    pub backend_message: Option<String>,
    pub selected_identity: Option<QualifiedIdentity>,
    pub selected_key: Option<IdentityPublicKey>,
    pub wallet: Option<Arc<RwLock<Wallet>>>,
    pub wallet_password: String,
    pub wallet_failure: Option<String>,
    pub show_password: bool,
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
}

impl DocumentActionScreen {
    pub fn new(
        app_context: Arc<AppContext>,
        selected_identity: Option<QualifiedIdentity>,
        action_type: DocumentActionType,
    ) -> Self {
        let known_contracts =
            if let Ok(contracts) = app_context.db.get_contracts(&app_context, None, None) {
                contracts
            } else {
                Vec::new()
            };

        let identities_map = if let Ok(identities) = app_context.load_local_qualified_identities() {
            identities
                .into_iter()
                .map(|identity| (identity.identity.id(), identity))
                .collect()
        } else {
            HashMap::new()
        };

        let selected_contract = known_contracts.into_iter().next();

        Self {
            app_context,
            action_type,
            backend_message: None,
            selected_identity,
            selected_key: None,
            wallet: None,
            wallet_password: String::new(),
            wallet_failure: None,
            show_password: false,
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
        }
    }

    fn render_contract_and_type_selection(&mut self, ui: &mut Ui) {
        ui.heading("1. Select a contract and document type:");
        ui.add_space(10.0);

        add_contract_doc_type_chooser_with_filtering(
            ui,
            &mut self.contract_search,
            &self.app_context,
            &mut self.selected_contract,
            &mut self.selected_document_type,
        );
        ui.add_space(10.0);
    }

    fn render_identity_and_key_selection(&mut self, ui: &mut Ui) {
        ui.heading("2. Select an identity and key:");
        ui.add_space(10.0);

        let identities_vec: Vec<_> = self.identities_map.values().cloned().collect();
        self.selected_identity =
            render_identity_selector(ui, &identities_vec, &self.selected_identity);

        if let Some(ref identity) = self.selected_identity {
            self.selected_key = render_key_selector(ui, identity, &self.selected_key);
        }
        ui.add_space(10.0);
    }

    fn render_action_specific_inputs(&mut self, ui: &mut Ui) -> AppAction {
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

            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 100.0)
                .show(ui, |ui| {
                    self.ui_field_inputs(ui, &doc_type, contract_id);

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    self.render_token_cost_info(ui, &doc_type);
                    action |= self.render_broadcast_button(ui);
                });
        }
        action
    }

    fn render_delete_inputs(&mut self, ui: &mut Ui) -> AppAction {
        ui.heading("3. Enter document ID to delete:");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Document ID:");
            ui.text_edit_singleline(&mut self.document_id_input);
        });

        ui.add_space(10.0);
        self.render_broadcast_button(ui)
    }

    fn render_purchase_inputs(&mut self, ui: &mut Ui) -> AppAction {
        ui.heading("3. Enter document ID to purchase:");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Document ID:");
            ui.text_edit_singleline(&mut self.document_id_input);
        });

        if let Some(price) = self.fetched_price {
            ui.add_space(10.0);
            ui.label(format!("Document price: {} credits", price));
        }

        ui.add_space(10.0);
        self.render_broadcast_button(ui)
    }

    fn render_replace_inputs(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.heading("3. Enter document ID and updated fields:");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Document ID:");
            ui.text_edit_singleline(&mut self.document_id_input);
        });

        if let Some(_original_doc) = &self.original_doc {
            ui.add_space(10.0);
            ui.label("Update document fields:");

            if let (Some(contract), Some(doc_type)) =
                (&self.selected_contract, &self.selected_document_type)
            {
                let contract_id = contract.contract.id();
                let doc_type = doc_type.clone();

                egui::ScrollArea::vertical()
                    .max_height(ui.available_height() - 100.0)
                    .show(ui, |ui| {
                        self.ui_field_inputs(ui, &doc_type, contract_id);

                        ui.add_space(10.0);
                        action |= self.render_broadcast_button(ui);
                    });
            }
        } else {
            ui.add_space(10.0);
            action |= self.render_broadcast_button(ui);
        }
        action
    }

    fn render_set_price_inputs(&mut self, ui: &mut Ui) -> AppAction {
        ui.heading("3. Set document price:");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Document ID:");
            ui.text_edit_singleline(&mut self.document_id_input);
        });

        ui.horizontal(|ui| {
            ui.label("Price (credits):");
            ui.text_edit_singleline(&mut self.price_input);
        });

        ui.add_space(10.0);
        self.render_broadcast_button(ui)
    }

    fn render_transfer_inputs(&mut self, ui: &mut Ui) -> AppAction {
        ui.heading("3. Transfer document:");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Document ID:");
            ui.text_edit_singleline(&mut self.document_id_input);
        });

        ui.horizontal(|ui| {
            ui.label("Recipient Identity:");
            ui.text_edit_singleline(&mut self.recipient_id_input);
        });

        ui.add_space(10.0);
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
                        | DocumentPropertyType::I32
                        | DocumentPropertyType::U16
                        | DocumentPropertyType::I16
                        | DocumentPropertyType::U8
                        | DocumentPropertyType::I8 => {
                            ui.add(egui::TextEdit::singleline(val).hint_text("integer"));
                        }
                        DocumentPropertyType::F64 => {
                            ui.add(egui::TextEdit::singleline(val).hint_text("floating-point"));
                        }
                        DocumentPropertyType::String(size) => {
                            ui.add({
                                let text_edit = egui::TextEdit::singleline(val);
                                if let Some(max_length) = size.max_length {
                                    text_edit.hint_text(format!("max {}", max_length).as_str())
                                } else {
                                    text_edit
                                }
                            });
                        }
                        DocumentPropertyType::ByteArray(_size) => {
                            ui.add(egui::TextEdit::singleline(val).hint_text("hex or base64"));
                        }
                        DocumentPropertyType::Identifier => {
                            ui.add(egui::TextEdit::singleline(val).hint_text("base58 identifier"));
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
                            ui.add(egui::TextEdit::singleline(val).hint_text("unix-ms"));
                        }
                        DocumentPropertyType::Object(_)
                        | DocumentPropertyType::Array(_)
                        | DocumentPropertyType::VariableTypeArray(_) => {
                            ui.add(egui::TextEdit::multiline(val).hint_text("JSON value"));
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
        if let Some(token_creation_cost) = doc_type.document_creation_token_cost() {
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
                    "Creation cost: {} \"{}\" tokens.\nTokens will be {}.\nGas fees will be paid by {}.",
                    token_amount, token_name, token_effect_string, gas_fees_paid_by_string
                ))
                .color(Color32::DARK_RED),
            );
        }
    }

    fn render_broadcast_button(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.add_space(10.0);
        let button_text = match self.action_type {
            DocumentActionType::Create => "Broadcast document",
            DocumentActionType::Delete => "Delete document",
            DocumentActionType::Purchase => "Purchase document",
            DocumentActionType::Replace => "Replace document",
            DocumentActionType::SetPrice => "Set document price",
            DocumentActionType::Transfer => "Transfer document",
        };

        let button = egui::Button::new(RichText::new(button_text).color(Color32::WHITE))
            .fill(Color32::from_rgb(0, 128, 255))
            .frame(true)
            .corner_radius(3.0)
            .min_size(egui::vec2(100.0, 30.0));

        if ui.add(button).clicked() && self.can_broadcast() {
            self.wallet_password.clear();
            self.broadcast_status = BroadcastStatus::Broadcasting;
            let task = self.create_document_action();
            action = AppAction::BackendTask(task);
        }

        // Status display
        match &self.broadcast_status {
            BroadcastStatus::Broadcasting => {
                ui.label("Broadcasting...");
            }
            _ => {}
        }

        action
    }

    fn create_document_action(&self) -> BackendTask {
        match self.action_type {
            DocumentActionType::Create => self.create_document_task(),
            DocumentActionType::Delete => self.create_delete_task(),
            DocumentActionType::Purchase => self.create_purchase_task(),
            DocumentActionType::Replace => self.create_replace_task(),
            DocumentActionType::SetPrice => self.create_set_price_task(),
            DocumentActionType::Transfer => self.create_transfer_task(),
        }
    }

    fn create_document_task(&self) -> BackendTask {
        match self.try_build_document() {
            Ok((doc, entropy)) => {
                let doc_type = self.selected_document_type.as_ref().unwrap();

                let token_payment_info =
                    if let Some(token_creation_cost) = doc_type.document_creation_token_cost() {
                        Some(TokenPaymentInfo::V0(TokenPaymentInfoV0 {
                            payment_token_contract_id: token_creation_cost.contract_id,
                            token_contract_position: token_creation_cost.token_contract_position,
                            gas_fees_paid_by: token_creation_cost.gas_fees_paid_by,
                            minimum_token_cost: None,
                            maximum_token_cost: Some(token_creation_cost.token_amount),
                        }))
                    } else {
                        None
                    };

                BackendTask::DocumentTask(DocumentTask::BroadcastDocument(
                    doc,
                    token_payment_info,
                    entropy,
                    doc_type.clone(),
                    self.selected_identity.as_ref().unwrap().clone(),
                    self.selected_key.as_ref().unwrap().clone(),
                ))
            }
            Err(_) => BackendTask::DocumentTask(DocumentTask::BroadcastDocument(
                DocumentV0::default().into(),
                None,
                [0; 32],
                self.selected_document_type.as_ref().unwrap().clone(),
                self.selected_identity.as_ref().unwrap().clone(),
                self.selected_key.as_ref().unwrap().clone(),
            )),
        }
    }

    fn create_delete_task(&self) -> BackendTask {
        let document_id =
            Identifier::from_string(&self.document_id_input, Encoding::Base58).unwrap_or_default();

        let doc_type = self.selected_document_type.as_ref().unwrap();

        BackendTask::DocumentTask(DocumentTask::DeleteDocument(
            document_id,
            doc_type.clone(),
            self.selected_contract.as_ref().unwrap().contract.clone(),
            self.selected_identity.as_ref().unwrap().clone(),
            self.selected_key.as_ref().unwrap().clone(),
            None,
        ))
    }

    fn create_purchase_task(&self) -> BackendTask {
        let document_id =
            Identifier::from_string(&self.document_id_input, Encoding::Base58).unwrap_or_default();

        let doc_type = self.selected_document_type.as_ref().unwrap();

        BackendTask::DocumentTask(DocumentTask::PurchaseDocument(
            self.fetched_price.unwrap_or(0),
            document_id,
            doc_type.clone(),
            self.selected_contract.as_ref().unwrap().contract.clone(),
            self.selected_identity.as_ref().unwrap().clone(),
            self.selected_key.as_ref().unwrap().clone(),
            None,
        ))
    }

    fn create_replace_task(&self) -> BackendTask {
        if let Some(_original_doc) = &self.original_doc {
            match self.try_build_document_from_original() {
                Ok((updated_doc, _entropy)) => {
                    let doc_type = self.selected_document_type.as_ref().unwrap();

                    BackendTask::DocumentTask(DocumentTask::ReplaceDocument(
                        updated_doc,
                        doc_type.clone(),
                        self.selected_contract.as_ref().unwrap().contract.clone(),
                        self.selected_identity.as_ref().unwrap().clone(),
                        self.selected_key.as_ref().unwrap().clone(),
                        None,
                    ))
                }
                Err(_) => {
                    let doc_type = self.selected_document_type.as_ref().unwrap();

                    BackendTask::DocumentTask(DocumentTask::ReplaceDocument(
                        DocumentV0::default().into(),
                        doc_type.clone(),
                        self.selected_contract.as_ref().unwrap().contract.clone(),
                        self.selected_identity.as_ref().unwrap().clone(),
                        self.selected_key.as_ref().unwrap().clone(),
                        None,
                    ))
                }
            }
        } else {
            let doc_type = self.selected_document_type.as_ref().unwrap();

            BackendTask::DocumentTask(DocumentTask::ReplaceDocument(
                DocumentV0::default().into(),
                doc_type.clone(),
                self.selected_contract.as_ref().unwrap().contract.clone(),
                self.selected_identity.as_ref().unwrap().clone(),
                self.selected_key.as_ref().unwrap().clone(),
                None,
            ))
        }
    }

    fn create_set_price_task(&self) -> BackendTask {
        let document_id =
            Identifier::from_string(&self.document_id_input, Encoding::Base58).unwrap_or_default();
        let price = self.price_input.parse::<u64>().unwrap_or(0);

        let doc_type = self.selected_document_type.as_ref().unwrap();

        BackendTask::DocumentTask(DocumentTask::SetDocumentPrice(
            price,
            document_id,
            doc_type.clone(),
            self.selected_contract.as_ref().unwrap().contract.clone(),
            self.selected_identity.as_ref().unwrap().clone(),
            self.selected_key.as_ref().unwrap().clone(),
            None,
        ))
    }

    fn create_transfer_task(&self) -> BackendTask {
        let document_id =
            Identifier::from_string(&self.document_id_input, Encoding::Base58).unwrap_or_default();
        let recipient_id =
            Identifier::from_string(&self.recipient_id_input, Encoding::Base58).unwrap_or_default();

        let doc_type = self.selected_document_type.as_ref().unwrap();

        BackendTask::DocumentTask(DocumentTask::TransferDocument(
            document_id,
            recipient_id,
            doc_type.clone(),
            self.selected_contract.as_ref().unwrap().contract.clone(),
            self.selected_identity.as_ref().unwrap().clone(),
            self.selected_key.as_ref().unwrap().clone(),
            None,
        ))
    }

    fn try_build_document(&self) -> Result<(Document, [u8; 32]), String> {
        let contract = self
            .selected_contract
            .as_ref()
            .ok_or("No contract selected")?;
        let doc_type = self
            .selected_document_type
            .as_ref()
            .ok_or("No document-type selected")?;
        let qi = self
            .selected_identity
            .as_ref()
            .ok_or("No identity selected")?;

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
                DocumentPropertyType::U32 => {
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
                    Value::Bytes(bytes.into())
                }
                DocumentPropertyType::Identifier => {
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
        let entropy: [u8; 32] = rng.gen();

        let owner_id = qi.identity.id();
        let id = Document::generate_document_id_v0(
            &contract.contract.id(),
            &owner_id,
            doc_type.name(),
            entropy.as_slice(),
        );

        let revision = if doc_type.requires_revision() {
            Some(1)
        } else {
            None
        };

        let raw_doc = DocumentV0 {
            id,
            properties,
            owner_id,
            revision,
            created_at: None,
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
        };

        Ok((raw_doc.into(), entropy))
    }

    fn try_build_document_from_original(&self) -> Result<(Document, [u8; 32]), String> {
        let original_doc = self.original_doc.as_ref().ok_or("No original document")?;
        let _contract = self
            .selected_contract
            .as_ref()
            .ok_or("No contract selected")?;
        let doc_type = self
            .selected_document_type
            .as_ref()
            .ok_or("No document-type selected")?;

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
                DocumentPropertyType::U32 => {
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
                    Value::Bytes(bytes.into())
                }
                DocumentPropertyType::Identifier => {
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
        let entropy: [u8; 32] = rng.gen();

        let new_revision = if let Some(current_revision) = original_doc.revision() {
            Some(current_revision + 1)
        } else {
            Some(1)
        };

        let updated_doc = DocumentV0 {
            id: original_doc.id(),
            properties,
            owner_id: original_doc.owner_id(),
            revision: new_revision,
            created_at: original_doc.created_at(),
            updated_at: None,
            transferred_at: original_doc.transferred_at(),
            created_at_block_height: original_doc.created_at_block_height(),
            updated_at_block_height: None,
            transferred_at_block_height: original_doc.transferred_at_block_height(),
            created_at_core_block_height: original_doc.created_at_core_block_height(),
            updated_at_core_block_height: None,
            transferred_at_core_block_height: original_doc.transferred_at_core_block_height(),
        };

        Ok((updated_doc.into(), entropy))
    }

    fn can_broadcast(&self) -> bool {
        match self.action_type {
            DocumentActionType::Create => !self.field_inputs.is_empty(),
            DocumentActionType::Delete => !self.document_id_input.is_empty(),
            DocumentActionType::Purchase => {
                !self.document_id_input.is_empty() && self.fetched_price.is_some()
            }
            DocumentActionType::Replace => self.original_doc.is_some(),
            DocumentActionType::SetPrice => {
                !self.document_id_input.is_empty() && !self.price_input.is_empty()
            }
            DocumentActionType::Transfer => {
                !self.document_id_input.is_empty() && !self.recipient_id_input.is_empty()
            }
        }
    }

    fn get_payment_info(&self) -> Option<String> {
        None
    }
}

impl ScreenLike for DocumentActionScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                (self.action_type.display_name(), AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenDocumentQuery,
        );

        egui::CentralPanel::default().show(ctx, |ui| match &self.broadcast_status {
            BroadcastStatus::Broadcasted => {
                ui.heading("Success!");
                ui.label(format!("{} successful!", self.action_type.display_name()));

                if ui.button("Back to Contracts").clicked() {
                    action = AppAction::GoToMainScreen;
                }
            }
            _ => {
                action |= self.render_main_content(ui);
            }
        });

        action
    }

    fn refresh(&mut self) {
        // Backend messages are handled via display_message
    }

    fn display_message(&mut self, message: &str, _message_type: crate::ui::MessageType) {
        self.backend_message = Some(message.to_string());
    }

    fn display_task_result(&mut self, _result: crate::ui::BackendTaskSuccessResult) {
        self.broadcast_status = BroadcastStatus::Broadcasted;
    }
}

impl DocumentActionScreen {
    fn render_main_content(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        if let Some(ref msg) = self.backend_message {
            ui.label(msg);
        }

        // Step 1: Contract and Document Type Selection
        self.render_contract_and_type_selection(ui);

        if self.selected_contract.is_none() || self.selected_document_type.is_none() {
            return action;
        }

        ui.separator();
        ui.add_space(10.0);

        // Step 2: Identity and Key Selection
        self.render_identity_and_key_selection(ui);

        if self.selected_identity.is_none() || self.selected_key.is_none() {
            return action;
        }

        ui.separator();
        ui.add_space(10.0);

        // Step 3: Action-specific inputs and broadcast
        action |= match self.action_type {
            DocumentActionType::Create => self.render_create_inputs(ui),
            _ => self.render_action_specific_inputs(ui),
        };

        action
    }
}

impl ScreenWithWalletUnlock for DocumentActionScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.wallet
    }

    fn wallet_password_ref(&self) -> &String {
        &self.wallet_password
    }

    fn wallet_password_mut(&mut self) -> &mut String {
        &mut self.wallet_password
    }

    fn show_password(&self) -> bool {
        self.show_password
    }

    fn show_password_mut(&mut self) -> &mut bool {
        &mut self.show_password
    }

    fn set_error_message(&mut self, error_message: Option<String>) {
        self.wallet_failure = error_message;
    }

    fn error_message(&self) -> Option<&String> {
        self.wallet_failure.as_ref()
    }
}
