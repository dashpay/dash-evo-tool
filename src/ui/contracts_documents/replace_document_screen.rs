use crate::app::AppAction;
use crate::backend_task::{document::DocumentTask, BackendTask};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::model::{qualified_contract::QualifiedContract, qualified_identity::QualifiedIdentity};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::helpers::add_contract_doc_type_chooser_with_filtering;
use crate::ui::identities::get_selected_wallet;
use crate::ui::BackendTaskSuccessResult;
use crate::ui::{MessageType, ScreenLike};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::{
    DocumentTypeV0Getters, DocumentTypeV1Getters,
};
use dash_sdk::dpp::data_contract::document_type::{DocumentPropertyType, DocumentType};
use dash_sdk::dpp::document::{Document, DocumentV0, DocumentV0Getters};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::tokens::gas_fees_paid_by::GasFeesPaidBy;
use dash_sdk::dpp::tokens::token_amount_on_contract_token::DocumentActionTokenEffect;
use dash_sdk::dpp::tokens::token_payment_info::v0::TokenPaymentInfoV0;
use dash_sdk::dpp::tokens::token_payment_info::TokenPaymentInfo;
use dash_sdk::dpp::{
    identity::{IdentityPublicKey, Purpose, SecurityLevel},
    platform_value::string_encoding::Encoding,
};
use dash_sdk::platform::{DocumentQuery, Identifier};
use eframe::egui::{self, Color32, Context, TextEdit, Ui};
use egui::RichText;
use std::collections::BTreeMap;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(PartialEq)]
enum BroadcastStatus {
    Idle,
    MissingField(String),
    Fetching(u64),
    Fetched,
    Broadcasting(u64),
    Error(String),
    Complete,
}

pub struct ReplaceDocumentScreen {
    pub app_context: Arc<AppContext>,
    backend_message: Option<String>,

    /* ---- identity & key ---- */
    qualified_identities: Vec<(QualifiedIdentity, Vec<IdentityPublicKey>)>,
    selected_qid: Option<QualifiedIdentity>,
    selected_key: Option<IdentityPublicKey>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    error_message: Option<String>,

    /* ---- contract + doc-type ---- */
    contract_search: String,
    selected_contract: Option<QualifiedContract>,
    selected_doc_type: Option<DocumentType>,

    /* ---- document ID ---- */
    document_id_input: String,
    fetched: bool,
    original_doc: Option<Document>,

    /* ---- dynamic field inputs ---- */
    field_inputs: HashMap<String, String>,

    /* ---- status ---- */
    broadcast_status: BroadcastStatus,
}

impl ReplaceDocumentScreen {
    pub fn new(ctx: &Arc<AppContext>) -> Self {
        let qids = ctx
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|qi| {
                let keys: Vec<_> = qi
                    .identity
                    .public_keys()
                    .values()
                    .filter(|k| {
                        k.purpose() == Purpose::AUTHENTICATION
                            && matches!(
                                k.security_level(),
                                SecurityLevel::HIGH | SecurityLevel::CRITICAL
                            )
                            && !k.is_disabled()
                    })
                    .cloned()
                    .collect();
                (!keys.is_empty()).then_some((qi, keys))
            })
            .collect::<Vec<_>>();

        Self {
            app_context: Arc::clone(ctx),
            backend_message: None,
            qualified_identities: qids,
            selected_qid: None,
            selected_key: None,
            selected_wallet: None,
            wallet_password: String::new(),
            show_password: false,
            error_message: None,
            contract_search: String::new(),
            selected_contract: None,
            selected_doc_type: None,
            document_id_input: String::new(),
            fetched: false,
            original_doc: None,
            field_inputs: HashMap::new(),
            broadcast_status: BroadcastStatus::Idle,
        }
    }

    pub fn ui_identity_picker(&mut self, ui: &mut Ui) {
        egui::Grid::new("identity_key_grid")
            .num_columns(2)
            .spacing([10.0, 5.0])
            .striped(false)
            .show(ui, |ui| {
                ui.label("Identity:");
                egui::ComboBox::from_id_salt("identity_combo")
                    .selected_text(
                        self.selected_qid
                            .as_ref()
                            .map(|q| {
                                q.alias
                                    .as_ref()
                                    .unwrap_or(&q.identity.id().to_string(Encoding::Base58))
                                    .clone()
                            })
                            .unwrap_or_else(|| "Choose identity…".into()),
                    )
                    .show_ui(ui, |cb| {
                        for (qi, _keys) in &self.qualified_identities {
                            let label = qi
                                .alias
                                .as_ref()
                                .unwrap_or(&qi.identity.id().to_string(Encoding::Base58))
                                .clone();
                            if cb
                                .selectable_label(self.selected_qid.as_ref() == Some(qi), label)
                                .clicked()
                            {
                                self.selected_qid = Some(qi.clone());
                                self.selected_key = None;
                                self.selected_wallet = get_selected_wallet(
                                    qi,
                                    Some(&self.app_context),
                                    None,
                                    &mut self.error_message,
                                );
                                if let Some(default_public_key) = qi
                                    .available_authentication_keys_with_high_security_level()
                                    .first()
                                {
                                    self.selected_key =
                                        Some(default_public_key.identity_public_key.clone());
                                }
                            }
                        }
                    });
                ui.end_row();

                if let Some(qi) = &self.selected_qid {
                    ui.label("Key:");
                    egui::ComboBox::from_id_salt("key_combo")
                        .selected_text(
                            self.selected_key
                                .as_ref()
                                .map(|k| format!("Key {} Security {}", k.id(), k.security_level()))
                                .unwrap_or_else(|| "Choose key…".into()),
                        )
                        .show_ui(ui, |cb| {
                            for (qi_ref, _keys) in &self.qualified_identities {
                                if qi_ref != qi {
                                    continue;
                                }
                                for k in qi_ref.available_authentication_keys_non_master() {
                                    if cb
                                        .selectable_label(
                                            self.selected_key.as_ref()
                                                == Some(&k.identity_public_key),
                                            format!(
                                                "Key {} Security {}",
                                                k.identity_public_key.id(),
                                                k.identity_public_key.security_level().to_string()
                                            ),
                                        )
                                        .clicked()
                                    {
                                        self.selected_key = Some(k.identity_public_key.clone());
                                        self.selected_wallet = get_selected_wallet(
                                            qi,
                                            Some(&self.app_context),
                                            Some(&k.identity_public_key),
                                            &mut self.error_message,
                                        );
                                    }
                                }
                            }
                        });
                    ui.end_row();
                }
            });
    }

    fn ui_field_inputs(&mut self, ui: &mut Ui) {
        if let Some(doc_type) = &self.selected_doc_type {
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
                                ui.add(TextEdit::singleline(val).hint_text("integer"));
                            }
                            DocumentPropertyType::F64 => {
                                ui.add(TextEdit::singleline(val).hint_text("floating-point"));
                            }
                            DocumentPropertyType::String(size) => {
                                ui.add({
                                    let te = TextEdit::singleline(val);
                                    if let Some(max) = size.max_length {
                                        te.hint_text(format!("max {}", max).as_str())
                                    } else {
                                        te
                                    }
                                });
                            }
                            DocumentPropertyType::ByteArray(_) => {
                                ui.add(TextEdit::singleline(val).hint_text("hex or base64"));
                            }
                            DocumentPropertyType::Identifier => {
                                ui.add(TextEdit::singleline(val).hint_text("base58 identifier"));
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
                                ui.add(TextEdit::singleline(val).hint_text("unix-ms"));
                            }
                            DocumentPropertyType::Object(_)
                            | DocumentPropertyType::Array(_)
                            | DocumentPropertyType::VariableTypeArray(_) => {
                                ui.add(TextEdit::multiline(val).hint_text("JSON value"));
                            }
                        }
                        ui.end_row();
                    }
                });
        }
    }

    fn try_build_document(&mut self) -> Result<(Document, [u8; 32]), String> {
        let doc_type = self
            .selected_doc_type
            .as_ref()
            .ok_or("No document-type selected")?;
        let qi = self.selected_qid.as_ref().ok_or("No identity selected")?;
        let orig = self
            .original_doc
            .as_ref()
            .ok_or("No document fetched to replace")?;

        for (prop, schema) in doc_type.properties() {
            let entry = self.field_inputs.get(prop).map(|s| s.trim());
            if schema.required && entry.unwrap_or("").is_empty() {
                return Err(format!("Field “{prop}” is required"));
            }
        }

        let mut properties = BTreeMap::new();
        for (name, input_str) in &self.field_inputs {
            let schema = doc_type
                .properties()
                .get(name)
                .ok_or_else(|| format!("Unknown property {name}"))?;
            let value = match &schema.property_type {
                DocumentPropertyType::U128 => {
                    let n = input_str
                        .parse::<u128>()
                        .map_err(|_| format!("“{name}” must be an unsigned 128-bit integer"))?;
                    Value::U128(n)
                }
                // ... handle other types identically to create screen ...
                DocumentPropertyType::String(_) => Value::Text(input_str.clone()),
                DocumentPropertyType::ByteArray(_) => {
                    let b = if let Ok(b) = hex::decode(input_str) {
                        b
                    } else {
                        STANDARD.decode(input_str).map_err(|_| {
                            format!("“{name}” must be hex or base64 for a ByteArray field")
                        })?
                    };
                    Value::Bytes(b.into())
                }
                DocumentPropertyType::Identifier => {
                    let id = Identifier::from_string(input_str, Encoding::Base58)
                        .map_err(|_| format!("“{name}” is not a valid Identifier (base58)"))?;
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
                        .map_err(|_| format!("“{name}” (Date) expects unix-ms integer"))?;
                    Value::U64(ts)
                }
                DocumentPropertyType::Object(_) => {
                    return Err(format!(
                        "Object field “{name}” must be supplied via JSON textarea"
                    ))
                }
                DocumentPropertyType::Array(_) | DocumentPropertyType::VariableTypeArray(_) => {
                    return Err(format!(
                        "Array field “{name}” must be supplied via JSON textarea"
                    ))
                }
                _ => unimplemented!(),
            };
            properties.insert(name.clone(), value);
        }

        // reuse original id and bump revision
        let original_id = orig.id().clone();
        let revision = orig.revision().map(|r| r + 1).or(Some(1));

        let raw_doc = DocumentV0 {
            id: original_id,
            properties,
            owner_id: qi.identity.id(),
            revision,
            created_at: orig.created_at(),
            updated_at: None,
            transferred_at: None,
            created_at_block_height: orig.created_at_block_height(),
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: orig.created_at_core_block_height(),
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
        };

        // use dummy entropy since id is fixed
        let entropy = [0u8; 32];
        Ok((raw_doc.into(), entropy))
    }

    fn show_success_screen(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.heading("🎉");
            if let Some(msg) = &self.backend_message {
                if msg.contains("Document replaced successfully") {
                    ui.heading(msg);
                }
            }
            ui.add_space(20.0);
            if ui.button("Back").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
            if ui.button("Replace another").clicked() {
                self.reset_fields();
            }
        });
        action
    }

    fn reset_fields(&mut self) {
        self.backend_message = None;
        self.broadcast_status = BroadcastStatus::Idle;
        self.field_inputs.clear();
        self.contract_search.clear();
        self.selected_contract = None;
        self.selected_doc_type = None;
        self.document_id_input.clear();
        self.fetched = false;
        self.original_doc = None;
        self.selected_qid = None;
        self.selected_key = None;
    }
}

impl ScreenLike for ReplaceDocumentScreen {
    fn display_message(&mut self, msg: &str, ty: MessageType) {
        match ty {
            MessageType::Error => self.broadcast_status = BroadcastStatus::Error(msg.into()),
            MessageType::Info => self.backend_message = Some(msg.to_string()),
            MessageType::Success => {
                if msg.contains("Document replaced successfully") {
                    self.backend_message = Some(msg.to_string());
                    self.broadcast_status = BroadcastStatus::Complete;
                }
            }
        }
    }

    fn display_task_result(&mut self, task_result: BackendTaskSuccessResult) {
        match task_result {
            BackendTaskSuccessResult::Documents(doc_map) => {
                // doc_map: IndexMap<Identifier, Option<Document>>
                self.broadcast_status = BroadcastStatus::Fetched;
                let maybe_doc = doc_map.values().find_map(|opt_doc| opt_doc.as_ref());
                if let Some(document) = maybe_doc {
                    self.original_doc = Some(document.clone());
                    // populate fields
                    self.field_inputs = document
                        .properties()
                        .iter()
                        .map(|(k, v)| {
                            let s = match v {
                                Value::Text(text) => text.clone(),
                                Value::Bytes(bytes) => hex::encode(bytes),
                                Value::Bool(b) => b.to_string(),
                                Value::U8(n) => n.to_string(),
                                Value::U16(n) => n.to_string(),
                                Value::U32(n) => n.to_string(),
                                Value::U64(n) => n.to_string(),
                                Value::I8(n) => n.to_string(),
                                Value::I16(n) => n.to_string(),
                                Value::I32(n) => n.to_string(),
                                Value::I64(n) => n.to_string(),
                                Value::Float(f) => f.to_string(),
                                Value::Identifier(id) => match Identifier::from_bytes(id) {
                                    Ok(identifier) => identifier.to_string(Encoding::Base58),
                                    Err(_) => "<invalid identifier>".to_string(),
                                },
                                _ => v.to_string(), // fallback for arrays/objects
                            };
                            (k.clone(), s)
                        })
                        .collect();
                    self.fetched = true;
                    self.backend_message =
                        Some("Document fetched. You can now edit fields.".into());
                } else {
                    self.broadcast_status =
                        BroadcastStatus::Error("No documents returned from the backend".into());
                }
            }
            _ => {}
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                ("Replace Document", AppAction::None),
            ],
            vec![],
        );
        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenDocumentQuery,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.broadcast_status == BroadcastStatus::Complete {
                action |= self.show_success_screen(ui);
                return;
            }

            ui.heading("1. Select a contract and document type:");
            ui.add_space(10.0);
            add_contract_doc_type_chooser_with_filtering(
                ui,
                &mut self.contract_search,
                &self.app_context,
                &mut self.selected_contract,
                &mut self.selected_doc_type,
            );
            if self.selected_doc_type.is_none() || self.selected_contract.is_none() {
                return;
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            if !self.fetched {
                ui.heading("2. Enter Document ID and fetch:");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.add(
                        TextEdit::singleline(&mut self.document_id_input)
                            .hint_text("Document ID (base58)"),
                    );
                    if ui.button("Fetch").clicked() {
                        match Identifier::from_string(&self.document_id_input, Encoding::Base58) {
                            Ok(id) => {
                                let document_query = DocumentQuery {
                                    data_contract: self
                                        .selected_contract
                                        .clone()
                                        .expect("Contract not selected")
                                        .contract
                                        .into(),
                                    document_type_name: self
                                        .selected_doc_type
                                        .clone()
                                        .expect("Doc type not selected")
                                        .name()
                                        .to_string(),
                                    where_clauses: vec![],
                                    order_by_clauses: vec![],
                                    limit: 1,
                                    start: None,
                                };
                                let query = document_query.clone().with_document_id(&id);
                                self.broadcast_status = BroadcastStatus::Fetching(
                                    SystemTime::now()
                                        .duration_since(UNIX_EPOCH)
                                        .unwrap()
                                        .as_secs(),
                                );
                                action |= AppAction::BackendTask(BackendTask::DocumentTask(
                                    DocumentTask::FetchDocuments(query),
                                ));
                            }
                            Err(_) => {
                                self.broadcast_status =
                                    BroadcastStatus::Error("Invalid Document ID format".into());
                            }
                        }
                    }
                });
                if let BroadcastStatus::Error(e) = &self.broadcast_status {
                    ui.add_space(10.0);
                    ui.colored_label(Color32::DARK_RED, e);
                }
                // If fetching, show fetching status
                if let BroadcastStatus::Fetching(start_fetching_time) = self.broadcast_status {
                    let secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        - start_fetching_time;
                    ui.label(format!("Fetching... {secs}s"));
                }
                return;
            } else {
                ui.heading(
                    RichText::new("2. Existing document fetched.").color(Color32::DARK_GREEN),
                );
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            ui.heading("3. Select an identity and key:");
            ui.add_space(10.0);
            self.ui_identity_picker(ui);
            if self.selected_qid.is_none() || self.selected_key.is_none() {
                return;
            }

            if let Some(_) = &self.selected_wallet {
                let (need, unlocked) = self.render_wallet_unlock_if_needed(ui);
                if need && !unlocked {
                    return;
                }
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            ui.heading("4. Edit fields and replace document:");
            ui.add_space(10.0);

            let error_height = 40.0;
            let max_height = if self.error_message.is_some() {
                ui.available_height() - error_height
            } else {
                ui.available_height()
            };
            egui::ScrollArea::vertical()
                .max_height(max_height)
                .show(ui, |ui| {
                    self.ui_field_inputs(ui);

                                        // Display token costs if any
                    if let Some(doc_type) = &self.selected_doc_type {
                        ui.add_space(10.0);

                        if let Some(token_creation_cost) = doc_type.document_creation_token_cost() {
                            let token_amount = token_creation_cost.token_amount;
                            let token_name = if let Some(contract) = &self.selected_contract {
                                let contract_id = contract.contract.id();
                                if let Ok(Some(contract)) = self
                                    .app_context
                                    .get_contract_by_id(&contract_id)
                                    .map_err(|_| "Contract not found locally") {
                                    contract
                                        .contract.tokens().get(&token_creation_cost.token_contract_position)
                                        .map(|t| t.conventions().singular_form_by_language_code_or_default("en").to_string())
                                        .unwrap_or_else(|| format!(
                                            "Token {}",
                                            token_creation_cost.token_contract_position
                                        ))
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
                                GasFeesPaidBy::PreferContractOwner => "the contract owner unless their balance is insufficient, in which case you pay",
                            };
                            ui.label(format!("Creation cost: {} {} tokens. Tokens will be {}. Gas fees paid by {}.", token_amount, token_name, token_effect_string, gas_fees_paid_by_string));
                        }
                    }

                    ui.add_space(10.0);
                    let btn =
                        egui::Button::new(RichText::new("Replace document").color(Color32::WHITE))
                            .fill(Color32::from_rgb(0, 128, 255))
                            .frame(true)
                            .corner_radius(3.0)
                            .min_size(egui::vec2(100.0, 30.0));
                    if ui.add(btn).clicked() {
                        match self.try_build_document() {
                            Ok((doc, _entropy)) => {
                                let token_payment_info = if let Some(token_creation_cost) = self
                                    .selected_doc_type
                                    .clone()
                                    .expect("Doc type not selected")
                                    .document_replacement_token_cost()
                                {
                                    Some(TokenPaymentInfo::V0(TokenPaymentInfoV0 {
                                        payment_token_contract_id: token_creation_cost.contract_id,
                                        token_contract_position: token_creation_cost
                                            .token_contract_position,
                                        gas_fees_paid_by: token_creation_cost.gas_fees_paid_by,
                                        minimum_token_cost: None,
                                        maximum_token_cost: Some(token_creation_cost.token_amount),
                                    }))
                                } else {
                                    None
                                };

                                self.broadcast_status = BroadcastStatus::Broadcasting(
                                    SystemTime::now()
                                        .duration_since(UNIX_EPOCH)
                                        .unwrap()
                                        .as_secs(),
                                );
                                action |= AppAction::BackendTask(BackendTask::DocumentTask(
                                    DocumentTask::ReplaceDocument(
                                        doc,
                                        self.selected_doc_type.clone().unwrap(),
                                        self.selected_contract.clone().unwrap().contract,
                                        self.selected_qid.clone().unwrap(),
                                        self.selected_key.clone().unwrap(),
                                        token_payment_info,
                                    ),
                                ));
                            }
                            Err(e) => self.broadcast_status = BroadcastStatus::MissingField(e),
                        }
                    }
                });

            ui.add_space(10.0);
            match &self.broadcast_status {
                BroadcastStatus::Idle => {}
                BroadcastStatus::MissingField(e) | BroadcastStatus::Error(e) => {
                    ui.colored_label(Color32::DARK_RED, e);
                }
                BroadcastStatus::Fetching(start) => {
                    let secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        - start;
                    ui.label(format!("Fetching... {secs}s"));
                }
                BroadcastStatus::Fetched => {
                    // nothing. handled above
                }
                BroadcastStatus::Broadcasting(start) => {
                    let secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        - start;
                    ui.label(format!(
                        "{}... {secs}s",
                        if !self.fetched {
                            "Fetching"
                        } else {
                            "Replacing"
                        }
                    ));
                }
                BroadcastStatus::Complete => {}
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for ReplaceDocumentScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.selected_wallet
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
    fn set_error_message(&mut self, msg: Option<String>) {
        self.error_message = msg;
    }
    fn error_message(&self) -> Option<&String> {
        self.error_message.as_ref()
    }
}
