use std::collections::{BTreeMap, HashSet};
use chrono::Utc;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPreset;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPresetFeatures::{MostRestrictive, WithAllAdvancedActions, WithExtremeActions, WithMintingAndBurningActions, WithOnlyEmergencyAction};
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::epaint::Color32;
use egui::{ComboBox, Context, RichText, TextEdit, Ui};
use crate::app::{AppAction, BackendTasksExecutionMode};
use crate::backend_task::BackendTask;
use crate::backend_task::tokens::TokenTask;
use crate::ui::components::styled::{StyledCheckbox};
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::helpers::{add_identity_key_chooser, TransactionType};
use crate::ui::tokens::tokens_screen::{TokenBuildArgs, TokenCreatorStatus, TokenNameLanguage, TokensScreen};

impl TokensScreen {
    pub(super) fn render_token_creator(&mut self, context: &Context, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // 1) If we've successfully completed contract creation, show a success UI
        if self.token_creator_status == TokenCreatorStatus::Complete {
            self.render_token_creator_success_screen(ui);
            return action;
        }

        ui.heading("Token Creator");
        ui.label(
            "Create custom tokens on Dash Platform with advanced features and distribution rules",
        );
        ui.add_space(20.0);

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
                                ui.colored_label(Color32::DARK_RED, format!("Error loading identities from local DB: {}", e));
                                return;
                            }
                        };
                        if all_identities.is_empty() {
                            ui.colored_label(
                                Color32::DARK_RED,
                                "No identities loaded. Please load or create one to register the token contract with first.",
                            );
                            return;
                        }

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
                        if let (Some(ref qid), Some(ref key)) = (&self.selected_identity, &self.selected_key) {
                            // If the key belongs to a wallet, set that wallet reference:
                            self.selected_wallet = crate::ui::identities::get_selected_wallet(
                                qid,
                                None,
                                Some(key),
                                &mut self.token_creator_error_message,
                            );
                        }

                        if self.selected_key.is_none() {
                            return;
                        }

                        ui.add_space(10.0);
                        ui.separator();

                        // 3) If the wallet is locked, show unlock
                        //    But only do this step if we actually have a wallet reference:
                        let mut need_unlock = false;
                        let mut just_unlocked = false;

                        if self.selected_wallet.is_some() {
                            let (n, j) = self.render_wallet_unlock_if_needed(ui);
                            need_unlock = n;
                            just_unlocked = j;
                        }

                        if need_unlock && !just_unlocked {
                            // We must wait for unlock before continuing
                            return;
                        }

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
                                        if i == 0 {
                                            ui.push_id(format!("combo_{}", i), |ui| {
                                                ui.style_mut().spacing.combo_height = 10.0;
                                                ui.style_mut().spacing.button_padding = egui::vec2(3.0, 0.0);
                                                ui.style_mut().visuals.widgets.inactive.fg_stroke.width = 1.0;
                                                ui.style_mut().text_styles.get_mut(&egui::TextStyle::Body).unwrap().size = 12.0;
                                                let combo_resp = ComboBox::from_id_salt(format!("token_name_language_selector_{}", i))
                                                    .selected_text(format!(
                                                        "{}",
                                                        self.token_names_input[i].2
                                                    ))
                                                    .width(100.0);
                                                combo_resp.show_ui(ui, |ui| {
                                                    ui.style_mut().text_styles.get_mut(&egui::TextStyle::Body).unwrap().size = 12.0;
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::English, "English");
                                                });
                                            });
                                        } else {
                                            ui.push_id(format!("combo_{}", i), |ui| {
                                                    ui.style_mut().spacing.combo_height = 10.0;
                                                    ui.style_mut().spacing.button_padding = egui::vec2(3.0, 0.0);
                                                    ui.style_mut().visuals.widgets.inactive.fg_stroke.width = 1.0;
                                                    ui.style_mut().text_styles.get_mut(&egui::TextStyle::Body).unwrap().size = 12.0;
                                                let combo_resp = ComboBox::from_id_salt(format!("token_name_language_selector_{}", i))
                                                    .selected_text(format!(
                                                        "{}",
                                                        self.token_names_input[i].2
                                                    ))
                                                    .width(100.0);
                                                combo_resp.show_ui(ui, |ui| {
                                                    ui.style_mut().text_styles.get_mut(&egui::TextStyle::Body).unwrap().size = 12.0;
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::English, "English");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Arabic, "Arabic");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Bengali, "Bengali");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Burmese, "Burmese");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Chinese, "Chinese");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Czech, "Czech");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Dutch, "Dutch");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Farsi, "Farsi (Persian)");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Filipino, "Filipino (Tagalog)");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::French, "French");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::German, "German");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Greek, "Greek");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Gujarati, "Gujarati");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Hausa, "Hausa");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Hebrew, "Hebrew");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Hindi, "Hindi");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Hungarian, "Hungarian");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Igbo, "Igbo");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Indonesian, "Indonesian");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Italian, "Italian");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Japanese, "Japanese");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Javanese, "Javanese");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Kannada, "Kannada");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Khmer, "Khmer");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Korean, "Korean");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Malay, "Malay");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Malayalam, "Malayalam");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Mandarin, "Mandarin Chinese");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Marathi, "Marathi");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Nepali, "Nepali");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Oriya, "Oriya");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Pashto, "Pashto");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Polish, "Polish");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Portuguese, "Portuguese");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Punjabi, "Punjabi");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Romanian, "Romanian");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Russian, "Russian");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Serbian, "Serbian");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Sindhi, "Sindhi");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Sinhala, "Sinhala");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Somali, "Somali");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Spanish, "Spanish");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Swahili, "Swahili");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Swedish, "Swedish");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Tamil, "Tamil");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Telugu, "Telugu");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Thai, "Thai");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Turkish, "Turkish");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Ukrainian, "Ukrainian");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Urdu, "Urdu");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Vietnamese, "Vietnamese");
                                                    ui.selectable_value(&mut self.token_names_input[i].2, TokenNameLanguage::Yoruba, "Yoruba");
                                                });
                                            });
                                        }

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
                                ui.label("Base Supply*:");
                                ui.text_edit_singleline(&mut self.base_supply_input);
                                ui.end_row();

                                // Row 3: Max Supply
                                ui.label("Max Supply:");
                                ui.text_edit_singleline(&mut self.max_supply_input);
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
                        let mut advanced_state = egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            ui.make_persistent_id("token_creator_advanced"),
                            false,
                        );

                        // Force close if we need to reset
                        if self.should_reset_collapsing_states {
                            advanced_state.set_open(false);
                        }

                        advanced_state.store(ui.ctx());

                        advanced_state.show_header(ui, |ui| {
                            ui.label("Advanced");
                        })
                        .body(|ui| {
                            ui.add_space(3.0);

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
                                            format!("Non Fractional Token (i.e 0, 1, 2 or 10 {})", token_name)
                                        } else {
                                            format!("Fractional Token (i.e 0.2 {})", token_name)
                                        };

                                        ui.label(RichText::new(message).color(Color32::GRAY));

                                        crate::ui::helpers::info_icon_button(ui, "The decimal places of the token, for example Dash and Bitcoin use 8. The minimum indivisible amount is a Duff or a Satoshi respectively. If you put a value greater than 0 this means that it is indicated that the consensus is that 10^(number entered) is what represents 1 full unit of the token.");
                                    });
                                    ui.end_row();
                                });
                        });

                        ui.add_space(5.0);

                        let mut action_rules_state = egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            ui.make_persistent_id("token_creator_action_rules"),
                            false,
                        );

                        // Force close if we need to reset
                        if self.should_reset_collapsing_states {
                            action_rules_state.set_open(false);
                        }

                        action_rules_state.store(ui.ctx());

                        action_rules_state.show_header(ui, |ui| {
                            ui.label("Action Rules");
                        })
                        .body(|ui| {
                            ui.horizontal(|ui| {
                                ui.label("Preset:");

                                ComboBox::from_id_salt("preset_selector")
                                    .selected_text(
                                        self.selected_token_preset
                                            .map(|p|                                         match p {
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

                            ui.add_space(3.0);

                            self.manual_minting_rules.render_mint_control_change_rules_ui(ui, &self.groups_ui, &mut self.new_tokens_destination_identity_should_default_to_contract_owner, &mut self.new_tokens_destination_other_identity_enabled, &mut self.minting_allow_choosing_destination, &mut self.new_tokens_destination_identity_rules, &mut self.new_tokens_destination_other_identity, &mut self.minting_allow_choosing_destination_rules);
                            self.manual_burning_rules.render_control_change_rules_ui(ui, &self.groups_ui,"Manual Burn", None);
                            self.freeze_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Freeze", Some(&mut self.allow_transfers_to_frozen_identities));
                            self.unfreeze_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Unfreeze", None);
                            self.destroy_frozen_funds_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Destroy Frozen Funds", None);
                            self.emergency_action_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Emergency Action", None);
                            self.max_supply_change_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Max Supply Change", None);
                            self.conventions_change_rules.render_control_change_rules_ui(ui, &self.groups_ui, "Conventions Change", None);

                            // Main control group change is slightly different so do this one manually.
                            let mut main_control_state = egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                ui.make_persistent_id("token_creator_main_control_group"),
                                false,
                            );

                            // Force close if we need to reset
                            if self.should_reset_collapsing_states {
                                main_control_state.set_open(false);
                            }

                            main_control_state.store(ui.ctx());

                            main_control_state.show_header(ui, |ui| {
                                ui.label("Main Control Group Change");
                            })
                            .body(|ui| {
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
                            });
                        });

                        self.render_distributions(context, ui);
                        self.render_groups(ui);
                        self.render_document_schemas(ui);

                        // 6) "Register Token Contract" button
                        ui.add_space(10.0);
                        let mut new_style = (**ui.style()).clone();
                        new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                        ui.set_style(new_style);
                        ui.horizontal(|ui| {
                            if ui.button("Register Token Contract").clicked() {
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

                            if ui.button("View JSON").clicked() {
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
            });

        // Reset the flag after processing all collapsing headers
        if self.should_reset_collapsing_states {
            self.should_reset_collapsing_states = false;
        }

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
                ui.add(egui::widgets::Spinner::default());
            });
        }

        // Show an error if we have one
        if let Some(err_msg) = &self.token_creator_error_message {
            ui.add_space(10.0);
            ui.colored_label(Color32::DARK_RED, err_msg.to_string());
            ui.add_space(10.0);
        }

        action
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
        let mut contract_keywords = if self.contract_keywords_input.trim().is_empty() {
            Vec::new()
        } else {
            self.contract_keywords_input
                .split(',')
                .map(|s| {
                    let trimmed = s.trim().to_string();
                    if trimmed.len() < 3 || trimmed.len() > 50 {
                        Err(format!("Invalid contract keyword {}, keyword must be between 3 and 50 characters", trimmed))
                    } else {
                        Ok(trimmed)
                    }
                })
                .collect::<Result<Vec<String>, String>>()?
        };

        // 2) Basic fields
        if self.token_names_input.is_empty() {
            return Err("Please enter a token name".to_string());
        }
        // If any name languages are duplicated, return an error
        let mut seen_languages = HashSet::new();
        for name_with_language in self.token_names_input.iter() {
            if seen_languages.contains(&name_with_language.2) {
                return Err(format!(
                    "Duplicate token name language: {:?}",
                    name_with_language.1
                ));
            }
            seen_languages.insert(name_with_language.2);
        }
        let mut token_names: Vec<(String, String, String)> = Vec::new();
        for name_with_language in self.token_names_input.iter() {
            let language = match name_with_language.2 {
                TokenNameLanguage::English => "en",
                TokenNameLanguage::Arabic => "ar",
                TokenNameLanguage::Bengali => "bn",
                TokenNameLanguage::Burmese => "my",
                TokenNameLanguage::Chinese => "zh",
                TokenNameLanguage::Czech => "cs",
                TokenNameLanguage::Dutch => "nl",
                TokenNameLanguage::Farsi => "fa",
                TokenNameLanguage::Filipino => "fil",
                TokenNameLanguage::French => "fr",
                TokenNameLanguage::German => "de",
                TokenNameLanguage::Greek => "el",
                TokenNameLanguage::Gujarati => "gu",
                TokenNameLanguage::Hausa => "ha",
                TokenNameLanguage::Hebrew => "he",
                TokenNameLanguage::Hindi => "hi",
                TokenNameLanguage::Hungarian => "hu",
                TokenNameLanguage::Igbo => "ig",
                TokenNameLanguage::Indonesian => "id",
                TokenNameLanguage::Italian => "it",
                TokenNameLanguage::Japanese => "ja",
                TokenNameLanguage::Javanese => "jv",
                TokenNameLanguage::Kannada => "kn",
                TokenNameLanguage::Khmer => "km",
                TokenNameLanguage::Korean => "ko",
                TokenNameLanguage::Malay => "ms",
                TokenNameLanguage::Malayalam => "ml",
                TokenNameLanguage::Mandarin => "zh", // synonym for Chinese
                TokenNameLanguage::Marathi => "mr",
                TokenNameLanguage::Nepali => "ne",
                TokenNameLanguage::Oriya => "or",
                TokenNameLanguage::Pashto => "ps",
                TokenNameLanguage::Polish => "pl",
                TokenNameLanguage::Portuguese => "pt",
                TokenNameLanguage::Punjabi => "pa",
                TokenNameLanguage::Romanian => "ro",
                TokenNameLanguage::Russian => "ru",
                TokenNameLanguage::Serbian => "sr",
                TokenNameLanguage::Sindhi => "sd",
                TokenNameLanguage::Sinhala => "si",
                TokenNameLanguage::Somali => "so",
                TokenNameLanguage::Spanish => "es",
                TokenNameLanguage::Swahili => "sw",
                TokenNameLanguage::Swedish => "sv",
                TokenNameLanguage::Tamil => "ta",
                TokenNameLanguage::Telugu => "te",
                TokenNameLanguage::Thai => "th",
                TokenNameLanguage::Turkish => "tr",
                TokenNameLanguage::Ukrainian => "uk",
                TokenNameLanguage::Urdu => "ur",
                TokenNameLanguage::Vietnamese => "vi",
                TokenNameLanguage::Yoruba => "yo",
            };

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
                language.to_owned(),
            ));

            // are we searchable?
            if name_with_language.3 {
                contract_keywords.push(name_with_language.0.clone());
            }
        }

        let token_description = if !self.token_description_input.is_empty() {
            Some(self.token_description_input.clone())
        } else {
            None
        };
        let decimals = self
            .decimals_input
            .parse::<u8>()
            .map_err(|_| "Invalid decimal places amount".to_string())?;
        let base_supply = self
            .base_supply_input
            .parse::<u64>()
            .map_err(|_| "Invalid base supply amount".to_string())?;
        let max_supply = if self.max_supply_input.is_empty() {
            None
        } else {
            // If parse fails, error out
            Some(
                self.max_supply_input
                    .parse::<u64>()
                    .map_err(|_| "Invalid Max Supply".to_string())?,
            )
        };

        let start_paused = self.start_as_paused_input;
        let allow_transfers_to_frozen_identities = self.allow_transfers_to_frozen_identities;
        let keeps_history = self.token_advanced_keeps_history.into();

        let main_control_group = if self.main_control_group_input.is_empty() {
            None
        } else {
            Some(
                self.main_control_group_input
                    .parse::<u16>()
                    .map_err(|_| "Invalid main control group".to_string())?,
            )
        };

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

        // 6) Put it all in a struct
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
        })
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

        // Reset optional identity/group inputs related to control group modification
        self.main_control_group_change_authorized_identity = None;
        self.main_control_group_change_authorized_group = None;

        // Set `selected_token_preset` so UI shows current preset (Optional)
        self.selected_token_preset = Some(preset.features);
    }

    /// Shows a popup "Are you sure?" for creating the token contract
    fn render_token_creator_confirmation_popup(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        let mut is_open = true;

        egui::Window::new("Confirm Token Contract Registration")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                ui.label(
                    "Are you sure you want to register a new token contract with these settings?\n",
                );
                let max_supply_display = if self.max_supply_input.is_empty() {
                    "None".to_string()
                } else {
                    self.max_supply_input.clone()
                };
                ui.label(format!(
                    "Name: {}\nBase Supply: {}\nMax Supply: {}",
                    self.token_names_input[0].0, self.base_supply_input, max_supply_display,
                ));

                ui.add_space(10.0);

                ui.label(format!(
                    "Estimated cost to register this token is {} Dash",
                    self.estimate_registration_cost() as f64 / 100_000_000_000.0
                ));

                ui.add_space(10.0);

                // Confirm
                if ui.button("Confirm").clicked() {
                    let args = match &self.cached_build_args {
                        Some(args) => args.clone(),
                        None => {
                            // fallback if we didn't store them
                            match self.parse_token_build_args() {
                                Ok(a) => a,
                                Err(err) => {
                                    self.token_creator_error_message = Some(err);
                                    self.show_token_creator_confirmation_popup = false;
                                    action = AppAction::None;
                                    return;
                                }
                            }
                        }
                    };

                    // Now create your tasks
                    let tasks = vec![
                        BackendTask::TokenTask(Box::new(TokenTask::RegisterTokenContract {
                            identity: self.selected_identity.clone().unwrap(),
                            signing_key: Box::new(self.selected_key.clone().unwrap()),

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
                        })),
                        BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)),
                    ];

                    action = AppAction::BackendTasks(tasks, BackendTasksExecutionMode::Sequential);
                    self.show_token_creator_confirmation_popup = false;
                    let now = Utc::now().timestamp() as u64;
                    self.token_creator_status = TokenCreatorStatus::WaitingForResult(now);
                }

                // Cancel
                if ui.button("Cancel").clicked() {
                    self.show_token_creator_confirmation_popup = false;
                    action = AppAction::None;
                }
            });

        if !is_open {
            self.show_token_creator_confirmation_popup = false;
        }

        action
    }

    /// Render the document schemas collapsible section
    fn render_document_schemas(&mut self, ui: &mut Ui) {
        let mut document_schemas_state =
            egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                ui.make_persistent_id("token_creator_document_schemas"),
                false,
            );

        // Force close if we need to reset
        if self.should_reset_collapsing_states {
            document_schemas_state.set_open(false);
        }

        document_schemas_state.store(ui.ctx());

        document_schemas_state
            .show_header(ui, |ui| {
                ui.label("Document Schemas");
            })
            .body(|ui| {
                ui.add_space(3.0);

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

                let schemas_response = ui.add_sized(
                    [ui.available_width(), 120.0],
                    TextEdit::multiline(&mut self.document_schemas_input),
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
