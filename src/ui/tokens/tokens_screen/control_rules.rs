use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::data_contract::change_control_rules::ChangeControlRules;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::change_control_rules::v0::ChangeControlRulesV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use eframe::egui::{self, Color32, Ui};
use egui::{ComboBox, RichText, TextEdit};

use super::GroupConfigUI;

#[derive(Clone, PartialEq, Eq, Default)]
pub struct ChangeControlRulesUI {
    pub rules: ChangeControlRulesV0,
    pub authorized_identity: Option<String>,
    pub admin_identity: Option<String>,
}

/// Holds the extra state for mint-specific UI controls rendered after the common
/// action taker / admin / boolean section.
pub struct MintExtras<'a> {
    pub new_tokens_destination_identity_should_default_to_contract_owner: &'a mut bool,
    pub new_tokens_destination_identity_enabled: &'a mut bool,
    pub minting_allow_choosing_destination: &'a mut bool,
    pub new_tokens_destination_identity_rules: &'a mut ChangeControlRulesUI,
    pub new_tokens_destination_identity: &'a mut String,
    pub minting_allow_choosing_destination_rules: &'a mut ChangeControlRulesUI,
    pub new_tokens_destination_expanded: &'a mut bool,
    pub minting_allow_choosing_expanded: &'a mut bool,
}

impl From<ChangeControlRulesV0> for ChangeControlRulesUI {
    fn from(rules: ChangeControlRulesV0) -> Self {
        ChangeControlRulesUI {
            rules,
            authorized_identity: None,
            admin_identity: None,
        }
    }
}

impl ChangeControlRulesUI {
    /// Renders an action taker combo box with optional identity text input.
    fn render_action_taker_combo(
        ui: &mut Ui,
        label: &str,
        id_salt: String,
        action_takers: &mut AuthorizedActionTakers,
        identity_input: &mut Option<String>,
        current_groups: &[GroupConfigUI],
    ) {
        ui.horizontal(|ui| {
            ui.label(label);
            ComboBox::from_id_salt(id_salt)
                .selected_text(match *action_takers {
                    AuthorizedActionTakers::NoOne => "No One".to_string(),
                    AuthorizedActionTakers::ContractOwner => "Contract Owner".to_string(),
                    AuthorizedActionTakers::Identity(id) => {
                        if id == Identifier::default() {
                            "Identity".to_string()
                        } else {
                            format!("Identity({})", id)
                        }
                    }
                    AuthorizedActionTakers::MainGroup => "Main Group".to_string(),
                    AuthorizedActionTakers::Group(position) => format!("Group {}", position),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(action_takers, AuthorizedActionTakers::NoOne, "No One");
                    ui.selectable_value(
                        action_takers,
                        AuthorizedActionTakers::ContractOwner,
                        "Contract Owner",
                    );
                    ui.selectable_value(
                        action_takers,
                        AuthorizedActionTakers::Identity(Identifier::default()),
                        "Identity",
                    );
                    if current_groups.is_empty() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("(No Groups Added Yet)").color(Color32::GRAY));
                        });
                    } else {
                        ui.selectable_value(
                            action_takers,
                            AuthorizedActionTakers::MainGroup,
                            "Main Group",
                        );
                    }
                    for (group_position, _group) in current_groups.iter().enumerate() {
                        ui.selectable_value(
                            action_takers,
                            AuthorizedActionTakers::Group(group_position as GroupContractPosition),
                            format!("Group {}", group_position),
                        );
                    }
                });

            if let AuthorizedActionTakers::Identity(_) = action_takers {
                identity_input.get_or_insert_with(String::new);
                if let Some(id_str) = identity_input {
                    ui.horizontal(|ui| {
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        ui.add_sized(
                            [300.0, 22.0],
                            TextEdit::singleline(id_str)
                                .hint_text("Enter base58 id")
                                .text_color(crate::ui::theme::DashColors::text_primary(dark_mode))
                                .background_color(crate::ui::theme::DashColors::input_background(
                                    dark_mode,
                                )),
                        );

                        if !id_str.is_empty() {
                            let is_valid =
                                Identifier::from_string(id_str.as_str(), Encoding::Base58).is_ok();

                            let (symbol, color) = if is_valid {
                                ("✔", Color32::DARK_GREEN)
                            } else {
                                ("×", Color32::DARK_RED)
                            };

                            ui.label(RichText::new(symbol).color(color).strong());
                        }
                    });
                }
            }
        });
        ui.end_row();
    }

    /// Renders the UI for a single action's configuration (mint, burn, freeze, etc.)
    ///
    /// When `mint_extras` is `Some`, additional mint-specific controls (destination
    /// identity, choosing destination, sub-rules) are rendered after the common
    /// checkboxes.
    pub fn render_control_change_rules_ui(
        &mut self,
        ui: &mut Ui,
        current_groups: &[GroupConfigUI],
        action_name: &str,
        special_case_option: Option<&mut bool>,
        is_expanded: &mut bool,
        mint_extras: Option<MintExtras<'_>>,
    ) {
        ui.horizontal(|ui| {
            let button_text = if *is_expanded { "−" } else { "+" };
            let button_response = ui.add(
                egui::Button::new(
                    RichText::new(button_text)
                        .size(20.0)
                        .color(crate::ui::theme::DashColors::DASH_BLUE),
                )
                .fill(Color32::TRANSPARENT)
                .stroke(egui::Stroke::NONE),
            );
            if button_response.clicked() {
                *is_expanded = !*is_expanded;
            }
            ui.label(action_name);
        });

        if *is_expanded {
            ui.indent(format!("{}_content", action_name), |ui| {
                egui::Grid::new(format!("{}_grid", action_name))
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        // Authorized action takers
                        Self::render_action_taker_combo(
                            ui,
                            "Authorized to perform action:",
                            format!("Authorized {} {}", action_name, current_groups.len()),
                            &mut self.rules.authorized_to_make_change,
                            &mut self.authorized_identity,
                            current_groups,
                        );

                        // Admin action takers
                        Self::render_action_taker_combo(
                            ui,
                            "Authorized to change rules:",
                            format!("Admin {} {}", action_name, current_groups.len()),
                            &mut self.rules.admin_action_takers,
                            &mut self.admin_identity,
                            current_groups,
                        );

                        // Booleans
                        ui.checkbox(
                            &mut self
                                .rules
                                .changing_authorized_action_takers_to_no_one_allowed,
                            "Changing authorized action takers to no one allowed",
                        );
                        ui.end_row();

                        ui.checkbox(
                            &mut self.rules.changing_admin_action_takers_to_no_one_allowed,
                            "Changing admin action takers to no one allowed",
                        );
                        ui.end_row();

                        ui.checkbox(
                            &mut self.rules.self_changing_admin_action_takers_allowed,
                            "Self-changing admin action takers allowed",
                        );
                        ui.end_row();

                        // Freeze special case
                        if let Some(special_case_option) = special_case_option
                            && action_name == "Freeze"
                            && self.rules.authorized_to_make_change
                                != AuthorizedActionTakers::NoOne
                        {
                            ui.horizontal(|ui| {
                                ui.checkbox(
                                    special_case_option,
                                    "Allow transfers to frozen identities",
                                );
                                ui.add_space(4.0);
                                ui.label(
                                    RichText::new("ℹ")
                                        .monospace()
                                        .color(Color32::LIGHT_BLUE),
                                )
                                .on_hover_text("Enabling this setting allows transfers to frozen identities, reducing gas usage by approximately 20% per transfer. Disable this if you want to make sure frozen identities can not receive transfers.");
                            });
                            ui.end_row();
                        }

                        // Mint-specific extras
                        if let Some(extras) = mint_extras
                            && self.rules.authorized_to_make_change
                                != AuthorizedActionTakers::NoOne
                        {
                            Self::render_mint_extras(ui, current_groups, extras);
                        }
                    });
            });
        }
    }

    /// Renders the mint-specific destination controls inside the grid.
    fn render_mint_extras(ui: &mut Ui, current_groups: &[GroupConfigUI], extras: MintExtras<'_>) {
        let mut default_to_owner_clicked = false;
        let mut default_to_identity_clicked = false;

        if ui
            .checkbox(
                extras.new_tokens_destination_identity_should_default_to_contract_owner,
                "Newly minted tokens should default to going to contract owner",
            )
            .clicked()
        {
            default_to_owner_clicked = true;
        }

        if ui
            .checkbox(
                extras.new_tokens_destination_identity_enabled,
                "Use a default identity to receive newly minted tokens",
            )
            .clicked()
        {
            default_to_identity_clicked = true;
        }

        // Apply exclusivity
        if default_to_owner_clicked {
            *extras.new_tokens_destination_identity_enabled = false;
        }

        if default_to_identity_clicked {
            *extras.new_tokens_destination_identity_should_default_to_contract_owner = false;
        }

        if *extras.new_tokens_destination_identity_enabled {
            ui.end_row();

            ui.label("Default Destination Identity (Base58):");
            ui.text_edit_singleline(extras.new_tokens_destination_identity);
            ui.end_row();

            extras
                .new_tokens_destination_identity_rules
                .render_control_change_rules_ui(
                    ui,
                    current_groups,
                    "New Tokens Destination Identity Rules",
                    None,
                    extras.new_tokens_destination_expanded,
                    None,
                );
        }

        ui.end_row();

        // Minting allow choosing destination
        ui.checkbox(
            extras.minting_allow_choosing_destination,
            "Allow user to pick a destination identity on each mint",
        );

        if *extras.minting_allow_choosing_destination {
            ui.end_row();
            extras
                .minting_allow_choosing_destination_rules
                .render_control_change_rules_ui(
                    ui,
                    current_groups,
                    "Minting Allow Choosing Destination Rules",
                    None,
                    extras.minting_allow_choosing_expanded,
                    None,
                );
        }
        ui.end_row();

        // Destination identity mode enforcement
        let none_selected = !*extras.new_tokens_destination_identity_enabled
            && !*extras.new_tokens_destination_identity_should_default_to_contract_owner
            && !*extras.minting_allow_choosing_destination;

        if none_selected {
            ui.colored_label(
                Color32::RED,
                "At least one minting destination mode must be enabled (default to contract owner, default to identity, or picked on each mint).",
            );
        }
    }

    pub fn extract_change_control_rules(
        &mut self,
        action_name: &str,
    ) -> Result<ChangeControlRules, String> {
        // 1) Update self.rules.authorized_to_make_change if it's Identity or Group
        if let AuthorizedActionTakers::Identity(_) = self.rules.authorized_to_make_change
            && let Some(ref id_str) = self.authorized_identity
        {
            let parsed = Identifier::from_string(id_str, Encoding::Base58).map_err(|_| {
                format!(
                    "Invalid base58 identifier for {} authorized identity",
                    action_name
                )
            })?;
            self.rules.authorized_to_make_change = AuthorizedActionTakers::Identity(parsed);
        }

        // 2) Update self.rules.admin_action_takers if it's Identity or Group
        if let AuthorizedActionTakers::Identity(_) = self.rules.admin_action_takers
            && let Some(ref id_str) = self.admin_identity
        {
            let parsed = Identifier::from_string(id_str, Encoding::Base58).map_err(|_| {
                format!(
                    "Invalid base58 identifier for {} admin identity",
                    action_name
                )
            })?;
            self.rules.admin_action_takers = AuthorizedActionTakers::Identity(parsed);
        }

        // 3) Construct the ChangeControlRules
        let rules = ChangeControlRules::V0(self.rules.clone());

        Ok(rules)
    }
}
