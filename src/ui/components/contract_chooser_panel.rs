use crate::app::AppAction;
use crate::context::AppContext;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use egui::{Context, Frame, Margin, SidePanel};
use std::sync::Arc;
pub fn add_contract_chooser_panel(
    ctx: &Context,
    current_search_term: &mut String,
    app_context: &Arc<AppContext>,
) -> AppAction {
    let mut action = AppAction::None;

    // Fetch contracts from the app context
    let contracts = app_context.get_contracts(None, None).unwrap_or_else(|e| {
        eprintln!("Error fetching contracts: {}", e);
        vec![]
    });

    // Filter the contracts based on the search term
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
            // Search bar at the top
            ui.horizontal(|ui| {
                ui.label("Search:");
                ui.text_edit_singleline(current_search_term);
            });

            ui.separator(); // Separator below the search bar

            // Display filtered contracts with nested document types and indexes
            ui.vertical(|ui| {
                for contract in filtered_contracts {
                    let name_or_id = contract
                        .alias
                        .clone()
                        .unwrap_or(contract.contract.id().to_string(Encoding::Base58));

                    // Expandable contract section
                    ui.collapsing(name_or_id, |ui| {
                        // Loop over the document types in the contract
                        for (doc_name, doc_type) in contract.contract.document_types() {
                            // Expandable section for each document type
                            ui.collapsing(doc_name, |ui| {
                                // Loop over the indexes in the document type
                                for index in doc_type.indexes().values() {
                                    ui.label(format!("Index: {}", index.name));
                                    ui.indent("index_properties", |ui| {
                                        for prop in &index.properties {
                                            ui.label(format!("Property: {:?}", prop));
                                        }
                                    });
                                }
                            });
                        }
                    });

                    ui.add_space(5.0); // Spacing between contracts
                }
            });
        });

    action
}
