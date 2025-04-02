use crate::app::AppAction;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::BackendTask;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::ui::contracts_documents::document_query_screen::DOCUMENT_PRIVATE_FIELDS;
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::document_type::Index;
use dash_sdk::dpp::data_contract::{
    accessors::v0::DataContractV0Getters, document_type::DocumentType,
};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui::{Color32, Context as EguiContext, Frame, Margin, RichText, SidePanel};
use std::collections::HashMap;
use std::sync::Arc;

pub fn add_contract_chooser_panel(
    ctx: &EguiContext,
    current_search_term: &mut String,
    app_context: &Arc<AppContext>,
    selected_data_contract: &mut QualifiedContract,
    selected_document_type: &mut DocumentType,
    selected_index: &mut Option<Index>,
    document_query: &mut String,
    pending_document_type: &mut DocumentType,
    pending_fields_selection: &mut HashMap<String, bool>,
) -> AppAction {
    let mut action = AppAction::None;

    // Retrieve the list of known contracts
    let contracts = app_context.get_contracts(None, None).unwrap_or_else(|e| {
        eprintln!("Error fetching contracts: {}", e);
        vec![]
    });

    // Filter contracts by name or ID
    let filtered_contracts: Vec<_> = contracts
        .iter()
        .filter(|contract| {
            let name_or_id = contract
                .alias
                .clone()
                .unwrap_or(contract.contract.id().to_string(Encoding::Base58));
            name_or_id
                .to_lowercase()
                .contains(&current_search_term.to_lowercase())
        })
        .collect();

    SidePanel::left("contract_chooser_panel")
        // Let the user resize this panel horizontally
        .resizable(true)
        .default_width(250.0)
        .frame(
            Frame::none()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(Margin::same(10.0)),
        )
        .show(ctx, |panel_ui| {
            // Make the whole panel scrollable (if it overflows vertically)
            egui::ScrollArea::vertical().show(panel_ui, |ui| {
                // Search box
                ui.horizontal(|ui| {
                    ui.label("Filter contracts:");
                    ui.text_edit_singleline(current_search_term);
                });

                // List out each matching contract
                ui.vertical(|ui| {
                    for contract in filtered_contracts {
                        ui.horizontal(|ui| {
                            let is_selected_contract = *selected_data_contract == *contract;

                            let name_or_id = contract
                                .alias
                                .clone()
                                .unwrap_or(contract.contract.id().to_string(Encoding::Base58));

                            // Highlight the contract if selected
                            let contract_header_text = if is_selected_contract {
                                RichText::new(name_or_id).color(Color32::from_rgb(21, 101, 192))
                            } else {
                                RichText::new(name_or_id)
                            };

                            // Expand/collapse the contract info
                            ui.collapsing(contract_header_text, |ui| {
                                //
                                // ===== Document Types Section =====
                                //
                                ui.collapsing("Document Types", |ui| {
                                    for (doc_name, doc_type) in contract.contract.document_types() {
                                        let is_selected_doc_type =
                                            *selected_document_type == *doc_type;

                                        let doc_type_header_text = if is_selected_doc_type {
                                            RichText::new(doc_name.clone())
                                                .color(Color32::from_rgb(21, 101, 192))
                                        } else {
                                            RichText::new(doc_name.clone())
                                        };

                                        let doc_resp = ui.collapsing(doc_type_header_text, |ui| {
                                            // Show the indexes
                                            if doc_type.indexes().is_empty() {
                                                ui.label("No indexes defined");
                                            } else {
                                                for (index_name, index) in doc_type.indexes() {
                                                    let is_selected_index =
                                                        *selected_index == Some(index.clone());

                                                    let index_header_text = if is_selected_index {
                                                        RichText::new(format!(
                                                            "Index: {}",
                                                            index_name
                                                        ))
                                                        .color(Color32::from_rgb(21, 101, 192))
                                                    } else {
                                                        RichText::new(format!(
                                                            "Index: {}",
                                                            index_name
                                                        ))
                                                    };

                                                    let index_resp =
                                                        ui.collapsing(index_header_text, |ui| {
                                                            // Show index properties if expanded
                                                            for prop in &index.properties {
                                                                ui.label(format!("{:?}", prop));
                                                            }
                                                        });

                                                    // If index was just clicked (opened)
                                                    if index_resp.header_response.clicked()
                                                        && index_resp.body_response.is_some()
                                                    {
                                                        *selected_index = Some(index.clone());
                                                        if let Ok(new_doc_type) = contract
                                                            .contract
                                                            .document_type_cloned_for_name(
                                                                &doc_name,
                                                            )
                                                        {
                                                            *selected_document_type = new_doc_type;
                                                            *selected_data_contract =
                                                                contract.clone();

                                                            // Build the WHERE clause using all property names
                                                            let conditions: Vec<String> = index
                                                                .property_names()
                                                                .iter()
                                                                .map(|property_name| {
                                                                    format!(
                                                                        "`{}` = '___'",
                                                                        property_name
                                                                    )
                                                                })
                                                                .collect();

                                                            let where_clause =
                                                                if conditions.is_empty() {
                                                                    String::new()
                                                                } else {
                                                                    format!(
                                                                        " WHERE {}",
                                                                        conditions.join(" AND ")
                                                                    )
                                                                };

                                                            *document_query = format!(
                                                                "SELECT * FROM {}{}",
                                                                selected_document_type.name(),
                                                                where_clause
                                                            );
                                                        }
                                                    }
                                                    // If index was just collapsed
                                                    else if index_resp.header_response.clicked()
                                                        && index_resp.body_response.is_none()
                                                    {
                                                        *selected_index = None;
                                                        *document_query = format!(
                                                            "SELECT * FROM {}",
                                                            selected_document_type.name()
                                                        );
                                                    }
                                                }
                                            }
                                        });

                                        // Document Type clicked
                                        if doc_resp.header_response.clicked()
                                            && doc_resp.body_response.is_some()
                                        {
                                            // Expand doc type
                                            if let Ok(new_doc_type) = contract
                                                .contract
                                                .document_type_cloned_for_name(&doc_name)
                                            {
                                                *pending_document_type = new_doc_type.clone();
                                                *selected_document_type = new_doc_type.clone();
                                                *selected_data_contract = contract.clone();
                                                *selected_index = None;
                                                *document_query = format!(
                                                    "SELECT * FROM {}",
                                                    selected_document_type.name()
                                                );

                                                // Reinitialize field selection
                                                pending_fields_selection.clear();

                                                // Mark doc-defined fields
                                                for (field_name, _schema) in
                                                    new_doc_type.properties().iter()
                                                {
                                                    pending_fields_selection
                                                        .insert(field_name.clone(), true);
                                                }
                                                // Show "internal" fields as unchecked by default
                                                for dash_field in DOCUMENT_PRIVATE_FIELDS {
                                                    pending_fields_selection
                                                        .insert(dash_field.to_string(), false);
                                                }
                                            }
                                        }
                                        // Document Type collapsed
                                        else if doc_resp.header_response.clicked()
                                            && doc_resp.body_response.is_none()
                                        {
                                            *selected_index = None;
                                            *document_query = format!(
                                                "SELECT * FROM {}",
                                                selected_document_type.name()
                                            );
                                        }
                                    }
                                });

                                //
                                // ===== Tokens Section =====
                                //
                                ui.collapsing("Tokens", |ui| {
                                    let tokens_map = contract.contract.tokens();
                                    if tokens_map.is_empty() {
                                        ui.label("No tokens defined for this contract.");
                                    } else {
                                        for (token_name, token) in tokens_map {
                                            // Each token is its own collapsible
                                            ui.collapsing(token_name.to_string(), |ui| {
                                                // Now you can display base supply, max supply, etc.
                                                ui.label(format!(
                                                    "Base Supply: {}",
                                                    token.base_supply()
                                                ));
                                                if let Some(max_supply) = token.max_supply() {
                                                    ui.label(format!("Max Supply: {}", max_supply));
                                                } else {
                                                    ui.label("Max Supply: None");
                                                }

                                                // Add more details here
                                            });
                                        }
                                    }
                                });

                                //
                                // ===== Entire Contract JSON =====
                                //
                                ui.collapsing("Contract JSON", |ui| {
                                    match contract.contract.to_json(app_context.platform_version) {
                                        Ok(json_value) => {
                                            let pretty_str =
                                                serde_json::to_string_pretty(&json_value)
                                                    .unwrap_or_else(|_| {
                                                        "Error formatting JSON".to_string()
                                                    });

                                            ui.add_space(2.0);

                                            // A resizable region that the user can drag to expand/shrink
                                            egui::Resize::default()
                                                .id_salt("json_resize_area_for_contract")
                                                .default_size([400.0, 400.0]) // initial w,h
                                                .show(ui, |ui| {
                                                    egui::ScrollArea::vertical()
                                                        .auto_shrink([false; 2])
                                                        .show(ui, |ui| {
                                                            ui.monospace(pretty_str);
                                                        });
                                                });

                                            ui.add_space(3.0);
                                        }
                                        Err(e) => {
                                            ui.label(format!(
                                                "Error converting contract to JSON: {e}"
                                            ));
                                        }
                                    }
                                });
                            });

                            // Right‐aligned Remove button
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                if contract.alias != Some("dpns".to_string())
                                    && contract.alias != Some("token_history".to_string())
                                    && contract.alias != Some("withdrawals".to_string())
                                {
                                    if ui.button("X").clicked() {
                                        action |=
                                            AppAction::BackendTask(BackendTask::ContractTask(
                                                ContractTask::RemoveContract(
                                                    contract.contract.id().clone(),
                                                ),
                                            ));
                                    }
                                }
                            });
                        });
                    }
                });
            });
        });

    action
}
