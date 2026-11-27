use crate::ui::components::wallet_unlock_popup::wallet_needs_unlock;
use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::{TokenConfigurationPreset, TokenConfigurationPresetFeatures};
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPresetFeatures::{MostRestrictive, WithAllAdvancedActions, WithExtremeActions, WithMintingAndBurningActions, WithOnlyEmergencyAction};
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::change_control_rules::v0::ChangeControlRulesV0;
use dash_sdk::dpp::data_contract::change_control_rules::ChangeControlRules;
use dash_sdk::dpp::data_contract::serialized_version::DataContractInSerializationFormat;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::version::TryFromPlatformVersioned;
use dash_sdk::platform::Identifier;
use eframe::epaint::Color32;
use egui::{ComboBox, Context, Frame, Margin, RichText, TextEdit, Ui};
use crate::ui::theme::{ComponentStyles, DashColors, ResponseExt};
use crate::ui::ScreenType;
use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::{BackendTask, BackendTaskContext};
use crate::backend_task::tokens::TokenTask;
use crate::model::amount::Amount;
use crate::model::identity_key_usability::KeyRequirements;
use crate::model::qualified_identity::{QualifiedIdentity, encrypted_key_storage::same_key};
use crate::model::token::validate_contract_keyword;
use crate::model::token::{
    distribution_rules_with_once_per_identity, once_per_identity_amount_in_rules,
    parse_once_per_identity_amount,
};
use dash_sdk::dpp::balances::credits::TokenAmount;
use crate::ui::components::styled::{StyledCheckbox};
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::Component;
use crate::ui::components::identity_selector::IdentitySelector;
use crate::ui::components::MessageBanner;
use crate::ui::MessageType;
use crate::ui::helpers::{add_identity_key_chooser, signable_key_now, TransactionType};
use dash_sdk::dpp::identity::{Purpose, SecurityLevel};
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::platform::IdentityPublicKey;
use crate::ui::tokens::tokens_screen::{
    TokenPresetSelection, MintRecipientSection, TokenBuildArgs, TokenCreatorStatus,
    TokenNameLanguage, TokensScreen, offered_by_token_creator,
};

pub(super) struct PendingTokenRegistration {
    args: TokenBuildArgs,
    signing_key: IdentityPublicKey,
    pub(super) dialog: ConfirmationDialog,
}

impl TokensScreen {
    /// The creator waits for the registration sent under `context`, under a
    /// progress banner.
    pub(super) fn mark_token_registration_started(&mut self, context: BackendTaskContext) {
        self.token_creator_status = TokenCreatorStatus::WaitingForResult;
        self.token_registration_context = Some(context);
        let handle = MessageBanner::set_global(
            self.app_context.egui_ctx(),
            "Creating token...",
            MessageType::Info,
        );
        handle.with_elapsed();
        self.registration_banner = Some(handle);
    }

    pub(super) fn render_token_creator(&mut self, context: &Context, ui: &mut Ui) -> AppAction {
        // Invalidate the confirmation before the form can pick a replacement key.
        self.cancel_invalid_token_registration();
        if crate::ui::helpers::refresh_contract_signing_key(
            &self.app_context,
            self.selected_identity.as_ref(),
            &mut self.selected_key,
            &[SecurityLevel::HIGH, SecurityLevel::CRITICAL],
        ) {
            self.update_selected_wallet();
        }
        let mut action = AppAction::None;
        self.reconcile_selected_preset();

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
                            Ok(identities) => identities.into_iter().filter(offered_by_token_creator).collect::<Vec<_>>(),
                            Err(error) => {
                                self.pending_token_registration = None;
                                tracing::error!(?error, "Token-creator identity loading failed");
                                ui.colored_label(
                                    Color32::DARK_RED,
                                    "Saved identities could not be loaded. Refresh the screen and try again.",
                                );
                                return;
                            }
                        };
                        if all_identities.is_empty() {
                            self.pending_token_registration = None;
                            let dark_mode = ui.style().visuals.dark_mode;
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

                                        if ComponentStyles::add_primary_button(ui, "Load Identity")
                                            .clicked()
                                        {
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

                            // Use IdentitySelector for simple mode — SYNC: write-back via syncing_global.
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
                                .width(300.0)
                                .syncing_global(self.app_context.clone()),
                            );

                            // Auto-select the first eligible key when:
                            // 1. Identity changed, OR
                            // 2. Identity is selected but no key is selected yet (first load)
                            // 3. The selected key belongs to another identity
                            let key_of_another_identity = !self.selected_key_held_by_selected_identity();
                            let should_auto_select_key = response.changed()
                                || key_of_another_identity
                                || (self.selected_identity.is_some() && self.selected_key.is_none());

                            if should_auto_select_key {
                                if response.changed() || key_of_another_identity {
                                    self.selected_key = None; // The key belongs to another identity
                                }
                                if let Some(ref identity) = self.selected_identity {
                                    let first_eligible_key = identity
                                        .signing_key_now(KeyRequirements::new(
                                            Purpose::AUTHENTICATION,
                                            &[SecurityLevel::HIGH, SecurityLevel::CRITICAL],
                                            self.app_context.contract_signing_scope(),
                                        ))
                                        .cloned();

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
                                    crate::ui::helpers::contract_signing_key_notice(&self.app_context, self.selected_identity.as_ref(), &[SecurityLevel::HIGH, SecurityLevel::CRITICAL]),
                                );
                            }

                            if self.selected_identity.is_none() {
                                return;
                            }

                            // Set wallet reference for the auto-selected key
                            self.update_selected_wallet();

                            ui.add_space(10.0);
                            ui.separator();

                            // Wallet unlock check for simple mode
                            self.ensure_wallet_unlocked(ui);
                        } else {
                            // =====================================================
                            // ADVANCED MODE - Full identity and key selection
                            // =====================================================
                            ui.heading("1. Select an identity and key to register the token contract with:");
                            ui.add_space(5.0);

                            // Use the helper function for identity and key selection.
                            // SYNC (W4): snapshot before, write-back after on change.
                            let before_id = self
                                .selected_identity
                                .as_ref()
                                .map(|qi| qi.identity.id());
                            action |= add_identity_key_chooser(
                                ui,
                                &self.app_context,
                                all_identities.iter(),
                                &mut self.selected_identity,
                                &mut self.selected_key,
                                TransactionType::RegisterContract,
                                self.app_context.contract_signing_scope(),
                            );
                            let after_id = self
                                .selected_identity
                                .as_ref()
                                .map(|qi| qi.identity.id());
                            if before_id != after_id {
                                // Simple mode's selector shows this string; keep it on the same identity.
                                self.identity_id_string = after_id
                                    .map(|id| id.to_string(Encoding::Base58))
                                    .unwrap_or_default();
                                self.app_context
                                    .set_selected_identity(after_id);
                            }

                            ui.add_space(5.0);

                            // If a key was selected, set the wallet reference
                            self.update_selected_wallet();


                            ui.add_space(10.0);
                            ui.separator();

                            // Wallet unlock check for advanced mode
                            self.ensure_wallet_unlocked(ui);
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
                                            3–100 characters, or leave blank.").clicked() {
                                            self.show_pop_up_info = Some(
                                                "Description\n\n\
                                                An optional description explaining what your token is for.\n\n\
                                                This helps users understand the purpose of your token.\n\n\
                                                3–100 characters, or leave blank.".to_string()
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
                                        .selected_text(self.preset_selection_label())
                                        .show_ui(ui, |ui| {
                                            for variant in CREATOR_PRESETS {
                                                let text = preset_label(variant);
                                                let description = match variant {
                                                    MostRestrictive => "No actions allowed after creation",
                                                    WithOnlyEmergencyAction => "Can pause/unpause token",
                                                    WithMintingAndBurningActions => "Can mint and burn tokens",
                                                    WithAllAdvancedActions => "Mint, burn, freeze, and more",
                                                    WithExtremeActions => "All actions enabled",
                                                };
                                                if ui.selectable_label(
                                                    self.selected_token_preset == TokenPresetSelection::Preset(variant),
                                                    format!("{text} - {description}"),
                                                ).clicked() {
                                                    self.change_to_preset(creator_preset(variant));
                                                }
                                            }
                                        });
                                    ui.end_row();
                                });

                            ui.add_space(20.0);

                            // Create Token button
                            let blocked_reason = self.simple_mode_blocked_reason();

                            ui.horizontal(|ui| {
                                let mut response = ComponentStyles::add_primary_button_enabled(
                                    ui,
                                    blocked_reason.is_none() && self.selected_key.is_some()
                                    && self.selected_wallet.as_ref().is_none_or(|wallet| !wallet_needs_unlock(wallet)),
                                    "Create Token",
                                );
                                if let Some(reason) = &blocked_reason {
                                    response = response.disabled_tooltip(reason.as_str());
                                    Self::render_blocked_hint(ui, reason);
                                }
                                if response.clicked() {
                                    // Auto-set plural name if empty (singular + "s")
                                    let singular = self.token_names_input[0].0.trim().to_string();
                                    if self.token_names_input[0].1.trim().is_empty() {
                                        self.token_names_input[0].1 = format!("{singular}s");
                                    }

                                    self.open_token_registration_confirmation(context);
                                }
                            });

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
                                    let singular_label = ui.label("Token Name (singular)*:");
                                    ui.text_edit_singleline(&mut self.token_names_input[i].0)
                                        .labelled_by(singular_label.id);
                                    ui.horizontal(|ui| {
                                        let allow_all_languages = i != 0;
                                        ui.push_id(format!("combo_{i}"), |ui| {
                                            let combo_id = format!("token_name_language_selector_{i}");
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
                                    let plural_label = ui.label("Token Name (plural)*:");
                                    ui.text_edit_singleline(&mut self.token_names_input[i].1)
                                        .labelled_by(plural_label.id);
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
                                let keywords_label = ui
                                    .horizontal(|ui| ui.label("Contract Keywords (comma separated):"))
                                    .inner;
                                ui.text_edit_singleline(&mut self.contract_keywords_input)
                                    .labelled_by(keywords_label.id);
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
                                                ui.colored_label(Color32::DARK_RED, format!("Duplicate contract keyword: {keyword}"));
                                            }
                                            seen_keywords.insert(keyword.to_string());
                                        }
                                    }
                                }
                                ui.end_row();

                                // Row 5: Token Description
                                let description_label = ui.label("Token Description (3–100 chars):");
                                ui.text_edit_singleline(&mut self.token_description_input)
                                    .labelled_by(description_label.id);
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

                                        self.transferable_row(ui);
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
                                                format!("Non Fractional Token (i.e. 0, 1, 2 or 10 {token_name})")
                                            } else {
                                                format!("Fractional Token (i.e. 0.2 {token_name})")
                                            };

                                            ui.label(RichText::new(message).color(Color32::GRAY));
                                            crate::ui::helpers::info_icon_button(ui, "The decimal places of the token, for example Dash and Bitcoin use 8. The minimum indivisible amount is a Duff or a Satoshi respectively. If you put a value greater than 0 this means that it is indicated that the consensus is that 10^(number entered) is what represents 1 full unit of the token.");
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
                                    .selected_text(self.preset_selection_label())
                                    .show_ui(ui, |ui| {
                                        // "Custom" keeps the current rules and lets the user edit them one by one.
                                        if ui.selectable_label(
                                            self.selected_token_preset == TokenPresetSelection::Custom,
                                            CUSTOM_PRESET_LABEL,
                                        ).clicked() {
                                            self.selected_token_preset = TokenPresetSelection::Custom;
                                        }

                                        for variant in CREATOR_PRESETS {
                                            if ui.selectable_label(
                                                self.selected_token_preset == TokenPresetSelection::Preset(variant),
                                                preset_label(variant),
                                            ).clicked() {
                                                self.change_to_preset(creator_preset(variant));
                                            }
                                        }
                                    });
                            });

                            ui.add_space(5.0);

                            ui.horizontal(|ui| {
                                ui.add_space(20.0); // Indentation for action rules
                                ui.vertical(|ui| {
                                    self.manual_minting_rules.render_control_change_rules_ui(
                                        ui,
                                        &self.groups_ui,
                                        "Manual Mint",
                                        None,
                                        &mut self.token_creator_manual_mint_expanded,
                                        Some(MintRecipientSection {
                                            should_default_to_contract_owner: &mut self
                                                .new_tokens_destination_identity_should_default_to_contract_owner,
                                            destination_identity_enabled: &mut self
                                                .new_tokens_destination_other_identity_enabled,
                                            allow_choosing_destination: &mut self
                                                .minting_allow_choosing_destination,
                                            destination_identity_rules: &mut self
                                                .new_tokens_destination_identity_rules,
                                            destination_identity: &mut self
                                                .new_tokens_destination_other_identity,
                                            allow_choosing_destination_rules: &mut self
                                                .minting_allow_choosing_destination_rules,
                                            destination_expanded: &mut self
                                                .token_creator_new_tokens_destination_expanded,
                                            allow_choosing_expanded: &mut self
                                                .token_creator_minting_allow_choosing_expanded,
                                        }),
                                    );
                                    self.manual_burning_rules.render_control_change_rules_ui(ui, &self.groups_ui,"Manual Burn", None, &mut self.token_creator_manual_burn_expanded, None);
                                    self.freeze_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Freeze", Some(&mut self.allow_transfers_to_frozen_identities), &mut self.token_creator_freeze_expanded, None);
                                    self.unfreeze_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Unfreeze", None, &mut self.token_creator_unfreeze_expanded, None);
                                    self.destroy_frozen_funds_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Destroy Frozen Funds", None, &mut self.token_creator_destroy_frozen_expanded, None);
                                    self.emergency_action_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Emergency Action", None, &mut self.token_creator_emergency_action_expanded, None);
                                    self.max_supply_change_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Max Supply Change", None, &mut self.token_creator_max_supply_change_expanded, None);
                                    self.conventions_change_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Conventions Change", None, &mut self.token_creator_conventions_change_expanded, None);
                                    self.marketplace_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Marketplace Trade Mode Change", None, &mut self.token_creator_marketplace_expanded, None);
                                    self.change_direct_purchase_pricing_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Direct Purchase Pricing Change", None, &mut self.token_creator_direct_purchase_pricing_expanded, None);
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
                                        .selected_text(
                                            self.authorized_main_control_group_change.to_string(),
                                        )
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
                            let blocked_reason = self
                                .signing_blocked_reason()
                                .or_else(|| self.advanced_mode_blocked_reason());
                            let mut register = ComponentStyles::add_primary_button_enabled(
                                ui,
                                blocked_reason.is_none(),
                                "Register Token Contract",
                            );
                            if let Some(reason) = &blocked_reason {
                                register = register.disabled_tooltip(reason.as_str());
                                Self::render_blocked_hint(ui, reason);
                            }
                            if register.clicked() {
                                self.open_token_registration_confirmation(context);
                            }

                            if ComponentStyles::add_primary_button(ui, "View JSON").clicked() {
                                match self.parse_token_build_args() {
                                    Ok(args) => {
                                        let owner_id = args.identity_id;
                                        let data_contract = match self.app_context.build_data_contract_v1_with_one_token(
                                            owner_id,
                                            args.into_contract_params(),
                                        ) {
                                            Ok(dc) => dc,
                                            Err(error) => {
                                                MessageBanner::set_global(
                                                    context,
                                                    "The token contract could not be prepared. Review its settings and try again.",
                                                    MessageType::Error,
                                                )
                                                .with_details(error);
                                                return;
                                            }
                                        };

                                        let data_contract_fmt = DataContractInSerializationFormat::try_from_platform_versioned(
                                            &data_contract,
                                            self.app_context.platform_version(),
                                        ).expect("Expected to map contract to serialization format");
                                        let data_contract_json = serde_json::to_value(&data_contract_fmt).expect("Expected to map contract to json");
                                        let json = serde_json::to_string_pretty(&data_contract_json).expect("Expected to serialize json");
                                        self.open_data_contract_json_popup(json);
                                    },
                                    Err(err_msg) => {
                                        MessageBanner::set_global(context, &err_msg, MessageType::Error);
                                    },
                                }
                            }
                        });

        // Reset the expanded states after processing
        if self.should_reset_collapsing_states {
            self.reset_token_creator_collapsing_states();
        }

                        } // Close advanced mode else block

        action |= self.render_token_creator_confirmation_popup(ui);

        if self.show_json_popup {
            self.render_data_contract_json_popup(ui);
        }

        // Elapsed display for token creation is handled by the global MessageBanner

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
        crate::ui::identity::sync_signing_wallet(
            self.selected_identity.as_ref(),
            self.selected_key.as_ref(),
            &mut self.selected_wallet,
            &mut self.wallet_open_attempted,
        );
    }

    /// Whether the selected identity holds the selected key; no key counts as held.
    fn selected_key_held_by_selected_identity(&self) -> bool {
        let (Some(identity), Some(selected)) = (&self.selected_identity, &self.selected_key) else {
            return true;
        };
        identity
            .private_keys
            .identity_public_keys()
            .iter()
            .map(|key_ref| &key_ref.1.identity_public_key)
            .any(|key| key.id() == selected.id() && key.data() == selected.data())
    }

    fn ensure_wallet_unlocked(&mut self, ui: &mut Ui) -> bool {
        if let Some(wallet) = &self.selected_wallet {
            use crate::ui::components::wallet_unlock_popup::{
                try_open_wallet_no_password, wallet_needs_unlock,
            };

            if !self.wallet_open_attempted {
                if let Err(e) = try_open_wallet_no_password(&self.app_context, wallet) {
                    MessageBanner::set_global(ui.ctx(), &e, MessageType::Error)
                        .disable_auto_dismiss();
                }
                self.wallet_open_attempted = true;
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
            .selected_text(current_language.to_string())
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
            })
            .response
            .accessible_name("Token name language");
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
            let len = self.token_description_input.chars().count();
            if !(3..=100).contains(&len) {
                return Err(
                    "Token description must be either empty or between 3 and 100 characters long"
                        .to_string(),
                );
            }
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

        // 4) Distribution data (perpetual, pre-programmed & once-per-identity)
        let distribution_rules = self.build_distribution_rules()?;
        let once_per_identity_amount = self.parse_once_per_identity_amount()?;

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

            distribution_rules: distribution_rules_with_once_per_identity(
                distribution_rules,
                once_per_identity_amount,
            ),
            groups,
            document_schemas: self.parsed_document_schemas.clone(),
            marketplace_rules,
            change_direct_purchase_pricing_rules,
            transferable: self.transferable_input,
        })
    }

    /// Validate the once-per-identity amount when that distribution is enabled.
    fn parse_once_per_identity_amount(&self) -> Result<Option<TokenAmount>, String> {
        if !self.enable_once_per_identity_distribution {
            return Ok(None);
        }
        parse_once_per_identity_amount(&self.once_per_identity_amount_input)
            .map(Some)
            .map_err(|error| error.to_string())
    }

    /// Configure transferability in the Advanced section on every supported network.
    pub(super) fn transferable_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            StyledCheckbox::new(
                &mut self.transferable_input,
                "Holders can send this token to other identities",
            ).show(ui);
            crate::ui::helpers::info_icon_button(ui, "When turned off, holders cannot send the token to other identities. It can still be minted, claimed, bought, burned, frozen and destroyed as configured. This cannot be changed after the token is created.");
        });
    }

    /// The confirmation's sentence about whether holders can send the token.
    /// The irreversible "cannot send" choice is always stated; the transferable
    /// default only with Advanced Options, the only place it can be changed.
    pub(super) fn transferability_summary(
        transferable: bool,
        advanced_options: bool,
    ) -> Option<&'static str> {
        match (transferable, advanced_options) {
            (false, _) => Some(
                "Holders cannot send this token to other identities. This cannot be changed later.",
            ),
            (true, true) => Some("Holders can send this token to other identities."),
            (true, false) => None,
        }
    }

    /// The registration confirmation text: what gets signed, and its cost.
    /// `marketplace_locked` adds the warning that the token can never be
    /// offered on the token marketplace.
    pub(super) fn registration_confirmation_message(
        name: &str,
        base_supply: &str,
        max_supply: &str,
        transferability: Option<&str>,
        once_per_identity_amount: Option<TokenAmount>,
        cost: f64,
        marketplace_locked: bool,
    ) -> String {
        let mut message = format!(
            "Are you sure you want to register a new token contract with these settings?\n\nName: {name}\nBase Supply: {base_supply}\nMax Supply: {max_supply}"
        );
        if let Some(transferability) = transferability {
            message.push('\n');
            message.push_str(transferability);
        }
        if let Some(amount) = once_per_identity_amount {
            message.push('\n');
            message.push_str(&format!(
                "Each identity can claim {amount} base tokens of this token, once."
            ));
        }
        message.push_str("\n\n");
        message.push_str(&format!(
            "Estimated cost to register this token is {cost} Dash"
        ));
        if marketplace_locked {
            message.push_str("\n\n");
            message.push_str(
                "WARNING: This token can never be offered for sale on the token marketplace. This cannot be changed later.",
            );
        }
        message
    }

    fn parse_contract_keywords(&self) -> Result<Vec<String>, String> {
        if self.contract_keywords_input.trim().is_empty() {
            return Ok(Vec::new());
        }

        self.contract_keywords_input
            .split(',')
            .map(|s| {
                let trimmed = s.trim().to_string();
                if let Err(error) = validate_contract_keyword(&trimmed) {
                    Err(format!(
                        "Contract keyword '{keyword}' must contain between {min} and {max} characters.",
                        keyword = trimmed,
                        min = error.min,
                        max = error.max
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
                    "Duplicate token name language: {language:?}",
                    language = name_with_language.2
                ));
            }
            seen_languages.insert(name_with_language.2);
        }

        let mut token_names = Vec::with_capacity(self.token_names_input.len());
        for name_with_language in &self.token_names_input {
            if name_with_language.0.len() < 3 || name_with_language.0.len() > 50 {
                return Err(format!(
                    "The name in {language:?} must be between 3 and 50 characters",
                    language = name_with_language.2
                ));
            }

            if name_with_language.1.len() < 3 || name_with_language.1.len() > 50 {
                return Err(format!(
                    "The plural form in {language:?} must be between 3 and 50 characters",
                    language = name_with_language.2
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
        let rules = PresetRules::from_preset(&preset);

        self.manual_minting_rules = rules.manual_minting.into();
        self.manual_burning_rules = rules.manual_burning.into();
        self.freeze_rules = rules.freeze.into();
        self.unfreeze_rules = rules.unfreeze.into();
        self.destroy_frozen_funds_rules = rules.destroy_frozen_funds.into();
        self.emergency_action_rules = rules.emergency_action.into();
        self.max_supply_change_rules = rules.max_supply_change.into();
        self.conventions_change_rules = rules.conventions_change.into();
        self.perpetual_distribution_rules = rules.perpetual_distribution.into();
        self.new_tokens_destination_identity_rules = rules.new_tokens_destination_identity.into();
        self.minting_allow_choosing_destination_rules =
            rules.minting_allow_choosing_destination.into();
        self.minting_allow_choosing_destination = rules.minting_may_choose_destination;
        self.marketplace_rules = rules.marketplace.into();
        self.change_direct_purchase_pricing_rules = rules.direct_purchase_pricing.into();
        self.authorized_main_control_group_change = rules.main_control_group_change;

        // Reset optional identity/group inputs related to control group modification
        self.main_control_group_change_authorized_identity = None;
        self.main_control_group_change_authorized_group = None;

        self.selected_token_preset = TokenPresetSelection::Preset(preset.features);
    }

    /// Whether every rule a preset controls currently holds exactly `expected`.
    fn rules_match(&self, expected: &PresetRules) -> bool {
        let PresetRules {
            manual_minting,
            manual_burning,
            freeze,
            unfreeze,
            destroy_frozen_funds,
            emergency_action,
            max_supply_change,
            conventions_change,
            perpetual_distribution,
            new_tokens_destination_identity,
            minting_allow_choosing_destination,
            minting_may_choose_destination,
            marketplace,
            direct_purchase_pricing,
            main_control_group_change,
        } = expected;

        self.manual_minting_rules.rules == *manual_minting
            && self.manual_burning_rules.rules == *manual_burning
            && self.freeze_rules.rules == *freeze
            && self.unfreeze_rules.rules == *unfreeze
            && self.destroy_frozen_funds_rules.rules == *destroy_frozen_funds
            && self.emergency_action_rules.rules == *emergency_action
            && self.max_supply_change_rules.rules == *max_supply_change
            && self.conventions_change_rules.rules == *conventions_change
            && self.perpetual_distribution_rules.rules == *perpetual_distribution
            && self.new_tokens_destination_identity_rules.rules == *new_tokens_destination_identity
            && self.minting_allow_choosing_destination_rules.rules
                == *minting_allow_choosing_destination
            && self.minting_allow_choosing_destination == *minting_may_choose_destination
            && self.marketplace_rules.rules == *marketplace
            && self.change_direct_purchase_pricing_rules.rules == *direct_purchase_pricing
            && self.authorized_main_control_group_change == *main_control_group_change
    }

    /// Keeps the preset selector truthful. Once the rules stop matching what it
    /// shows — a rule edited in Advanced mode after picking a preset, or before
    /// picking any — it shows "Custom", and the token is built from those rules.
    pub(super) fn reconcile_selected_preset(&mut self) {
        let still_accurate = match self.selected_token_preset {
            TokenPresetSelection::Unselected => self.rules_match(PresetRules::initial()),
            TokenPresetSelection::Preset(features) => {
                self.rules_match(PresetRules::for_creator_preset(features))
            }
            TokenPresetSelection::Custom => true,
        };
        if !still_accurate {
            self.selected_token_preset = TokenPresetSelection::Custom;
        }
    }

    /// Whether Simple mode's "Create Token" is enabled: a name, an initial supply
    /// and a preset — or individually chosen ("Custom") rules — are required.
    #[cfg(test)]
    pub(super) fn can_create_in_simple_mode(&self) -> bool {
        self.simple_mode_blocked_reason().is_none()
    }

    /// Why Simple mode's "Create Token" is blocked, shown next to the button:
    /// a registration in flight first, then missing inputs, then the preset.
    pub(super) fn simple_mode_blocked_reason(&self) -> Option<String> {
        if self.awaits_token_registration() {
            Some(REGISTRATION_IN_FLIGHT_HINT.to_owned())
        } else if self.token_names_input[0].0.trim().is_empty() {
            Some("Enter a token name to continue.".to_owned())
        } else if self.base_supply_amount.is_none() {
            Some("Enter an initial supply to continue.".to_owned())
        } else if !self.preset_chosen() {
            Some(SIMPLE_PRESET_HINT.to_owned())
        } else {
            None
        }
    }

    /// Why Advanced mode's "Register Token Contract" is blocked, shown next to
    /// the button: a registration in flight first, then the preset.
    pub(super) fn advanced_mode_blocked_reason(&self) -> Option<String> {
        if self.awaits_token_registration() {
            Some(REGISTRATION_IN_FLIGHT_HINT.to_owned())
        } else if !self.preset_chosen() {
            Some(ADVANCED_PRESET_HINT.to_owned())
        } else {
            None
        }
    }

    /// Why the registration cannot be signed yet: no signing key, or its
    /// wallet is locked.
    fn signing_blocked_reason(&self) -> Option<String> {
        if self.selected_key.is_none() {
            Some(crate::ui::tokens::CHOOSE_SIGNING_KEY.to_owned())
        } else if self
            .selected_wallet
            .as_ref()
            .is_some_and(wallet_needs_unlock)
        {
            Some("Unlock the wallet to continue.".to_owned())
        } else {
            None
        }
    }

    /// Visible explanation next to a blocked control, so the reason does not
    /// depend on hovering.
    fn render_blocked_hint(ui: &mut Ui, reason: &str) {
        let dark_mode = ui.style().visuals.dark_mode;
        ui.label(
            RichText::new(reason)
                .color(DashColors::muted_color(dark_mode))
                .italics(),
        );
    }

    /// Whether a preset or individually chosen ("Custom") rules were picked. Both
    /// Simple and Advanced mode refuse to create a token before that, so nobody
    /// registers the all-"no one" starting rules without choosing them.
    pub(super) fn preset_chosen(&self) -> bool {
        self.selected_token_preset != TokenPresetSelection::Unselected
    }

    /// The text the preset selector shows for the current selection.
    pub(super) fn preset_selection_label(&self) -> &'static str {
        match self.selected_token_preset {
            TokenPresetSelection::Unselected => "Select a preset...",
            TokenPresetSelection::Preset(features) => preset_label(features),
            TokenPresetSelection::Custom => CUSTOM_PRESET_LABEL,
        }
    }

    fn selected_registration_signer(&self) -> Option<(&QualifiedIdentity, &IdentityPublicKey)> {
        if self
            .selected_wallet
            .as_ref()
            .is_some_and(wallet_needs_unlock)
        {
            return None;
        }
        let identity = self.selected_identity.as_ref()?;
        let signing_key = signable_key_now(
            identity,
            self.selected_key.as_ref()?,
            self.app_context.contract_signing_scope(),
        )?;
        Some((identity, signing_key))
    }

    fn pending_registration_signer(
        &self,
        pending: &PendingTokenRegistration,
    ) -> Option<(&QualifiedIdentity, &IdentityPublicKey)> {
        self.selected_registration_signer()
            .filter(|(identity, key)| {
                identity.identity.id() == pending.args.identity_id
                    && same_key(&pending.signing_key, key)
            })
    }

    pub(super) fn cancel_invalid_token_registration(&mut self) {
        if self
            .pending_token_registration
            .as_ref()
            .is_some_and(|pending| self.pending_registration_signer(pending).is_none())
        {
            self.pending_token_registration = None;
        }
    }

    pub(super) fn open_token_registration_confirmation(&mut self, context: &Context) {
        let Some((_, signing_key)) = self.selected_registration_signer() else {
            return;
        };
        let signing_key = signing_key.clone();
        let args = match self.parse_token_build_args() {
            Ok(args) => args,
            Err(error) => {
                MessageBanner::set_global(context, &error, MessageType::Error);
                return;
            }
        };
        let base_supply_display =
            Amount::new(args.base_supply, args.decimals).to_string_opts(false, false);
        let max_supply_display = args
            .max_supply
            .map(|amount| Amount::new(amount, args.decimals).to_string_opts(false, false))
            .unwrap_or_else(|| "None".to_string());

        // Tokens are created closed to the marketplace; warn if that can never change.
        let is_danger_mode = matches!(
            args.marketplace_rules,
            ChangeControlRules::V0(ChangeControlRulesV0 {
                authorized_to_make_change: AuthorizedActionTakers::NoOne,
                admin_action_takers: AuthorizedActionTakers::NoOne,
                ..
            })
        );
        let cost = self.estimate_registration_cost() as f64 / 100_000_000_000.0;
        let transferability =
            Self::transferability_summary(args.transferable, self.show_advanced_token_creator);
        let once_per_identity_amount = once_per_identity_amount_in_rules(&args.distribution_rules);
        let confirmation_message = Self::registration_confirmation_message(
            &args.token_names[0].0,
            &base_supply_display,
            &max_supply_display,
            transferability,
            once_per_identity_amount,
            cost,
            is_danger_mode,
        );

        let dialog =
            ConfirmationDialog::new("Confirm Token Contract Registration", confirmation_message)
                .confirm_text(Some("Confirm"))
                .cancel_text(Some("Cancel"))
                .danger_mode(is_danger_mode)
                .blocks_input(true);
        self.pending_token_registration = Some(PendingTokenRegistration {
            args,
            signing_key,
            dialog,
        });
    }

    pub(super) fn render_token_creator_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let Some(mut pending) = self.pending_token_registration.take() else {
            return AppAction::None;
        };
        if self.pending_registration_signer(&pending).is_none() {
            return AppAction::None;
        }
        match pending.dialog.show(ui).inner.dialog_response {
            None => {
                self.pending_token_registration = Some(pending);
                return AppAction::None;
            }
            Some(ConfirmationStatus::Canceled) => return AppAction::None,
            Some(ConfirmationStatus::Confirmed) => {}
        }

        let Some((identity, signing_key)) = self.pending_registration_signer(&pending) else {
            return AppAction::None;
        };
        let register = BackendTask::TokenTask(Box::new(TokenTask::RegisterTokenContract {
            identity: identity.clone(),
            signing_key: Box::new(signing_key.clone()),
            params: Box::new(pending.args.into_contract_params()),
        }));
        let network = self.app_context.network;
        let register_context = BackendTaskContext::for_dispatch_on(&register, network);
        let refresh = BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances));
        let refresh_context = BackendTaskContext::for_dispatch_on(&refresh, network);
        let tasks = vec![
            (register, register_context.clone()),
            (refresh, refresh_context),
        ];
        self.mark_token_registration_started(register_context);
        AppAction::BackendTasksWithContexts(tasks, BackendTasksExecutionMode::Sequential)
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
                ui.label(
                    "Paste the document-schemas JSON object for this contract, with one schema entry per document type.",
                );

                ui.add_space(5.0);

                let dark_mode = ui.style().visuals.dark_mode;
                let schemas_response = ui.add_sized(
                    [ui.available_width(), 120.0],
                    TextEdit::multiline(&mut self.document_schemas_input)
                        .text_color(DashColors::text_primary(dark_mode))
                        .background_color(DashColors::input_background(dark_mode)),
                );

                if schemas_response.changed() {
                    self.parse_document_schemas();
                }

                ui.add_space(5.0);

                // Show validation result
                if let Some(ref error) = self.document_schemas_error {
                    ui.colored_label(
                        Color32::DARK_RED,
                        format!("Schema validation error: {error}"),
                    );
                } else if let Some(parsed_document_schemas) = self.parsed_document_schemas.as_ref() {
                    let schema_count = parsed_document_schemas.len();
                    if schema_count > 0 {
                        ui.colored_label(
                            Color32::DARK_GREEN,
                            format!("✓ {schema_count} valid document schema(s) parsed"),
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
                                        "Document schema '{key}' is missing the required 'type' field"
                                    ));
                                    return;
                                }
                            } else {
                                self.document_schemas_error =
                                    Some(format!("Document schema '{key}' must be an object"));
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
            Err(error) => {
                tracing::debug!(?error, "Token document-schema JSON parsing failed");
                self.document_schemas_error = Some(
                    "The document schema JSON is not valid. Check its syntax and try again."
                        .to_string(),
                );
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

/// Why creating is blocked while a registration is in flight: a second one
/// would pay a second fee.
const REGISTRATION_IN_FLIGHT_HINT: &str =
    "Your token is being created. Wait for the result before creating another token.";

/// Why Simple mode's "Create Token" is blocked until a preset is chosen.
const SIMPLE_PRESET_HINT: &str = "Choose a preset to create the token.";

/// Why Advanced mode's "Register Token Contract" is blocked until a preset or
/// "Custom" is chosen; editing a rule below also switches to "Custom".
const ADVANCED_PRESET_HINT: &str = "Choose a preset, or Custom to use the rules below.";

/// Every preset the creator offers, in selector order.
const CREATOR_PRESETS: [TokenConfigurationPresetFeatures; 5] = [
    MostRestrictive,
    WithOnlyEmergencyAction,
    WithMintingAndBurningActions,
    WithAllAdvancedActions,
    WithExtremeActions,
];

/// Selector text for rules that are not exactly one preset's.
const CUSTOM_PRESET_LABEL: &str = "Custom";

/// Selector text for a preset.
fn preset_label(features: TokenConfigurationPresetFeatures) -> &'static str {
    match features {
        MostRestrictive => "Most Restrictive",
        WithOnlyEmergencyAction => "Only Emergency Action",
        WithMintingAndBurningActions => "Minting and Burning",
        WithAllAdvancedActions => "Advanced Actions",
        WithExtremeActions => "All Allowed",
    }
}

/// The preset the Token Creator offers for `features`: the contract owner is the
/// one authorized for every action the preset allows.
fn creator_preset(features: TokenConfigurationPresetFeatures) -> TokenConfigurationPreset {
    TokenConfigurationPreset {
        features,
        action_taker: AuthorizedActionTakers::ContractOwner,
    }
}

/// Every rule a Token Creator preset sets, taken from the Platform preset
/// definition. The single mapping used both to apply a preset and to check that
/// the rules on screen still match it.
#[derive(PartialEq)]
struct PresetRules {
    manual_minting: ChangeControlRulesV0,
    manual_burning: ChangeControlRulesV0,
    freeze: ChangeControlRulesV0,
    unfreeze: ChangeControlRulesV0,
    destroy_frozen_funds: ChangeControlRulesV0,
    emergency_action: ChangeControlRulesV0,
    max_supply_change: ChangeControlRulesV0,
    conventions_change: ChangeControlRulesV0,
    perpetual_distribution: ChangeControlRulesV0,
    new_tokens_destination_identity: ChangeControlRulesV0,
    minting_allow_choosing_destination: ChangeControlRulesV0,
    /// Whether a mint may send the new tokens to an identity other than the default.
    minting_may_choose_destination: bool,
    marketplace: ChangeControlRulesV0,
    direct_purchase_pricing: ChangeControlRulesV0,
    main_control_group_change: AuthorizedActionTakers,
}

impl PresetRules {
    /// The rules of a freshly opened creator, before any preset or rule is chosen.
    /// Built once: the selector compares against it every frame.
    fn initial() -> &'static Self {
        static INITIAL: LazyLock<PresetRules> = LazyLock::new(PresetRules::initial_rules);
        &INITIAL
    }

    /// The rules of `features` as the creator offers it (owner-authorized).
    /// Built once per preset: the selector compares against them every frame,
    /// and deriving them builds Platform's whole token configuration.
    fn for_creator_preset(features: TokenConfigurationPresetFeatures) -> &'static Self {
        static TABLE: LazyLock<[PresetRules; CREATOR_PRESETS.len()]> = LazyLock::new(|| {
            CREATOR_PRESETS.map(|features| PresetRules::from_preset(&creator_preset(features)))
        });
        // Positions in `CREATOR_PRESETS`; the match keeps every preset covered.
        let index = match features {
            MostRestrictive => 0,
            WithOnlyEmergencyAction => 1,
            WithMintingAndBurningActions => 2,
            WithAllAdvancedActions => 3,
            WithExtremeActions => 4,
        };
        &TABLE[index]
    }

    fn initial_rules() -> Self {
        Self {
            manual_minting: ChangeControlRulesV0::default(),
            manual_burning: ChangeControlRulesV0::default(),
            freeze: ChangeControlRulesV0::default(),
            unfreeze: ChangeControlRulesV0::default(),
            destroy_frozen_funds: ChangeControlRulesV0::default(),
            emergency_action: ChangeControlRulesV0::default(),
            max_supply_change: ChangeControlRulesV0::default(),
            conventions_change: ChangeControlRulesV0::default(),
            perpetual_distribution: ChangeControlRulesV0::default(),
            new_tokens_destination_identity: ChangeControlRulesV0::default(),
            minting_allow_choosing_destination: ChangeControlRulesV0::default(),
            minting_may_choose_destination: false,
            marketplace: ChangeControlRulesV0::default(),
            direct_purchase_pricing: ChangeControlRulesV0::default(),
            main_control_group_change: AuthorizedActionTakers::NoOne,
        }
    }

    /// Takes every value straight from Platform's own configuration for the preset,
    /// built "with direct pricing" so whoever may manage the token may also set its
    /// direct purchase price.
    fn from_preset(preset: &TokenConfigurationPreset) -> Self {
        use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::v0::TokenConfigurationConventionV0;
        use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
        use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::accessors::v0::TokenDistributionRulesV0Getters;
        use dash_sdk::dpp::data_contract::associated_token::token_marketplace_rules::TokenMarketplaceRules;

        fn v0(rules: &ChangeControlRules) -> ChangeControlRulesV0 {
            let ChangeControlRules::V0(rules) = rules;
            rules.clone()
        }

        let config = preset.token_configuration_v0(
            TokenConfigurationConvention::V0(TokenConfigurationConventionV0 {
                localizations: Default::default(),
                decimals: 8,
            }),
            0,
            None,
            true,
            true,
        );
        let distribution = &config.distribution_rules;
        let TokenMarketplaceRules::V0(marketplace) = &config.marketplace_rules;

        Self {
            manual_minting: v0(&config.manual_minting_rules),
            manual_burning: v0(&config.manual_burning_rules),
            freeze: v0(&config.freeze_rules),
            unfreeze: v0(&config.unfreeze_rules),
            destroy_frozen_funds: v0(&config.destroy_frozen_funds_rules),
            emergency_action: v0(&config.emergency_action_rules),
            max_supply_change: v0(&config.max_supply_change_rules),
            conventions_change: v0(&config.conventions_change_rules),
            perpetual_distribution: v0(distribution.perpetual_distribution_rules()),
            new_tokens_destination_identity: v0(
                distribution.new_tokens_destination_identity_rules()
            ),
            minting_allow_choosing_destination: v0(
                distribution.minting_allow_choosing_destination_rules()
            ),
            minting_may_choose_destination: distribution.minting_allow_choosing_destination(),
            marketplace: v0(&marketplace.trade_mode_change_rules),
            direct_purchase_pricing: v0(distribution.change_direct_purchase_pricing_rules()),
            main_control_group_change: config.main_control_group_can_be_modified,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The selector reconciles every frame, so each creator preset's rules are
    /// derived once and reused, and they equal what applying the preset sets.
    #[test]
    fn creator_preset_rules_are_built_once_and_match_the_preset() {
        for features in CREATOR_PRESETS {
            let cached = PresetRules::for_creator_preset(features);
            assert!(std::ptr::eq(
                cached,
                PresetRules::for_creator_preset(features)
            ));
            assert!(*cached == PresetRules::from_preset(&creator_preset(features)));
        }
        assert!(std::ptr::eq(PresetRules::initial(), PresetRules::initial()));
    }
}
