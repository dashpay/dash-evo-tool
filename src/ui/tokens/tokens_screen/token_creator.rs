use std::collections::{BTreeMap, HashSet};
use chrono::Utc;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::{TokenConfigurationPreset, TokenConfigurationPresetFeatures};
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPresetFeatures::{MostRestrictive, WithAllAdvancedActions, WithExtremeActions, WithMintingAndBurningActions, WithOnlyEmergencyAction};
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::change_control_rules::v0::ChangeControlRulesV0;
use dash_sdk::dpp::data_contract::change_control_rules::ChangeControlRules;
use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::epaint::Color32;
use egui::{ComboBox, Context, Frame, Margin, RichText, TextEdit, Ui};
use crate::ui::theme::DashColors;
use crate::ui::ScreenType;
use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::BackendTask;
use crate::backend_task::tokens::TokenTask;
use crate::ui::components::styled::{StyledCheckbox};
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::Component;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::helpers::{add_identity_key_chooser, TransactionType};
use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use crate::ui::tokens::tokens_screen::{TokenBuildArgs, TokenCreatorStatus, TokenNameLanguage, TokensScreen, ChangeControlRulesUI};

impl TokensScreen {
    pub(super) fn render_token_creator(&mut self, context: &Context, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // 1) If we've successfully completed contract creation, show a success UI
        if self.token_creator_status == TokenCreatorStatus::Complete {
            self.render_token_creator_success_screen(ui);
            return action;
        }

        // Heading with checkbox on the same line
        ui.horizontal(|ui| {
            ui.heading("Token Creator");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(
                    &mut self.show_advanced_token_creator,
                    "Show Advanced Options",
                );
            });
        });
        ui.add_space(5.0);
        if self.show_advanced_token_creator {
            ui.label(
                "Create custom tokens on Dash Platform with advanced features and distribution rules.",
            );
        } else {
            ui.label(
                "Create a simple token on Dash Platform. Enable Advanced Options for more control.",
            );
        }
        ui.add_space(10.0);

        let mut load_identity_clicked = false;

        egui::ScrollArea::horizontal()
            .show(ui, |ui| {
                // Stretch the panel to fill the available width
                ui.set_min_width(ui.available_width());
                ui.set_max_width(ui.available_width());
                        // Identity and key selection
                        ui.add_space(10.0);
                        let all_identities = match self.app_context.load_local_user_identities() {
                            Ok(identities) => identities.into_iter().filter(|qi| !qi.private_keys.private_keys.is_empty()).collect::<Vec<_>>(),
                            Err(e) => {
                                tracing::error!(err=?e, "Error loading identities from local DB.");
                                ui.colored_label(Color32::DARK_RED,format!("Error loading identities from local DB: {}", e));
                                return;
                            }
                        };
                        if all_identities.is_empty() {
                            let dark_mode = ui.ctx().style().visuals.dark_mode;
                            Frame::group(ui.style())
                                .fill(ui.visuals().extreme_bg_color)
                                .corner_radius(5.0)
                                .outer_margin(Margin::same(20))
                                .shadow(ui.visuals().window_shadow)
                                .show(ui, |ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.add_space(5.0);
                                        ui.label(
                                            RichText::new("No Identities Loaded")
                                                .strong()
                                                .size(25.0)
                                                .color(DashColors::text_primary(dark_mode)),
                                        );

                                        ui.add_space(5.0);
                                        ui.separator();
                                        ui.add_space(10.0);

                                        ui.label(
                                            "To create a token, you need to load or create an identity first.",
                                        );

                                        ui.add_space(10.0);

                                        ui.heading(
                                            RichText::new("Here's what you can do:")
                                                .strong()
                                                .size(18.0)
                                                .color(DashColors::text_primary(dark_mode)),
                                        );
                                        ui.add_space(5.0);

                                        ui.label("- LOAD an existing identity by clicking the button below, or");
                                        ui.add_space(1.0);
                                        ui.label("- CREATE a new identity from the Identities screen after setting up a wallet.");

                                        ui.add_space(15.0);

                                        let button = egui::Button::new(
                                            RichText::new("Load Identity")
                                                .color(egui::Color32::WHITE)
                                                .strong(),
                                        )
                                        .fill(DashColors::DASH_BLUE)
                                        .min_size(egui::vec2(150.0, 36.0));

                                        if ui.add(button).clicked() {
                                            load_identity_clicked = true;
                                        }

                                        ui.add_space(10.0);
                                    });
                                });
                            return;
                        }

                        // Branch: Simple mode vs Advanced mode for identity/key selection
                        if !self.show_advanced_token_creator {
                            // =====================================================
                            // SIMPLE MODE - Identity selector only (no key selector)
                            // =====================================================
                            ui.heading("1. Select an identity:");
                            ui.add_space(5.0);

                            // Use IdentitySelector for simple mode
                            let response = ui.add(
                                IdentitySelector::new(
                                    "simple_identity_selector",
                                    &mut self.identity_id_string,
                                    &all_identities,
                                )
                                .selected_identity(&mut self.selected_identity)
                                .expect("selected_identity should not fail")
                                .other_option(false)
                                .label("Identity:")
                                .width(300.0),
                            );

                            // Auto-select the first eligible key when:
                            // 1. Identity changed, OR
                            // 2. Identity is selected but no key is selected yet (first load)
                            let should_auto_select_key = response.changed()
                                || (self.selected_identity.is_some() && self.selected_key.is_none());

                            if should_auto_select_key {
                                if response.changed() {
                                    self.selected_key = None; // Clear previous key only on identity change
                                }
                                if let Some(ref identity) = self.selected_identity {
                                    // Find first eligible key for RegisterContract
                                    // Requires Authentication purpose with High or Critical security level
                                    let first_eligible_key = identity
                                        .private_keys
                                        .identity_public_keys()
                                        .iter()
                                        .find(|key_ref| {
                                            let key = &key_ref.1.identity_public_key;
                                            key.purpose() == Purpose::AUTHENTICATION
                                                && (key.security_level() == SecurityLevel::CRITICAL
                                                    || key.security_level() == SecurityLevel::HIGH)
                                        })
                                        .map(|key_ref| key_ref.1.identity_public_key.clone());

                                    if first_eligible_key.is_some() {
                                        self.selected_key = first_eligible_key;
                                    }
                                }
                            }

                            // If identity is selected but no eligible key could be found, show warning
                            if self.selected_identity.is_some() && self.selected_key.is_none() {
                                ui.add_space(5.0);
                                ui.colored_label(
                                    egui::Color32::from_rgb(200, 100, 100),
                                    "No eligible key found for this identity. Please use Advanced Options or add a suitable key.",
                                );
                                return;
                            }

                            if self.selected_identity.is_none() {
                                return;
                            }

                            // Set wallet reference for the auto-selected key
                            self.update_selected_wallet();

                            ui.add_space(10.0);
                            ui.separator();

                            // Wallet unlock check for simple mode
                            if !self.ensure_wallet_unlocked(ui) {
                                return;
                            }
                        } else {
                            // =====================================================
                            // ADVANCED MODE - Full identity and key selection
                            // =====================================================
                            ui.heading("1. Select an identity and key to register the token contract with:");
                            ui.add_space(5.0);

                            // Use the helper function for identity and key selection
                            add_identity_key_chooser(
                                ui,
                                &self.app_context,
                                all_identities.iter(),
                                &mut self.selected_identity,
                                &mut self.selected_key,
                                TransactionType::RegisterContract,
                            );

                            ui.add_space(5.0);

                            // If a key was selected, set the wallet reference
                            self.update_selected_wallet();

                            if self.selected_key.is_none() {
                                return;
                            }

                            ui.add_space(10.0);
                            ui.separator();

                            // Wallet unlock check for advanced mode
                            if !self.ensure_wallet_unlocked(ui) {
                                return;
                            }
                        }

                        // Continue with mode-specific content
                        if !self.show_advanced_token_creator {
                            // =====================================================
                            // SIMPLE MODE - Beginner-friendly options with info icons
                            // =====================================================
                            ui.add_space(10.0);
                            ui.heading("2. Enter token details:");
                            ui.add_space(5.0);

                            egui::Grid::new("simple_token_info_grid")
                                .num_columns(2)
                                .spacing([8.0, 8.0])
                                .show(ui, |ui| {
                                    // Token Name
                                    ui.horizontal(|ui| {
                                        ui.label("Token Name*:");
                                        if crate::ui::helpers::info_icon_button(ui,
                                            "The name of your token (e.g., 'MyCoin', 'GameToken').\n\n\
                                            This is how your token will be displayed to users.\n\n\
                                            Must be between 3 and 50 characters.").clicked() {
                                            self.show_pop_up_info = Some(
                                                "Token Name\n\n\
                                                The name of your token (e.g., 'MyCoin', 'GameToken').\n\n\
                                                This is how your token will be displayed to users.\n\n\
                                                Must be between 3 and 50 characters.".to_string()
                                            );
                                        }
                                    });
                                    ui.text_edit_singleline(&mut self.token_names_input[0].0);
                                    ui.end_row();

                                    // Token Description
                                    ui.horizontal(|ui| {
                                        ui.label("Description:");
                                        if crate::ui::helpers::info_icon_button(ui,
                                            "An optional description explaining what your token is for.\n\n\
                                            This helps users understand the purpose of your token.\n\n\
                                            Maximum 100 characters.").clicked() {
                                            self.show_pop_up_info = Some(
                                                "Description\n\n\
                                                An optional description explaining what your token is for.\n\n\
                                                This helps users understand the purpose of your token.\n\n\
                                                Maximum 100 characters.".to_string()
                                            );
                                        }
                                    });
                                    ui.text_edit_singleline(&mut self.token_description_input);
                                    ui.end_row();

                                    // Initial Supply
                                    ui.horizontal(|ui| {
                                        ui.label("Initial Supply*:");
                                        if crate::ui::helpers::info_icon_button(ui,
                                            "The number of tokens to create when the token is registered.\n\n\
                                            These tokens will be owned by you (the token creator).\n\n\
                                            You can mint more tokens later if minting is enabled.").clicked() {
                                            self.show_pop_up_info = Some(
                                                "Initial Supply\n\n\
                                                The number of tokens to create when the token is registered.\n\n\
                                                These tokens will be owned by you (the token creator).\n\n\
                                                You can mint more tokens later if minting is enabled.".to_string()
                                            );
                                        }
                                    });
                                    self.render_base_supply_input(ui);
                                    ui.end_row();

                                    // Max Supply
                                    ui.horizontal(|ui| {
                                        ui.label("Max Supply:");
                                        if crate::ui::helpers::info_icon_button(ui,
                                            "The maximum number of tokens that can ever exist.\n\n\
                                            Leave empty or set to 0 for no maximum (unlimited supply).\n\n\
                                            Once set, this cannot be increased.").clicked() {
                                            self.show_pop_up_info = Some(
                                                "Max Supply\n\n\
                                                The maximum number of tokens that can ever exist.\n\n\
                                                Leave empty or set to 0 for no maximum (unlimited supply).\n\n\
                                                Once set, this cannot be increased.".to_string()
                                            );
                                        }
                                    });
                                    self.render_max_supply_input(ui);
                                    ui.end_row();

                                    // Preset selector
                                    ui.vertical(|ui| {
                                        ui.add_space(15.0);
                                        ui.horizontal(|ui| {
                                            ui.label("Token Preset*:");
                                            if crate::ui::helpers::info_icon_button(ui,
                                                "Choose a preset that determines what actions are allowed on your token.\n\n\
                                            Click for more details on each preset.").clicked() {
                                                self.show_pop_up_info = Some(
                                                    "Token Presets\n\n\
                                                Presets control what actions can be performed on your token after creation:\n\n\
                                                - Most Restrictive: No additional actions allowed. Token is fixed after creation. Best for simple, immutable tokens.\n\n\
                                                - Only Emergency Action: Allows pausing/unpausing the token in emergencies. Good for tokens that need a safety mechanism.\n\n\
                                                - Minting and Burning: Allows creating new tokens (minting) and destroying tokens (burning). Good for flexible supply tokens.\n\n\
                                                - Advanced Actions: Allows minting, burning, freezing accounts, and more. For tokens needing moderation capabilities.\n\n\
                                                - All Allowed: All actions enabled including destroying frozen funds. Maximum flexibility but requires careful management.".to_string()
                                                );
                                            }
                                        });
                                    });
                                    ComboBox::from_id_salt("simple_preset_selector")
                                        .width(200.0)
                                        .selected_text(
                                            self.selected_token_preset
                                                .map(|p| match p {
                                                    MostRestrictive => "Most Restrictive",
                                                    WithOnlyEmergencyAction => "Only Emergency Action",
                                                    WithMintingAndBurningActions => "Minting and Burning",
                                                    WithAllAdvancedActions => "Advanced Actions",
                                                    WithExtremeActions => "All Allowed",
                                                })
                                                .unwrap_or("Select a preset..."),
                                        )
                                        .show_ui(ui, |ui| {
                                            for variant in [
                                                MostRestrictive,
                                                WithOnlyEmergencyAction,
                                                WithMintingAndBurningActions,
                                                WithAllAdvancedActions,
                                                WithExtremeActions,
                                            ] {
                                                let (text, description) = match variant {
                                                    MostRestrictive => ("Most Restrictive", "No actions allowed after creation"),
                                                    WithOnlyEmergencyAction => ("Only Emergency Action", "Can pause/unpause token"),
                                                    WithMintingAndBurningActions => ("Minting and Burning", "Can mint and burn tokens"),
                                                    WithAllAdvancedActions => ("Advanced Actions", "Mint, burn, freeze, and more"),
                                                    WithExtremeActions => ("All Allowed", "All actions enabled"),
                                                };
                                                if ui.selectable_value(
                                                    &mut self.selected_token_preset,
                                                    Some(variant),
                                                    format!("{} - {}", text, description),
                                                ).clicked() {
                                                    let preset = TokenConfigurationPreset {
                                                        features: variant,
                                                        action_taker: AuthorizedActionTakers::ContractOwner,
                                                    };
                                                    self.change_to_preset(preset);
                                                }
                                            }
                                        });
                                    ui.end_row();
                                });

                            ui.add_space(20.0);

                            // Create Token button
                            let can_create = !self.token_names_input[0].0.trim().is_empty()
                                && self.base_supply_amount.is_some()
                                && self.selected_token_preset.is_some();

                            ui.horizontal(|ui| {
                                let button = egui::Button::new(
                                    RichText::new("Create Token")
                                        .color(egui::Color32::WHITE)
                                        .strong(),
                                )
                                .fill(if can_create {
                                    DashColors::DASH_BLUE
                                } else {
                                    egui::Color32::GRAY
                                })
                                .min_size(egui::vec2(150.0, 36.0));

                                if ui.add_enabled(can_create, button).clicked() {
                                    // Auto-set plural name if empty (singular + "s")
                                    let singular = self.token_names_input[0].0.trim().to_string();
                                    if self.token_names_input[0].1.trim().is_empty() {
                                        self.token_names_input[0].1 = format!("{}s", singular);
                                    }

                                    // Trigger the token creation confirmation
                                    match self.parse_token_build_args() {
                                        Ok(args) => {
                                            self.cached_build_args = Some(args);
                                            self.token_creator_error_message = None;
                                            self.show_token_creator_confirmation_popup = true;
                                        }
                                        Err(err_msg) => {
                                            self.token_creator_error_message = Some(err_msg);
                                        }
                                    }
                                }
                            });

                            if !can_create {
                                ui.add_space(5.0);
                                let missing = if self.token_names_input[0].0.trim().is_empty() {
                                    "token name"
                                } else if self.base_supply_amount.is_none() {
                                    "initial supply"
                                } else {
                                    "token preset"
                                };
                                ui.label(
                                    RichText::new(format!("Please select a {}", missing))
                                        .color(egui::Color32::GRAY)
                                        .italics(),
                                );
                            }
                        } else {
                            // =====================================================
                            // ADVANCED MODE - Full options
                            // =====================================================

                        // 4) Show input fields for token name, decimals, base supply, etc.
                        ui.add_space(10.0);
                        ui.heading("2. Enter basic token info:");
                        ui.add_space(5.0);

                        // Use `Grid` to align labels and text edits
                        egui::Grid::new("basic_token_info_grid")
                            .num_columns(2)
                            .spacing([16.0, 8.0]) // Horizontal, vertical spacing
                            .show(ui, |ui| {
                                // Row 1: Token Name
                                let mut token_to_remove: Option<u8> = None;
                                for i in 0..self.token_names_input.len() {
                                    ui.label("Token Name (singular)*:");
                                    ui.text_edit_singleline(&mut self.token_names_input[i].0);
                                    ui.horizontal(|ui| {
                                        let allow_all_languages = i != 0;
                                        ui.push_id(format!("combo_{}", i), |ui| {
                                            let combo_id = format!("token_name_language_selector_{}", i);
                                            Self::render_token_name_language_selector(
                                                ui,
                                                &mut self.token_names_input[i].2,
                                                allow_all_languages,
                                                &combo_id,
                                            );
                                        });

                                        if ui.add(egui::Button::new("➕ Add Language").small()).clicked() {
                                            let used_languages: HashSet<_> = self.token_names_input.iter().map(|(_, _, lang, _)| *lang).collect();
                                            let next_non_used_language = enum_iterator::all::<TokenNameLanguage>()
                                                .find(|lang| !used_languages.contains(lang))
                                                .unwrap_or(TokenNameLanguage::English);
                                            // Add a new token name input
                                            self.token_names_input.push((String::new(), String::new(), next_non_used_language, false));
                                        }
                                        if i != 0 && ui.add(egui::Button::new("➖").small()).clicked() {
                                            token_to_remove = Some(i.try_into().expect("Failed to convert index"));
                                        }

                                        // This is really ugly
                                        // StyledCheckbox::new(&mut self.token_names_input[i].3, "Keyword").show(ui);

                                        // let response = crate::ui::helpers::info_icon_button(ui, "Checking this box adds this token name to the contract keywords.\nEach searchable keyword costs 0.1 Dash.\n");
                                        // if response.clicked() {
                                        //     self.show_pop_up_info = Some("Checking this box adds this token name to the contract keywords.\nEach searchable keyword costs 0.1 Dash".to_string());
                                        // }
                                    });
                                    ui.end_row();

                                    // Plural name
                                    ui.label("Token Name (plural)*:");
                                    ui.text_edit_singleline(&mut self.token_names_input[i].1);
                                    ui.end_row();

                                }

                                if let Some(token) = token_to_remove {
                                    self.token_names_input.remove(token.into());
                                }

                                // Row 2: Base Supply
                                // We put label manually to comply with grid layout;
                                // errors will be rendered in second column
                                ui.label("Base Supply*:");
                                self.render_base_supply_input(ui);
                                ui.end_row();

                                // Row 3: Max Supply
                                ui.label("Max Supply:");
                                 self.render_max_supply_input(ui);
                                ui.end_row();

                                // Row 4: Contract Keywords
                                ui.horizontal(|ui| {
                                    ui.label("Contract Keywords (comma separated):");
                                });
                                ui.text_edit_singleline(&mut self.contract_keywords_input);
                                let response = crate::ui::helpers::info_icon_button(ui, "Each searchable keyword costs 0.1 Dash");
                                    if response.clicked() {
                                        self.show_pop_up_info = Some("Each searchable keyword costs 0.1 Dash".to_string());
                                    }

                                for name in self.token_names_input.iter() {
                                    if !name.0.is_empty() && name.3 {
                                        let contract_keywords = self.contract_keywords_input.split(',').map(|s| s.trim()).collect::<Vec<_>>();

                                        // If there are any duplicate keywords, show an error
                                        let mut seen_keywords = HashSet::new();
                                        seen_keywords.insert(name.0.clone());
                                        for keyword in contract_keywords.iter() {
                                            if seen_keywords.contains(*keyword) {
                                                ui.colored_label(Color32::DARK_RED, format!("Duplicate contract keyword: {}", keyword));
                                            }
                                            seen_keywords.insert(keyword.to_string());
                                        }
                                    }
                                }
                                ui.end_row();

                                // Row 5: Token Description
                                ui.label("Token Description (max 100 chars):");
                                ui.text_edit_singleline(&mut self.token_description_input);
                                ui.end_row();
                            });

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);

                        // 5) Advanced settings toggle
                        ui.horizontal(|ui| {
                            // +/- button
                            let button_text = if self.token_creator_advanced_expanded { "−" } else { "+" };
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
                                self.token_creator_advanced_expanded = !self.token_creator_advanced_expanded;
                            }
                            ui.label("Advanced");
                        });

                        if self.token_creator_advanced_expanded {
                            ui.add_space(3.0);

                            ui.indent("advanced_section", |ui| {
                                // Use `Grid` to align labels and text edits
                                egui::Grid::new("advanced_token_info_grid")
                                    .num_columns(2)
                                    .spacing([16.0, 8.0]) // Horizontal, vertical spacing
                                    .show(ui, |ui| {
                                        // Start as paused
                                        ui.horizontal(|ui| {
                                            StyledCheckbox::new(&mut self.start_as_paused_input, "Start as paused").show(ui);
                                            crate::ui::helpers::info_icon_button(ui, "When enabled, the token will be created in a paused state, meaning transfers will be disabled by default. All other token features—such as distributions and manual minting—remain fully functional. To allow transfers in the future, the token must be unpaused via an emergency action. It is strongly recommended to enable emergency actions if this option is selected, unless the intention is to permanently disable transfers.");
                                        });
                                        ui.end_row();

                                        self.history_row(ui);
                                        ui.end_row();

                                        // Name should be capitalized
                                        ui.horizontal(|ui| {
                                            StyledCheckbox::new(&mut self.should_capitalize_input, "Name should be capitalized").show(ui);
                                            crate::ui::helpers::info_icon_button(ui, "This is used only as helper information to client applications that will use token. This informs them on whether to capitalize the token name or not by default.");
                                        });
                                        ui.end_row();

                                        // Decimals
                                        ui.horizontal(|ui| {
                                            ui.label("Max Decimals:");
                                            // Restrict input to digits only
                                            let response = ui.add(
                                                TextEdit::singleline(&mut self.decimals_input).desired_width(50.0)
                                            );

                                            // Optionally filter out non-digit input
                                            if response.changed() {
                                                self.decimals_input.retain(|c| c.is_ascii_digit());
                                                self.decimals_input.truncate(2);
                                            }

                                            let token_name = self.token_names_input
                                                .first()
                                                .as_ref()
                                                .and_then(|(_, name, _, _)| if name.is_empty() { None} else { Some(name.as_str())})
                                                .unwrap_or("<Token Name>");

                                            let message = if self.decimals_input == "0" {
                                                format!("Non Fractional Token (i.e. 0, 1, 2 or 10 {})", token_name)
                                            } else {
                                                format!("Fractional Token (i.e. 0.2 {})", token_name)
                                            };

                                            ui.label(RichText::new(message).color(Color32::GRAY));
                                            crate::ui::helpers::info_icon_button(ui, "The decimal places of the token, for example Dash and Bitcoin use 8. The minimum indivisible amount is a Duff or a Satoshi respectively. If you put a value greater than 0 this means that it is indicated that the consensus is that 10^(number entered) is what represents 1 full unit of the token.");
                                        });
                                        ui.end_row();

                                        // Marketplace Trade Mode
                                        ui.horizontal(|ui| {
                                            ui.label("Marketplace Trade Mode:");
                                            ComboBox::from_id_salt("marketplace_trade_mode_selector")
                                                .selected_text("Not Tradeable")
                                                .show_ui(ui, |ui| {
                                                    ui.selectable_value(
                                                        &mut self.marketplace_trade_mode,
                                                        0,
                                                        "Not Tradeable",
                                                    );
                                                    // Future trade modes can be added here when SDK supports them
                                                });

                                            crate::ui::helpers::info_icon_button(ui,
                                                "Currently, all tokens are created as 'Not Tradeable'. \
                                                Future updates will add more trade mode options.\n\n\
                                                IMPORTANT: If you want to enable marketplace trading in the future, \
                                                make sure to set the 'Marketplace Trade Mode Change' rules in the Action Rules \
                                                section to something other than 'No One'. Otherwise, trading can never be enabled."
                                            );
                                        });
                                        ui.end_row();
                                    });
                            });
                        }

                        ui.add_space(5.0);

                        ui.horizontal(|ui| {
                            // +/- button
                            let button_text = if self.token_creator_action_rules_expanded { "−" } else { "+" };
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
                                self.token_creator_action_rules_expanded = !self.token_creator_action_rules_expanded;
                            }
                            ui.label("Action Rules");
                        });

                        if self.token_creator_action_rules_expanded {
                            ui.add_space(3.0);

                            ui.horizontal(|ui| {
                                ui.add_space(40.0); // Indentation
                                ui.label("Preset:");

                                ComboBox::from_id_salt("preset_selector")
                                    .selected_text(
                                        self.selected_token_preset
                                            .map(|p| match p {
                                                MostRestrictive => "Most Restrictive",
                                                WithOnlyEmergencyAction => "Only Emergency Action",
                                                WithMintingAndBurningActions => "Minting And Burning",
                                                WithAllAdvancedActions => "Advanced Actions",
                                                WithExtremeActions => "All Allowed",
                                            })
                                            .unwrap_or("Custom"),
                                    )
                                    .show_ui(ui, |ui| {
                                        // First, the "Custom" option
                                        ui.selectable_value(
                                            &mut self.selected_token_preset,
                                            None,
                                            "Custom",
                                        );

                                        for variant in [
                                            MostRestrictive,
                                            WithOnlyEmergencyAction,
                                            WithMintingAndBurningActions,
                                            WithAllAdvancedActions,
                                            WithExtremeActions,
                                        ] {
                                            let text = match variant {
                                                MostRestrictive => "Most Restrictive",
                                                WithOnlyEmergencyAction => "Only Emergency Action",
                                                WithMintingAndBurningActions => "Minting And Burning",
                                                WithAllAdvancedActions => "Advanced Actions",
                                                WithExtremeActions => "All Allowed",
                                            };
                                            if ui.selectable_value(
                                                &mut self.selected_token_preset,
                                                Some(variant),
                                                text,
                                            ).clicked() {
                                                let preset = TokenConfigurationPreset {
                                                    features: variant,
                                                    action_taker: AuthorizedActionTakers::ContractOwner, // Or from a field the user selects
                                                };
                                                self.change_to_preset(preset);
                                            }
                                        }
                                    });
                            });

                            ui.add_space(5.0);

                            ui.horizontal(|ui| {
                                ui.add_space(20.0); // Indentation for action rules
                                ui.vertical(|ui| {
                                    self.manual_minting_rules.render_mint_control_change_rules_ui(ui, &self.groups_ui, &mut self.new_tokens_destination_identity_should_default_to_contract_owner, &mut self.new_tokens_destination_other_identity_enabled, &mut self.minting_allow_choosing_destination, &mut self.new_tokens_destination_identity_rules, &mut self.new_tokens_destination_other_identity, &mut self.minting_allow_choosing_destination_rules, &mut self.token_creator_manual_mint_expanded, &mut self.token_creator_new_tokens_destination_expanded, &mut self.token_creator_minting_allow_choosing_expanded);
                                    self.manual_burning_rules.render_control_change_rules_ui(ui, &self.groups_ui,"Manual Burn", None, &mut self.token_creator_manual_burn_expanded);
                                    self.freeze_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Freeze", Some(&mut self.allow_transfers_to_frozen_identities), &mut self.token_creator_freeze_expanded);
                                    self.unfreeze_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Unfreeze", None, &mut self.token_creator_unfreeze_expanded);
                                    self.destroy_frozen_funds_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Destroy Frozen Funds", None, &mut self.token_creator_destroy_frozen_expanded);
                                    self.emergency_action_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Emergency Action", None, &mut self.token_creator_emergency_action_expanded);
                                    self.max_supply_change_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Max Supply Change", None, &mut self.token_creator_max_supply_change_expanded);
                                    self.conventions_change_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Conventions Change", None, &mut self.token_creator_conventions_change_expanded);
                                    self.marketplace_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Marketplace Trade Mode Change", None, &mut self.token_creator_marketplace_expanded);
                                    self.change_direct_purchase_pricing_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Direct Purchase Pricing Change", None, &mut self.token_creator_direct_purchase_pricing_expanded);
                                });
                            });

                            // Main control group change is slightly different so do this one manually.
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.add_space(20.0); // Indentation for main control group change
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        // +/- button
                                        let button_text = if self.token_creator_main_control_expanded { "−" } else { "+" };
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
                                            self.token_creator_main_control_expanded = !self.token_creator_main_control_expanded;
                                        }
                                        ui.label("Main Control Group Change");
                                    });

                                    if self.token_creator_main_control_expanded {
                                ui.add_space(3.0);

                                // A) authorized_to_make_change
                                ui.horizontal(|ui| {
                                    ui.label("Allow main control group change:");
                                    ComboBox::from_id_salt("main_control_group_change_selector")
                                        .selected_text(format!(
                                            "{}",
                                            self.authorized_main_control_group_change
                                        ))
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut self.authorized_main_control_group_change,
                                                AuthorizedActionTakers::NoOne,
                                                "No One",
                                            );
                                            ui.selectable_value(
                                                &mut self.authorized_main_control_group_change,
                                                AuthorizedActionTakers::ContractOwner,
                                                "Contract Owner",
                                            );
                                            ui.selectable_value(
                                                &mut self.authorized_main_control_group_change,
                                                AuthorizedActionTakers::Identity(Identifier::default()),
                                                "Identity",
                                            );
                                            ui.selectable_value(
                                                &mut self.authorized_main_control_group_change,
                                                AuthorizedActionTakers::MainGroup,
                                                "Main Group",
                                            );
                                            ui.selectable_value(
                                                &mut self.authorized_main_control_group_change,
                                                AuthorizedActionTakers::Group(0),
                                                "Group",
                                            );
                                        });
                                    match &mut self.authorized_main_control_group_change {
                                        AuthorizedActionTakers::Identity(_) => {
                                            if self.main_control_group_change_authorized_identity.is_none() {
                                                self.main_control_group_change_authorized_identity = Some(String::new());
                                            }
                                            if let Some(ref mut id) = self.main_control_group_change_authorized_identity {
                                                ui.add(TextEdit::singleline(id).hint_text("base58 id"));
                                            }
                                        }
                                        AuthorizedActionTakers::Group(_) => {
                                            if self.main_control_group_change_authorized_group.is_none() {
                                                self.main_control_group_change_authorized_group = Some("0".to_string());
                                            }
                                            if let Some(ref mut group) = self.main_control_group_change_authorized_group {
                                                ui.add(TextEdit::singleline(group).hint_text("group contract position"));
                                            }
                                        }
                                        _ => {}
                                    }
                                });
                                    }
                                });
                            });
                        }

                        self.render_distributions(context, ui);
                        self.render_groups(ui);
                        self.render_document_schemas(ui);

                        // 6) "Register Token Contract" button
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let register_button = egui::Button::new(
                                RichText::new("Register Token Contract")
                                    .color(egui::Color32::WHITE)
                                    .strong(),
                            )
                            .fill(DashColors::DASH_BLUE)
                            .min_size(egui::vec2(200.0, 36.0));

                            if ui.add(register_button).clicked() {
                                match self.parse_token_build_args() {
                                    Ok(args) => {
                                        // If success, show the "confirmation popup"
                                        // Or skip the popup entirely and dispatch tasks right now
                                        self.cached_build_args = Some(args);
                                        self.token_creator_error_message = None;
                                        self.show_token_creator_confirmation_popup = true;
                                    },
                                    Err(err) => {
                                        self.token_creator_error_message = Some(err);
                                    }
                                }
                            }

                            let view_json_button = egui::Button::new(
                                RichText::new("View JSON")
                                    .color(egui::Color32::WHITE)
                                    .strong(),
                            )
                            .fill(DashColors::DASH_BLUE)
                            .min_size(egui::vec2(120.0, 36.0));

                            if ui.add(view_json_button).clicked() {
                                match self.parse_token_build_args() {
                                    Ok(args) => {
                                        // We have the parsed token creation arguments
                                        // We can now call build_data_contract_v1_with_one_token using `args`
                                        self.cached_build_args = Some(args.clone());
                                        let data_contract = match self.app_context.build_data_contract_v1_with_one_token(
                                            args.identity_id,
                                            args.token_names,
                                            args.contract_keywords,
                                            args.token_description,
                                            args.should_capitalize,
                                            args.decimals,
                                            args.base_supply,
                                            args.max_supply,
                                            args.start_paused,
                                            args.allow_transfers_to_frozen_identities,
                                            args.keeps_history,
                                            args.main_control_group,
                                            args.manual_minting_rules,
                                            args.manual_burning_rules,
                                            args.freeze_rules,
                                            args.unfreeze_rules,
                                            args.destroy_frozen_funds_rules,
                                            args.emergency_action_rules,
                                            args.max_supply_change_rules,
                                            args.conventions_change_rules,
                                            args.main_control_group_change_authorized,
                                            args.distribution_rules,
                                            args.groups,
                                            args.document_schemas,
                                            args.marketplace_trade_mode,
                                            args.marketplace_rules,
                                        ) {
                                            Ok(dc) => dc,
                                            Err(e) => {
                                                self.token_creator_error_message = Some(format!("Error building contract V1: {e}"));
                                                return;
                                            }
                                        };

                                        let data_contract_json = data_contract.to_json(self.app_context.platform_version()).expect("Expected to map contract to json");
                                        self.show_json_popup = true;
                                        self.json_popup_text = serde_json::to_string_pretty(&data_contract_json).expect("Expected to serialize json");
                                    },
                                    Err(err_msg) => {
                                        self.token_creator_error_message = Some(err_msg);
                                    },
                                }
                            }
                        });

        // Reset the expanded states after processing
        if self.should_reset_collapsing_states {
            self.reset_token_creator_collapsing_states();
        }

                        } // Close advanced mode else block

        // 7) If the user pressed "Register Token Contract," show a popup confirmation
        if self.show_token_creator_confirmation_popup {
            action |= self.render_token_creator_confirmation_popup(ui);
        }

        if self.show_json_popup {
            self.render_data_contract_json_popup(ui);
        }

        // 8) If we are waiting, show spinner / time elapsed
        if let TokenCreatorStatus::WaitingForResult(start_time) = self.token_creator_status {
            let now = Utc::now().timestamp() as u64;
            let elapsed = now - start_time;
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Registering token contract... elapsed {}s",
                    elapsed
                ));
                ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
            });
        }

        // Show an error if we have one
        if let Some(err_msg) = self.token_creator_error_message.clone() {
            ui.add_space(10.0);
            let error_color = Color32::from_rgb(255, 100, 100);
            Frame::new()
                .fill(error_color.gamma_multiply(0.1))
                .inner_margin(Margin::symmetric(10, 8))
                .corner_radius(5.0)
                .stroke(egui::Stroke::new(1.0, error_color))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Error: {}", err_msg)).color(error_color));
                        ui.add_space(10.0);
                        if ui.small_button("Dismiss").clicked() {
                            self.token_creator_error_message = None;
                        }
                    });
                });
            ui.add_space(10.0);
        }

            }); // Close the ScrollArea from line 40

        // Handle Load Identity button click from within the ScrollArea
        if load_identity_clicked {
            return AppAction::AddScreen(
                ScreenType::AddExistingIdentity.create_screen(&self.app_context),
            );
        }

        action
    }

    fn update_selected_wallet(&mut self) {
        if let (Some(qid), Some(key)) = (&self.selected_identity, &self.selected_key) {
            self.selected_wallet = crate::ui::identities::get_selected_wallet(
                qid,
                None,
                Some(key),
                &mut self.token_creator_error_message,
            );
        }
    }

    fn ensure_wallet_unlocked(&mut self, ui: &mut Ui) -> bool {
        if let Some(wallet) = &self.selected_wallet {
            use crate::ui::components::wallet_unlock_popup::{
                try_open_wallet_no_password, wallet_needs_unlock,
            };

            if let Err(e) = try_open_wallet_no_password(wallet) {
                self.token_creator_error_message = Some(e);
            }

            if wallet_needs_unlock(wallet) {
                ui.add_space(10.0);
                ui.colored_label(
                    egui::Color32::from_rgb(200, 150, 50),
                    "Wallet is locked. Please unlock to continue.",
                );
                ui.add_space(8.0);
                if ui.button("Unlock Wallet").clicked() {
                    self.wallet_unlock_popup.open();
                }
                return false;
            }
        }

        true
    }

    fn render_token_name_language_selector(
        ui: &mut Ui,
        current_language: &mut TokenNameLanguage,
        allow_all_languages: bool,
        id_salt: &str,
    ) {
        ui.style_mut().spacing.combo_height = 10.0;
        ui.style_mut().spacing.button_padding = egui::vec2(3.0, 0.0);
        ui.style_mut().visuals.widgets.inactive.fg_stroke.width = 1.0;
        ui.style_mut()
            .text_styles
            .get_mut(&egui::TextStyle::Body)
            .unwrap()
            .size = 12.0;

        ComboBox::from_id_salt(id_salt)
            .selected_text(format!("{}", current_language))
            .width(100.0)
            .show_ui(ui, |ui| {
                ui.style_mut()
                    .text_styles
                    .get_mut(&egui::TextStyle::Body)
                    .unwrap()
                    .size = 12.0;
                for &language in TokenNameLanguage::selection_order() {
                    if !allow_all_languages && language != TokenNameLanguage::English {
                        continue;
                    }
                    ui.selectable_value(current_language, language, language.ui_label());
                }
            });
    }

    fn reset_token_creator_collapsing_states(&mut self) {
        self.token_creator_advanced_expanded = false;
        self.token_creator_action_rules_expanded = false;
        self.token_creator_main_control_expanded = false;
        self.token_creator_distribution_expanded = false;
        self.token_creator_groups_expanded = false;
        self.token_creator_document_schemas_expanded = false;
        // Individual action rules
        self.token_creator_manual_mint_expanded = false;
        self.token_creator_manual_burn_expanded = false;
        self.token_creator_freeze_expanded = false;
        self.token_creator_unfreeze_expanded = false;
        self.token_creator_destroy_frozen_expanded = false;
        self.token_creator_emergency_action_expanded = false;
        self.token_creator_max_supply_change_expanded = false;
        self.token_creator_conventions_change_expanded = false;
        self.token_creator_marketplace_expanded = false;
        self.token_creator_direct_purchase_pricing_expanded = false;
        // Nested rules
        self.token_creator_new_tokens_destination_expanded = false;
        self.token_creator_minting_allow_choosing_expanded = false;
        self.token_creator_perpetual_distribution_rules_expanded = false;
        self.should_reset_collapsing_states = false;
    }

    /// Gathers user input and produces the arguments needed by
    /// `build_data_contract_v1_with_one_token`.
    /// Returns Err(error_msg) on invalid input.
    pub fn parse_token_build_args(&mut self) -> Result<TokenBuildArgs, String> {
        // 1) We must have a selected identity
        let identity = self
            .selected_identity
            .clone()
            .ok_or_else(|| "Please select an identity".to_string())?;
        let identity_id = identity.identity.id();

        // Remove whitespace and parse the comma separated string into a vec
        let mut contract_keywords = self.parse_contract_keywords()?;

        // 2) Basic fields
        let token_names = self.parse_token_names(&mut contract_keywords)?;

        let token_description = if !self.token_description_input.is_empty() {
            Some(self.token_description_input.clone())
        } else {
            None
        };
        let decimals = self.parse_decimals()?;
        let base_supply = self.parse_base_supply()?;
        let max_supply = self.parse_max_supply();

        let start_paused = self.start_as_paused_input;
        let allow_transfers_to_frozen_identities = self.allow_transfers_to_frozen_identities;
        let keeps_history = self.token_advanced_keeps_history.into();

        let main_control_group = self.parse_main_control_group()?;

        // 3) Convert your ActionChangeControlUI fields to real rules
        // (or do the manual parse for each if needed)
        let manual_minting_rules = self
            .manual_minting_rules
            .extract_change_control_rules("Manual Mint")?;
        let manual_burning_rules = self
            .manual_burning_rules
            .extract_change_control_rules("Manual Burn")?;
        let freeze_rules = self.freeze_rules.extract_change_control_rules("Freeze")?;
        let unfreeze_rules = self
            .unfreeze_rules
            .extract_change_control_rules("Unfreeze")?;
        let destroy_frozen_funds_rules = self
            .destroy_frozen_funds_rules
            .extract_change_control_rules("Destroy Frozen Funds")?;
        let emergency_action_rules = self
            .emergency_action_rules
            .extract_change_control_rules("Emergency Action")?;
        let max_supply_change_rules = self
            .max_supply_change_rules
            .extract_change_control_rules("Max Supply Change")?;
        let conventions_change_rules = self
            .conventions_change_rules
            .extract_change_control_rules("Conventions Change")?;

        // The main_control_group_change_authorized is done manually in your code,
        // parse identity or group if needed. Reuse your existing logic:
        let main_control_group_change_authorized =
            self.parse_main_control_group_change_authorized()?;

        // 4) Distribution data (perpetual & pre_programmed)
        let distribution_rules = self.build_distribution_rules()?;

        // 5) Groups
        let groups = self.parse_groups()?;

        // 6) Marketplace rules
        let marketplace_rules = self
            .marketplace_rules
            .extract_change_control_rules("Marketplace Trade Mode")?;

        // 7) Direct purchase pricing rules
        let change_direct_purchase_pricing_rules = self
            .change_direct_purchase_pricing_rules
            .extract_change_control_rules("Direct Purchase Pricing Change")?;

        // 8) Put it all in a struct
        Ok(TokenBuildArgs {
            identity_id,
            token_names,
            contract_keywords,
            token_description,
            should_capitalize: self.should_capitalize_input,
            decimals,
            base_supply,
            max_supply,
            start_paused,
            allow_transfers_to_frozen_identities,
            keeps_history,
            main_control_group,

            manual_minting_rules,
            manual_burning_rules,
            freeze_rules,
            unfreeze_rules,
            destroy_frozen_funds_rules,
            emergency_action_rules,
            max_supply_change_rules,
            conventions_change_rules,
            main_control_group_change_authorized,

            distribution_rules: TokenDistributionRules::V0(distribution_rules),
            groups,
            document_schemas: self.parsed_document_schemas.clone(),
            marketplace_trade_mode: self.marketplace_trade_mode,
            marketplace_rules,
            change_direct_purchase_pricing_rules,
        })
    }

    fn parse_contract_keywords(&self) -> Result<Vec<String>, String> {
        if self.contract_keywords_input.trim().is_empty() {
            return Ok(Vec::new());
        }

        self.contract_keywords_input
            .split(',')
            .map(|s| {
                let trimmed = s.trim().to_string();
                if trimmed.len() < 3 || trimmed.len() > 50 {
                    Err(format!(
                        "Invalid contract keyword {}, keyword must be between 3 and 50 characters",
                        trimmed
                    ))
                } else {
                    Ok(trimmed)
                }
            })
            .collect::<Result<Vec<String>, String>>()
    }

    fn parse_token_names(
        &self,
        contract_keywords: &mut Vec<String>,
    ) -> Result<Vec<(String, String, String)>, String> {
        if self.token_names_input.is_empty() {
            return Err("Please enter a token name".to_string());
        }

        let mut seen_languages = HashSet::new();
        for name_with_language in &self.token_names_input {
            if seen_languages.contains(&name_with_language.2) {
                return Err(format!(
                    "Duplicate token name language: {:?}",
                    name_with_language.1
                ));
            }
            seen_languages.insert(name_with_language.2);
        }

        let mut token_names = Vec::with_capacity(self.token_names_input.len());
        for name_with_language in &self.token_names_input {
            if name_with_language.0.len() < 3 || name_with_language.0.len() > 50 {
                return Err(format!(
                    "The name in {:?} must be between 3 and 50 characters",
                    name_with_language.2
                ));
            }

            if name_with_language.1.len() < 3 || name_with_language.1.len() > 50 {
                return Err(format!(
                    "The plural form in {:?} must be between 3 and 50 characters",
                    name_with_language.2
                ));
            }

            token_names.push((
                name_with_language.0.clone(),
                name_with_language.1.clone(),
                name_with_language.2.iso_code().to_owned(),
            ));

            // are we searchable?
            if name_with_language.3 {
                contract_keywords.push(name_with_language.0.clone());
            }
        }

        Ok(token_names)
    }

    fn parse_decimals(&self) -> Result<u8, String> {
        self.decimals_input
            .parse::<u8>()
            .map_err(|_| "Invalid decimal places amount".to_string())
    }

    fn parse_base_supply(&self) -> Result<u64, String> {
        self.base_supply_amount
            .as_ref()
            .map(|amount| amount.value())
            .ok_or_else(|| "Please enter a valid base supply amount".to_string())
    }

    fn parse_max_supply(&self) -> Option<u64> {
        self.max_supply_amount.as_ref().and_then(|amount| {
            let value = amount.value();
            if value > 0 { Some(value) } else { None }
        })
    }

    fn parse_main_control_group(&self) -> Result<Option<u16>, String> {
        if self.main_control_group_input.is_empty() {
            return Ok(None);
        }

        self.main_control_group_input
            .parse::<u16>()
            .map(Some)
            .map_err(|_| "Invalid main control group".to_string())
    }

    /// Example of pulling out the logic to parse main_control_group_change_authorized
    fn parse_main_control_group_change_authorized(
        &mut self,
    ) -> Result<AuthorizedActionTakers, String> {
        match &mut self.authorized_main_control_group_change {
            AuthorizedActionTakers::Identity(_) => {
                if let Some(ref id_str) = self.main_control_group_change_authorized_identity {
                    if let Ok(id) = Identifier::from_string(id_str, Encoding::Base58) {
                        Ok(AuthorizedActionTakers::Identity(id))
                    } else {
                        Err("Invalid base58 identifier for main control group change authorized identity".to_owned())
                    }
                } else {
                    Ok(AuthorizedActionTakers::Identity(Identifier::default()))
                }
            }
            AuthorizedActionTakers::Group(_) => {
                if let Some(ref group_str) = self.main_control_group_change_authorized_group {
                    if let Ok(g) = group_str.parse::<u16>() {
                        Ok(AuthorizedActionTakers::Group(g))
                    } else {
                        Err("Invalid group contract position for main control group".to_owned())
                    }
                } else {
                    Ok(AuthorizedActionTakers::Group(0))
                }
            }
            other => {
                // For ContractOwner or NoOne, just return them as-is
                Ok(*other)
            }
        }
    }

    pub fn change_to_preset(&mut self, preset: TokenConfigurationPreset) {
        let basic_rules = preset.default_basic_change_control_rules_v0();
        let advanced_rules = preset.default_advanced_change_control_rules_v0();
        let emergency_rules = preset.default_emergency_action_change_control_rules_v0();

        self.manual_minting_rules = basic_rules.clone().into();
        self.manual_burning_rules = basic_rules.clone().into();
        self.freeze_rules = advanced_rules.clone().into();
        self.unfreeze_rules = advanced_rules.clone().into();
        self.destroy_frozen_funds_rules = advanced_rules.clone().into();
        self.emergency_action_rules = emergency_rules.clone().into();
        self.max_supply_change_rules = advanced_rules.clone().into();
        self.conventions_change_rules = basic_rules.clone().into();
        self.perpetual_distribution_rules = advanced_rules.clone().into();
        self.new_tokens_destination_identity_rules = basic_rules.clone().into();
        self.minting_allow_choosing_destination_rules = basic_rules.clone().into();
        self.authorized_main_control_group_change =
            preset.default_main_control_group_can_be_modified();

        // Marketplace settings
        self.marketplace_rules =
            if preset.features == TokenConfigurationPresetFeatures::MostRestrictive {
                // Most restrictive = no one can change marketplace rules
                ChangeControlRulesUI::from(ChangeControlRulesV0 {
                    authorized_to_make_change: AuthorizedActionTakers::NoOne,
                    admin_action_takers: AuthorizedActionTakers::NoOne,
                    changing_authorized_action_takers_to_no_one_allowed: false,
                    changing_admin_action_takers_to_no_one_allowed: false,
                    self_changing_admin_action_takers_allowed: false,
                })
            } else {
                advanced_rules.clone().into()
            };

        // Direct purchase pricing rules follow the same pattern as marketplace rules
        self.change_direct_purchase_pricing_rules =
            if preset.features == TokenConfigurationPresetFeatures::MostRestrictive {
                ChangeControlRulesUI::from(ChangeControlRulesV0 {
                    authorized_to_make_change: AuthorizedActionTakers::NoOne,
                    admin_action_takers: AuthorizedActionTakers::NoOne,
                    changing_authorized_action_takers_to_no_one_allowed: false,
                    changing_admin_action_takers_to_no_one_allowed: false,
                    self_changing_admin_action_takers_allowed: false,
                })
            } else {
                advanced_rules.clone().into()
            };

        // Reset optional identity/group inputs related to control group modification
        self.main_control_group_change_authorized_identity = None;
        self.main_control_group_change_authorized_group = None;

        // Set `selected_token_preset` so UI shows current preset (Optional)
        self.selected_token_preset = Some(preset.features);
    }

    fn close_token_creator_confirmation_popup(&mut self) {
        self.show_token_creator_confirmation_popup = false;
        self.token_creator_confirmation_dialog = None;
    }

    /// Shows a popup "Are you sure?" for creating the token contract
    fn render_token_creator_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // Prepare the confirmation message
        let mut confirmation_message =
            "Are you sure you want to register a new token contract with these settings?\n\n"
                .to_string();
        let base_supply_display = self
            .base_supply_amount
            .as_ref()
            .map(|amount| amount.to_string_opts(true, false))
            .unwrap_or_else(|| "0".to_string());
        let max_supply_display = self
            .max_supply_amount
            .as_ref()
            .filter(|amount| amount.value() > 0)
            .map(|amount| amount.to_string_opts(true, false))
            .unwrap_or_else(|| "None".to_string());

        confirmation_message.push_str(&format!(
            "Name: {}\nBase Supply: {}\nMax Supply: {}\n\n",
            self.token_names_input[0].0, base_supply_display, max_supply_display,
        ));

        confirmation_message.push_str(&format!(
            "Estimated cost to register this token is {} Dash",
            self.estimate_registration_cost() as f64 / 100_000_000_000.0
        ));

        // Check if marketplace is locked to NotTradeable forever
        let mut is_danger_mode = false;
        if let Some(args) = &self.cached_build_args {
            let is_not_tradeable = args.marketplace_trade_mode == 0;
            let marketplace_rules_locked = matches!(
                args.marketplace_rules,
                ChangeControlRules::V0(ChangeControlRulesV0 {
                    authorized_to_make_change: AuthorizedActionTakers::NoOne,
                    admin_action_takers: AuthorizedActionTakers::NoOne,
                    ..
                })
            );

            if is_not_tradeable && marketplace_rules_locked {
                confirmation_message.push_str("\n\nWARNING: This token will be permanently set to NotTradeable and can NEVER be made tradeable in the future!");
                is_danger_mode = true;
            }
        }

        // Always create a fresh confirmation dialog to ensure current state is reflected
        let confirmation_dialog = self.token_creator_confirmation_dialog.insert(
            ConfirmationDialog::new("Confirm Token Contract Registration", confirmation_message)
                .confirm_text(Some("Confirm"))
                .cancel_text(Some("Cancel"))
                .danger_mode(is_danger_mode),
        );

        // Show the dialog and handle the response
        let response = confirmation_dialog.show(ui).inner;

        if let Some(status) = response.dialog_response {
            match status {
                ConfirmationStatus::Confirmed => {
                    let args = match &self.cached_build_args {
                        Some(args) => args.clone(),
                        None => {
                            // fallback if we didn't store them
                            match self.parse_token_build_args() {
                                Ok(a) => a,
                                Err(err) => {
                                    self.token_creator_error_message = Some(err);
                                    self.close_token_creator_confirmation_popup();
                                    return AppAction::None;
                                }
                            }
                        }
                    };

                    // Validate identity and key are selected
                    let (identity, signing_key) =
                        match (&self.selected_identity, &self.selected_key) {
                            (Some(id), Some(key)) => (id.clone(), key.clone()),
                            _ => {
                                self.token_creator_error_message =
                                    Some("Please select an identity and signing key.".to_string());
                                self.close_token_creator_confirmation_popup();
                                return AppAction::None;
                            }
                        };

                    // Now create your tasks
                    let tasks = vec![
                        BackendTask::TokenTask(Box::new(TokenTask::RegisterTokenContract {
                            identity,
                            signing_key: Box::new(signing_key),

                            token_names: args.token_names,
                            contract_keywords: args.contract_keywords,
                            token_description: args.token_description,
                            should_capitalize: args.should_capitalize,
                            decimals: args.decimals,
                            base_supply: args.base_supply,
                            max_supply: args.max_supply,
                            start_paused: args.start_paused,
                            allow_transfers_to_frozen_identities: args
                                .allow_transfers_to_frozen_identities,
                            keeps_history: args.keeps_history,
                            main_control_group: args.main_control_group,

                            manual_minting_rules: args.manual_minting_rules,
                            manual_burning_rules: args.manual_burning_rules,
                            freeze_rules: args.freeze_rules,
                            unfreeze_rules: Box::new(args.unfreeze_rules),
                            destroy_frozen_funds_rules: Box::new(args.destroy_frozen_funds_rules),
                            emergency_action_rules: Box::new(args.emergency_action_rules),
                            max_supply_change_rules: Box::new(args.max_supply_change_rules),
                            conventions_change_rules: Box::new(args.conventions_change_rules),
                            main_control_group_change_authorized: args
                                .main_control_group_change_authorized,
                            distribution_rules: args.distribution_rules,
                            groups: args.groups,
                            document_schemas: args.document_schemas,
                            marketplace_trade_mode: args.marketplace_trade_mode,
                            marketplace_rules: args.marketplace_rules,
                        })),
                        BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)),
                    ];

                    action = AppAction::BackendTasks(tasks, BackendTasksExecutionMode::Sequential);
                    let now = Utc::now().timestamp() as u64;
                    self.token_creator_status = TokenCreatorStatus::WaitingForResult(now);
                    self.close_token_creator_confirmation_popup();
                }
                ConfirmationStatus::Canceled => {
                    self.close_token_creator_confirmation_popup();
                    action = AppAction::None;
                }
            }
        }

        action
    }

    /// Render the document schemas collapsible section
    fn render_document_schemas(&mut self, ui: &mut Ui) {
        ui.add_space(5.0);

        ui.horizontal(|ui| {
            // +/- button
            let button_text = if self.token_creator_document_schemas_expanded {
                "−"
            } else {
                "+"
            };
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
                self.token_creator_document_schemas_expanded =
                    !self.token_creator_document_schemas_expanded;
            }
            ui.label("Document Schemas");
        });

        if self.token_creator_document_schemas_expanded {
            ui.add_space(3.0);

            ui.indent("document_schemas_section", |ui| {
                // Add link to dashpay.io
            ui.horizontal(|ui| {
                    ui.label("Paste JSON document schemas to include in the contract. Easily create document schemas here:");
                    ui.add(egui::Hyperlink::from_label_and_url(
                        RichText::new("dashpay.io")
                            .underline()
                            .color(Color32::from_rgb(0, 128, 255)),
                        "https://dashpay.io",
                    ));
                });

            ui.add_space(5.0);

            let dark_mode = ui.ctx().style().visuals.dark_mode;
            let schemas_response = ui.add_sized(
                [ui.available_width(), 120.0],
                TextEdit::multiline(&mut self.document_schemas_input)
                    .text_color(crate::ui::theme::DashColors::text_primary(dark_mode))
                    .background_color(crate::ui::theme::DashColors::input_background(dark_mode)),
            );

            if schemas_response.changed() {
                self.parse_document_schemas();
            }

            ui.add_space(5.0);

            // Show validation result
            if let Some(ref error) = self.document_schemas_error {
                ui.colored_label(
                    Color32::DARK_RED,
                    format!("Schema validation error: {}", error),
                );
            } else if self.parsed_document_schemas.is_some() {
                let schema_count = self.parsed_document_schemas.as_ref().unwrap().len();
                if schema_count > 0 {
                    ui.colored_label(
                        Color32::DARK_GREEN,
                        format!("✓ {} valid document schema(s) parsed", schema_count),
                    );
                }
            }
            });
        }
    }

    /// Parse and validate the document schemas JSON input
    fn parse_document_schemas(&mut self) {
        self.document_schemas_error = None;
        self.parsed_document_schemas = None;

        if self.document_schemas_input.trim().is_empty() {
            return;
        }

        match serde_json::from_str::<serde_json::Value>(&self.document_schemas_input) {
            Ok(json_value) => {
                match json_value.as_object() {
                    Some(obj) => {
                        let mut schemas = BTreeMap::new();

                        for (key, value) in obj {
                            // Basic validation - ensure it's an object with required fields
                            if let Some(schema_obj) = value.as_object() {
                                if schema_obj.contains_key("type") {
                                    schemas.insert(key.clone(), value.clone());
                                } else {
                                    self.document_schemas_error = Some(format!(
                                        "Document schema '{}' missing required 'type' field",
                                        key
                                    ));
                                    return;
                                }
                            } else {
                                self.document_schemas_error =
                                    Some(format!("Document schema '{}' must be an object", key));
                                return;
                            }
                        }

                        self.parsed_document_schemas = Some(schemas);
                    }
                    None => {
                        self.document_schemas_error =
                            Some("Document schemas must be a JSON object".to_string());
                    }
                }
            }
            Err(e) => {
                self.document_schemas_error = Some(format!("Invalid JSON: {}", e));
            }
        }
    }

    /// Once the contract creation is done (status=Complete),
    /// render a simple "Success" screen
    fn render_token_creator_success_screen(&mut self, ui: &mut Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);
            ui.heading("Token Contract Created Successfully! 🎉");
            ui.add_space(10.0);
            if ui.button("Back").clicked() {
                self.reset_token_creator();
            }
        });
    }
}
