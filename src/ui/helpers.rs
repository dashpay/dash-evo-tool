use std::sync::Arc;

use crate::{
    context::AppContext,
    model::{qualified_contract::QualifiedContract, qualified_identity::QualifiedIdentity},
};
use dash_sdk::{
    dpp::{
        data_contract::{
            accessors::v0::DataContractV0Getters,
            document_type::{accessors::DocumentTypeV0Getters, DocumentType},
        },
        identity::{
            accessors::IdentityGettersV0,
            identity_public_key::accessors::v0::IdentityPublicKeyGettersV0,
        },
        platform_value::string_encoding::Encoding,
    },
    platform::IdentityPublicKey,
};
use egui::{ComboBox, Ui};

/// Returns the newly selected identity (if changed), otherwise the existing one.
pub fn render_identity_selector(
    ui: &mut Ui,
    qualified_identities: &[QualifiedIdentity],
    selected_identity: &Option<QualifiedIdentity>,
) -> Option<QualifiedIdentity> {
    let mut new_selected_identity = selected_identity.clone();

    ui.horizontal(|ui| {
        ui.label("Identity:");
        ComboBox::from_id_salt("identity_selector")
            .selected_text(
                selected_identity
                    .as_ref()
                    .map(|qi| {
                        qi.alias
                            .as_ref()
                            .unwrap_or(&qi.identity.id().to_string(Encoding::Base58))
                            .clone()
                    })
                    .unwrap_or_else(|| "Choose identity…".into()),
            )
            .show_ui(ui, |cb| {
                for qi in qualified_identities {
                    let label = qi
                        .alias
                        .as_ref()
                        .unwrap_or(&qi.identity.id().to_string(Encoding::Base58))
                        .clone();

                    if cb
                        .selectable_label(selected_identity.as_ref() == Some(qi), label)
                        .clicked()
                    {
                        new_selected_identity = Some(qi.clone());
                    }
                }
            });
    });

    new_selected_identity
}

/// Returns the newly selected key (if changed), otherwise the existing one.
pub fn render_key_selector(
    ui: &mut Ui,
    selected_identity: &QualifiedIdentity,
    selected_key: &Option<IdentityPublicKey>,
) -> Option<IdentityPublicKey> {
    let mut new_selected_key = selected_key.clone();

    ui.label("Key:");
    ComboBox::from_id_salt("key_selector")
        .selected_text(
            selected_key
                .as_ref()
                .map(|k| format!("Key {} Security {}", k.id(), k.security_level()))
                .unwrap_or_else(|| "Choose key…".into()),
        )
        .show_ui(ui, |cb| {
            for key_ref in selected_identity.available_authentication_keys_non_master() {
                let key = &key_ref.identity_public_key;
                let label = format!("Key {} Security {}", key.id(), key.security_level());
                if cb
                    .selectable_label(Some(key) == selected_key.as_ref(), label)
                    .clicked()
                {
                    new_selected_key = Some(key.clone());
                }
            }
        });

    new_selected_key
}

pub fn add_contract_doc_type_chooser_with_filtering(
    ui: &mut Ui,
    search_term: &mut String,
    app_context: &Arc<AppContext>,
    selected_contract: &mut Option<QualifiedContract>,
    selected_doc_type: &mut Option<DocumentType>,
) {
    let contracts = app_context.get_contracts(None, None).unwrap_or_default();
    let search_term_lowercase = search_term.to_lowercase();
    let filtered = contracts.iter().filter(|qc| {
        let key = qc
            .alias
            .clone()
            .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58));
        key.to_lowercase().contains(&search_term_lowercase)
    });

    add_contract_doc_type_chooser_pre_filtered(
        ui,
        search_term,
        filtered,
        selected_contract,
        selected_doc_type,
    );
}

/// Extremely compact chooser: just two combo-boxes (Contract ▸ Doc-Type)
///
/// * No collapsible tree.
/// * Optional search box on top.
/// * Emits `ContractTask::RemoveContract` via a small “🗑” button next to the contract picker.
pub fn add_contract_doc_type_chooser_pre_filtered<'a, T>(
    ui: &mut Ui,
    search_term: &mut String,
    filtered_contracts: T,
    selected_contract: &mut Option<QualifiedContract>,
    selected_doc_type: &mut Option<DocumentType>,
) where
    T: Iterator<Item = &'a QualifiedContract>,
{
    egui::Grid::new("contract_doc_type_grid")
        .num_columns(2)
        .spacing([10.0, 5.0])
        .striped(false)
        .show(ui, |ui| {
            ui.label("Filter contracts:");
            ui.text_edit_singleline(search_term);
            ui.end_row();

            ui.label("Contract:");
            ComboBox::from_id_salt("contract_combo")
                .width(220.0)
                .selected_text(match selected_contract {
                    Some(qc) => qc
                        .alias
                        .clone()
                        .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58)),
                    None => "Select Contract…".into(),
                })
                .show_ui(ui, |cui| {
                    for qc in filtered_contracts {
                        let label = qc
                            .alias
                            .clone()
                            .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58));
                        if cui
                            .selectable_label(selected_contract.as_ref() == Some(qc), label.clone())
                            .clicked()
                        {
                            *selected_contract = Some(qc.clone());
                        }
                    }
                });

            ui.end_row();

            ui.label("Doc Type:");
            ComboBox::from_id_salt("doctype_combo")
                .width(220.0)
                .selected_text(
                    selected_doc_type
                        .as_ref()
                        .map(|d| d.name().to_owned())
                        .unwrap_or_else(|| "Select Doc Type…".into()),
                )
                .show_ui(ui, |dui| {
                    if let Some(qc) = selected_contract {
                        for (name, _dt) in qc.contract.document_types() {
                            if dui
                                .selectable_label(
                                    selected_doc_type
                                        .as_ref()
                                        .map(|cur| cur.name() == name)
                                        .unwrap_or(false),
                                    name,
                                )
                                .clicked()
                            {
                                *selected_doc_type =
                                    qc.contract.document_type_cloned_for_name(name).ok();
                            }
                        }
                    } else {
                        dui.label("Pick a contract first");
                    }
                });
            ui.end_row();
        });
}

/// Extremely compact chooser: just two combo-boxes (Contract ▸ Doc-Type)
///
/// * No collapsible tree.
/// * Optional search box on top.
pub fn add_contract_chooser_pre_filtered<'a, T>(
    ui: &mut Ui,
    search_term: &mut String,
    filtered_contracts: T,
    selected_contract: &mut Option<QualifiedContract>,
) where
    T: Iterator<Item = &'a QualifiedContract>,
{
    egui::Grid::new("contract_doc_type_grid")
        .num_columns(2)
        .spacing([10.0, 5.0])
        .striped(false)
        .show(ui, |ui| {
            ui.label("Filter contracts:");
            ui.text_edit_singleline(search_term);
            ui.end_row();

            ui.label("Contract:");
            ComboBox::from_id_salt("contract_chooser")
                .width(220.0)
                .selected_text(match selected_contract {
                    Some(qc) => qc
                        .alias
                        .clone()
                        .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58)),
                    None => "Select Contract…".into(),
                })
                .show_ui(ui, |cui| {
                    for qc in filtered_contracts {
                        let label = qc
                            .alias
                            .clone()
                            .unwrap_or_else(|| qc.contract.id().to_string(Encoding::Base58));
                        if cui
                            .selectable_label(selected_contract.as_ref() == Some(qc), label.clone())
                            .clicked()
                        {
                            *selected_contract = Some(qc.clone());
                        }
                    }
                });

            ui.end_row();
        });
}
