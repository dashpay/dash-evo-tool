//! ui/document_creator_screen.rs
use crate::app::AppAction;
use crate::backend_task::{document::DocumentTask, BackendTask};
use crate::context::AppContext;
use crate::model::wallet::Wallet;
use crate::model::{qualified_contract::QualifiedContract, qualified_identity::QualifiedIdentity};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tools_subscreen_chooser_panel::add_tools_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::identities::get_selected_wallet;
use crate::ui::BackendTaskSuccessResult;
use crate::ui::{MessageType, ScreenLike};

use crate::ui::tools::document_visualizer_screen::add_simple_contract_doc_type_chooser;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::document_type::methods::DocumentTypeBasicMethods;
use dash_sdk::dpp::data_contract::document_type::DocumentPropertyType;
use dash_sdk::dpp::document::DocumentV0;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::Value;
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
    BuildingError(String),
    Broadcasting(u64),
    Error(String),
    Complete,
}

/* ---------------------------------------------------------------- *\
                    DocumentCreatorScreen struct
\* ---------------------------------------------------------------- */

pub struct CreateDocumentScreen {
    pub app_context: Arc<AppContext>,

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
    fn ui_identity_picker(&mut self, ui: &mut Ui) {
        ui.heading("1. Select Identity");
        ui.add_space(4.0);

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
                    }
                }
            });

        if let Some(qi) = &self.selected_qid {
            ui.horizontal(|ui| {
                ui.label("Key:");
                egui::ComboBox::from_id_salt("key_combo")
                    .selected_text(
                        self.selected_key
                            .as_ref()
                            .map(|k| format!("Key {} (SL {:?})", k.id(), k.security_level()))
                            .unwrap_or_else(|| "Choose key…".into()),
                    )
                    .show_ui(ui, |cb| {
                        for (qi_ref, keys) in &self.qualified_identities {
                            if qi_ref != qi {
                                continue;
                            }
                            for k in keys {
                                if cb
                                    .selectable_label(
                                        self.selected_key.as_ref() == Some(k),
                                        format!("Key {}", k.id()),
                                    )
                                    .clicked()
                                {
                                    self.selected_key = Some(k.clone());
                                    self.selected_wallet = get_selected_wallet(
                                        qi,
                                        Some(&self.app_context),
                                        Some(k),
                                        &mut self.error_message,
                                    );
                                }
                            }
                        }
                    });
            });
        }
    }
    /* ---------------------------------------------------------- *
     *                dynamic property editors                    *
     * ---------------------------------------------------------- */
    fn ui_field_inputs(&mut self, ui: &mut Ui) {
        if let Some(doc_type) = &self.selected_doc_type {
            ui.heading("3. Fill Document Fields");
            ui.add_space(4.0);

            for (prop_name, schema) in doc_type.properties() {
                // 1 get or create backing string
                let val = self.field_inputs.entry(prop_name.clone()).or_default();

                // 2 one horizontal line per property
                ui.horizontal(|ui| {
                    // star for required fields
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
                            ui.add(
                                TextEdit::singleline(val)
                                    .hint_text("integer")
                                    .desired_width(140.0),
                            );
                        }

                        /* ---------- floats ---------- */
                        DocumentPropertyType::F64 => {
                            ui.add(
                                TextEdit::singleline(val)
                                    .hint_text("floating-point")
                                    .desired_width(140.0),
                            );
                        }

                        /* ---------- string ---------- */
                        DocumentPropertyType::String(size) => {
                            ui.add({
                                let text_edit = TextEdit::singleline(val).desired_width(220.0);
                                if let Some(max_length) = size.max_length {
                                    text_edit.hint_text(format!("max {}", max_length).as_str())
                                } else {
                                    text_edit
                                }
                            });
                        }

                        /* ---------- byte array ---------- */
                        DocumentPropertyType::ByteArray(_size) => {
                            ui.add(
                                TextEdit::singleline(val)
                                    .hint_text("hex or base64")
                                    .desired_width(260.0),
                            );
                        }

                        /* ---------- identifier ---------- */
                        DocumentPropertyType::Identifier => {
                            ui.add(
                                TextEdit::singleline(val)
                                    .hint_text("base58 identifier")
                                    .desired_width(260.0),
                            );
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
                            ui.add(
                                TextEdit::singleline(val)
                                    .hint_text("unix-ms")
                                    .desired_width(160.0),
                            );
                        }

                        /* ---------- JSON objects / arrays ---------- */
                        DocumentPropertyType::Object(_)
                        | DocumentPropertyType::Array(_)
                        | DocumentPropertyType::VariableTypeArray(_) => {
                            ui.add(
                                TextEdit::multiline(val)
                                    .hint_text("JSON value")
                                    .desired_rows(2)
                                    .desired_width(ui.available_width() * 0.6),
                            );
                        }
                    }
                });
            }
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
                        base64::decode(input_str).map_err(|_| {
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
}

/* ---------------------------------------------------------------- *\
                         ScreenLike impl
\* ---------------------------------------------------------------- */
impl ScreenLike for CreateDocumentScreen {
    /* ---------- message plumbing ---------- */
    fn display_message(&mut self, msg: &str, ty: MessageType) {
        match ty {
            MessageType::Error => self.broadcast_status = BroadcastStatus::Error(msg.into()),
            MessageType::Success => self.broadcast_status = BroadcastStatus::Complete,
            _ => {}
        }
    }
    fn display_task_result(&mut self, _: BackendTaskSuccessResult) {}

    /* ---------- main UI ---------- */
    fn ui(&mut self, ctx: &Context) -> AppAction {
        /* top / left panels */
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Docs", AppAction::None)],
            vec![],
        );
        action |= add_left_panel(
            ctx,
            &self.app_context,
            crate::ui::RootScreenType::RootScreenToolsTransitionVisualizerScreen,
        );
        action |= add_tools_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        /* floating contract/doc-type chooser */
        action |= add_simple_contract_doc_type_chooser(
            ctx,
            &mut self.contract_search,
            &self.app_context,
            &mut self.selected_contract,
            &mut self.selected_doc_type,
        );

        /* central panel logic */
        egui::CentralPanel::default().show(ctx, |ui| {
            /* 1 ───────── Contract selected? ───────────────────── */
            let Some(contract) = &self.selected_contract else {
                ui.heading("Select a data contract on the left to begin.");
                return;
            };
            ui.label(format!(
                "Contract: {}",
                contract
                    .alias
                    .as_ref()
                    .unwrap_or(&contract.contract.id().to_string(Encoding::Base58))
            ));

            /* 2 ───────── Doc-type selected? ──────────────────── */
            let Some(doc_type) = self.selected_doc_type.clone() else {
                ui.add_space(6.0);
                ui.label("Now pick a document-type.");
                return;
            };
            ui.label(format!("Doc-type: {}", doc_type.name()));
            ui.add_space(10.0);

            /* 3 ───────── Identity & key ─────────────────────── */
            self.ui_identity_picker(ui);

            if self.selected_qid.is_none() || self.selected_key.is_none() {
                ui.add_space(6.0);
                ui.label("Choose an identity and key to sign with.");
                return;
            }

            /* 4 ───────── Wallet unlock (if any) ─────────────── */
            if let Some(_) = &self.selected_wallet {
                let (need, unlocked) = self.render_wallet_unlock_if_needed(ui);
                if need && !unlocked {
                    return; // wait for unlock
                }
            }

            /* 5 ───────── Field inputs ───────────────────────── */
            self.ui_field_inputs(ui);

            /* 6 ───────── Broadcast button & status ─────────── */
            ui.add_space(12.0);
            let button = ui
                .add_enabled(
                    !matches!(self.broadcast_status, BroadcastStatus::Broadcasting(_)),
                    egui::Button::new(
                        RichText::new("Broadcast document")
                            .color(Color32::WHITE)
                            .background_color(Color32::from_rgb(0, 128, 255)),
                    ),
                )
                .on_hover_text("Send to platform");

            if button.clicked() {
                match self.try_build_document() {
                    Ok((doc, entropy)) => {
                        self.broadcast_status = BroadcastStatus::Broadcasting(
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                        );
                        action |= AppAction::BackendTask(BackendTask::DocumentTask(
                            DocumentTask::BroadcastDocument(
                                doc,
                                entropy,
                                doc_type,
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

            /* status read-out */
            match &self.broadcast_status {
                BroadcastStatus::Idle => {}
                BroadcastStatus::MissingField(e) | BroadcastStatus::Error(e) => {
                    ui.colored_label(Color32::RED, e);
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
                    ui.colored_label(Color32::GREEN, "Document broadcasted!");
                }
                BroadcastStatus::BuildingError(e) => {
                    ui.colored_label(Color32::RED, e);
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
