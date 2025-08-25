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
    pub expanded_contracts: std::collections::HashSet<String>,
    pub expanded_sections: std::collections::HashMap<String, std::collections::HashSet<String>>,
    pub expanded_doc_types: std::collections::HashMap<String, std::collections::HashSet<String>>,
    pub expanded_indexes: std::collections::HashMap<String, std::collections::HashSet<String>>,
    pub expanded_tokens: std::collections::HashMap<String, std::collections::HashSet<String>>,
}

impl Default for ContractChooserState {
    fn default() -> Self {
        Self {
            right_click_contract_id: None,
            show_context_menu: false,
            context_menu_position: egui::Pos2::ZERO,
            expanded_contracts: std::collections::HashSet::new(),
            expanded_sections: std::collections::HashMap::new(),
            expanded_doc_types: std::collections::HashMap::new(),
            expanded_indexes: std::collections::HashMap::new(),
            expanded_tokens: std::collections::HashMap::new(),
        }
    }
}

// Helper function to render a custom collapsing header with +/- button
fn render_collapsing_header(
    ui: &mut egui::Ui,
    text: impl Into<String>,
    is_expanded: bool,
    is_selected: bool,
    indent_level: usize,
) -> bool {
    let text = text.into();
    let dark_mode = ui.ctx().style().visuals.dark_mode;
    let indent = indent_level as f32 * 16.0;

    let mut clicked = false;

    ui.horizontal(|ui| {
        ui.add_space(indent);

        // +/- button
        let button_text = if is_expanded { "−" } else { "+" };
        let button_response = ui.add(
            egui::Button::new(
                RichText::new(button_text)
                    .size(20.0)
                    .color(DashColors::DASH_BLUE),
            )
            .fill(Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE),
        );

        if button_response.clicked() {
            clicked = true;
        }

        // Label - make contract names (level 0) larger
        let label_text = if indent_level == 0 {
            // Contract names - make them the largest with heading font
            if is_selected {
                RichText::new(text)
                    .size(16.0)
                    .heading()
                    .color(DashColors::DASH_BLUE)
            } else {
                RichText::new(text)
                    .size(16.0)
                    .heading()
                    .color(DashColors::text_primary(dark_mode))
            }
        } else if indent_level == 1 {
            // Section headers (Document Types, Tokens, Contract JSON) - medium size
            if is_selected {
                RichText::new(text)
                    .size(14.0)
                    .heading()
                    .color(DashColors::DASH_BLUE)
            } else {
                RichText::new(text)
                    .size(14.0)
                    .heading()
                    .color(DashColors::text_primary(dark_mode))
            }
        } else if indent_level == 2 {
            // Document type names - smaller
            if is_selected {
                RichText::new(text)
                    .size(13.0)
                    .heading()
                    .color(DashColors::DASH_BLUE)
            } else {
                RichText::new(text)
                    .size(13.0)
                    .heading()
                    .color(DashColors::text_primary(dark_mode))
            }
        } else {
            // Indexes and other sub-items - smallest
            if is_selected {
                RichText::new(text)
                    .size(12.0)
                    .heading()
                    .color(DashColors::DASH_BLUE)
            } else {
                RichText::new(text)
                    .size(12.0)
                    .heading()
                    .color(DashColors::text_primary(dark_mode))
            }
        };

        let label_response = ui.add(egui::Label::new(label_text).sense(egui::Sense::click()));
        if label_response.clicked() {
            clicked = true;
        }
    });

    clicked
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
                        ui.vertical_centered(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0; // Remove vertical spacing between contracts

                            for contract in filtered_contracts {
                                let contract_id = contract.contract.id().to_string(Encoding::Base58);
                                let is_selected_contract = *selected_data_contract == *contract;

                                // Format built-in contract names nicely
                                let display_name = match contract.alias.as_deref() {
                                    Some("dpns") => "DPNS".to_string(),
                                    Some("keyword_search") => "Keyword Search".to_string(),
                                    Some("token_history") => "Token History".to_string(),
                                    Some("withdrawals") => "Withdrawals".to_string(),
                                    Some("dashpay") => "DashPay".to_string(),
                                    Some(alias) => alias.to_string(),
                                    None => contract_id.clone(),
                                };

                                // Check if this contract is expanded
                                let is_expanded = chooser_state.expanded_contracts.contains(&contract_id);

                                // Render the custom collapsing header for the contract
                                if render_collapsing_header(ui, &display_name, is_expanded, is_selected_contract, 0) {
                                    if is_expanded {
                                        chooser_state.expanded_contracts.remove(&contract_id);
                                    } else {
                                        chooser_state.expanded_contracts.insert(contract_id.clone());
                                    }
                                }

                                // Show contract content if expanded
                                if is_expanded {
                                    ui.push_id(&contract_id, |ui| {
                                        ui.vertical(|ui| {
                                            //
                                            // ===== Document Types Section =====
                                            //
                                            // Only show Document Types section if there are document types
                                            if !contract.contract.document_types().is_empty() {
                                                let doc_types_key = format!("{}_doc_types", contract_id);
                                                let doc_types_expanded = chooser_state.expanded_sections
                                                    .get(&contract_id)
                                                    .map(|s| s.contains(&doc_types_key))
                                                    .unwrap_or(false);

                                                if render_collapsing_header(ui, "Document Types", doc_types_expanded, false, 1) {
                                                    let sections = chooser_state.expanded_sections
                                                        .entry(contract_id.clone())
                                                        .or_default();
                                                    if doc_types_expanded {
                                                        sections.remove(&doc_types_key);
                                                    } else {
                                                        sections.insert(doc_types_key.clone());
                                                    }
                                                }

                                                if doc_types_expanded {
                                                ui.vertical(|ui| {
                                                    for (doc_name, doc_type) in contract.contract.document_types() {
                                                        let is_selected_doc_type = *selected_document_type == *doc_type;
                                                        let doc_type_key = format!("{}_{}", contract_id, doc_name);

                                                        let doc_expanded = chooser_state.expanded_doc_types
                                                            .get(&contract_id)
                                                            .map(|s| s.contains(&doc_type_key))
                                                            .unwrap_or(false);

                                                        if render_collapsing_header(ui, doc_name, doc_expanded, is_selected_doc_type, 2) {
                                                            let doc_types = chooser_state.expanded_doc_types
                                                                .entry(contract_id.clone())
                                                                .or_default();
                                                            if doc_expanded {
                                                                doc_types.remove(&doc_type_key);
                                                                // Document Type collapsed
                                                                *selected_index = None;
                                                                *document_query = format!("SELECT * FROM {}", selected_document_type.name());
                                                            } else {
                                                                doc_types.insert(doc_type_key.clone());
                                                                // Document Type expanded
                                                                if let Ok(new_doc_type) = contract.contract.document_type_cloned_for_name(doc_name) {
                                                                    *pending_document_type = new_doc_type.clone();
                                                                    *selected_document_type = new_doc_type.clone();
                                                                    *selected_data_contract = contract.clone();
                                                                    *selected_index = None;
                                                                    *document_query = format!("SELECT * FROM {}", selected_document_type.name());

                                                                    // Reinitialize field selection
                                                                    pending_fields_selection.clear();

                                                                    // Mark doc-defined fields
                                                                    for (field_name, _schema) in new_doc_type.properties().iter() {
                                                                        pending_fields_selection.insert(field_name.clone(), true);
                                                                    }
                                                                    // Show "internal" fields as unchecked by default,
                                                                    // except for $ownerId and $id, which are checked
                                                                    for dash_field in DOCUMENT_PRIVATE_FIELDS {
                                                                        let checked = *dash_field == "$ownerId" || *dash_field == "$id";
                                                                        pending_fields_selection.insert(dash_field.to_string(), checked);
                                                                    }
                                                                }
                                                            }
                                                        }

                                                        if doc_expanded {
                                                            ui.vertical(|ui| {
                                                                // Show the indexes
                                                                if doc_type.indexes().is_empty() {
                                                                    ui.add_space(4.0);
                                                                    ui.label("No indexes defined");
                                                                } else {
                                                                    for (index_name, index) in doc_type.indexes() {
                                                                        let is_selected_index = *selected_index == Some(index.clone());
                                                                        let index_key = format!("{}_{}_{}", contract_id, doc_name, index_name);

                                                                        let index_expanded = chooser_state.expanded_indexes
                                                                            .get(&contract_id)
                                                                            .map(|s| s.contains(&index_key))
                                                                            .unwrap_or(false);

                                                                        let index_label = format!("Index: {}", index_name);
                                                                        if render_collapsing_header(ui, &index_label, index_expanded, is_selected_index, 3) {
                                                                            let indexes = chooser_state.expanded_indexes
                                                                                .entry(contract_id.clone())
                                                                                .or_default();
                                                                            if index_expanded {
                                                                                indexes.remove(&index_key);
                                                                                // Index collapsed
                                                                                *selected_index = None;
                                                                                *document_query = format!("SELECT * FROM {}", selected_document_type.name());
                                                                            } else {
                                                                                indexes.insert(index_key.clone());
                                                                                // Index expanded
                                                                                *selected_index = Some(index.clone());
                                                                                if let Ok(new_doc_type) = contract.contract.document_type_cloned_for_name(doc_name) {
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
                                                                            }
                                                                        }

                                                                        if index_expanded {
                                                                            ui.vertical(|ui| {
                                                                                ui.add_space(4.0);
                                                                                for prop in &index.properties {
                                                                                    ui.horizontal(|ui| {
                                                                                        ui.add_space(64.0);
                                                                                        ui.label(format!("{:?}", prop));
                                                                                    });
                                                                                }
                                                                            });
                                                                        }
                                                                    }
                                                                }
                                                            });
                                                        }
                                                    }
                                                });
                                                }
                                            }

                                            //
                                            // ===== Tokens Section =====
                                            //
                                            // Only show Tokens section if there are tokens
                                            let tokens_map = contract.contract.tokens();
                                            if !tokens_map.is_empty() {
                                                let tokens_key = format!("{}_tokens", contract_id);
                                                let tokens_expanded = chooser_state.expanded_sections
                                                    .get(&contract_id)
                                                    .map(|s| s.contains(&tokens_key))
                                                    .unwrap_or(false);

                                                if render_collapsing_header(ui, "Tokens", tokens_expanded, false, 1) {
                                                    let sections = chooser_state.expanded_sections
                                                        .entry(contract_id.clone())
                                                        .or_default();
                                                    if tokens_expanded {
                                                        sections.remove(&tokens_key);
                                                    } else {
                                                        sections.insert(tokens_key.clone());
                                                    }
                                                }

                                                if tokens_expanded {
                                                    ui.vertical(|ui| {
                                                        for (token_name, token) in tokens_map {
                                                            let token_key = format!("{}_token_{}", contract_id, token_name);
                                                            let token_expanded = chooser_state.expanded_tokens
                                                                .get(&contract_id)
                                                                .map(|s| s.contains(&token_key))
                                                                .unwrap_or(false);

                                                            if render_collapsing_header(ui, token_name.to_string(), token_expanded, false, 2) {
                                                                let tokens = chooser_state.expanded_tokens
                                                                    .entry(contract_id.clone())
                                                                    .or_default();
                                                                if token_expanded {
                                                                    tokens.remove(&token_key);
                                                                } else {
                                                                    tokens.insert(token_key.clone());
                                                                }
                                                            }

                                                            if token_expanded {
                                                                ui.vertical(|ui| {
                                                                    ui.add_space(4.0);
                                                                    ui.horizontal(|ui| {
                                                                        ui.add_space(32.0);
                                                                        ui.label(format!("Base Supply: {}", token.base_supply()));
                                                                    });
                                                                    ui.horizontal(|ui| {
                                                                        ui.add_space(32.0);
                                                                        if let Some(max_supply) = token.max_supply() {
                                                                            ui.label(format!("Max Supply: {}", max_supply));
                                                                        } else {
                                                                            ui.label("Max Supply: None");
                                                                        }
                                                                    });
                                                                });
                                                            }
                                                        }
                                                    });
                                                }
                                            }

                                            //
                                            // ===== Entire Contract JSON =====
                                            //
                                            let json_key = format!("{}_json", contract_id);
                                            let json_expanded = chooser_state.expanded_sections
                                                .get(&contract_id)
                                                .map(|s| s.contains(&json_key))
                                                .unwrap_or(false);

                                            if render_collapsing_header(ui, "Contract JSON", json_expanded, false, 1) {
                                                let sections = chooser_state.expanded_sections
                                                    .entry(contract_id.clone())
                                                    .or_default();
                                                if json_expanded {
                                                    sections.remove(&json_key);
                                                } else {
                                                    sections.insert(json_key.clone());
                                                }
                                            }

                                            if json_expanded {
                                                ui.vertical(|ui| {
                                                    match contract.contract.to_json(app_context.platform_version()) {
                                                        Ok(json_value) => {
                                                            let pretty_str = serde_json::to_string_pretty(&json_value)
                                                                .unwrap_or_else(|_| "Error formatting JSON".to_string());

                                                            ui.add_space(2.0);

                                                            // A resizable region that the user can drag to expand/shrink
                                                            egui::Resize::default()
                                                                .id_salt(format!("json_resize_{}", contract_id))
                                                                .default_size([400.0, 400.0])
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
                                                            ui.label(format!("Error converting contract to JSON: {e}"));
                                                        }
                                                    }
                                                });
                                            }
                                        });

                                        // Check for right-click on the contract header
                                        // TODO: Add right-click support to custom header if needed

                                        // Right‐aligned Remove button
                                        ui.horizontal(|ui| {
                                            ui.add_space(8.0);
                                            if contract.alias != Some("dpns".to_string())
                                                && contract.alias != Some("token_history".to_string())
                                                && contract.alias != Some("withdrawals".to_string())
                                                && contract.alias != Some("keyword_search".to_string())
                                                && ui.add(
                                                    egui::Button::new("Remove")
                                                        .min_size(egui::Vec2::new(60.0, 20.0))
                                                        .small()
                                                ).clicked()
                                            {
                                                action |= AppAction::BackendTask(
                                                    BackendTask::ContractTask(Box::new(
                                                        ContractTask::RemoveContract(contract.contract.id()),
                                                    )),
                                                );
                                            }
                                        });
                                    });
                                }
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
