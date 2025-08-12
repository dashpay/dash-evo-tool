use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::contract::ContractTask;
use crate::context::AppContext;
use crate::model::qualified_contract::QualifiedContract;
use crate::ui::contracts_documents::contracts_documents_screen::DOCUMENT_PRIVATE_FIELDS;
use crate::ui::theme::{DashColors, Shadow, Shape, Spacing};
use dash_sdk::dpp::data_contract::accessors::v1::DataContractV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
use dash_sdk::dpp::data_contract::document_type::Index;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::data_contract::{
    accessors::v0::DataContractV0Getters, document_type::DocumentType,
};
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::serialization::PlatformSerializableWithPlatformVersion;
use egui::{Color32, Context as EguiContext, Frame, Margin, RichText, SidePanel};
use std::collections::HashMap;
use std::sync::Arc;

pub struct ContractChooserState {
    pub right_click_contract_id: Option<String>,
    pub show_context_menu: bool,
    pub context_menu_position: egui::Pos2,
}

impl Default for ContractChooserState {
    fn default() -> Self {
        Self {
            right_click_contract_id: None,
            show_context_menu: false,
            context_menu_position: egui::Pos2::ZERO,
        }
    }
}

#[allow(clippy::too_many_arguments)]
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
    chooser_state: &mut ContractChooserState,
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

    let dark_mode = ctx.style().visuals.dark_mode;

    SidePanel::left("contract_chooser_panel")
        // Let the user resize this panel horizontally
        .resizable(true)
        .default_width(270.0) // Increased to account for margins
        .frame(
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin::symmetric(10, 10)), // Add margins for island effect
        )
        .show(ctx, |ui| {
            // Fill the entire available height
            let available_height = ui.available_height();

            // Create an island panel with rounded edges that fills the height
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .inner_margin(Margin::same(Spacing::MD_I8))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                .shadow(Shadow::elevated())
                .show(ui, |panel_ui| {
                    // Account for both outer margin (10px * 2) and inner margin
                    panel_ui.set_min_height(available_height - 2.0 - (Spacing::MD_I8 as f32 * 2.0));

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
                                ui.push_id(
                                    contract.contract.id().to_string(Encoding::Base58),
                                    |ui| {
                                        ui.horizontal(|ui| {
                                            let is_selected_contract =
                                                *selected_data_contract == *contract;

                                            let name_or_id = contract.alias.clone().unwrap_or(
                                                contract.contract.id().to_string(Encoding::Base58),
                                            );

                                            // Highlight the contract if selected
                                            let contract_header_text = if is_selected_contract {
                                                RichText::new(name_or_id)
                                                    .color(Color32::from_rgb(21, 101, 192))
                                            } else {
                                                RichText::new(name_or_id)
                                            };

                                            // Expand/collapse the contract info
                                            let collapsing_response =
                                                ui.collapsing(contract_header_text, |ui| {
                                                    //
                                                    // ===== Document Types Section =====
                                                    //
                                                    ui.collapsing("Document Types", |ui| {
                                                        for (doc_name, doc_type) in
                                                            contract.contract.document_types()
                                                        {
                                                            let is_selected_doc_type =
                                                                *selected_document_type
                                                                    == *doc_type;

                                                            let doc_type_header_text =
                                                                if is_selected_doc_type {
                                                                    RichText::new(doc_name.clone())
                                                                        .color(Color32::from_rgb(
                                                                            21, 101, 192,
                                                                        ))
                                                                } else {
                                                                    RichText::new(doc_name.clone())
                                                                };

                                                            let doc_resp =
                                                ui.collapsing(doc_type_header_text, |ui| {
                                                    // Show the indexes
                                                    if doc_type.indexes().is_empty() {
                                                        ui.label("No indexes defined");
                                                    } else {
                                                        for (index_name, index) in
                                                            doc_type.indexes()
                                                        {
                                                            let is_selected_index = *selected_index
                                                                == Some(index.clone());

                                                            let index_header_text =
                                                                if is_selected_index {
                                                                    RichText::new(format!(
                                                                        "Index: {}",
                                                                        index_name
                                                                    ))
                                                                    .color(Color32::from_rgb(
                                                                        21, 101, 192,
                                                                    ))
                                                                } else {
                                                                    RichText::new(format!(
                                                                        "Index: {}",
                                                                        index_name
                                                                    ))
                                                                };

                                                            let index_resp = ui.collapsing(
                                                                index_header_text,
                                                                |ui| {
                                                                    // Show index properties if expanded
                                                                    for prop in &index.properties {
                                                                        ui.label(format!(
                                                                            "{:?}",
                                                                            prop
                                                                        ));
                                                                    }
                                                                },
                                                            );

                                                            // If index was just clicked (opened)
                                                            if index_resp.header_response.clicked()
                                                                && index_resp
                                                                    .body_response
                                                                    .is_some()
                                                            {
                                                                *selected_index =
                                                                    Some(index.clone());
                                                                if let Ok(new_doc_type) = contract
                                                                    .contract
                                                                    .document_type_cloned_for_name(
                                                                        doc_name,
                                                                    )
                                                                {
                                                                    *selected_document_type =
                                                                        new_doc_type;
                                                                    *selected_data_contract =
                                                                        contract.clone();

                                                                    // Build the WHERE clause using all property names
                                                                    let conditions: Vec<String> =
                                                                        index
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
                                                                                conditions
                                                                                    .join(" AND ")
                                                                            )
                                                                        };

                                                                    *document_query = format!(
                                                                        "SELECT * FROM {}{}",
                                                                        selected_document_type
                                                                            .name(),
                                                                        where_clause
                                                                    );
                                                                }
                                                            }
                                                            // If index was just collapsed
                                                            else if index_resp
                                                                .header_response
                                                                .clicked()
                                                                && index_resp
                                                                    .body_response
                                                                    .is_none()
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
                                                                    .document_type_cloned_for_name(
                                                                        doc_name,
                                                                    )
                                                                {
                                                                    *pending_document_type =
                                                                        new_doc_type.clone();
                                                                    *selected_document_type =
                                                                        new_doc_type.clone();
                                                                    *selected_data_contract =
                                                                        contract.clone();
                                                                    *selected_index = None;
                                                                    *document_query = format!(
                                                                        "SELECT * FROM {}",
                                                                        selected_document_type
                                                                            .name()
                                                                    );

                                                                    // Reinitialize field selection
                                                                    pending_fields_selection
                                                                        .clear();

                                                                    // Mark doc-defined fields
                                                                    for (field_name, _schema) in
                                                                        new_doc_type
                                                                            .properties()
                                                                            .iter()
                                                                    {
                                                                        pending_fields_selection
                                                                            .insert(
                                                                                field_name.clone(),
                                                                                true,
                                                                            );
                                                                    }
                                                                    // Show "internal" fields as unchecked by default,
                                                                    // except for $ownerId and $id, which are checked
                                                                    for dash_field in
                                                                        DOCUMENT_PRIVATE_FIELDS
                                                                    {
                                                                        let checked = *dash_field
                                                                            == "$ownerId"
                                                                            || *dash_field == "$id";
                                                                        pending_fields_selection
                                                                            .insert(
                                                                                dash_field
                                                                                    .to_string(),
                                                                                checked,
                                                                            );
                                                                    }
                                                                }
                                                            }
                                                            // Document Type collapsed
                                                            else if doc_resp
                                                                .header_response
                                                                .clicked()
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
                                                            ui.label(
                                                            "No tokens defined for this contract.",
                                                        );
                                                        } else {
                                                            for (token_name, token) in tokens_map {
                                                                // Each token is its own collapsible
                                                                ui.collapsing(
                                                                    token_name.to_string(),
                                                                    |ui| {
                                                                        // Now you can display base supply, max supply, etc.
                                                                        ui.label(format!(
                                                                            "Base Supply: {}",
                                                                            token.base_supply()
                                                                        ));
                                                                        if let Some(max_supply) =
                                                                            token.max_supply()
                                                                        {
                                                                            ui.label(format!(
                                                                                "Max Supply: {}",
                                                                                max_supply
                                                                            ));
                                                                        } else {
                                                                            ui.label(
                                                                                "Max Supply: None",
                                                                            );
                                                                        }

                                                                        // Add more details here
                                                                    },
                                                                );
                                                            }
                                                        }
                                                    });

                                                    //
                                                    // ===== Entire Contract JSON =====
                                                    //
                                                    ui.collapsing("Contract JSON", |ui| {
                                                        match contract
                                                            .contract
                                                            .to_json(app_context.platform_version())
                                                        {
                                                            Ok(json_value) => {
                                                                let pretty_str =
                                                                    serde_json::to_string_pretty(
                                                                        &json_value,
                                                                    )
                                                                    .unwrap_or_else(|_| {
                                                                        "Error formatting JSON"
                                                                            .to_string()
                                                                    });

                                                                ui.add_space(2.0);

                                                                // A resizable region that the user can drag to expand/shrink
                                                                egui::Resize::default()
                                                                .id_salt(
                                                                    "json_resize_area_for_contract",
                                                                )
                                                                .default_size([400.0, 400.0]) // initial w,h
                                                                .show(ui, |ui| {
                                                                    egui::ScrollArea::vertical()
                                                                        .auto_shrink([false; 2])
                                                                        .show(ui, |ui| {
                                                                            ui.monospace(
                                                                                pretty_str,
                                                                            );
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

                                            // Check for right-click on the contract header
                                            if collapsing_response
                                                .header_response
                                                .secondary_clicked()
                                            {
                                                let contract_id = contract
                                                    .contract
                                                    .id()
                                                    .to_string(Encoding::Base58);
                                                chooser_state.right_click_contract_id =
                                                    Some(contract_id);
                                                chooser_state.show_context_menu = true;
                                                chooser_state.context_menu_position = ui
                                                    .ctx()
                                                    .pointer_interact_pos()
                                                    .unwrap_or(egui::Pos2::ZERO);
                                            }

                                            // Right‐aligned Remove button
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.add_space(2.0); // Push down a few pixels
                                                    if contract.alias != Some("dpns".to_string())
                                                        && contract.alias
                                                            != Some("token_history".to_string())
                                                        && contract.alias
                                                            != Some("withdrawals".to_string())
                                                        && contract.alias
                                                            != Some("keyword_search".to_string())
                                                        && ui
                                                            .add(
                                                                egui::Button::new("X")
                                                                    .min_size(egui::Vec2::new(
                                                                        20.0, 20.0,
                                                                    ))
                                                                    .small(),
                                                            )
                                                            .clicked()
                                                    {
                                                        action |= AppAction::BackendTask(
                                                            BackendTask::ContractTask(Box::new(
                                                                ContractTask::RemoveContract(
                                                                    contract.contract.id(),
                                                                ),
                                                            )),
                                                        );
                                                    }
                                                },
                                            );
                                        });
                                    },
                                );
                            }
                        });
                    });
                }); // Close the island frame
        });

    // Show context menu if right-clicked
    if chooser_state.show_context_menu {
        if let Some(ref contract_id_str) = chooser_state.right_click_contract_id {
            // Find the contract that was right-clicked
            let contract_opt = contracts
                .iter()
                .find(|c| c.contract.id().to_string(Encoding::Base58) == *contract_id_str);

            if let Some(contract) = contract_opt {
                egui::Window::new("Contract Menu")
                    .id(egui::Id::new("contract_context_menu"))
                    .title_bar(false)
                    .resizable(false)
                    .collapsible(false)
                    .fixed_pos(chooser_state.context_menu_position)
                    .show(ctx, |ui| {
                        ui.set_min_width(150.0);

                        // Copy Hex option
                        if ui.button("Copy (Hex)").clicked() {
                            // Serialize contract to bytes
                            if let Ok(bytes) =
                                contract.contract.serialize_to_bytes_with_platform_version(
                                    app_context.platform_version(),
                                )
                            {
                                let hex_string = hex::encode(&bytes);
                                ui.ctx().copy_text(hex_string);
                            }
                            chooser_state.show_context_menu = false;
                        }

                        // Copy JSON option
                        if ui.button("Copy (JSON)").clicked() {
                            // Convert contract to JSON
                            if let Ok(json_value) =
                                contract.contract.to_json(app_context.platform_version())
                            {
                                if let Ok(json_string) = serde_json::to_string_pretty(&json_value) {
                                    ui.ctx().copy_text(json_string);
                                }
                            }
                            chooser_state.show_context_menu = false;
                        }
                    });

                // Close menu if clicked elsewhere
                if ctx.input(|i| i.pointer.any_click()) {
                    // Check if click was outside the menu
                    let menu_rect = egui::Rect::from_min_size(
                        chooser_state.context_menu_position,
                        egui::vec2(150.0, 70.0), // Approximate size
                    );
                    if let Some(pointer_pos) = ctx.pointer_interact_pos() {
                        if !menu_rect.contains(pointer_pos) {
                            chooser_state.show_context_menu = false;
                        }
                    }
                }
            }
        }
    }

    action
}
