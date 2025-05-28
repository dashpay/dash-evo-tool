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
use dash_sdk::dpp::data_contract::document_type::methods::DocumentTypeBasicMethods;
use dash_sdk::dpp::data_contract::document_type::DocumentPropertyType;
use dash_sdk::dpp::document::{DocumentV0, DocumentV0Getters};
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::tokens::gas_fees_paid_by::GasFeesPaidBy;
use dash_sdk::dpp::tokens::token_amount_on_contract_token::DocumentActionTokenEffect;
use dash_sdk::dpp::tokens::token_payment_info::v0::TokenPaymentInfoV0;
use dash_sdk::dpp::tokens::token_payment_info::TokenPaymentInfo;
use dash_sdk::dpp::{
    data_contract::document_type::DocumentType,
    document::Document,
    identity::{IdentityPublicKey, Purpose, SecurityLevel},
    platform_value::string_encoding::Encoding,
};
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Color32, Context, TextEdit, Ui};
use egui::RichText;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::collections::BTreeMap;
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};
/* ---------------------------------------------------------------- *\
                      Screen-state helpers
\* ---------------------------------------------------------------- */

#[derive(PartialEq)]
enum BroadcastStatus {
    Idle,
    MissingField(String),
    Broadcasting(u64),
    Error(String),
    Complete,
}

/* ---------------------------------------------------------------- *\
                    DocumentCreatorScreen struct
\* ---------------------------------------------------------------- */

pub struct CreateDocumentScreen {
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

    /* ---- dynamic field inputs ---- */
    field_inputs: HashMap<String, String>,

    /* ---- status ---- */
    broadcast_status: BroadcastStatus,
}

impl CreateDocumentScreen {
    pub fn new(ctx: &Arc<AppContext>) -> Self {
        // pre-filter auth keys as before
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

            field_inputs: HashMap::new(),
            broadcast_status: BroadcastStatus::Idle,
        }
    }

    /* ---------------------------------------------------------- *
     *   identity + key selector (shorter than original version)  *
     * ---------------------------------------------------------- */
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
                                self.selected_key = None; // force key re-select
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

    /* ---------------------------------------------------------- *
     *                dynamic property editors                    *
     * ---------------------------------------------------------- */
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
                            format!("{prop_name}:")
                        };
                        ui.label(label);
                        match &schema.property_type {
                            /* ---------- integers (all sizes) ---------- */
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

                            /* ---------- floats ---------- */
                            DocumentPropertyType::F64 => {
                                ui.add(TextEdit::singleline(val).hint_text("floating-point"));
                            }

                            /* ---------- string ---------- */
                            DocumentPropertyType::String(size) => {
                                ui.add({
                                    let text_edit = TextEdit::singleline(val);
                                    if let Some(max_length) = size.max_length {
                                        text_edit.hint_text(format!("max {}", max_length).as_str())
                                    } else {
                                        text_edit
                                    }
                                });
                            }

                            /* ---------- byte array ---------- */
                            DocumentPropertyType::ByteArray(_size) => {
                                ui.add(TextEdit::singleline(val).hint_text("hex or base64"));
                            }

                            /* ---------- identifier ---------- */
                            DocumentPropertyType::Identifier => {
                                ui.add(TextEdit::singleline(val).hint_text("base58 identifier"));
                            }

                            /* ---------- boolean ---------- */
                            DocumentPropertyType::Boolean => {
                                let mut checked = matches!(
                                    val.to_ascii_lowercase().as_str(),
                                    "true" | "1" | "yes" | "on"
                                );
                                if ui.checkbox(&mut checked, "").changed() {
                                    *val = checked.to_string();
                                }
                            }

                            /* ---------- date (unix-ms) ---------- */
                            DocumentPropertyType::Date => {
                                ui.add(TextEdit::singleline(val).hint_text("unix-ms"));
                            }

                            /* ---------- JSON objects / arrays ---------- */
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

    /* ---------------------------------------------------------- *
     *          assemble + validate + broadcast                   *
     * ---------------------------------------------------------- */
    fn try_build_document(&mut self) -> Result<(Document, [u8; 32]), String> {
        /* -------- sanity checks -------- */
        let contract = self
            .selected_contract
            .as_ref()
            .ok_or("No contract selected")?;
        let doc_type = self
            .selected_doc_type
            .as_ref()
            .ok_or("No document-type selected")?;
        let qi = self.selected_qid.as_ref().ok_or("No identity selected")?;

        /* -------- required-field check -------- */
        for (prop, schema) in doc_type.properties() {
            let entry = self.field_inputs.get(prop).map(|s| s.trim());
            if schema.required && entry.unwrap_or("").is_empty() {
                return Err(format!("Field “{prop}” is required"));
            }
        }

        let mut properties = BTreeMap::new();

        /* -------- set user fields, type-aware -------- */
        for (name, input_str) in &self.field_inputs {
            let schema = doc_type
                .properties()
                .get(name)
                .ok_or_else(|| format!("Unknown property {name}"))?;

            // convert UI string into proper Value
            let value = match &schema.property_type {
                DocumentPropertyType::U128 => {
                    let n = input_str
                        .parse::<u128>()
                        .map_err(|_| format!("“{name}” must be an unsigned 128-bit integer"))?;
                    Value::U128(n)
                }
                DocumentPropertyType::I128 => {
                    let n = input_str
                        .parse::<i128>()
                        .map_err(|_| format!("“{name}” must be a signed 128-bit integer"))?;
                    Value::I128(n)
                }
                DocumentPropertyType::U64 => {
                    let n = input_str
                        .parse::<u64>()
                        .map_err(|_| format!("“{name}” must be an unsigned 64-bit integer"))?;
                    Value::U64(n)
                }
                DocumentPropertyType::I64 => {
                    let n = input_str
                        .parse::<i64>()
                        .map_err(|_| format!("“{name}” must be a signed 64-bit integer"))?;
                    Value::I64(n)
                }
                DocumentPropertyType::U32 => {
                    let n = input_str
                        .parse::<u32>()
                        .map_err(|_| format!("“{name}” must be an unsigned 32-bit integer"))?;
                    Value::U32(n)
                }
                DocumentPropertyType::I32 => {
                    let n = input_str
                        .parse::<i32>()
                        .map_err(|_| format!("“{name}” must be a signed 32-bit integer"))?;
                    Value::I32(n)
                }
                DocumentPropertyType::U16 => {
                    let n = input_str
                        .parse::<u16>()
                        .map_err(|_| format!("“{name}” must be an unsigned 16-bit integer"))?;
                    Value::U16(n)
                }
                DocumentPropertyType::I16 => {
                    let n = input_str
                        .parse::<i16>()
                        .map_err(|_| format!("“{name}” must be a signed 16-bit integer"))?;
                    Value::I16(n)
                }
                DocumentPropertyType::U8 => {
                    let n = input_str
                        .parse::<u8>()
                        .map_err(|_| format!("“{name}” must be an unsigned 8-bit integer"))?;
                    Value::U8(n)
                }
                DocumentPropertyType::I8 => {
                    let n = input_str
                        .parse::<i8>()
                        .map_err(|_| format!("“{name}” must be a signed 8-bit integer"))?;
                    Value::I8(n)
                }
                DocumentPropertyType::F64 => {
                    let f = input_str
                        .parse::<f64>()
                        .map_err(|_| format!("“{name}” must be a floating-point number"))?;
                    Value::Float(f)
                }
                DocumentPropertyType::String(_size) => Value::Text(input_str.clone()),
                DocumentPropertyType::ByteArray(_size) => {
                    let bytes = if let Ok(b) = hex::decode(input_str) {
                        b
                    } else {
                        STANDARD.decode(input_str).map_err(|_| {
                            format!("“{name}” must be hex or base64 for a ByteArray field")
                        })?
                    };
                    Value::Bytes(bytes.into())
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
                    ));
                }
                DocumentPropertyType::Array(_) | DocumentPropertyType::VariableTypeArray(_) => {
                    return Err(format!(
                        "Array field “{name}” must be supplied via JSON textarea"
                    ));
                }
            };

            properties.insert(name.clone(), value);
        }

        let mut rng = StdRng::from_entropy();

        let entropy: [u8; 32] = rng.gen();

        /* ------------ system fields ----------------- */
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

        /* ------------ build DocumentV0 -------------- */
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

    fn show_success_screen(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        ui.vertical_centered(|ui| {
            ui.add_space(100.0);

            ui.heading("🎉");
            if let Some(msg) = &self.backend_message {
                if msg.contains("Document broadcasted successfully") {
                    ui.heading(msg);
                }
            }
            ui.add_space(20.0);

            if ui.button("Back").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }

            if ui.button("Add another document").clicked() {
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
        self.selected_qid = None;
        self.selected_key = None;
    }
}

/* ---------------------------------------------------------------- *\
                         ScreenLike impl
\* ---------------------------------------------------------------- */
impl ScreenLike for CreateDocumentScreen {
    /* ---------- message plumbing ---------- */
    fn display_message(&mut self, msg: &str, ty: MessageType) {
        match ty {
            MessageType::Error => self.broadcast_status = BroadcastStatus::Error(msg.into()),
            MessageType::Info => self.backend_message = Some(msg.to_string()),
            MessageType::Success => {
                if msg.contains("Document broadcasted successfully") {
                    self.backend_message = Some(msg.to_string())
                }
            }
        }
    }

    fn display_task_result(&mut self, task_result: BackendTaskSuccessResult) {
        match task_result {
            BackendTaskSuccessResult::Document(doc) => {
                self.broadcast_status = BroadcastStatus::Complete;
                self.display_message(
                    &format!("Document broadcasted successfully.\n\nID: {}", doc.id()),
                    MessageType::Success,
                );
            }
            _ => {}
        }
    }

    /* ---------- main UI ---------- */
    fn ui(&mut self, ctx: &Context) -> AppAction {
        /* top / left panels */
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                ("Create Document", AppAction::None),
            ],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenDocumentQuery,
        );

        /* central panel logic */
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
            ui.add_space(10.0);

            if self.selected_doc_type.is_none() {
                return;
            }

            ui.separator();
            ui.add_space(10.0);

            /* ───────── Identity & key ─────────────────────── */
            ui.heading("2. Select an identity and key:");
            ui.add_space(10.0);

            self.ui_identity_picker(ui);
            ui.add_space(10.0);

            if self.selected_qid.is_none() || self.selected_key.is_none() {
                return;
            }

            ui.separator();
            ui.add_space(10.0);

            /* ───────── Wallet unlock (if any) ─────────────── */
            if let Some(_) = &self.selected_wallet {
                let (need, unlocked) = self.render_wallet_unlock_if_needed(ui);
                if need && !unlocked {
                    return; // wait for unlock
                }
            }

            /* ───────── Field inputs ───────────────────────── */
            ui.heading("3. Fill out the document type fields:");
            ui.add_space(10.0);

            // Allocate space for error message
            let error_message_height = 40.0;
            let max_scroll_height = if self.error_message.is_some() {
                ui.available_height() - error_message_height
            } else {
                ui.available_height()
            };

            // A simple table with columns: [Token Name | Token ID | Total Balance]
            egui::ScrollArea::vertical()
                .max_height(max_scroll_height)
                .show(ui, |ui| {
                    self.ui_field_inputs(ui);

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // Display token costs if any
                    if let Some(doc_type) = &self.selected_doc_type {
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
                            ui.label(
                                RichText::new(format!(
                                    "Creation cost: {} \"{}\" tokens.\nTokens will be {}.\nGas fees will be paid by {}.",
                                    token_amount, token_name, token_effect_string, gas_fees_paid_by_string
                                ))
                                .color(Color32::DARK_RED),
                            );
                        }
                    }

                    /* ───────── Broadcast button & status ─────────── */
                    ui.add_space(10.0);
                    let button = egui::Button::new(
                        RichText::new("Broadcast document").color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(0, 128, 255))
                    .frame(true)
                    .corner_radius(3.0)
                    .min_size(egui::vec2(100.0, 30.0));

                    if ui.add(button).clicked() {
                        match self.try_build_document() {
                            Ok((doc, entropy)) => {
                                let doc_type = match &self.selected_doc_type {
                                    Some(dt) => dt.clone(),
                                    None => {
                                        self.broadcast_status = BroadcastStatus::Error("Document type not set".to_string());
                                        return;
                                    }
                                };
                                let token_payment_info = if let Some(token_creation_cost) =
                                    doc_type.document_creation_token_cost()
                                {
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
                                self.broadcast_status = BroadcastStatus::Broadcasting(
                                    SystemTime::now()
                                        .duration_since(UNIX_EPOCH)
                                        .unwrap()
                                        .as_secs(),
                                );
                                action |= AppAction::BackendTask(BackendTask::DocumentTask(
                                    DocumentTask::BroadcastDocument(
                                        doc,
                                        token_payment_info,
                                        entropy,
                                        self.selected_doc_type
                                            .as_ref()
                                            .expect("Selected Doc Type not set")
                                            .clone(),
                                        self.selected_qid
                                            .as_ref()
                                            .expect("Selected QID not set")
                                            .clone(),
                                        self.selected_key
                                            .as_ref()
                                            .expect("Selected Key not set")
                                            .clone(),
                                    ),
                                ));
                            }
                            Err(e) => self.broadcast_status = BroadcastStatus::MissingField(e),
                        }
                    }
                });

            /* status read-out */
            ui.add_space(10.0);
            match &self.broadcast_status {
                BroadcastStatus::Idle => {}
                BroadcastStatus::MissingField(e) | BroadcastStatus::Error(e) => {
                    ui.colored_label(Color32::DARK_RED, e);
                }
                BroadcastStatus::Broadcasting(start) => {
                    let secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs()
                        - start;
                    ui.label(format!("Broadcasting… {secs}s"));
                }
                BroadcastStatus::Complete => {
                    // this is handled at the beginning of the CentralPanel
                }
            }
        });

        action
    }
}

/* ---------------------------------------------------------------- *\
         Wallet-unlock helper for ScreenWithWalletUnlock
\* ---------------------------------------------------------------- */

impl ScreenWithWalletUnlock for CreateDocumentScreen {
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
