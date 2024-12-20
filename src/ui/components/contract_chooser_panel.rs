use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::document_type::Index;
use dash_sdk::dpp::data_contract::{
    accessors::v0::DataContractV0Getters, document_type::DocumentType,
};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui::{Color32, Context, Frame, Margin, RichText, SidePanel};
use std::sync::Arc;

pub fn add_contract_chooser_panel(
    ctx: &Context,
    current_search_term: &mut String,
    app_context: &Arc<AppContext>,
    selected_data_contract: &mut QualifiedContract,
    selected_document_type: &mut DocumentType,
    selected_index: &mut Option<Index>,
    document_query: &mut String,
) -> AppAction {
    let action = AppAction::None;

    let contracts = app_context.get_contracts(None, None).unwrap_or_else(|e| {
        eprintln!("Error fetching contracts: {}", e);
        vec![]
    });

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
        .default_width(250.0)
        .frame(
            Frame::none()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(Margin::same(10.0)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(current_search_term);
            });

            ui.separator();

            ui.vertical(|ui| {
                for contract in filtered_contracts {
                    let is_selected_contract = *selected_data_contract == *contract;

                    let name_or_id = contract
                        .alias
                        .clone()
                        .unwrap_or(contract.contract.id().to_string(Encoding::Base58));

                    let contract_header_text = if is_selected_contract {
                        RichText::new(name_or_id).color(Color32::from_rgb(21, 101, 192))
                    } else {
                        RichText::new(name_or_id)
                    };

                    ui.collapsing(contract_header_text, |ui| {
                        for (doc_name, doc_type) in contract.contract.document_types() {
                            let is_selected_doc_type = *selected_document_type == *doc_type;

                            let doc_type_header_text = if is_selected_doc_type {
                                RichText::new(doc_name.clone())
                                    .color(Color32::from_rgb(21, 101, 192))
                            } else {
                                RichText::new(doc_name.clone())
                            };

                            let doc_resp = ui.collapsing(doc_type_header_text, |ui| {
                                // Display indexes as collapsible items
                                for (index_name, index) in doc_type.indexes() {
                                    let is_selected_index = *selected_index == Some(index.clone());

                                    let index_header_text = if is_selected_index {
                                        RichText::new(format!("Index: {}", index_name))
                                            .color(Color32::from_rgb(21, 101, 192))
                                    } else {
                                        RichText::new(format!("Index: {}", index_name))
                                    };

                                    let index_resp = ui.collapsing(index_header_text, |ui| {
                                        // Show index properties if expanded
                                        for prop in &index.properties {
                                            ui.label(format!("{:?}", prop));
                                        }
                                    });

                                    // Handle toggling of index
                                    // If the index is selected (expanded), build a WHERE clause for all properties:
                                    if index_resp.header_response.clicked()
                                        && index_resp.body_response.is_some()
                                    {
                                        *selected_index = Some(index.clone());
                                        if let Ok(new_doc_type) = contract
                                            .contract
                                            .document_type_cloned_for_name(&doc_name)
                                        {
                                            *selected_document_type = new_doc_type;
                                            *selected_data_contract = contract.clone();

                                            // Build the WHERE clause using all property names
                                            let conditions: Vec<String> = index
                                                .property_names()
                                                .iter()
                                                .map(|property_name| {
                                                    format!("`{}` = '___'", property_name)
                                                })
                                                .collect();

                                            let where_clause = if conditions.is_empty() {
                                                String::new()
                                            } else {
                                                format!(" WHERE {}", conditions.join(" AND "))
                                            };

                                            *document_query = format!(
                                                "SELECT * FROM {}{}",
                                                selected_document_type.name(),
                                                where_clause
                                            );
                                        }
                                    } else if index_resp.header_response.clicked()
                                        && index_resp.body_response.is_none()
                                    {
                                        // Index closed (collapsed)
                                        *selected_index = None;
                                        // Rebuild the query without index constraint
                                        *document_query = format!(
                                            "SELECT * FROM {}",
                                            selected_document_type.name()
                                        );
                                    }
                                }
                            });

                            // Check doc type toggling
                            if doc_resp.header_response.clicked()
                                && doc_resp.body_response.is_some()
                            {
                                if let Ok(new_doc_type) =
                                    contract.contract.document_type_cloned_for_name(&doc_name)
                                {
                                    *selected_document_type = new_doc_type;
                                    *selected_data_contract = contract.clone();
                                    *selected_index = None;
                                    *document_query =
                                        format!("SELECT * FROM {}", selected_document_type.name());
                                }
                            } else if doc_resp.header_response.clicked()
                                && doc_resp.body_response.is_none()
                            {
                                // Doc type collapsed again: still have doc type & contract
                                // required, so do not clear them. Just clear index if any.
                                *selected_index = None;
                                *document_query =
                                    format!("SELECT * FROM {}", selected_document_type.name());
                            }
                        }
                    });

                    ui.add_space(5.0);
                }
            });
        });

    action
}
