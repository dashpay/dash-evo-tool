mod contract_details;
mod data_contract_json_pop_up;
mod distributions;
mod groups;
mod keyword_search;
mod my_tokens;
mod structs;
mod token_creator;

pub use structs::*;

pub use groups::*;

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Duration, Utc};
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::{TokenConfigurationPresetFeatures, TokenConfigurationV0};
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::v0::TokenDistributionRulesV0;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
use dash_sdk::dpp::data_contract::associated_token::token_keeps_history_rules::TokenKeepsHistoryRules;
use dash_sdk::dpp::data_contract::associated_token::token_keeps_history_rules::v0::TokenKeepsHistoryRulesV0;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::DistributionFunction;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_recipient::TokenDistributionRecipient;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_type::RewardDistributionType;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::v0::TokenPerpetualDistributionV0;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::TokenPerpetualDistribution;
use dash_sdk::dpp::data_contract::associated_token::token_pre_programmed_distribution::v0::TokenPreProgrammedDistributionV0;
use dash_sdk::dpp::data_contract::associated_token::token_pre_programmed_distribution::TokenPreProgrammedDistribution;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::data_contract::change_control_rules::v0::ChangeControlRulesV0;
use dash_sdk::dpp::data_contract::change_control_rules::ChangeControlRules;
use dash_sdk::dpp::data_contract::group::Group;
use dash_sdk::dpp::data_contract::GroupContractPosition;
use dash_sdk::dpp::fee::Credits;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::dpp::prelude::TimestampMillisInterval;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use dash_sdk::query_types::IndexMap;
use eframe::egui::{self, Color32, Context, Ui};
use egui::{Checkbox, ColorImage, ComboBox, Response, RichText, TextEdit, TextureHandle};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use enum_iterator::Sequence;
use image::ImageReader;
use crate::app::BackendTasksExecutionMode;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, NO_IDENTITIES_FOUND};

use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, ScreenLike, ScreenType};

const EXP_FORMULA_PNG: &[u8] = include_bytes!("../../../../assets/exp_function.png");
const INV_LOG_FORMULA_PNG: &[u8] = include_bytes!("../../../../assets/inv_log_function.png");
const LOG_FORMULA_PNG: &[u8] = include_bytes!("../../../../assets/log_function.png");
const LINEAR_FORMULA_PNG: &[u8] = include_bytes!("../../../../assets/linear_function.png");
const POLYNOMIAL_FORMULA_PNG: &[u8] = include_bytes!("../../../../assets/polynomial_function.png");

pub fn load_formula_image(bytes: &[u8]) -> ColorImage {
    let image = ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .expect("Failed to guess image format")
        .decode()
        .expect("Failed to decode image")
        .to_rgba8();

    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.as_flat_samples();
    let color_image = ColorImage::from_rgba_unmultiplied(size, pixels.as_slice());
    color_image
}

pub fn validate_perpetual_distribution_recipient(
    contract_owner_id: Identifier,
    recipient: TokenDistributionRecipient,
    identity: &QualifiedIdentity,
) -> Result<(), String> {
    match recipient {
        TokenDistributionRecipient::ContractOwner => {
            if contract_owner_id != identity.identity.id() {
                Err("This token's distribution recipient is the contract owner, and this identity is not the contract owner".to_string())
            } else {
                Ok(())
            }
        }
        TokenDistributionRecipient::Identity(identifier) => {
            if identifier != identity.identity.id() {
                Err(
                    "This identity is not a valid distribution recipient for this token"
                        .to_string(),
                )
            } else {
                Ok(())
            }
        }
        TokenDistributionRecipient::EvonodesByParticipation => {
            if identity.identity_type != IdentityType::Evonode {
                Err("This token's distribution recipient is EvonodesByParticipation, and this identity is not an evonode".to_string())
            } else {
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContractDescriptionInfo {
    pub data_contract_id: Identifier,
    pub description: String,
}

/* helper: tiny checkbox with no extra spacing */
fn sub_checkbox(ui: &mut Ui, flag: &mut bool, label: &str) {
    ui.horizontal(|ui| {
        ui.checkbox(flag, label);
    });
}

/// helper: draw a tri-state parent checkbox backed by `Option<bool>`
fn tri_state(ui: &mut Ui, state: &mut Option<bool>, label: &str) -> Response {
    // temporary bool just for the click interaction
    let mut tmp = state.unwrap_or(false);

    let resp = ui.add(Checkbox::new(&mut tmp, label).indeterminate(state.is_none()));

    if resp.clicked() {
        *state = match *state {
            Some(false) => Some(true),
            Some(true) => Some(false),
            None => Some(true),
        };
    }
    resp
}

fn sanitize_i64(input: &mut String) {
    input.retain(|c| c.is_ascii_digit() || c == '-' || c == '+');
}

fn sanitize_u64(input: &mut String) {
    input.retain(|c| c.is_ascii_digit());
}

/// Which token sub-screen is currently showing.
#[derive(PartialEq)]
pub enum TokensSubscreen {
    MyTokens,
    SearchTokens,
    TokenCreator,
}

impl TokensSubscreen {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::MyTokens => "My Tokens",
            Self::SearchTokens => "Search Tokens",
            Self::TokenCreator => "Token Creator",
        }
    }
}

#[derive(PartialEq)]
pub enum RefreshingStatus {
    Refreshing(u64),
    NotRefreshing,
}

/// Represents the status of the user’s search
#[derive(PartialEq, Eq, Clone)]
pub enum ContractSearchStatus {
    NotStarted,
    WaitingForResult(u64),
    Complete,
    ErrorMessage(String),
}

#[derive(Debug, PartialEq)]
pub enum TokenCreatorStatus {
    NotStarted,
    WaitingForResult(u64),
    Complete,
    ErrorMessage(String),
}

impl Default for TokenCreatorStatus {
    fn default() -> Self {
        Self::NotStarted
    }
}

/// Sorting columns
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    OwnerIdentity,
    OwnerIdentityAlias,
    Balance,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortOrder {
    Ascending,
    Descending,
}

#[derive(Clone, PartialEq, Eq, Default)]
pub struct ChangeControlRulesUI {
    pub rules: ChangeControlRulesV0,
    pub authorized_identity: Option<String>,
    pub admin_identity: Option<String>,
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
    /// Renders the UI for a single action’s configuration (mint, burn, freeze, etc.)
    pub fn render_control_change_rules_ui(
        &mut self,
        ui: &mut Ui,
        current_groups: &[GroupConfigUI],
        action_name: &str,
        special_case_option: Option<&mut bool>,
    ) {
        ui.collapsing(action_name, |ui| {
            ui.add_space(3.0);

            egui::Grid::new("basic_token_info_grid")
                .num_columns(2)
                .spacing([16.0, 8.0]) // Horizontal, vertical spacing
                .show(ui, |ui| {
                    // Authorized action takers
                    ui.horizontal(|ui| {
                        ui.label("Authorized to perform action:");
                        ComboBox::from_id_salt(format!("Authorized {} {}", action_name, current_groups.len()))
                            .selected_text(match self.rules.authorized_to_make_change {
                                AuthorizedActionTakers::NoOne => "No One".to_string(),
                                AuthorizedActionTakers::ContractOwner => "Contract Owner".to_string(),
                                AuthorizedActionTakers::Identity(id) => {
                                    if id == Identifier::default() {
                                        "Identity".to_string()
                                    } else {
                                        format!("Identity({})", id)
                                    }
                                },
                                AuthorizedActionTakers::MainGroup => "Main Group".to_string(),
                                AuthorizedActionTakers::Group(position) => format!("Group {}", position),
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.rules.authorized_to_make_change,
                                    AuthorizedActionTakers::NoOne,
                                    "No One",
                                );
                                ui.selectable_value(
                                    &mut self.rules.authorized_to_make_change,
                                    AuthorizedActionTakers::ContractOwner,
                                    "Contract Owner",
                                );
                                ui.selectable_value(
                                    &mut self.rules.authorized_to_make_change,
                                    AuthorizedActionTakers::Identity(Identifier::default()),
                                    "Identity",
                                );
                                if current_groups.is_empty() {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("(No Groups Added Yet)").color(Color32::GRAY));
                                    });
                                } else {
                                    ui.selectable_value(
                                        &mut self.rules.authorized_to_make_change,
                                        AuthorizedActionTakers::MainGroup,
                                        "Main Group",
                                    );
                                }
                                for (group_position, _group) in current_groups.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut self.rules.authorized_to_make_change,
                                        AuthorizedActionTakers::Group(group_position as GroupContractPosition),
                                        format!("Group {}", group_position),
                                    );
                                }
                            });

                        // If user selected Identity or Group, show text edit
                        if let AuthorizedActionTakers::Identity(_) = &mut self.rules.authorized_to_make_change {
                            self.authorized_identity.get_or_insert_with(String::new);
                            if let Some(ref mut id_str) = self.authorized_identity {
                                ui.horizontal(|ui| {
                                    ui.add_sized(
                                        [300.0, 22.0],
                                        TextEdit::singleline(id_str).hint_text("Enter base58 id"),
                                    );

                                    if !id_str.is_empty() {
                                        let is_valid = Identifier::from_string(id_str.as_str(), Encoding::Base58).is_ok();

                                        let (symbol, color) = if is_valid {
                                            ("✔", Color32::GREEN)
                                        } else {
                                            ("×", Color32::RED)
                                        };

                                        ui.label(RichText::new(symbol).color(color).strong());
                                    }
                                });
                            }
                        }
                    });
                    ui.end_row();

                    // Admin action takers
                    ui.horizontal(|ui| {
                        ui.label("Authorized to change rules:");
                        ComboBox::from_id_salt(format!("Admin {} {}", action_name, current_groups.len()))
                            .selected_text(match self.rules.admin_action_takers {
                                AuthorizedActionTakers::NoOne => "No One".to_string(),
                                AuthorizedActionTakers::ContractOwner => "Contract Owner".to_string(),
                                AuthorizedActionTakers::Identity(id) => {
                                    if id == Identifier::default() {
                                        "Identity".to_string()
                                    } else {
                                        format!("Identity({})", id)
                                    }
                                },
                                AuthorizedActionTakers::MainGroup => "Main Group".to_string(),
                                AuthorizedActionTakers::Group(position) => format!("Group {}", position),
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.rules.admin_action_takers,
                                    AuthorizedActionTakers::NoOne,
                                    "No One",
                                );
                                ui.selectable_value(
                                    &mut self.rules.admin_action_takers,
                                    AuthorizedActionTakers::ContractOwner,
                                    "Contract Owner",
                                );
                                ui.selectable_value(
                                    &mut self.rules.admin_action_takers,
                                    AuthorizedActionTakers::Identity(Identifier::default()),
                                    "Identity",
                                );
                                if current_groups.is_empty() {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("(No Groups Added Yet)").color(Color32::GRAY));
                                    });
                                } else {
                                    ui.selectable_value(
                                        &mut self.rules.admin_action_takers,
                                        AuthorizedActionTakers::MainGroup,
                                        "Main Group",
                                    );
                                }
                                for (group_position, _group) in current_groups.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut self.rules.admin_action_takers,
                                        AuthorizedActionTakers::Group(group_position as GroupContractPosition),
                                        format!("Group {}", group_position),
                                    );
                                }
                            });

                        if let AuthorizedActionTakers::Identity(_) = &mut self.rules.admin_action_takers {
                            self.admin_identity.get_or_insert_with(String::new);
                            if let Some(ref mut id_str) = self.admin_identity {
                                ui.horizontal(|ui| {
                                    ui.add_sized(
                                        [300.0, 22.0],
                                        TextEdit::singleline(id_str).hint_text("Enter base58 id"),
                                    );

                                    if !id_str.is_empty() {
                                        let is_valid = Identifier::from_string(id_str.as_str(), Encoding::Base58).is_ok();

                                        let (symbol, color) = if is_valid {
                                            ("✔", Color32::GREEN)
                                        } else {
                                            ("×", Color32::RED)
                                        };

                                        ui.label(RichText::new(symbol).color(color).strong());
                                    }
                                });
                            }
                        }
                    });
                    ui.end_row();

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

                    if let Some(special_case_option) = special_case_option {
                        if action_name == "Freeze" && self.rules.authorized_to_make_change != AuthorizedActionTakers::NoOne {
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
                    }
                });

            ui.add_space(3.0);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_mint_control_change_rules_ui(
        &mut self,
        ui: &mut Ui,
        current_groups: &[GroupConfigUI],
        new_tokens_destination_identity_should_default_to_contract_owner: &mut bool,
        new_tokens_destination_identity_enabled: &mut bool,
        minting_allow_choosing_destination: &mut bool,
        new_tokens_destination_identity_rules: &mut ChangeControlRulesUI,
        new_tokens_destination_identity: &mut String,
        minting_allow_choosing_destination_rules: &mut ChangeControlRulesUI,
    ) {
        ui.collapsing("Manual Mint", |ui| {
            ui.add_space(3.0);

            egui::Grid::new("basic_token_info_grid")
                .num_columns(2)
                .spacing([16.0, 8.0]) // Horizontal, vertical spacing
                .show(ui, |ui| {
                    // Authorized action takers
                    ui.horizontal(|ui| {
                        ui.label("Authorized to perform action:");
                        ComboBox::from_id_salt(format!("Authorized Manual Mint {}", current_groups.len()))
                            .selected_text(match self.rules.authorized_to_make_change {
                                AuthorizedActionTakers::NoOne => "No One".to_string(),
                                AuthorizedActionTakers::ContractOwner => "Contract Owner".to_string(),
                                AuthorizedActionTakers::Identity(id) => {
                                    if id == Identifier::default() {
                                        "Identity".to_string()
                                    } else {
                                        format!("Identity({})", id)
                                    }
                                },
                                AuthorizedActionTakers::MainGroup => "Main Group".to_string(),
                                AuthorizedActionTakers::Group(position) => format!("Group {}", position),
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.rules.authorized_to_make_change,
                                    AuthorizedActionTakers::NoOne,
                                    "No One",
                                );
                                ui.selectable_value(
                                    &mut self.rules.authorized_to_make_change,
                                    AuthorizedActionTakers::ContractOwner,
                                    "Contract Owner",
                                );
                                ui.selectable_value(
                                    &mut self.rules.authorized_to_make_change,
                                    AuthorizedActionTakers::Identity(Identifier::default()),
                                    "Identity",
                                );
                                if current_groups.is_empty() {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("(No Groups Added Yet)").color(Color32::GRAY));
                                    });
                                } else {
                                    ui.selectable_value(
                                        &mut self.rules.authorized_to_make_change,
                                        AuthorizedActionTakers::MainGroup,
                                        "Main Group",
                                    );
                                }
                                for (group_position, _group) in current_groups.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut self.rules.authorized_to_make_change,
                                        AuthorizedActionTakers::Group(group_position as GroupContractPosition),
                                        format!("Group {}", group_position),
                                    );
                                }
                            });

                        // If user selected Identity or Group, show text edit
                        if let AuthorizedActionTakers::Identity(_) = &mut self.rules.authorized_to_make_change {
                            self.authorized_identity.get_or_insert_with(String::new);
                            if let Some(ref mut id_str) = self.authorized_identity {
                                ui.horizontal(|ui| {
                                    ui.add_sized(
                                        [300.0, 22.0],
                                        TextEdit::singleline(id_str).hint_text("Enter base58 id"),
                                    );

                                    if !id_str.is_empty() {
                                        let is_valid = Identifier::from_string(id_str.as_str(), Encoding::Base58).is_ok();

                                        let (symbol, color) = if is_valid {
                                            ("✔", Color32::GREEN)
                                        } else {
                                            ("×", Color32::RED)
                                        };

                                        ui.label(RichText::new(symbol).color(color).strong());
                                    }
                                });
                            }
                        }
                    });
                    ui.end_row();

                    // Admin action takers
                    ui.horizontal(|ui| {
                        ui.label("Authorized to change rules:");
                        ComboBox::from_id_salt(format!("Admin Manual Mint {}", current_groups.len()))
                            .selected_text(match self.rules.admin_action_takers {
                                AuthorizedActionTakers::NoOne => "No One".to_string(),
                                AuthorizedActionTakers::ContractOwner => "Contract Owner".to_string(),
                                AuthorizedActionTakers::Identity(id) => {
                                    if id == Identifier::default() {
                                        "Identity".to_string()
                                    } else {
                                        format!("Identity({})", id)
                                    }
                                },
                                AuthorizedActionTakers::MainGroup => "Main Group".to_string(),
                                AuthorizedActionTakers::Group(position) => format!("Group {}", position),
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.rules.admin_action_takers,
                                    AuthorizedActionTakers::NoOne,
                                    "No One",
                                );
                                ui.selectable_value(
                                    &mut self.rules.admin_action_takers,
                                    AuthorizedActionTakers::ContractOwner,
                                    "Contract Owner",
                                );
                                ui.selectable_value(
                                    &mut self.rules.admin_action_takers,
                                    AuthorizedActionTakers::Identity(Identifier::default()),
                                    "Identity",
                                );
                                if current_groups.is_empty() {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("(No Groups Added Yet)").color(Color32::GRAY));
                                    });
                                } else {
                                    ui.selectable_value(
                                        &mut self.rules.admin_action_takers,
                                        AuthorizedActionTakers::MainGroup,
                                        "Main Group",
                                    );
                                }
                                for (group_position, _group) in current_groups.iter().enumerate() {
                                    ui.selectable_value(
                                        &mut self.rules.admin_action_takers,
                                        AuthorizedActionTakers::Group(group_position as GroupContractPosition),
                                        format!("Group {}", group_position),
                                    );
                                }
                            });

                        if let AuthorizedActionTakers::Identity(_) = &mut self.rules.admin_action_takers {
                            self.admin_identity.get_or_insert_with(String::new);
                            if let Some(ref mut id_str) = self.admin_identity {
                                ui.horizontal(|ui| {
                                    ui.add_sized(
                                        [300.0, 22.0],
                                        TextEdit::singleline(id_str).hint_text("Enter base58 id"),
                                    );

                                    if !id_str.is_empty() {
                                        let is_valid = Identifier::from_string(id_str.as_str(), Encoding::Base58).is_ok();

                                        let (symbol, color) = if is_valid {
                                            ("✔", Color32::GREEN)
                                        } else {
                                            ("×", Color32::RED)
                                        };

                                        ui.label(RichText::new(symbol).color(color).strong());
                                    }
                                });
                            }
                        }
                    });
                    ui.end_row();

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

                    if self.rules.authorized_to_make_change != AuthorizedActionTakers::NoOne {

                        let mut default_to_owner_clicked = false;
                        let mut default_to_identity_clicked = false;

                        if ui
                            .checkbox(
                                new_tokens_destination_identity_should_default_to_contract_owner,
                                "Newly minted tokens should default to going to contract owner",
                            )
                            .clicked()
                        {
                            default_to_owner_clicked = true;
                        }

                        if ui
                            .checkbox(
                                new_tokens_destination_identity_enabled,
                                "Use a default identity to receive newly minted tokens",
                            )
                            .clicked()
                        {
                            default_to_identity_clicked = true;
                        }

                        // Apply exclusivity
                        if default_to_owner_clicked {
                            *new_tokens_destination_identity_enabled = false;
                        }

                        if default_to_identity_clicked {
                            *new_tokens_destination_identity_should_default_to_contract_owner = false;
                        }

                        if *new_tokens_destination_identity_enabled {
                            ui.end_row();

                            ui.label("Default Destination Identity (Base58):");
                            ui.text_edit_singleline(new_tokens_destination_identity);
                            ui.end_row();

                            new_tokens_destination_identity_rules.render_control_change_rules_ui(ui, current_groups,"New Tokens Destination Identity Rules", None);
                        }

                        ui.end_row();

                        // MINTING ALLOW CHOOSING DESTINATION
                        ui.checkbox(
                            minting_allow_choosing_destination,
                            "Allow user to pick a destination identity on each mint",
                        );


                        if *minting_allow_choosing_destination {
                            ui.end_row();
                            minting_allow_choosing_destination_rules.render_control_change_rules_ui(ui, current_groups, "Minting Allow Choosing Destination Rules", None);
                        }
                        ui.end_row();

                        // Destination Identity Mode Enforcement
                        let none_selected = !*new_tokens_destination_identity_enabled
                            && !*new_tokens_destination_identity_should_default_to_contract_owner
                            && !*minting_allow_choosing_destination;

                        if none_selected {
                            ui.colored_label(
                                Color32::RED,
                                "At least one minting destination mode must be enabled (default to contract owner, default to identity, or picked on each mint).",
                            );
                        }
                    }
                });

            ui.add_space(3.0);
        });
    }

    pub fn extract_change_control_rules(
        &mut self,
        action_name: &str,
    ) -> Result<ChangeControlRules, String> {
        // 1) Update self.rules.authorized_to_make_change if it’s Identity or Group
        if let AuthorizedActionTakers::Identity(_) = self.rules.authorized_to_make_change {
            if let Some(ref id_str) = self.authorized_identity {
                let parsed = Identifier::from_string(id_str, Encoding::Base58).map_err(|_| {
                    format!(
                        "Invalid base58 identifier for {} authorized identity",
                        action_name
                    )
                })?;
                self.rules.authorized_to_make_change = AuthorizedActionTakers::Identity(parsed);
            }
        }

        // 2) Update self.rules.admin_action_takers if it’s Identity or Group
        if let AuthorizedActionTakers::Identity(_) = self.rules.admin_action_takers {
            if let Some(ref id_str) = self.admin_identity {
                let parsed = Identifier::from_string(id_str, Encoding::Base58).map_err(|_| {
                    format!(
                        "Invalid base58 identifier for {} admin identity",
                        action_name
                    )
                })?;
                self.rules.admin_action_takers = AuthorizedActionTakers::Identity(parsed);
            }
        }

        // 3) Construct the ChangeControlRules
        let rules = ChangeControlRules::V0(self.rules.clone());

        Ok(rules)
    }
}

/// A lightweight enum for the user’s choice of distribution type
#[derive(Debug, Clone, PartialEq)]
pub enum PerpetualDistributionIntervalTypeUI {
    None,
    BlockBased,
    TimeBased,
    EpochBased,
}

/// A lightweight enum for the user’s choice of distribution function
#[derive(Debug, Clone, PartialEq, Ord, PartialOrd, Eq, Hash)]
pub enum DistributionFunctionUI {
    FixedAmount,
    StepDecreasingAmount,
    Stepwise,
    Linear,
    Polynomial,
    Exponential,
    Logarithmic,
    InvertedLogarithmic,
}

impl DistributionFunctionUI {
    pub(crate) fn name(&self) -> &str {
        match self {
            DistributionFunctionUI::FixedAmount => "fixed_amount",
            DistributionFunctionUI::StepDecreasingAmount => "step_decreasing_amount",
            DistributionFunctionUI::Stepwise => "stepwise",
            DistributionFunctionUI::Linear => "linear",
            DistributionFunctionUI::Polynomial => "polynomial",
            DistributionFunctionUI::Exponential => "exponential",
            DistributionFunctionUI::Logarithmic => "logarithmic",
            DistributionFunctionUI::InvertedLogarithmic => "inverted_logarithmic",
        }
    }
}

/// A lightweight enum for the user’s recipient selection
#[derive(Debug, Clone, PartialEq)]
pub enum TokenDistributionRecipientUI {
    ContractOwner,
    Identity,
    EvonodesByParticipation,
}

#[derive(Default, Clone)]
pub struct DistributionEntry {
    /// Time from token contract registration when the distribution should occur
    pub days: i32,
    pub hours: i32,
    pub minutes: i32,

    /// The base58 identity to receive distribution
    pub identity_str: String,

    /// The distribution amount
    pub amount_str: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Sequence)]
pub enum TokenNameLanguage {
    Arabic,
    Bengali,
    Burmese,
    Chinese,
    Czech,
    Dutch,
    English,
    Farsi,
    Filipino,
    French,
    German,
    Greek,
    Gujarati,
    Hausa,
    Hebrew,
    Hindi,
    Hungarian,
    Igbo,
    Indonesian,
    Italian,
    Japanese,
    Javanese,
    Kannada,
    Khmer,
    Korean,
    Malay,
    Malayalam,
    Mandarin,
    Marathi,
    Nepali,
    Oriya,
    Pashto,
    Polish,
    Portuguese,
    Punjabi,
    Romanian,
    Russian,
    Serbian,
    Sindhi,
    Sinhala,
    Somali,
    Spanish,
    Swahili,
    Swedish,
    Tamil,
    Telugu,
    Thai,
    Turkish,
    Ukrainian,
    Urdu,
    Vietnamese,
    Yoruba,
}

impl std::fmt::Display for TokenNameLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone, Debug)]
/// All arguments needed by `build_data_contract_v1_with_one_token`.
pub struct TokenBuildArgs {
    pub identity_id: Identifier,

    pub token_names: Vec<(String, String, String)>,
    pub contract_keywords: Vec<String>,
    pub token_description: Option<String>,
    pub should_capitalize: bool,
    pub decimals: u8,
    pub base_supply: u64,
    pub max_supply: Option<u64>,
    pub start_paused: bool,
    pub allow_transfers_to_frozen_identities: bool,
    pub keeps_history: TokenKeepsHistoryRules,
    pub main_control_group: Option<u16>,

    pub manual_minting_rules: ChangeControlRules,
    pub manual_burning_rules: ChangeControlRules,
    pub freeze_rules: ChangeControlRules,
    pub unfreeze_rules: ChangeControlRules,
    pub destroy_frozen_funds_rules: ChangeControlRules,
    pub emergency_action_rules: ChangeControlRules,
    pub max_supply_change_rules: ChangeControlRules,
    pub conventions_change_rules: ChangeControlRules,
    pub main_control_group_change_authorized: AuthorizedActionTakers,

    pub distribution_rules: TokenDistributionRules,
    pub groups: BTreeMap<u16, Group>,
}

pub type TokenSearchable = bool;

/// The main, combined TokensScreen:
/// - Displays token balances or a search UI
/// - Allows reordering of tokens if desired
pub struct TokensScreen {
    pub app_context: Arc<AppContext>,
    pub tokens_subscreen: TokensSubscreen,
    all_known_tokens: IndexMap<Identifier, TokenInfoWithDataContract>,
    identities: IndexMap<Identifier, QualifiedIdentity>,
    my_tokens: IndexMap<IdentityTokenIdentifier, IdentityTokenBalanceWithActions>,
    pub selected_token: Option<Identifier>,
    token_pricing_data: IndexMap<
        Identifier,
        Option<dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule>,
    >,
    pricing_loading_state: IndexMap<Identifier, bool>,
    backend_message: Option<(String, MessageType, DateTime<Utc>)>,
    pending_backend_task: Option<BackendTask>,
    refreshing_status: RefreshingStatus,
    should_reset_collapsing_states: bool,

    // Contract Search
    pub selected_contract_id: Option<Identifier>,
    selected_contract_description: Option<ContractDescriptionInfo>,
    selected_token_infos: Vec<TokenInfo>,
    search_results: Arc<Mutex<Vec<ContractDescriptionInfo>>>,
    contract_search_status: ContractSearchStatus,
    contract_details_loading: bool,

    // Token Search
    token_search_query: Option<String>,
    search_current_page: usize,
    search_has_next_page: bool,
    next_cursors: Vec<Start>,
    previous_cursors: Vec<Start>,

    // Sorting
    sort_column: SortColumn,
    sort_order: SortOrder,
    use_custom_order: bool,

    // Remove token
    confirm_remove_identity_token_balance_popup: bool,
    identity_token_balance_to_remove: Option<IdentityTokenBasicInfo>,
    confirm_remove_token_popup: bool,
    token_to_remove: Option<Identifier>,

    // ====================================
    //           Token Creator
    // ====================================
    selected_token_preset: Option<TokenConfigurationPresetFeatures>,
    show_pop_up_info: Option<String>,
    selected_identity: Option<QualifiedIdentity>,
    selected_key: Option<IdentityPublicKey>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    token_names_input: Vec<(String, String, TokenNameLanguage, TokenSearchable)>,
    contract_keywords_input: String,
    token_description_input: String,
    should_capitalize_input: bool,
    decimals_input: String,
    base_supply_input: String,
    max_supply_input: String,
    start_as_paused_input: bool,
    main_control_group_input: String,
    show_token_creator_confirmation_popup: bool,
    token_creator_status: TokenCreatorStatus,
    token_creator_error_message: Option<String>,
    show_advanced_keeps_history: bool,
    token_advanced_keeps_history: TokenKeepsHistoryRulesV0,
    groups_ui: Vec<GroupConfigUI>,
    cached_build_args: Option<TokenBuildArgs>,
    show_json_popup: bool,
    json_popup_text: String,
    allow_transfers_to_frozen_identities: bool,

    // Action Rules
    manual_minting_rules: ChangeControlRulesUI,
    manual_burning_rules: ChangeControlRulesUI,
    freeze_rules: ChangeControlRulesUI,
    unfreeze_rules: ChangeControlRulesUI,
    destroy_frozen_funds_rules: ChangeControlRulesUI,
    emergency_action_rules: ChangeControlRulesUI,
    max_supply_change_rules: ChangeControlRulesUI,
    conventions_change_rules: ChangeControlRulesUI,
    authorized_main_control_group_change: AuthorizedActionTakers,
    main_control_group_change_authorized_identity: Option<String>,
    main_control_group_change_authorized_group: Option<String>,

    // Perpetual Distribution
    pub enable_perpetual_distribution: bool,
    pub perpetual_distribution_rules: ChangeControlRulesUI,
    pub perpetual_dist_type: PerpetualDistributionIntervalTypeUI,
    pub perpetual_dist_interval_input: String,
    pub perpetual_dist_interval_unit: IntervalTimeUnit,
    pub perpetual_dist_function: DistributionFunctionUI,
    pub perpetual_dist_recipient: TokenDistributionRecipientUI,
    pub perpetual_dist_recipient_identity_input: Option<String>,

    // Pre-programmed distribution
    pub enable_pre_programmed_distribution: bool,
    pub pre_programmed_distributions: Vec<DistributionEntry>,

    // New Tokens Destination Identity
    pub new_tokens_destination_identity_should_default_to_contract_owner: bool,
    pub new_tokens_destination_other_identity_enabled: bool,
    pub new_tokens_destination_other_identity: String,
    pub new_tokens_destination_identity_rules: ChangeControlRulesUI,

    // Minting Allow Choosing Destination
    pub minting_allow_choosing_destination: bool,
    pub minting_allow_choosing_destination_rules: ChangeControlRulesUI,

    // --- FixedAmount ---
    pub fixed_amount_input: String,

    // --- Random ---
    pub random_min_input: String,
    pub random_max_input: String,

    // --- StepDecreasingAmount ---
    pub step_count_input: String,
    pub decrease_per_interval_numerator_input: String,
    pub decrease_per_interval_denominator_input: String,
    pub step_decreasing_start_period_offset_input: String,
    pub step_decreasing_initial_emission_input: String,
    pub step_decreasing_min_value_input: String,
    pub step_decreasing_max_interval_count_input: String,
    pub step_decreasing_trailing_distribution_interval_amount_input: String,

    // --- Stepwise ---
    pub stepwise_steps: Vec<(String, String)>,

    // --- Linear ---
    pub linear_int_a_input: String,
    pub linear_int_d_input: String,
    pub linear_int_start_step_input: String,
    pub linear_int_starting_amount_input: String,
    pub linear_int_min_value_input: String,
    pub linear_int_max_value_input: String,

    // --- Polynomial ---
    pub poly_int_a_input: String,
    pub poly_int_m_input: String,
    pub poly_int_n_input: String,
    pub poly_int_d_input: String,
    pub poly_int_s_input: String,
    pub poly_int_o_input: String,
    pub poly_int_b_input: String,
    pub poly_int_min_value_input: String,
    pub poly_int_max_value_input: String,

    // --- Exponential ---
    pub exp_a_input: String,
    pub exp_m_input: String,
    pub exp_n_input: String,
    pub exp_d_input: String,
    pub exp_s_input: String,
    pub exp_o_input: String,
    pub exp_b_input: String,
    pub exp_min_value_input: String,
    pub exp_max_value_input: String,

    // --- Logarithmic ---
    pub log_a_input: String,
    pub log_d_input: String,
    pub log_m_input: String,
    pub log_n_input: String,
    pub log_s_input: String,
    pub log_o_input: String,
    pub log_b_input: String,
    pub log_min_value_input: String,
    pub log_max_value_input: String,

    // --- Inverted Logarithmic ---
    pub inv_log_a_input: String,
    pub inv_log_d_input: String,
    pub inv_log_m_input: String,
    pub inv_log_n_input: String,
    pub inv_log_s_input: String,
    pub inv_log_o_input: String,
    pub inv_log_b_input: String,
    pub inv_log_min_value_input: String,
    pub inv_log_max_value_input: String,

    pub function_images: BTreeMap<DistributionFunctionUI, ColorImage>,
    pub function_textures: BTreeMap<DistributionFunctionUI, TextureHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntervalTimeUnit {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Year,
}

impl IntervalTimeUnit {
    /// Returns the equivalent duration in milliseconds
    /// for the given amount of the unit.
    pub fn ms_for_amount(&self, amount: u64) -> TimestampMillisInterval {
        match self {
            IntervalTimeUnit::Second => amount.saturating_mul(1_000),
            IntervalTimeUnit::Minute => amount.saturating_mul(60_000),
            IntervalTimeUnit::Hour => amount.saturating_mul(3_600_000),
            IntervalTimeUnit::Day => amount.saturating_mul(86_400_000),
            IntervalTimeUnit::Week => amount.saturating_mul(7).saturating_mul(86_400_000),
            IntervalTimeUnit::Year => amount.saturating_mul(31_556_952_000),
        }
    }
    pub fn label_for_amount(&self, amount_str: &str) -> &'static str {
        let is_singular = amount_str == "1";
        match (self, is_singular) {
            (IntervalTimeUnit::Second, true) => "second",
            (IntervalTimeUnit::Second, false) => "seconds",
            (IntervalTimeUnit::Minute, true) => "minute",
            (IntervalTimeUnit::Minute, false) => "minutes",
            (IntervalTimeUnit::Hour, true) => "hour",
            (IntervalTimeUnit::Hour, false) => "hours",
            (IntervalTimeUnit::Day, true) => "day",
            (IntervalTimeUnit::Day, false) => "days",
            (IntervalTimeUnit::Week, true) => "week",
            (IntervalTimeUnit::Week, false) => "weeks",
            (IntervalTimeUnit::Year, true) => "year",
            (IntervalTimeUnit::Year, false) => "years",
        }
    }

    pub fn label_for_num_amount(&self, amount: u64) -> &'static str {
        let is_singular = amount == 1;
        match (self, is_singular) {
            (IntervalTimeUnit::Second, true) => "second",
            (IntervalTimeUnit::Second, false) => "seconds",
            (IntervalTimeUnit::Minute, true) => "minute",
            (IntervalTimeUnit::Minute, false) => "minutes",
            (IntervalTimeUnit::Hour, true) => "hour",
            (IntervalTimeUnit::Hour, false) => "hours",
            (IntervalTimeUnit::Day, true) => "day",
            (IntervalTimeUnit::Day, false) => "days",
            (IntervalTimeUnit::Week, true) => "week",
            (IntervalTimeUnit::Week, false) => "weeks",
            (IntervalTimeUnit::Year, true) => "year",
            (IntervalTimeUnit::Year, false) => "years",
        }
    }

    pub fn capitalized_label_for_num_amount(&self, amount: u64) -> &'static str {
        let is_singular = amount == 1;
        match (self, is_singular) {
            (IntervalTimeUnit::Second, true) => "Second",
            (IntervalTimeUnit::Second, false) => "Seconds",
            (IntervalTimeUnit::Minute, true) => "Minute",
            (IntervalTimeUnit::Minute, false) => "Minutes",
            (IntervalTimeUnit::Hour, true) => "Hour",
            (IntervalTimeUnit::Hour, false) => "Hours",
            (IntervalTimeUnit::Day, true) => "Day",
            (IntervalTimeUnit::Day, false) => "Days",
            (IntervalTimeUnit::Week, true) => "Week",
            (IntervalTimeUnit::Week, false) => "Weeks",
            (IntervalTimeUnit::Year, true) => "Year",
            (IntervalTimeUnit::Year, false) => "Years",
        }
    }
}

fn my_tokens(
    app_context: &Arc<AppContext>,
    identities: &IndexMap<Identifier, QualifiedIdentity>,
    all_known_tokens: &IndexMap<Identifier, TokenInfoWithDataContract>,
    token_pricing_data: &IndexMap<
        Identifier,
        Option<dash_sdk::dpp::tokens::token_pricing_schedule::TokenPricingSchedule>,
    >,
) -> IndexMap<IdentityTokenIdentifier, IdentityTokenBalanceWithActions> {
    let in_dev_mode = app_context.developer_mode.load(Ordering::Relaxed);

    app_context
        .identity_token_balances()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id_token_identifier, token_balance)| {
            // Lookup identity
            let identity = identities.get(&token_balance.identity_id)?;
            // Lookup contract
            let contract = all_known_tokens
                .get(&token_balance.token_id)
                .map(|info| &info.data_contract)?;

            let token_pricing = token_pricing_data
                .get(&token_balance.token_id)
                .and_then(|opt| opt.as_ref());
            let token_with_actions =
                token_balance.into_with_actions(identity, contract, in_dev_mode, token_pricing);
            Some((id_token_identifier, token_with_actions))
        })
        .collect()
}

impl TokensScreen {
    pub fn new(app_context: &Arc<AppContext>, tokens_subscreen: TokensSubscreen) -> Self {
        let identities = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .map(|qi| (qi.identity.id(), qi))
            .collect();
        let all_known_tokens = app_context
            .db
            .get_all_known_tokens_with_data_contract(app_context)
            .unwrap_or_default();

        let my_tokens = my_tokens(
            app_context,
            &identities,
            &all_known_tokens,
            &IndexMap::new(),
        );

        let mut function_images = BTreeMap::new();

        function_images.insert(
            DistributionFunctionUI::Exponential,
            load_formula_image(EXP_FORMULA_PNG),
        );
        function_images.insert(
            DistributionFunctionUI::Logarithmic,
            load_formula_image(LOG_FORMULA_PNG),
        );
        function_images.insert(
            DistributionFunctionUI::InvertedLogarithmic,
            load_formula_image(INV_LOG_FORMULA_PNG),
        );
        function_images.insert(
            DistributionFunctionUI::Polynomial,
            load_formula_image(POLYNOMIAL_FORMULA_PNG),
        );
        function_images.insert(
            DistributionFunctionUI::Linear,
            load_formula_image(LINEAR_FORMULA_PNG),
        );

        let mut screen = Self {
            app_context: app_context.clone(),
            identities,
            all_known_tokens,
            my_tokens,
            selected_token: None,
            token_pricing_data: IndexMap::new(),
            pricing_loading_state: IndexMap::new(),
            selected_contract_id: None,
            selected_contract_description: None,
            selected_token_infos: Vec::new(),
            contract_details_loading: false,
            token_search_query: None,
            contract_search_status: ContractSearchStatus::NotStarted,
            search_current_page: 1,
            search_has_next_page: false,
            next_cursors: vec![],
            previous_cursors: vec![],
            search_results: Arc::new(Mutex::new(Vec::new())),
            backend_message: None,
            sort_column: SortColumn::OwnerIdentityAlias,
            sort_order: SortOrder::Ascending,
            use_custom_order: false,
            pending_backend_task: None,
            tokens_subscreen,
            refreshing_status: RefreshingStatus::NotRefreshing,

            // Remove token
            confirm_remove_identity_token_balance_popup: false,
            identity_token_balance_to_remove: None,
            confirm_remove_token_popup: false,
            token_to_remove: None,

            // Token Creator
            selected_token_preset: None,
            show_pop_up_info: None,
            selected_identity: None,
            selected_key: None,
            selected_wallet: None,
            wallet_password: String::new(),
            show_password: false,
            show_token_creator_confirmation_popup: false,
            token_creator_status: TokenCreatorStatus::NotStarted,
            token_creator_error_message: None,
            token_names_input: vec![(
                String::new(),
                String::new(),
                TokenNameLanguage::English,
                true,
            )],
            contract_keywords_input: String::new(),
            token_description_input: String::new(),
            should_capitalize_input: true,
            decimals_input: 0.to_string(),
            base_supply_input: TokenConfigurationV0::default_most_restrictive()
                .base_supply()
                .to_string(),
            max_supply_input: String::new(),
            start_as_paused_input: false,
            show_advanced_keeps_history: false,
            token_advanced_keeps_history: TokenKeepsHistoryRulesV0::default_for_keeping_all_history(
                true,
            ),
            main_control_group_input: String::new(),
            groups_ui: Vec::new(),
            cached_build_args: None,
            show_json_popup: false,
            json_popup_text: String::new(),

            // Action rules
            allow_transfers_to_frozen_identities: true,
            manual_minting_rules: ChangeControlRulesUI::default(),
            manual_burning_rules: ChangeControlRulesUI::default(),
            freeze_rules: ChangeControlRulesUI::default(),
            unfreeze_rules: ChangeControlRulesUI::default(),
            destroy_frozen_funds_rules: ChangeControlRulesUI::default(),
            emergency_action_rules: ChangeControlRulesUI::default(),
            max_supply_change_rules: ChangeControlRulesUI::default(),
            conventions_change_rules: ChangeControlRulesUI::default(),

            // Main control group change rules
            authorized_main_control_group_change: AuthorizedActionTakers::NoOne,
            main_control_group_change_authorized_identity: None,
            main_control_group_change_authorized_group: None,

            // Distribution (perpetual) toggles/fields
            enable_perpetual_distribution: false,
            perpetual_distribution_rules: ChangeControlRulesUI::default(),

            // Which distribution type is selected?
            perpetual_dist_type: PerpetualDistributionIntervalTypeUI::None,

            // Block-based / time-based / epoch-based inputs
            perpetual_dist_interval_input: String::new(),

            // Distribution function selection
            perpetual_dist_interval_unit: IntervalTimeUnit::Day,
            perpetual_dist_function: DistributionFunctionUI::FixedAmount,
            fixed_amount_input: String::new(),
            random_min_input: String::new(),
            random_max_input: String::new(),
            step_count_input: String::new(),
            decrease_per_interval_numerator_input: String::new(),
            decrease_per_interval_denominator_input: String::new(),
            step_decreasing_start_period_offset_input: String::new(),
            step_decreasing_initial_emission_input: String::new(),
            step_decreasing_min_value_input: String::new(),
            step_decreasing_max_interval_count_input: String::new(),
            step_decreasing_trailing_distribution_interval_amount_input: String::new(),
            stepwise_steps: Vec::new(),
            linear_int_a_input: String::new(),
            linear_int_d_input: String::new(),
            linear_int_start_step_input: String::new(),
            linear_int_starting_amount_input: String::new(),
            linear_int_min_value_input: String::new(),
            linear_int_max_value_input: String::new(),
            poly_int_a_input: String::new(),
            poly_int_m_input: String::new(),
            poly_int_n_input: String::new(),
            poly_int_d_input: String::new(),
            poly_int_s_input: String::new(),
            poly_int_o_input: String::new(),
            poly_int_b_input: String::new(),
            poly_int_min_value_input: String::new(),
            poly_int_max_value_input: String::new(),
            exp_a_input: String::new(),
            exp_m_input: String::new(),
            exp_n_input: String::new(),
            exp_d_input: String::new(),
            exp_s_input: String::new(),
            exp_o_input: String::new(),
            exp_b_input: String::new(),
            exp_min_value_input: String::new(),
            exp_max_value_input: String::new(),
            log_a_input: String::new(),
            log_d_input: String::new(),
            log_m_input: String::new(),
            log_n_input: String::new(),
            log_s_input: String::new(),
            log_o_input: String::new(),
            log_b_input: String::new(),
            log_min_value_input: String::new(),
            log_max_value_input: String::new(),
            inv_log_a_input: String::new(),
            inv_log_d_input: String::new(),
            inv_log_m_input: String::new(),
            inv_log_n_input: String::new(),
            inv_log_s_input: String::new(),
            inv_log_o_input: String::new(),
            inv_log_b_input: String::new(),
            inv_log_min_value_input: String::new(),
            inv_log_max_value_input: String::new(),

            // Similarly for identity recipients, you might store:
            perpetual_dist_recipient: TokenDistributionRecipientUI::ContractOwner,
            perpetual_dist_recipient_identity_input: None,

            // Pre-programmed distribution
            enable_pre_programmed_distribution: false,
            // Possibly let them paste in a JSON schedule, or some minimal UI for (timestamp -> {id -> amount}).
            // For an example, we'll keep it simple:
            pre_programmed_distributions: Vec::new(),

            // new_tokens_destination_identity
            new_tokens_destination_identity_should_default_to_contract_owner: true,
            new_tokens_destination_other_identity_enabled: false,
            new_tokens_destination_other_identity: String::new(),
            new_tokens_destination_identity_rules: ChangeControlRulesUI::default(),

            // minting_allow_choosing_destination
            minting_allow_choosing_destination: false,
            minting_allow_choosing_destination_rules: ChangeControlRulesUI::default(),
            function_images,
            function_textures: BTreeMap::default(),
            should_reset_collapsing_states: false,
        };

        if let Ok(saved_ids) = screen.app_context.db.load_token_order() {
            screen.reorder_vec_to(saved_ids);
            screen.use_custom_order = true;
        }

        screen
    }

    // ─────────────────────────────────────────────────────────────────
    // Reordering
    // ─────────────────────────────────────────────────────────────────

    /// Reorder `my_tokens` to match a given list of (token_id, identity_id).
    fn reorder_vec_to(&mut self, new_order: Vec<(Identifier, Identifier)>) {
        // Create a temporary new IndexMap in the desired order
        let mut reordered = IndexMap::with_capacity(self.my_tokens.len());

        for (token_id, identity_id) in new_order {
            if let Some((key, value)) = self
                .my_tokens
                .iter()
                .find(|(_, v)| v.token_id == token_id && v.identity_id == identity_id)
                .map(|(k, v)| (*k, v.clone()))
            {
                reordered.insert(key, value);
            }
        }

        // Replace the original with the reordered map
        //self.my_tokens = reordered;
    }

    /// Save the current map's order of token IDs to the DB
    fn save_current_order(&self) {
        let all_ids = self
            .my_tokens
            .iter()
            .map(|(_, token)| (token.token_id, token.identity_id))
            .collect::<Vec<_>>();

        self.app_context
            .db
            .save_token_order(all_ids)
            .map_err(|e| {
                eprintln!("Error saving token order: {}", e);
                e
            })
            .ok();
    }

    fn toggle_sort(&mut self, column: SortColumn) {
        self.use_custom_order = false;
        if self.sort_column == column {
            self.sort_order = match self.sort_order {
                SortOrder::Ascending => SortOrder::Descending,
                SortOrder::Descending => SortOrder::Ascending,
            };
            self.save_current_order();
        } else {
            self.sort_column = column;
            self.sort_order = SortOrder::Ascending;
            self.save_current_order();
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // Message handling
    // ─────────────────────────────────────────────────────────────────

    fn dismiss_message(&mut self) {
        self.backend_message = None;
    }

    fn check_error_expiration(&mut self) {
        if let Some((_, _, timestamp)) = &self.backend_message {
            let now = Utc::now();
            let elapsed = now.signed_duration_since(*timestamp);
            if elapsed.num_seconds() >= 10 {
                self.dismiss_message();
            }
        }
    }

    fn history_row(&mut self, ui: &mut Ui) {
        // --- 1.  pull or create the rules object --------------------------------
        let rules = self.token_advanced_keeps_history;

        let TokenKeepsHistoryRulesV0 {
            keeps_transfer_history,
            keeps_freezing_history,
            keeps_minting_history,
            keeps_burning_history,
            keeps_direct_pricing_history,
            keeps_direct_purchase_history,
        } = rules;

        let flags = [
            keeps_transfer_history,
            keeps_freezing_history,
            keeps_minting_history,
            keeps_burning_history,
            keeps_direct_pricing_history,
            keeps_direct_purchase_history,
        ];

        let all_on = flags.iter().all(|b| *b);
        let none_on = flags.iter().all(|b| !*b);

        // --- 2.  parent tri-state checkbox --------------------------------------
        let mut parent_state: Option<bool> = if all_on {
            Some(true)
        } else if none_on {
            Some(false)
        } else {
            None // ⇒ indeterminate
        };

        //--------------------------------------------------------------
        // 2. parent tri-state + “Advanced” button in **one** cell
        //--------------------------------------------------------------
        ui.horizontal(|ui| {
            // tri-state checkbox
            let response = tri_state(ui, &mut parent_state, "Keep history");

            // propagate changes from parent to all children
            if response.clicked() {
                if let Some(val) = parent_state {
                    self.token_advanced_keeps_history.keeps_transfer_history = val;
                    self.token_advanced_keeps_history.keeps_freezing_history = val;
                    self.token_advanced_keeps_history.keeps_minting_history = val;
                    self.token_advanced_keeps_history.keeps_burning_history = val;
                    self.token_advanced_keeps_history
                        .keeps_direct_pricing_history = val;
                    self.token_advanced_keeps_history
                        .keeps_direct_purchase_history = val;
                }
            }

            ui.add_space(8.0);
            let arrow = if self.show_advanced_keeps_history {
                "[-]"
            } else {
                "[+]"
            };
            if ui
                .small_button(format!("Advanced {arrow}"))
                .on_hover_text("Configure individual history ledgers")
                .clicked()
            {
                self.show_advanced_keeps_history = !self.show_advanced_keeps_history;
            }
        });

        // --- 4.  indented sub-checkboxes when advanced is open ------------------
        if self.show_advanced_keeps_history {
            sub_checkbox(
                ui,
                &mut self.token_advanced_keeps_history.keeps_transfer_history,
                "Transfers",
            );
            sub_checkbox(
                ui,
                &mut self.token_advanced_keeps_history.keeps_freezing_history,
                "Freezes / unfreezes",
            );
            sub_checkbox(
                ui,
                &mut self.token_advanced_keeps_history.keeps_minting_history,
                "Mints",
            );
            sub_checkbox(
                ui,
                &mut self.token_advanced_keeps_history.keeps_burning_history,
                "Burns",
            );
            sub_checkbox(
                ui,
                &mut self
                    .token_advanced_keeps_history
                    .keeps_direct_pricing_history,
                "Direct-pricing changes",
            );
            sub_checkbox(
                ui,
                &mut self
                    .token_advanced_keeps_history
                    .keeps_direct_purchase_history,
                "Direct purchases",
            );
        }
    }

    fn estimate_registration_cost(&self) -> Credits {
        let registration_fees = &self
            .app_context
            .platform_version()
            .fee_version
            .data_contract_registration;
        let mut fee = registration_fees.base_contract_registration_fee;
        fee += registration_fees.token_registration_fee;
        if self.enable_perpetual_distribution {
            fee += registration_fees.token_uses_perpetual_distribution_fee;
        }
        if self.enable_pre_programmed_distribution {
            fee += registration_fees.token_uses_pre_programmed_distribution_fee;
        }
        let contract_keywords = if self.contract_keywords_input.trim().is_empty() {
            Vec::new()
        } else {
            self.contract_keywords_input
                .split(',')
                .filter_map(|s| {
                    let trimmed = s.trim().to_string();
                    if trimmed.len() < 3 || trimmed.len() > 50 {
                        None
                    } else {
                        Some(trimmed)
                    }
                })
                .collect::<Vec<String>>()
        };

        fee += registration_fees.search_keyword_fee * contract_keywords.len() as u64;
        let searchable_count = self
            .token_names_input
            .iter()
            .filter(|(_, _, _, searchable)| *searchable) // or `.is_searchable()` if it's a method
            .count();

        fee += registration_fees.search_keyword_fee * searchable_count as u64;

        fee += 200_000_000; //just an extra estimate

        fee
    }

    fn build_distribution_rules(&mut self) -> Result<TokenDistributionRulesV0, String> {
        // 1) Validate distribution input, parse numeric fields, etc.
        let distribution_function = match self.perpetual_dist_function {
            DistributionFunctionUI::FixedAmount => DistributionFunction::FixedAmount {
                amount: self.fixed_amount_input.parse::<u64>().unwrap_or(0),
            },
            DistributionFunctionUI::StepDecreasingAmount => {
                DistributionFunction::StepDecreasingAmount {
                    step_count: self.step_count_input.parse::<u32>().unwrap_or(0),
                    decrease_per_interval_numerator: self
                        .decrease_per_interval_numerator_input
                        .parse::<u16>()
                        .unwrap_or(0),
                    decrease_per_interval_denominator: self
                        .decrease_per_interval_denominator_input
                        .parse::<u16>()
                        .unwrap_or(0),
                    start_decreasing_offset: if self
                        .step_decreasing_start_period_offset_input
                        .is_empty()
                    {
                        None
                    } else {
                        match self
                            .step_decreasing_start_period_offset_input
                            .parse::<u64>()
                        {
                            Ok(0) => None,
                            Ok(v) => Some(v),
                            Err(_) => {
                                return Err("Invalid start decreasing offset for StepDecreasingAmount distribution. Put 0 for None.".to_string());
                            }
                        }
                    },
                    distribution_start_amount: self
                        .step_decreasing_initial_emission_input
                        .parse::<u64>()
                        .unwrap_or(0),
                    min_value: if self.step_decreasing_start_period_offset_input.is_empty() {
                        None
                    } else {
                        match self.step_decreasing_min_value_input.parse::<u64>() {
                            Ok(0) => None,
                            Ok(v) => Some(v),
                            Err(_) => {
                                return Err(
                                    "Invalid min value for StepDecreasingAmount distribution. Put 0 for None."
                                        .to_string(),
                                );
                            }
                        }
                    },
                    max_interval_count: if self.step_decreasing_start_period_offset_input.is_empty()
                    {
                        None
                    } else {
                        match self.step_decreasing_max_interval_count_input.parse::<u16>() {
                            Ok(0) => None,
                            Ok(v) => Some(v),
                            Err(_) => {
                                return Err(
                                    "Invalid max interval count for StepDecreasingAmount distribution. Put 0 for None."
                                        .to_string(),
                                );
                            }
                        }
                    },
                    trailing_distribution_interval_amount: self
                        .step_decreasing_trailing_distribution_interval_amount_input
                        .parse::<u64>()
                        .unwrap_or(0),
                }
            }
            DistributionFunctionUI::Stepwise => {
                let steps: BTreeMap<u64, TokenAmount> = self
                    .stepwise_steps
                    .iter()
                    .map(|(k, v)| (k.parse::<u64>().unwrap_or(0), v.parse::<u64>().unwrap_or(0)))
                    .collect();
                DistributionFunction::Stepwise(steps)
            }
            DistributionFunctionUI::Linear => DistributionFunction::Linear {
                a: self.linear_int_a_input.parse::<i64>().unwrap_or(0),
                d: self.linear_int_d_input.parse::<u64>().unwrap_or(1),
                start_step: self
                    .linear_int_start_step_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                starting_amount: self
                    .linear_int_starting_amount_input
                    .parse::<u64>()
                    .unwrap_or(0),
                min_value: self
                    .linear_int_min_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                max_value: self
                    .linear_int_max_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
            },
            DistributionFunctionUI::Polynomial => DistributionFunction::Polynomial {
                a: self.poly_int_a_input.parse::<i64>().unwrap_or(0),
                m: self.poly_int_m_input.parse::<i64>().unwrap_or(0),
                n: self.poly_int_n_input.parse::<u64>().unwrap_or(0),
                d: self.poly_int_d_input.parse::<u64>().unwrap_or(0),
                start_moment: self
                    .poly_int_s_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                o: self.poly_int_o_input.parse::<i64>().unwrap_or(0),
                b: self.poly_int_b_input.parse::<u64>().unwrap_or(0),
                min_value: self
                    .poly_int_min_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                max_value: self
                    .poly_int_max_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
            },
            DistributionFunctionUI::Exponential => DistributionFunction::Exponential {
                a: self.exp_a_input.parse::<u64>().unwrap_or(0),
                d: self.exp_d_input.parse::<u64>().unwrap_or(0),
                m: self.exp_m_input.parse::<i64>().unwrap_or(0),
                n: self.exp_n_input.parse::<u64>().unwrap_or(0),
                o: self.exp_o_input.parse::<i64>().unwrap_or(0),
                start_moment: self.exp_s_input.parse::<u64>().map(Some).unwrap_or(None),
                b: self.exp_b_input.parse::<u64>().unwrap_or(0),
                min_value: self
                    .exp_min_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                max_value: self
                    .exp_max_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
            },
            DistributionFunctionUI::Logarithmic => DistributionFunction::Logarithmic {
                a: self.log_a_input.parse::<i64>().unwrap_or(0),
                d: self.log_d_input.parse::<u64>().unwrap_or(0),
                m: self.log_m_input.parse::<u64>().unwrap_or(0),
                n: self.log_n_input.parse::<u64>().unwrap_or(0),
                start_moment: self.log_s_input.parse::<u64>().map(Some).unwrap_or(None),
                o: self.log_o_input.parse::<i64>().unwrap_or(0),
                b: self.log_b_input.parse::<u64>().unwrap_or(0),
                min_value: self
                    .log_min_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
                max_value: self
                    .log_max_value_input
                    .parse::<u64>()
                    .map(Some)
                    .unwrap_or(None),
            },
            DistributionFunctionUI::InvertedLogarithmic => {
                DistributionFunction::InvertedLogarithmic {
                    a: self.inv_log_a_input.parse::<i64>().unwrap_or(0),
                    d: self.inv_log_d_input.parse::<u64>().unwrap_or(0),
                    m: self.inv_log_m_input.parse::<u64>().unwrap_or(0),
                    n: self.inv_log_n_input.parse::<u64>().unwrap_or(0),
                    start_moment: self
                        .inv_log_s_input
                        .parse::<u64>()
                        .map(Some)
                        .unwrap_or(None),
                    o: self.inv_log_o_input.parse::<i64>().unwrap_or(0),
                    b: self.inv_log_b_input.parse::<u64>().unwrap_or(0),
                    min_value: self
                        .inv_log_min_value_input
                        .parse::<u64>()
                        .map(Some)
                        .unwrap_or(None),
                    max_value: self
                        .inv_log_max_value_input
                        .parse::<u64>()
                        .map(Some)
                        .unwrap_or(None),
                }
            }
        };
        let maybe_perpetual_distribution = if self.enable_perpetual_distribution {
            // Construct the `TokenPerpetualDistributionV0` from your selected type + function
            let dist_type =
                match self.perpetual_dist_type {
                    PerpetualDistributionIntervalTypeUI::BlockBased => {
                        // parse interval, parse emission
                        // parse distribution function
                        RewardDistributionType::BlockBasedDistribution {
                            interval: self.perpetual_dist_interval_input.parse::<u64>().map_err(
                                |_| "Distribution interval not a valid number".to_string(),
                            )?,
                            function: distribution_function,
                        }
                    }
                    PerpetualDistributionIntervalTypeUI::EpochBased => {
                        RewardDistributionType::EpochBasedDistribution {
                            interval: self.perpetual_dist_interval_input.parse::<u16>().map_err(
                                |_| "Distribution interval not a valid number".to_string(),
                            )?,
                            function: distribution_function,
                        }
                    }
                    PerpetualDistributionIntervalTypeUI::TimeBased => {
                        RewardDistributionType::TimeBasedDistribution {
                            interval: self.perpetual_dist_interval_unit.ms_for_amount(
                                self.perpetual_dist_interval_input.parse::<u64>().map_err(
                                    |_| "Distribution interval not a valid number".to_string(),
                                )?,
                            ),
                            function: distribution_function,
                        }
                    }
                    _ => RewardDistributionType::BlockBasedDistribution {
                        interval: 0,
                        function: DistributionFunction::FixedAmount { amount: 0 },
                    },
                };

            let recipient = match self.perpetual_dist_recipient {
                TokenDistributionRecipientUI::ContractOwner => {
                    TokenDistributionRecipient::ContractOwner
                }
                TokenDistributionRecipientUI::Identity => {
                    if let Some(id) = self.perpetual_dist_recipient_identity_input.as_ref() {
                        let id_res = Identifier::from_string(id, Encoding::Base58);
                        TokenDistributionRecipient::Identity(id_res.unwrap_or_default())
                    } else {
                        self.token_creator_error_message = Some(
                            "Invalid base58 identifier for perpetual distribution recipient"
                                .to_string(),
                        );
                        return Err(
                            "Invalid base58 identifier for perpetual distribution recipient"
                                .to_string(),
                        );
                    }
                }
                TokenDistributionRecipientUI::EvonodesByParticipation => {
                    TokenDistributionRecipient::EvonodesByParticipation
                }
            };

            Some(TokenPerpetualDistribution::V0(
                TokenPerpetualDistributionV0 {
                    distribution_type: dist_type,
                    distribution_recipient: recipient,
                },
            ))
        } else {
            None
        };

        // 2) Build the distribution rules structure
        let dist_rules_v0 = TokenDistributionRulesV0 {
            perpetual_distribution: maybe_perpetual_distribution,
            perpetual_distribution_rules: self
                .perpetual_distribution_rules
                .extract_change_control_rules("Perpetual Distribution")?,
            pre_programmed_distribution: if self.enable_pre_programmed_distribution {
                let distributions: BTreeMap<u64, BTreeMap<Identifier, u64>> =
                    match self.parse_pre_programmed_distributions() {
                        Ok(distributions) => distributions
                            .into_iter()
                            .map(|(k, v)| (k, std::iter::once(v).collect()))
                            .collect(),
                        Err(err) => {
                            self.token_creator_error_message = Some(err.clone());
                            return Err(err.to_string());
                        }
                    };

                Some(TokenPreProgrammedDistribution::V0(
                    TokenPreProgrammedDistributionV0 { distributions },
                ))
            } else {
                None
            },
            new_tokens_destination_identity: if self
                .new_tokens_destination_identity_should_default_to_contract_owner
            {
                Some(
                    self.selected_identity
                        .as_ref()
                        .ok_or("No selected identity".to_string())?
                        .identity
                        .id(),
                )
            } else if self.new_tokens_destination_other_identity_enabled {
                Some(
                    Identifier::from_string(
                        &self.new_tokens_destination_other_identity,
                        Encoding::Base58,
                    )
                    .map_err(|e| e.to_string())?,
                )
            } else {
                None
            },
            new_tokens_destination_identity_rules: self
                .new_tokens_destination_identity_rules
                .extract_change_control_rules("New Tokens Destination Identity")?,
            minting_allow_choosing_destination: self.minting_allow_choosing_destination,
            minting_allow_choosing_destination_rules: self
                .minting_allow_choosing_destination_rules
                .extract_change_control_rules("Minting Allow Choosing Destination")?,
            change_direct_purchase_pricing_rules: self
                .minting_allow_choosing_destination_rules // TODO!
                .extract_change_control_rules("Change Direct Purchase Pricing")?,
        };

        Ok(dist_rules_v0)
    }

    /// Attempts to parse the `pre_programmed_distributions` into a BTreeMap.
    /// Returns an error string if any row fails.
    pub fn parse_pre_programmed_distributions(
        &mut self,
    ) -> Result<BTreeMap<u64, (Identifier, u64)>, String> {
        let mut map = BTreeMap::new();

        let now = Utc::now();

        for (i, entry) in self.pre_programmed_distributions.iter().enumerate() {
            // Convert days/hours/minutes into a timestamp.
            let offset = Duration::days(entry.days as i64)
                + Duration::hours(entry.hours as i64)
                + Duration::minutes(entry.minutes as i64);
            let timestamp = (now + offset).timestamp_millis() as u64;

            // Parse identity
            let id =
                Identifier::from_string(&entry.identity_str, Encoding::Base58).map_err(|_| {
                    format!(
                        "Row {}: invalid base58 identity '{}'",
                        i + 1,
                        entry.identity_str
                    )
                })?;

            // Parse amount
            let amount = entry.amount_str.parse::<u64>().map_err(|_| {
                format!(
                    "Row {}: invalid distribution amount (expected u64). Got '{}'",
                    i + 1,
                    entry.amount_str
                )
            })?;

            // Insert into the map
            map.insert(timestamp, (id, amount));
        }
        Ok(map)
    }

    fn reset_token_creator(&mut self) {
        self.selected_identity = None;
        self.selected_key = None;
        self.token_creator_status = TokenCreatorStatus::NotStarted;
        self.token_names_input = vec![(
            String::new(),
            String::new(),
            TokenNameLanguage::English,
            true,
        )];
        self.contract_keywords_input = "".to_string();
        self.token_description_input = "".to_string();
        self.decimals_input = "8".to_string();
        self.base_supply_input = "100000".to_string();
        self.max_supply_input = "".to_string();
        self.start_as_paused_input = false;
        self.should_capitalize_input = true;
        self.token_advanced_keeps_history =
            TokenKeepsHistoryRulesV0::default_for_keeping_all_history(true);
        self.show_advanced_keeps_history = false;
        self.manual_minting_rules = ChangeControlRulesUI::default();
        self.manual_burning_rules = ChangeControlRulesUI::default();
        self.freeze_rules = ChangeControlRulesUI::default();
        self.unfreeze_rules = ChangeControlRulesUI::default();
        self.destroy_frozen_funds_rules = ChangeControlRulesUI::default();
        self.emergency_action_rules = ChangeControlRulesUI::default();
        self.max_supply_change_rules = ChangeControlRulesUI::default();
        self.conventions_change_rules = ChangeControlRulesUI::default();
        self.authorized_main_control_group_change = AuthorizedActionTakers::NoOne;
        self.main_control_group_change_authorized_identity = None;
        self.main_control_group_change_authorized_group = None;
        self.main_control_group_input = "".to_string();
        self.groups_ui = vec![];

        self.perpetual_dist_function = DistributionFunctionUI::FixedAmount;
        self.perpetual_dist_type = PerpetualDistributionIntervalTypeUI::None;
        self.perpetual_dist_interval_input = "".to_string();
        self.fixed_amount_input = "".to_string();
        self.random_min_input = "".to_string();
        self.random_max_input = "".to_string();
        self.step_count_input = "".to_string();
        self.decrease_per_interval_numerator_input = "".to_string();
        self.decrease_per_interval_denominator_input = "".to_string();
        self.step_decreasing_start_period_offset_input = "".to_string();
        self.step_decreasing_initial_emission_input = "".to_string();
        self.step_decreasing_min_value_input = "".to_string();
        self.step_decreasing_max_interval_count_input = "".to_string();
        self.step_decreasing_trailing_distribution_interval_amount_input = "".to_string();
        self.stepwise_steps = vec![(String::new(), String::new())];
        self.linear_int_a_input = "".to_string();
        self.linear_int_d_input = "".to_string();
        self.linear_int_start_step_input = "".to_string();
        self.linear_int_starting_amount_input = "".to_string();
        self.linear_int_min_value_input = "".to_string();
        self.linear_int_max_value_input = "".to_string();
        self.poly_int_a_input = "".to_string();
        self.poly_int_m_input = "".to_string();
        self.poly_int_n_input = "".to_string();
        self.poly_int_d_input = "".to_string();
        self.poly_int_s_input = "".to_string();
        self.poly_int_o_input = "".to_string();
        self.poly_int_b_input = "".to_string();
        self.poly_int_min_value_input = "".to_string();
        self.poly_int_max_value_input = "".to_string();
        self.exp_a_input = "".to_string();
        self.exp_m_input = "".to_string();
        self.exp_n_input = "".to_string();
        self.exp_d_input = "".to_string();
        self.exp_s_input = "".to_string();
        self.exp_o_input = "".to_string();
        self.exp_b_input = "".to_string();
        self.exp_min_value_input = "".to_string();
        self.exp_max_value_input = "".to_string();
        self.log_a_input = "".to_string();
        self.log_d_input = "".to_string();
        self.log_m_input = "".to_string();
        self.log_n_input = "".to_string();
        self.log_s_input = "".to_string();
        self.log_o_input = "".to_string();
        self.log_b_input = "".to_string();
        self.log_min_value_input = "".to_string();
        self.log_max_value_input = "".to_string();
        self.inv_log_a_input = "".to_string();
        self.inv_log_d_input = "".to_string();
        self.inv_log_m_input = "".to_string();
        self.inv_log_n_input = "".to_string();
        self.inv_log_s_input = "".to_string();
        self.inv_log_o_input = "".to_string();
        self.inv_log_b_input = "".to_string();
        self.inv_log_min_value_input = "".to_string();
        self.inv_log_max_value_input = "".to_string();
        self.perpetual_dist_recipient = TokenDistributionRecipientUI::ContractOwner;
        self.perpetual_dist_recipient_identity_input = None;
        self.enable_perpetual_distribution = false;
        self.perpetual_distribution_rules = ChangeControlRulesUI::default();
        self.enable_pre_programmed_distribution = false;
        self.pre_programmed_distributions = Vec::new();
        self.new_tokens_destination_other_identity_enabled = false;
        self.new_tokens_destination_identity_rules = ChangeControlRulesUI::default();
        self.new_tokens_destination_other_identity = "".to_string();
        self.minting_allow_choosing_destination = false;
        self.minting_allow_choosing_destination_rules = ChangeControlRulesUI::default();

        self.show_token_creator_confirmation_popup = false;
        self.token_creator_error_message = None;
    }

    fn add_token_to_tracked_tokens(&mut self, token_info: TokenInfo) -> Result<AppAction, String> {
        let contract = self
            .app_context
            .get_contract_by_id(&token_info.data_contract_id)
            .map_err(|e| e.to_string())?
            .ok_or("Could not find contract")?;

        self.all_known_tokens.insert(
            token_info.token_id,
            TokenInfoWithDataContract::from_with_data_contract(
                token_info.clone(),
                contract.contract,
            ),
        );

        self.display_message("Added token", MessageType::Success);

        Ok(AppAction::BackendTasks(
            vec![
                BackendTask::TokenTask(Box::new(TokenTask::SaveTokenLocally(token_info))),
                BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances)),
            ],
            BackendTasksExecutionMode::Sequential,
        ))
    }

    fn goto_next_search_page(&mut self) -> AppAction {
        // If we have a next cursor:
        if let Some(next_cursor) = self.next_cursors.last().cloned() {
            // set status
            let now = Utc::now().timestamp() as u64;
            self.contract_search_status = ContractSearchStatus::WaitingForResult(now);

            // push the current one onto “previous” so we can go back
            // if the user is on page N, and we have a nextCursor in next_cursors[N - 1] or so
            self.previous_cursors.push(next_cursor.clone());

            self.search_current_page += 1;

            // Dispatch
            let query_string = self.token_search_query.clone().unwrap_or_default();

            return AppAction::BackendTask(BackendTask::TokenTask(Box::new(
                TokenTask::QueryDescriptionsByKeyword(query_string, Some(next_cursor)),
            )));
        }
        AppAction::None
    }

    fn goto_previous_search_page(&mut self) -> AppAction {
        if self.search_current_page > 1 {
            // Move to (page - 1)
            self.search_current_page -= 1;
            let now = Utc::now().timestamp() as u64;
            self.contract_search_status = ContractSearchStatus::WaitingForResult(now);

            // The “last” previous_cursors item is the new page’s state
            if let Some(prev_cursor) = self.previous_cursors.pop() {
                // Possibly pop from next_cursors if we want to re-insert it later
                // self.next_cursors.truncate(self.search_current_page - 1);
                let query_string = self.token_search_query.clone().unwrap_or_default();
                return AppAction::BackendTask(BackendTask::TokenTask(Box::new(
                    TokenTask::QueryDescriptionsByKeyword(query_string, Some(prev_cursor)),
                )));
            }
        }
        AppAction::None
    }

    fn show_remove_identity_token_balance_popup(&mut self, ui: &mut Ui) {
        // If no token is set, nothing to confirm
        let token_to_remove = match &self.identity_token_balance_to_remove {
            Some(token) => token.clone(),
            None => {
                self.confirm_remove_identity_token_balance_popup = false;
                return;
            }
        };

        let mut is_open = true;

        egui::Window::new("Confirm Stop Tracking Balance")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                ui.label(format!(
                    "Are you sure you want to stop tracking the token \"{}\" for identity \"{}\"?",
                    token_to_remove.token_alias,
                    token_to_remove.identity_id.to_string(Encoding::Base58)
                ));

                // Confirm button
                if ui.button("Confirm").clicked() {
                    if let Err(e) = self
                        .app_context
                        .remove_token_balance(token_to_remove.token_id, token_to_remove.identity_id)
                    {
                        self.backend_message = Some((
                            format!("Error removing token balance: {}", e),
                            MessageType::Error,
                            Utc::now(),
                        ));
                        self.confirm_remove_identity_token_balance_popup = false;
                        self.identity_token_balance_to_remove = None;
                    } else {
                        self.confirm_remove_identity_token_balance_popup = false;
                        self.identity_token_balance_to_remove = None;
                        self.refresh();
                    };
                }

                // Cancel button
                if ui.button("Cancel").clicked() {
                    self.confirm_remove_identity_token_balance_popup = false;
                    self.identity_token_balance_to_remove = None;
                }
            });

        // If user closes the popup window (the [x] button), also reset state
        if !is_open {
            self.confirm_remove_identity_token_balance_popup = false;
            self.identity_token_balance_to_remove = None;
        }
    }

    fn show_remove_token_popup(&mut self, ui: &mut Ui) {
        // If no token is set, nothing to confirm
        let token_to_remove = match &self.token_to_remove {
            Some(token) => *token,
            None => {
                self.confirm_remove_token_popup = false;
                return;
            }
        };

        // find the token name from one of the identity token balances in my tokens
        let token_name = self
            .all_known_tokens
            .get(&token_to_remove)
            .map(|t| t.token_name.clone())
            .unwrap_or_else(|| token_to_remove.to_string(Encoding::Base58));

        let mut is_open = true;

        egui::Window::new("Confirm Remove Token")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                ui.label(format!(
                    "Are you sure you want to stop tracking the token \"{}\"? You can re-add it later. Your actual token balance will not change with this action.",
                    token_name,
                ));

                // Confirm button
                if ui.button("Confirm").clicked() {
                    if let Err(e) = self.app_context.db.remove_token(
                        &token_to_remove,
                        &self.app_context,
                    ) {
                        self.backend_message = Some((
                            format!("Error removing token balance: {}", e),
                            MessageType::Error,
                            Utc::now(),
                        ));
                        self.confirm_remove_token_popup = false;
                        self.token_to_remove = None;
                    } else {
                        self.confirm_remove_token_popup = false;
                        self.token_to_remove = None;
                        self.refresh();
                    }
                }

                // Cancel button
                if ui.button("Cancel").clicked() {
                    self.confirm_remove_token_popup = false;
                    self.token_to_remove = None;
                }
            });

        // If user closes the popup window (the [x] button), also reset state
        if !is_open {
            self.confirm_remove_token_popup = false;
            self.token_to_remove = None;
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// ScreenLike implementation
// ─────────────────────────────────────────────────────────────────
impl ScreenLike for TokensScreen {
    fn refresh(&mut self) {
        self.all_known_tokens = self
            .app_context
            .db
            .get_all_known_tokens_with_data_contract(&self.app_context)
            .unwrap_or_default();

        self.identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .map(|qi| (qi.identity.id(), qi))
            .collect();

        self.my_tokens = my_tokens(
            &self.app_context,
            &self.identities,
            &self.all_known_tokens,
            &self.token_pricing_data,
        );

        match self.app_context.db.load_token_order() {
            Ok(saved_ids) => {
                self.reorder_vec_to(saved_ids);

                self.use_custom_order = true;
            }
            Err(e) => {
                eprintln!("Error loading token order: {}", e);
            }
        }
    }

    fn refresh_on_arrival(&mut self) {
        self.selected_token = None;
        self.should_reset_collapsing_states = true;

        self.all_known_tokens = self
            .app_context
            .db
            .get_all_known_tokens_with_data_contract(&self.app_context)
            .unwrap_or_default();
        self.identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .map(|qi| (qi.identity.id(), qi))
            .collect();

        self.my_tokens = my_tokens(
            &self.app_context,
            &self.identities,
            &self.all_known_tokens,
            &self.token_pricing_data,
        );
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;

        self.check_error_expiration();

        // Build top-right buttons
        let right_buttons = if self.app_context.network != Network::Dash {
            match self.tokens_subscreen {
                TokensSubscreen::MyTokens => vec![
                    (
                        "Add Token",
                        DesiredAppAction::AddScreenType(Box::new(ScreenType::AddTokenById)),
                    ),
                    (
                        "Refresh",
                        DesiredAppAction::BackendTask(Box::new(BackendTask::TokenTask(Box::new(
                            TokenTask::QueryMyTokenBalances,
                        )))),
                    ),
                ],
                TokensSubscreen::SearchTokens => vec![],
                TokensSubscreen::TokenCreator => vec![],
            }
        } else {
            vec![]
        };

        // Top panel
        if let Some(token_id) = self.selected_token {
            let token_name: String = self
                .all_known_tokens
                .get(&token_id)
                .map(|t| t.token_name.clone())
                .unwrap_or_else(|| token_id.to_string(Encoding::Base58));

            action |= add_top_panel(
                ctx,
                &self.app_context,
                vec![
                    ("Tokens", AppAction::Custom("Back to tokens".to_string())),
                    (&token_name.to_string(), AppAction::None),
                ],
                right_buttons.clone(),
            );
        } else if let Some(contract_id) = self.selected_contract_id {
            let contract_name = format!(
                "{}...",
                contract_id
                    .to_string(Encoding::Base58)
                    .chars()
                    .take(6)
                    .collect::<String>()
            );

            action |= add_top_panel(
                ctx,
                &self.app_context,
                vec![
                    (
                        "Tokens",
                        AppAction::Custom("Back to tokens from contract".to_string()),
                    ),
                    (&format!("Contract {contract_name}"), AppAction::None),
                ],
                right_buttons.clone(),
            );
        } else {
            action |= add_top_panel(
                ctx,
                &self.app_context,
                vec![("Tokens", AppAction::None)],
                right_buttons.clone(),
            );
        }

        // Left panel
        match self.tokens_subscreen {
            TokensSubscreen::MyTokens => {
                action |= add_left_panel(
                    ctx,
                    &self.app_context,
                    RootScreenType::RootScreenMyTokenBalances,
                );
            }
            TokensSubscreen::SearchTokens => {
                action |= add_left_panel(
                    ctx,
                    &self.app_context,
                    RootScreenType::RootScreenTokenSearch,
                );
            }
            TokensSubscreen::TokenCreator => {
                action |= add_left_panel(
                    ctx,
                    &self.app_context,
                    RootScreenType::RootScreenTokenCreator,
                );
            }
        }

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ctx, self.app_context.as_ref());

        // Main panel
        action |= island_central_panel(ctx, |ui| {
            let mut inner_action = AppAction::None;

            if self.app_context.network == Network::Dash {
                ui.add_space(50.0);
                ui.vertical_centered(|ui| {
                    ui.heading(
                        RichText::new("Tokens not supported on Mainnet yet. Testnet only.")
                            .strong(),
                    );
                });
                return inner_action;
            }

            match self.tokens_subscreen {
                TokensSubscreen::MyTokens => {
                    inner_action |= self.render_my_tokens_subscreen(ui);
                }
                TokensSubscreen::SearchTokens => {
                    if self.selected_contract_id.is_some() {
                        inner_action |=
                            self.render_contract_details(ui, &self.selected_contract_id.unwrap());
                        // Render the JSON popup if needed
                        if self.show_json_popup {
                            self.render_data_contract_json_popup(ui);
                        }
                    } else {
                        inner_action |= self.render_keyword_search(ui);
                    }
                }
                TokensSubscreen::TokenCreator => {
                    inner_action |= self.render_token_creator(ctx, ui);
                }
            }

            // If we are refreshing, show a spinner at the bottom
            if let RefreshingStatus::Refreshing(start_time) = self.refreshing_status {
                ui.add_space(5.0);
                let now = Utc::now().timestamp() as u64;
                let elapsed = now - start_time;
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.label(format!("Refreshing... Time so far: {}", elapsed));
                    ui.add(egui::widgets::Spinner::default().color(Color32::from_rgb(0, 128, 255)));
                });
                ui.add_space(10.0);
            }

            // If there's a backend message, show it at the bottom
            if let Some((msg, msg_type, timestamp)) = self.backend_message.clone() {
                let color = match msg_type {
                    MessageType::Error => Color32::DARK_RED,
                    MessageType::Info => Color32::BLACK,
                    MessageType::Success => Color32::DARK_GREEN,
                };
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(color, &msg);
                        let now = Utc::now();
                        let elapsed = now.signed_duration_since(timestamp);
                        if ui
                            .button(format!("Dismiss ({})", 10 - elapsed.num_seconds()))
                            .clicked()
                        {
                            self.dismiss_message();
                        }
                    });
                });
            }

            if self.confirm_remove_identity_token_balance_popup {
                self.show_remove_identity_token_balance_popup(ui);
            }
            if self.confirm_remove_token_popup {
                self.show_remove_token_popup(ui);
            }

            // If we have info text, open a pop-up window to show it
            if let Some(info_text) = self.show_pop_up_info.clone() {
                egui::Window::new("Distribution Type Info")
                    .collapsible(false)
                    .resizable(true)
                    .show(ui.ctx(), |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            let mut cache = CommonMarkCache::default();
                            CommonMarkViewer::new().show(ui, &mut cache, &info_text);
                        });

                        if ui.button("Close").clicked() {
                            self.show_pop_up_info = None;
                        }
                    });
            }

            inner_action
        });

        // Post-processing on user actions
        match action {
            AppAction::BackendTask(BackendTask::TokenTask(ref token_task))
                if matches!(token_task.as_ref(), TokenTask::QueryMyTokenBalances) =>
            {
                self.refreshing_status =
                    RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
            }
            AppAction::SetMainScreenThenGoToMainScreen(_) => {
                self.refreshing_status = RefreshingStatus::NotRefreshing;

                // should put these in a fn
                self.contract_search_status = ContractSearchStatus::NotStarted;
                self.selected_token = None;
                self.selected_contract_id = None;
                self.token_search_query = None;
                self.search_current_page = 1;
                self.search_has_next_page = false;
                self.search_results = Arc::new(Mutex::new(Vec::new()));
                self.selected_contract_id = None;
                self.selected_contract_description = None;

                self.reset_token_creator();
            }
            AppAction::Custom(ref s) if s == "Back to tokens" => {
                self.selected_token = None;
            }
            AppAction::Custom(ref s) if s == "Back to tokens from contract" => {
                self.selected_contract_id = None;
            }
            _ => {
                // No extra processing needed
            }
        }

        if action == AppAction::None {
            if let Some(bt) = self.pending_backend_task.take() {
                action = AppAction::BackendTask(bt);
            }
        }
        action
    }

    fn display_message(&mut self, msg: &str, msg_type: MessageType) {
        // Reset contract details loading on any error
        if msg_type == MessageType::Error && self.contract_details_loading {
            self.contract_details_loading = false;
        }

        match self.tokens_subscreen {
            TokensSubscreen::TokenCreator => {
                if msg.contains("Successfully registered token contract") {
                    self.token_creator_status = TokenCreatorStatus::Complete;
                } else if msg.contains("Failed to register token contract")
                    | msg.contains("Error building contract V1")
                {
                    self.token_creator_status = TokenCreatorStatus::ErrorMessage(msg.to_string());
                    self.token_creator_error_message = Some(msg.to_string());
                } else {
                    return;
                }
            }
            TokensSubscreen::MyTokens => {
                if msg.contains("Successfully fetched token balances")
                    || msg.contains("Failed to fetch token balances")
                    || msg.contains("Failed to get estimated rewards")
                    || msg.eq(NO_IDENTITIES_FOUND)
                {
                    self.backend_message = Some((msg.to_string(), msg_type, Utc::now()));
                    self.refreshing_status = RefreshingStatus::NotRefreshing;
                } else {
                    tracing::debug!(
                        ?msg,
                        ?msg_type,
                        "unsupported message received in token screen"
                    );
                }
            }
            TokensSubscreen::SearchTokens => {
                if msg.contains("Error fetching tokens") {
                    self.contract_search_status =
                        ContractSearchStatus::ErrorMessage(msg.to_string());
                    self.backend_message = Some((msg.to_string(), msg_type, Utc::now()));
                } else if msg.contains("Added token") | msg.contains("Token already added") {
                    self.backend_message = Some((msg.to_string(), msg_type, Utc::now()));
                } else {
                    return;
                }
            }
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::DescriptionsByKeyword(descriptions, next_cursor) => {
                let mut sr = self.search_results.lock().unwrap();
                *sr = descriptions;
                self.search_has_next_page = next_cursor.is_some();
                if let Some(cursor) = next_cursor {
                    self.next_cursors.push(cursor);
                }
                self.contract_search_status = ContractSearchStatus::Complete;
                self.refreshing_status = RefreshingStatus::NotRefreshing;
            }
            BackendTaskSuccessResult::ContractsWithDescriptions(contracts_with_descriptions) => {
                let default_info = (None, vec![]);
                let info = contracts_with_descriptions
                    .get(&self.selected_contract_id.unwrap())
                    .unwrap_or(&default_info);

                self.selected_contract_description = info.0.clone();
                self.selected_token_infos = info.1.clone();
                self.refreshing_status = RefreshingStatus::NotRefreshing;
                self.contract_details_loading = false;
            }
            BackendTaskSuccessResult::TokenEstimatedNonClaimedPerpetualDistributionAmount(
                identity_token_id,
                amount,
            ) => {
                self.refreshing_status = RefreshingStatus::NotRefreshing;
                if let Some(itb) = self.my_tokens.get_mut(&identity_token_id) {
                    itb.estimated_unclaimed_rewards = Some(amount)
                }
            }
            BackendTaskSuccessResult::TokenPricing { token_id, prices } => {
                // Store the pricing data
                self.token_pricing_data.insert(token_id, prices);
                // Clear loading state
                self.pricing_loading_state.insert(token_id, false);
                // Refresh my_tokens to update available actions with new pricing data
                self.my_tokens = my_tokens(&self.app_context, &self.identities, &self.all_known_tokens, &self.token_pricing_data);
                // Refresh display
                self.refreshing_status = RefreshingStatus::NotRefreshing;
            }
            _ => {}
        }
    }
}

impl ScreenWithWalletUnlock for TokensScreen {
    fn selected_wallet_ref(&self) -> &Option<Arc<RwLock<Wallet>>> {
        &self.selected_wallet
    }

    fn wallet_password_ref(&self) -> &String {
        &self.wallet_password
    }

    fn wallet_password_mut(&mut self) -> &mut String {
        &mut self.wallet_password
    }

    fn show_password(&self) -> bool {
        self.show_password
    }

    fn show_password_mut(&mut self) -> &mut bool {
        &mut self.show_password
    }

    fn set_error_message(&mut self, error_message: Option<String>) {
        self.token_creator_error_message = error_message;
    }

    fn error_message(&self) -> Option<&String> {
        self.token_creator_error_message.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::database::Database;
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;

    use super::*; use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration_localization::accessors::v0::TokenConfigurationLocalizationV0Getters;
    use dash_sdk::dpp::data_contract::associated_token::token_keeps_history_rules::TokenKeepsHistoryRules;
    use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
    use dash_sdk::dpp::data_contract::TokenConfiguration;
    use dash_sdk::dpp::identifier::Identifier;
    use dash_sdk::platform::{DataContract, Identity};

    impl ChangeControlRulesUI {
        /// Sets every field to some dummy/test value to ensure coverage in tests.
        pub fn set_all_fields_for_testing(&mut self) {
            self.rules.authorized_to_make_change =
                AuthorizedActionTakers::Identity(Identifier::default());
            self.authorized_identity =
                Some("ACMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9".to_owned());

            self.rules.admin_action_takers =
                AuthorizedActionTakers::Identity(Identifier::default());
            self.admin_identity = Some("CCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9".to_owned());

            self.rules
                .changing_authorized_action_takers_to_no_one_allowed = true;
            self.rules.changing_admin_action_takers_to_no_one_allowed = true;
            self.rules.self_changing_admin_action_takers_allowed = true;
        }
    }

    #[test]
    fn test_token_creator_ui_builds_correct_contract() {
        let db_file_path = "test_db";
        let db = Arc::new(Database::new(db_file_path).unwrap());
        db.initialize(Path::new(&db_file_path)).unwrap();

        let app_context =
            AppContext::new(Network::Regtest, db, None).expect("Expected to create AppContext");
        let mut token_creator_ui = TokensScreen::new(&app_context, TokensSubscreen::TokenCreator);

        // Identity selection
        let test_identity_id = Identifier::from_string(
            "BCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9",
            Encoding::Base58,
        )
        .unwrap();
        let mock =
            Identity::create_basic_identity(test_identity_id, app_context.platform_version())
                .expect("Expected to create Identity");
        let mock_identity = QualifiedIdentity {
            identity: mock,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: crate::model::qualified_identity::IdentityType::User,
            alias: None,
            private_keys: KeyStorage {
                private_keys: BTreeMap::new(),
            },
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            wallet_index: None,
            top_ups: BTreeMap::new(),
        };

        token_creator_ui.selected_identity = Some(mock_identity);

        // Key selection
        let mock_key = IdentityPublicKey::random_key(0, None, app_context.platform_version());
        token_creator_ui.selected_key = Some(mock_key);

        // Basic token info
        token_creator_ui.token_names_input = vec![(
            "AcmeCoin".to_string(),
            "AcmeCoins".to_string(),
            TokenNameLanguage::English,
            true,
        )];
        token_creator_ui.base_supply_input = "5000000".to_string();
        token_creator_ui.max_supply_input = "10000000".to_string();
        token_creator_ui.decimals_input = "8".to_string();
        token_creator_ui.start_as_paused_input = true;
        token_creator_ui.token_advanced_keeps_history =
            TokenKeepsHistoryRulesV0::default_for_keeping_all_history(true);
        token_creator_ui.should_capitalize_input = true;

        // Main control group
        token_creator_ui.main_control_group_input = "2".to_string();

        // Each action's rules
        token_creator_ui
            .manual_minting_rules
            .set_all_fields_for_testing();
        token_creator_ui
            .manual_burning_rules
            .set_all_fields_for_testing();
        token_creator_ui.freeze_rules.set_all_fields_for_testing();
        token_creator_ui.unfreeze_rules.set_all_fields_for_testing();
        token_creator_ui
            .destroy_frozen_funds_rules
            .set_all_fields_for_testing();
        token_creator_ui
            .emergency_action_rules
            .set_all_fields_for_testing();
        token_creator_ui
            .max_supply_change_rules
            .set_all_fields_for_testing();
        token_creator_ui
            .conventions_change_rules
            .set_all_fields_for_testing();

        // main_control_group_change_authorized
        token_creator_ui.authorized_main_control_group_change = AuthorizedActionTakers::Group(99);
        token_creator_ui.main_control_group_change_authorized_group = Some("99".to_string());

        // -------------------------------------------------
        // Distribution
        // -------------------------------------------------
        // Perpetual distribution
        token_creator_ui.enable_perpetual_distribution = true;
        token_creator_ui.perpetual_dist_type = PerpetualDistributionIntervalTypeUI::BlockBased;
        token_creator_ui.perpetual_dist_interval_input = "42".to_string();
        token_creator_ui.perpetual_dist_function = DistributionFunctionUI::FixedAmount;
        token_creator_ui.fixed_amount_input = "12345".to_string();
        token_creator_ui.perpetual_dist_recipient = TokenDistributionRecipientUI::Identity;
        token_creator_ui.perpetual_dist_recipient_identity_input =
            Some("DCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9".to_string());
        token_creator_ui
            .perpetual_distribution_rules
            .set_all_fields_for_testing();

        // new_tokens_destination_identity
        token_creator_ui.new_tokens_destination_other_identity_enabled = true;
        token_creator_ui.new_tokens_destination_other_identity =
            "GCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9".to_string();
        token_creator_ui
            .new_tokens_destination_identity_rules
            .set_all_fields_for_testing();

        // minting_allow_choosing_destination
        token_creator_ui.minting_allow_choosing_destination = true;
        token_creator_ui
            .minting_allow_choosing_destination_rules
            .set_all_fields_for_testing();

        // -------------------------------------------------
        // Groups
        // -------------------------------------------------
        // We'll define 2 groups for testing: positions 2 (main) and 7
        token_creator_ui.groups_ui = vec![
            GroupConfigUI {
                required_power_str: "2".to_string(),
                members: vec![
                    GroupMemberUI {
                        identity_str: "HCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9".to_string(),
                        power_str: "5".to_string(),
                    },
                    GroupMemberUI {
                        identity_str: "JCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9".to_string(),
                        power_str: "5".to_string(),
                    },
                ],
            },
            GroupConfigUI {
                required_power_str: "1".to_string(),
                members: vec![],
            },
        ];

        // -------------------------------------------------
        // 3) Parse arguments, then build the DataContract
        // -------------------------------------------------
        let build_args = token_creator_ui
            .parse_token_build_args()
            .expect("parse_token_build_args should succeed");
        let data_contract = app_context
            .build_data_contract_v1_with_one_token(
                build_args.identity_id,
                build_args.token_names,
                build_args.contract_keywords,
                build_args.token_description,
                build_args.should_capitalize,
                build_args.decimals,
                build_args.base_supply,
                build_args.max_supply,
                build_args.start_paused,
                build_args.allow_transfers_to_frozen_identities,
                build_args.keeps_history,
                build_args.main_control_group,
                build_args.manual_minting_rules,
                build_args.manual_burning_rules,
                build_args.freeze_rules,
                build_args.unfreeze_rules,
                build_args.destroy_frozen_funds_rules,
                build_args.emergency_action_rules,
                build_args.max_supply_change_rules,
                build_args.conventions_change_rules,
                build_args.main_control_group_change_authorized,
                build_args.distribution_rules,
                build_args.groups,
            )
            .expect("Contract build failed");

        // -------------------------------------------------
        // 4) Validate the result
        // -------------------------------------------------
        // Unwrap it to the V1
        let DataContract::V1(contract_v1) = data_contract else {
            panic!("Expected DataContract::V1");
        };

        // A) Check the top-level fields
        assert_eq!(contract_v1.version, 1);
        assert_eq!(
            contract_v1.tokens.len(),
            1,
            "We expected exactly one token config"
        );

        // B) Check the token config
        let (token_pos, token_config) = contract_v1.tokens.iter().next().unwrap();
        assert_eq!({ *token_pos }, 0, "Should be at position 0 by default");

        let TokenConfiguration::V0(token_v0) = token_config;
        let TokenConfigurationConvention::V0(conv_v0) = &token_v0.conventions;

        assert_eq!(conv_v0.decimals, 8, "Decimals from UI not matched");
        assert_eq!(
            conv_v0.localizations["en"].singular_form(),
            "AcmeCoin",
            "Token name did not match"
        );
        assert_eq!(
            conv_v0.localizations["en"].plural_form(),
            "AcmeCoins",
            "Plural form not automatically set in test"
        );
        let keeps_history_rules = &token_v0.keeps_history;
        let TokenKeepsHistoryRules::V0(keeps_history_v0) = keeps_history_rules;
        assert!(keeps_history_v0.keeps_transfer_history);
        assert!(keeps_history_v0.keeps_freezing_history);
        assert_eq!(token_v0.base_supply, 5_000_000);
        assert_eq!(token_v0.max_supply, Some(10_000_000));
        assert!(token_v0.start_as_paused);
        assert_eq!(
            token_v0.main_control_group,
            Some(2),
            "Parsed main control group mismatch"
        );

        // C) Check each ChangeControlRules field
        assert_eq!(
            *token_v0
                .manual_minting_rules
                .authorized_to_make_change_action_takers(),
            token_creator_ui
                .manual_minting_rules
                .rules
                .authorized_to_make_change
        );
        // ... etc.

        // D) Check main_control_group_can_be_modified
        match token_v0.main_control_group_can_be_modified {
            AuthorizedActionTakers::Group(group_id) => {
                assert_eq!(group_id, 99, "Expected group 99 from UI");
            }
            _ => panic!("Expected group(99) from the UI but got something else"),
        }

        // E) Check distribution rules
        let TokenDistributionRules::V0(dist_rules_v0) = &token_v0.distribution_rules;
        // -- Perpetual
        let Some(TokenPerpetualDistribution::V0(perp_v0)) = &dist_rules_v0.perpetual_distribution
        else {
            panic!("Expected Some(TokenPerpetualDistribution::V0)");
        };
        match &perp_v0.distribution_type {
            RewardDistributionType::BlockBasedDistribution { interval, function } => {
                assert_eq!(*interval, 42, "Interval mismatch");
                match function {
                    DistributionFunction::FixedAmount { amount } => {
                        assert_eq!(*amount, 12345, "Fixed amount mismatch");
                    }
                    _ => panic!("Expected DistributionFunction::FixedAmount"),
                }
            }
            _ => panic!("Expected a BlockBasedDistribution"),
        }
        match &perp_v0.distribution_recipient {
            TokenDistributionRecipient::Identity(rec_id) => {
                assert_eq!(
                    rec_id.to_string(Encoding::Base58),
                    "DCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9"
                );
            }
            _ => panic!("Expected distribution recipient Identity(...)"),
        }

        // -- New tokens destination
        let Some(new_dest_id) = &dist_rules_v0.new_tokens_destination_identity else {
            panic!("Expected new_tokens_destination_identity to be Some(...)");
        };
        assert_eq!(
            new_dest_id.to_string(Encoding::Base58),
            "GCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9"
        );
        assert!(dist_rules_v0.minting_allow_choosing_destination);

        // F) Check the Groups
        //    (Positions 2 and 7, from above)
        assert_eq!(contract_v1.groups.len(), 2, "We added two groups in the UI");
        let group2 = contract_v1.groups.get(&2).expect("Expected group pos=2");
        assert_eq!(
            group2.required_power(),
            2,
            "Group #2 required_power mismatch"
        );
        let members = &group2.members();
        assert_eq!(members.len(), 2);

        let group7 = contract_v1.groups.get(&7).expect("Expected group pos=7");
        assert_eq!(group7.required_power(), 1);
        assert_eq!(group7.members().len(), 0);
    }

    #[test]
    fn test_distribution_function_random() {
        let db_file_path = "test_db";
        let db = Arc::new(Database::new(db_file_path).unwrap());
        db.initialize(Path::new(&db_file_path)).unwrap();

        let app_context =
            AppContext::new(Network::Regtest, db, None).expect("Expected to create AppContext");
        let mut token_creator_ui = TokensScreen::new(&app_context, TokensSubscreen::TokenCreator);

        // Identity selection
        let test_identity_id = Identifier::from_string(
            "BCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9",
            Encoding::Base58,
        )
        .unwrap();
        let mock =
            Identity::create_basic_identity(test_identity_id, app_context.platform_version())
                .expect("Expected to create Identity");
        let mock_identity = QualifiedIdentity {
            identity: mock,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: crate::model::qualified_identity::IdentityType::User,
            alias: None,
            private_keys: KeyStorage {
                private_keys: BTreeMap::new(),
            },
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            wallet_index: None,
            top_ups: BTreeMap::new(),
        };

        token_creator_ui.selected_identity = Some(mock_identity);

        // Key selection
        let mock_key = IdentityPublicKey::random_key(0, None, app_context.platform_version());
        token_creator_ui.selected_key = Some(mock_key);

        token_creator_ui.token_names_input = vec![(
            "TestToken".to_owned(),
            "TestToken".to_owned(),
            TokenNameLanguage::English,
            true,
        )];

        // Enable perpetual distribution, select Random
        token_creator_ui.enable_perpetual_distribution = true;
        token_creator_ui.perpetual_dist_type = PerpetualDistributionIntervalTypeUI::TimeBased;
        token_creator_ui.perpetual_dist_interval_input = "60000".to_string();
        token_creator_ui.random_min_input = "100".to_string();
        token_creator_ui.random_max_input = "200".to_string();

        // Parse + build
        let build_args = token_creator_ui
            .parse_token_build_args()
            .expect("Should parse");
        let data_contract = app_context
            .build_data_contract_v1_with_one_token(
                build_args.identity_id,
                build_args.token_names,
                build_args.contract_keywords,
                build_args.token_description,
                build_args.should_capitalize,
                build_args.decimals,
                build_args.base_supply,
                build_args.max_supply,
                build_args.start_paused,
                build_args.allow_transfers_to_frozen_identities,
                build_args.keeps_history,
                build_args.main_control_group,
                build_args.manual_minting_rules,
                build_args.manual_burning_rules,
                build_args.freeze_rules,
                build_args.unfreeze_rules,
                build_args.destroy_frozen_funds_rules,
                build_args.emergency_action_rules,
                build_args.max_supply_change_rules,
                build_args.conventions_change_rules,
                build_args.main_control_group_change_authorized,
                build_args.distribution_rules,
                build_args.groups,
            )
            .expect("Should build successfully");
        let contract_v1 = data_contract.as_v1().expect("Expected DataContract::V1");

        let TokenConfiguration::V0(ref token_v0) = contract_v1.tokens[&0u16];
        let TokenDistributionRules::V0(dist_rules_v0) = &token_v0.distribution_rules;
        let Some(TokenPerpetualDistribution::V0(perp_v0)) = &dist_rules_v0.perpetual_distribution
        else {
            panic!("Expected a perpetual distribution");
        };

        match &perp_v0.distribution_type {
            RewardDistributionType::TimeBasedDistribution { interval, function } => {
                assert_eq!(*interval, 60000, "Expected 60s (in ms)");
                match function {
                    DistributionFunction::Random { min, max } => {
                        assert_eq!(*min, 100);
                        assert_eq!(*max, 200);
                    }
                    _ => panic!("Expected DistributionFunction::Random"),
                }
            }
            _ => panic!("Expected TimeBasedDistribution"),
        }
    }

    #[test]
    fn test_parse_token_build_args_fails_with_empty_token_name() {
        let db_file_path = "test_db";
        let db = Arc::new(Database::new(db_file_path).unwrap());
        db.initialize(Path::new(&db_file_path)).unwrap();

        let app_context =
            AppContext::new(Network::Regtest, db, None).expect("Expected to create AppContext");
        let mut token_creator_ui = TokensScreen::new(&app_context, TokensSubscreen::TokenCreator);

        // Identity selection
        let test_identity_id = Identifier::from_string(
            "BCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9",
            Encoding::Base58,
        )
        .unwrap();
        let mock =
            Identity::create_basic_identity(test_identity_id, app_context.platform_version())
                .expect("Expected to create Identity");
        let mock_identity = QualifiedIdentity {
            identity: mock,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: crate::model::qualified_identity::IdentityType::User,
            alias: None,
            private_keys: KeyStorage {
                private_keys: BTreeMap::new(),
            },
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            wallet_index: None,
            top_ups: BTreeMap::new(),
        };

        token_creator_ui.selected_identity = Some(mock_identity);

        // Key selection
        let mock_key = IdentityPublicKey::random_key(0, None, app_context.platform_version());
        token_creator_ui.selected_key = Some(mock_key);

        // Intentionally leave token_name_input empty
        token_creator_ui.token_names_input = vec![];

        let err = token_creator_ui
            .parse_token_build_args()
            .expect_err("Should fail if token name is empty");
        assert_eq!(err, "Please enter a token name");
    }
}
