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
use crate::ui::{MessageType, ScreenLike};

use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV1Getters;
use dash_sdk::dpp::data_contract::document_type::DocumentType;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::tokens::gas_fees_paid_by::GasFeesPaidBy;
use dash_sdk::dpp::tokens::token_amount_on_contract_token::DocumentActionTokenEffect;
use dash_sdk::dpp::tokens::token_payment_info::v0::TokenPaymentInfoV0;
use dash_sdk::dpp::tokens::token_payment_info::TokenPaymentInfo;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, Color32, Context, Ui};
use egui::RichText;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(PartialEq)]
enum BroadcastStatus {
    Idle,
    MissingField(String),
    Broadcasting(u64),
    Error(String),
    Complete,
}

pub struct SetDocumentPriceScreen {
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

    /* ---- doc-id ---- */
    doc_id_input: String,

    /* ---- price input ---- */
    price_input: String,

    /* ---- status ---- */
    broadcast_status: BroadcastStatus,
}

impl SetDocumentPriceScreen {
    pub fn new(ctx: &Arc<AppContext>) -> Self {
        let qids = ctx
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|qi| {
                let keys = qi.available_authentication_keys_with_high_security_level();
                (!keys.is_empty()).then_some((
                    qi.clone(),
                    keys.into_iter()
                        .map(|k| k.identity_public_key.clone())
                        .collect(),
                ))
            })
            .collect();

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
            doc_id_input: String::new(),
            price_input: String::new(),
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

    fn parse_doc_id(&self) -> Result<Identifier, String> {
        Identifier::from_string(self.doc_id_input.trim(), Encoding::Base58)
            .map_err(|_| "Document ID is not valid base58".to_string())
    }

    fn show_success_screen(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.heading("🎉");
            if let Some(msg) = &self.backend_message {
                ui.heading(msg);
            }
            ui.add_space(20.0);
            if ui.button("Back").clicked() {
                action = AppAction::PopScreenAndRefresh;
            }
            if ui.button("Set Price for Another Document").clicked() {
                self.reset_fields();
                action = AppAction::None;
            }
        });
        action
    }

    fn reset_fields(&mut self) {
        self.broadcast_status = BroadcastStatus::Idle;
        self.backend_message = None;
        self.contract_search.clear();
        self.selected_contract = None;
        self.selected_doc_type = None;
        self.doc_id_input.clear();
        self.price_input.clear();
        self.selected_qid = None;
        self.selected_key = None;
    }
}

impl ScreenLike for SetDocumentPriceScreen {
    fn display_message(&mut self, msg: &str, ty: MessageType) {
        match ty {
            MessageType::Error => self.broadcast_status = BroadcastStatus::Error(msg.into()),
            MessageType::Info => self.backend_message = Some(msg.to_string()),
            MessageType::Success => {
                if msg.contains("Document price set successfully") {
                    self.broadcast_status = BroadcastStatus::Complete;
                };
                self.backend_message = Some(msg.to_string());
            }
        }
    }

    fn display_task_result(&mut self, task_result: crate::ui::BackendTaskSuccessResult) {
        self.broadcast_status = BroadcastStatus::Complete;
        self.display_message(&format!("{:?}", task_result), MessageType::Success);
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                ("Set Document Price", AppAction::None),
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
            if self.selected_doc_type.is_none() {
                return;
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            ui.heading("2. Select an identity and key:");
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

            ui.heading("3. Enter the Document ID to set price:");
            ui.add_space(10.0);
            ui.text_edit_singleline(&mut self.doc_id_input);

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            ui.heading("4. Enter the new price in Credits for the document:");
            ui.add_space(10.0);
            ui.text_edit_singleline(&mut self.price_input);

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            // Display token costs if any
            if let Some(doc_type) = &self.selected_doc_type {
                if let Some(token_creation_cost) = doc_type.document_update_price_token_cost() {
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
                            "Set Price cost: {} \"{}\" tokens.\nTokens will be {}.\nGas fees will be paid by {}.",
                            token_amount, token_name, token_effect_string, gas_fees_paid_by_string
                        ))
                        .color(Color32::DARK_RED),
                    );
                }
            }

            let button = egui::Button::new(RichText::new("Set document price").color(Color32::WHITE))
                .fill(Color32::from_rgb(220, 30, 30))
                .frame(true)
                .corner_radius(3.0)
                .min_size(egui::vec2(120.0, 30.0));

            ui.add_space(10.0);
            if ui.add(button).clicked() {
                match self.parse_doc_id() {
                    Ok(doc_id) => {
                        let selected_qid = self.selected_qid.clone().unwrap();
                        let selected_key = self.selected_key.clone().unwrap();
                        let doc_type = self.selected_doc_type.clone().unwrap();

                        self.broadcast_status = BroadcastStatus::Broadcasting(
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                        );

                        let token_payment_info =
                            if let Some(token_cost) = doc_type.document_update_price_token_cost() {
                                Some(TokenPaymentInfo::V0(TokenPaymentInfoV0 {
                                    payment_token_contract_id: token_cost.contract_id,
                                    token_contract_position: token_cost.token_contract_position,
                                    gas_fees_paid_by: token_cost.gas_fees_paid_by,
                                    minimum_token_cost: None,
                                    maximum_token_cost: Some(token_cost.token_amount),
                                }))
                            } else {
                                None
                            };

                        let price = match self.price_input.trim().parse::<u64>() {
                            Ok(val) => val,
                            Err(_) => {
                                self.broadcast_status = BroadcastStatus::Error("Invalid price input".to_string());
                                return;
                            }
                        };

                        action |= AppAction::BackendTask(BackendTask::DocumentTask(
                            DocumentTask::SetDocumentPrice(
                                price,
                                doc_id,
                                doc_type,
                                self.selected_contract
                                    .clone()
                                    .expect("Contract should be selected")
                                    .contract,
                                selected_qid,
                                selected_key,
                                token_payment_info,
                            ),
                        ));
                    }
                    Err(e) => self.broadcast_status = BroadcastStatus::MissingField(e),
                }
            }

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
                BroadcastStatus::Complete => {}
            }
        });

        action
    }
}

impl ScreenWithWalletUnlock for SetDocumentPriceScreen {
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
