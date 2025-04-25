use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Duration, Utc};
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::dashcore::Network::Devnet;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::{TokenConfigurationPreset, TokenConfigurationPresetFeatures, TokenConfigurationV0};
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPresetFeatures::{MostRestrictive, WithAllAdvancedActions, WithExtremeActions, WithMintingAndBurningActions, WithOnlyEmergencyAction};
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::v0::TokenDistributionRulesV0;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
use dash_sdk::dpp::data_contract::associated_token::token_keeps_history_rules::accessors::v0::{TokenKeepsHistoryRulesV0Getters, TokenKeepsHistoryRulesV0Setters};
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
use dash_sdk::dpp::data_contract::conversion::json::DataContractJsonConversionMethodsV0;
use dash_sdk::dpp::data_contract::group::v0::GroupV0;
use dash_sdk::dpp::data_contract::group::{Group, GroupMemberPower, GroupRequiredPower};
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::SecurityLevel;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use dash_sdk::query_types::IndexMap;
use eframe::egui::{self, CentralPanel, Color32, Context, Frame, Margin, Ui};
use egui::{Align, Checkbox, ColorImage, Label, Response, RichText, Sense, TextureHandle};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use egui_extras::{Column, TableBuilder};
use image::ImageReader;
use crate::app::BackendTasksExecutionMode;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::BackendTask;

use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock::ScreenWithWalletUnlock;
use crate::ui::{
    BackendTaskSuccessResult, MessageType, RootScreenType, Screen, ScreenLike, ScreenType,
};

use super::burn_tokens_screen::BurnTokensScreen;
use super::claim_tokens_screen::ClaimTokensScreen;
use super::destroy_frozen_funds_screen::DestroyFrozenFundsScreen;
use super::freeze_tokens_screen::FreezeTokensScreen;
use super::mint_tokens_screen::MintTokensScreen;
use super::pause_tokens_screen::PauseTokensScreen;
use super::resume_tokens_screen::ResumeTokensScreen;
use super::transfer_tokens_screen::TransferTokensScreen;
use super::unfreeze_tokens_screen::UnfreezeTokensScreen;
use super::view_token_claims_screen::ViewTokenClaimsScreen;

const EXP_FORMULA_PNG: &[u8] = include_bytes!("../../../assets/exp_function.png");
const INV_LOG_FORMULA_PNG: &[u8] = include_bytes!("../../../assets/inv_log_function.png");
const LOG_FORMULA_PNG: &[u8] = include_bytes!("../../../assets/log_function.png");
const LINEAR_FORMULA_PNG: &[u8] = include_bytes!("../../../assets/linear_function.png");
const POLYNOMIAL_FORMULA_PNG: &[u8] = include_bytes!("../../../assets/polynomial_function.png");

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

/// Token info
#[derive(Clone, Debug, PartialEq)]
pub struct TokenInfo {
    pub token_id: Identifier,
    pub token_name: String,
    pub data_contract_id: Identifier,
    pub token_position: u16,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContractDescriptionInfo {
    pub data_contract_id: Identifier,
    pub description: String,
}

/* helper: flip all flags on/off */
trait SetAll {
    fn set_all(&mut self, value: bool);
}
impl SetAll for TokenKeepsHistoryRulesV0 {
    fn set_all(&mut self, value: bool) {
        self.set_keeps_transfer_history(value);
        self.set_keeps_freezing_history(value);
        self.set_keeps_minting_history(value);
        self.set_keeps_burning_history(value);
        self.set_keeps_direct_pricing_history(value);
        self.set_keeps_direct_purchase_history(value);
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct IdentityTokenIdentifier {
    pub identity_id: Identifier,
    pub token_id: Identifier,
}

impl From<IdentityTokenBalance> for IdentityTokenIdentifier {
    fn from(value: IdentityTokenBalance) -> Self {
        let IdentityTokenBalance {
            token_id,
            identity_id,
            ..
        } = value;

        IdentityTokenIdentifier {
            identity_id,
            token_id,
        }
    }
}

impl From<IdentityTokenMaybeBalance> for IdentityTokenIdentifier {
    fn from(value: IdentityTokenMaybeBalance) -> Self {
        let IdentityTokenMaybeBalance {
            token_id,
            identity_id,
            ..
        } = value;

        IdentityTokenIdentifier {
            identity_id,
            token_id,
        }
    }
}

/// A token owned by an identity.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityTokenMaybeBalance {
    pub token_id: Identifier,
    pub token_name: String,
    pub identity_id: Identifier,
    pub identity_alias: Option<String>,
    pub balance: Option<IdentityTokenBalance>,
}

/// A token owned by an identity.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityTokenBalance {
    pub token_id: Identifier,
    pub token_name: String,
    pub identity_id: Identifier,
    pub balance: TokenAmount,
    pub estimated_unclaimed_rewards: Option<TokenAmount>,
    pub data_contract_id: Identifier,
    pub token_position: u16,
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
    TokenName,
    TokenID,
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
    pub authorized_group: Option<String>,
    pub admin_identity: Option<String>,
    pub admin_group: Option<String>,
}

impl From<ChangeControlRulesV0> for ChangeControlRulesUI {
    fn from(rules: ChangeControlRulesV0) -> Self {
        ChangeControlRulesUI {
            rules,
            authorized_identity: None,
            authorized_group: None,
            admin_identity: None,
            admin_group: None,
        }
    }
}

impl ChangeControlRulesUI {
    /// Renders the UI for a single action’s configuration (mint, burn, freeze, etc.)
    pub fn render_control_change_rules_ui(&mut self, ui: &mut Ui, action_name: &str) {
        ui.collapsing(action_name, |ui| {
            ui.add_space(3.0);

            egui::Grid::new("basic_token_info_grid")
                .num_columns(2)
                .spacing([16.0, 8.0]) // Horizontal, vertical spacing
                .show(ui, |ui| {
                    // Authorized action takers
                    ui.horizontal(|ui| {
                        ui.label("Authorized to perform action:");
                        egui::ComboBox::from_id_salt(format!("Authorized {}", action_name))
                            .selected_text(self.rules.authorized_to_make_change.to_string())
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
                                ui.selectable_value(
                                    &mut self.rules.authorized_to_make_change,
                                    AuthorizedActionTakers::MainGroup,
                                    "Main Group",
                                );
                                ui.selectable_value(
                                    &mut self.rules.authorized_to_make_change,
                                    AuthorizedActionTakers::Group(0),
                                    "Group",
                                );
                            });

                        // If user selected Identity or Group, show text edit
                        match &mut self.rules.authorized_to_make_change {
                            AuthorizedActionTakers::Identity(_) => {
                                self.authorized_identity.get_or_insert_with(String::new);
                                if let Some(ref mut id) = self.authorized_identity {
                                    ui.add(
                                        egui::TextEdit::singleline(id).hint_text("Enter base58 id"),
                                    );
                                }
                            }
                            AuthorizedActionTakers::Group(_) => {
                                self.authorized_group.get_or_insert_with(|| "0".to_owned());
                                if let Some(ref mut group_str) = self.authorized_group {
                                    ui.add(
                                        egui::TextEdit::singleline(group_str)
                                            .hint_text("Group contract position"),
                                    );
                                }
                            }
                            _ => {}
                        }
                    });
                    ui.end_row();

                    // Admin action takers
                    ui.horizontal(|ui| {
                        ui.label("Authorized to change rules:");
                        egui::ComboBox::from_id_salt(format!("Admin {}", action_name))
                            .selected_text(self.rules.admin_action_takers.to_string())
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
                                ui.selectable_value(
                                    &mut self.rules.admin_action_takers,
                                    AuthorizedActionTakers::MainGroup,
                                    "Main Group",
                                );
                                ui.selectable_value(
                                    &mut self.rules.admin_action_takers,
                                    AuthorizedActionTakers::Group(0),
                                    "Group",
                                );
                            });

                        match &mut self.rules.admin_action_takers {
                            AuthorizedActionTakers::Identity(_) => {
                                self.admin_identity.get_or_insert_with(String::new);
                                if let Some(ref mut id) = self.admin_identity {
                                    ui.add(
                                        egui::TextEdit::singleline(id).hint_text("Enter base58 id"),
                                    );
                                }
                            }
                            AuthorizedActionTakers::Group(_) => {
                                self.admin_group.get_or_insert_with(|| "0".to_owned());
                                if let Some(ref mut group_str) = self.admin_group {
                                    ui.add(
                                        egui::TextEdit::singleline(group_str)
                                            .hint_text("Group contract position"),
                                    );
                                }
                            }
                            _ => {}
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
                });

            ui.add_space(3.0);
        });
    }

    pub fn extract_change_control_rules(
        &mut self,
        action_name: &str,
    ) -> Result<ChangeControlRules, String> {
        // 1) Update self.rules.authorized_to_make_change if it’s Identity or Group
        match self.rules.authorized_to_make_change {
            AuthorizedActionTakers::Identity(_) => {
                if let Some(ref id_str) = self.authorized_identity {
                    let parsed =
                        Identifier::from_string(id_str, Encoding::Base58).map_err(|_| {
                            format!(
                                "Invalid base58 identifier for {} authorized identity",
                                action_name
                            )
                        })?;
                    self.rules.authorized_to_make_change = AuthorizedActionTakers::Identity(parsed);
                }
            }
            AuthorizedActionTakers::Group(_) => {
                if let Some(ref group_str) = self.authorized_group {
                    let parsed = group_str.parse::<u16>().map_err(|_| {
                        format!(
                            "Invalid group contract position for {} authorized group",
                            action_name
                        )
                    })?;
                    self.rules.authorized_to_make_change = AuthorizedActionTakers::Group(parsed);
                }
            }
            _ => {}
        }

        // 2) Update self.rules.admin_action_takersif it’s Identity or Group
        match self.rules.admin_action_takers {
            AuthorizedActionTakers::Identity(_) => {
                if let Some(ref id_str) = self.admin_identity {
                    let parsed =
                        Identifier::from_string(id_str, Encoding::Base58).map_err(|_| {
                            format!(
                                "Invalid base58 identifier for {} admin identity",
                                action_name
                            )
                        })?;
                    self.rules.admin_action_takers = AuthorizedActionTakers::Identity(parsed);
                }
            }
            AuthorizedActionTakers::Group(_) => {
                if let Some(ref group_str) = self.admin_group {
                    let parsed = group_str.parse::<u16>().map_err(|_| {
                        format!(
                            "Invalid group contract position for {} admin group",
                            action_name
                        )
                    })?;
                    self.rules.admin_action_takers = AuthorizedActionTakers::Group(parsed);
                }
            }
            _ => {}
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
    Random,
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
            DistributionFunctionUI::Random => "random",
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TokenNameLanguage {
    English,
    French,
    Spanish,
    Portuguese,
    German,
    Polish,
    Russian,
    Mandarin,
    Japanese,
    Vietnamese,
    Korean,
    Javanese,
    Malay,
    Telugu,
    Arabic,
    Bengali,
    Punjabi,
    Hindi,
}

impl std::fmt::Display for TokenNameLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Default, Clone)]
pub struct GroupMemberUI {
    /// The base58 identity for this member
    pub identity_str: String,
    /// The power (u32) as a string for user input
    pub power_str: String,
}

#[derive(Default, Clone)]
pub struct GroupConfigUI {
    /// The "position" in the contract's groups map (like 0, 1, etc.)
    pub group_position_str: String,
    /// Required power for the group (u32), user enters as string
    pub required_power_str: String,
    /// The members for this group
    pub members: Vec<GroupMemberUI>,
}

impl GroupConfigUI {
    /// Try converting this UI struct into a real `Group` (specifically `Group::V0`).
    /// We also return the `u16` key that this group should be inserted under in the contract’s `groups` map.
    fn parse_into_group(&self) -> Result<(u16, Group), String> {
        // 1) Parse group position
        let group_position = self.group_position_str.parse::<u16>().map_err(|_| {
            format!(
                "Invalid group position: '{}'. Must be an unsigned integer.",
                self.group_position_str
            )
        })?;

        // 2) Parse required power
        let required_power = self.required_power_str.parse::<u32>().map_err(|_| {
            format!(
                "Invalid required power: '{}'. Must be an unsigned integer.",
                self.required_power_str
            )
        })? as GroupRequiredPower;

        // 3) Build a BTreeMap<Identifier, u32> for members
        let mut members_map = BTreeMap::new();
        for (i, member) in self.members.iter().enumerate() {
            // A) Parse member identity from base58
            let id =
                Identifier::from_string(&member.identity_str, Encoding::Base58).map_err(|_| {
                    format!(
                        "Member #{}: invalid base58 identity '{}'",
                        i + 1,
                        member.identity_str
                    )
                })?;

            // B) Parse power
            let power =
                member.power_str.parse::<u32>().map_err(|_| {
                    format!("Member #{}: invalid power '{}'", i + 1, member.power_str)
                })? as GroupMemberPower;

            // Insert into the map
            members_map.insert(id, power);
        }

        // 4) Construct Group::V0
        let group_v0 = GroupV0 {
            members: members_map,
            required_power,
        };

        // 5) Return as (group_position, Group::V0 wrapped in Group::V0())
        Ok((group_position, Group::V0(group_v0)))
    }
}

#[derive(Clone, Debug)]
/// All arguments needed by `build_data_contract_v1_with_one_token`.
pub struct TokenBuildArgs {
    pub identity_id: Identifier,

    pub token_names: Vec<(String, String)>,
    pub contract_keywords: Vec<String>,
    pub token_description: Option<String>,
    pub should_capitalize: bool,
    pub decimals: u16,
    pub base_supply: u64,
    pub max_supply: Option<u64>,
    pub start_paused: bool,
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

/// The main, combined TokensScreen:
/// - Displays token balances or a search UI
/// - Allows reordering of tokens if desired
pub struct TokensScreen {
    pub app_context: Arc<AppContext>,
    pub tokens_subscreen: TokensSubscreen,
    all_known_tokens: IndexMap<Identifier, TokenInfo>,
    identities: IndexMap<Identifier, QualifiedIdentity>,
    my_tokens: IndexMap<IdentityTokenIdentifier, IdentityTokenBalance>,
    pub selected_token_id: Option<Identifier>,
    show_token_info: Option<Identifier>,
    backend_message: Option<(String, MessageType, DateTime<Utc>)>,
    pending_backend_task: Option<BackendTask>,
    refreshing_status: RefreshingStatus,

    // Contract Search
    pub selected_contract_id: Option<Identifier>,
    selected_contract_description: Option<ContractDescriptionInfo>,
    selected_token_infos: Vec<TokenInfo>,
    search_results: Arc<Mutex<Vec<ContractDescriptionInfo>>>,
    contract_search_status: ContractSearchStatus,

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
    identity_token_balance_to_remove: Option<IdentityTokenBalance>,
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
    token_names_input: Vec<(String, TokenNameLanguage)>,
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
    pub perpetual_dist_function: DistributionFunctionUI,
    pub perpetual_dist_recipient: TokenDistributionRecipientUI,
    pub perpetual_dist_recipient_identity_input: Option<String>,

    // Pre-programmed distribution
    pub enable_pre_programmed_distribution: bool,
    pub pre_programmed_distributions: Vec<DistributionEntry>,

    // New Tokens Destination Identity
    pub new_tokens_destination_identity_enabled: bool,
    pub new_tokens_destination_identity: String,
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
            .get_all_known_tokens(&app_context)
            .unwrap_or_default();

        let my_tokens = app_context.identity_token_balances().unwrap_or_default();

        if app_context.network == Devnet {
            println!("my tokens {}", my_tokens.len());
        }

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
            selected_token_id: None,
            selected_contract_id: None,
            selected_contract_description: None,
            selected_token_infos: Vec::new(),
            show_token_info: None,
            token_search_query: None,
            contract_search_status: ContractSearchStatus::NotStarted,
            search_current_page: 1,
            search_has_next_page: false,
            next_cursors: vec![],
            previous_cursors: vec![],
            search_results: Arc::new(Mutex::new(Vec::new())),
            backend_message: None,
            sort_column: SortColumn::TokenName,
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
            token_names_input: vec![(String::new(), TokenNameLanguage::English)],
            contract_keywords_input: String::new(),
            token_description_input: String::new(),
            should_capitalize_input: false,
            decimals_input: 8.to_string(),
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
            new_tokens_destination_identity_enabled: false,
            new_tokens_destination_identity: String::new(),
            new_tokens_destination_identity_rules: ChangeControlRulesUI::default(),

            // minting_allow_choosing_destination
            minting_allow_choosing_destination: false,
            minting_allow_choosing_destination_rules: ChangeControlRulesUI::default(),
            function_images,
            function_textures: BTreeMap::default(),
        };

        if let Ok(saved_ids) = screen.app_context.db.load_token_order() {
            screen.reorder_vec_to(saved_ids);
            screen.use_custom_order = true;
        }

        screen
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
            .map(|(_, token)| (token.token_id.clone(), token.identity_id.clone()))
            .collect::<Vec<_>>();

        self.app_context
            .db
            .save_token_order(all_ids)
            .or_else(|e| {
                eprintln!("Error saving token order: {}", e);
                Err(e)
            })
            .ok();
    }

    // ─────────────────────────────────────────────────────────────────
    // Sorting
    // ─────────────────────────────────────────────────────────────────

    /// Sort the vector by the user-specified column/order, overriding any custom order.
    fn sort_vec(&self, list: &mut [IdentityTokenBalance]) {
        list.sort_by(|a, b| {
            let ordering = match self.sort_column {
                SortColumn::Balance => a.balance.cmp(&b.balance),
                SortColumn::OwnerIdentity => a.identity_id.cmp(&b.identity_id),
                SortColumn::OwnerIdentityAlias => {
                    let alias_a = self
                        .app_context
                        .get_alias(&a.identity_id)
                        .expect("Expected to get alias")
                        .unwrap_or("".to_string());
                    let alias_b = self
                        .app_context
                        .get_alias(&b.identity_id)
                        .expect("Expected to get alias")
                        .unwrap_or("".to_string());
                    alias_a.cmp(&alias_b)
                }
                SortColumn::TokenName => a.token_name.cmp(&b.token_name),
                SortColumn::TokenID => a.token_id.cmp(&b.token_id),
            };
            match self.sort_order {
                SortOrder::Ascending => ordering,
                SortOrder::Descending => ordering.reverse(),
            }
        });
        self.save_current_order();
    }

    fn sort_vec_of_groups(&self, list: &mut [(Identifier, String, u64)]) {
        list.sort_by(|a, b| {
            let ordering = match self.sort_column {
                SortColumn::Balance => a.2.cmp(&b.2),
                SortColumn::TokenName => a.1.cmp(&b.1),
                SortColumn::TokenID => a.0.cmp(&b.0),
                _ => a.0.cmp(&b.0),
            };
            match self.sort_order {
                SortOrder::Ascending => ordering,
                SortOrder::Descending => ordering.reverse(),
            }
        });
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

    /// Group all IdentityTokenBalance objects by token_identifier.
    /// Returns a Vec of (token_identifier, token_name, total_balance).
    fn group_tokens_by_identifier(
        &self,
        tokens: &IndexMap<IdentityTokenIdentifier, IdentityTokenBalance>,
    ) -> Vec<(Identifier, String, u64)> {
        let mut map: HashMap<Identifier, (String, u64)> = HashMap::new();
        for tb in tokens.values() {
            let entry = map.entry(tb.token_id.clone()).or_insert_with(|| {
                // Store (token_name, running_total_balance)
                (tb.token_name.clone(), 0u64)
            });
            entry.1 += tb.balance;
        }

        // Convert to a vec for display
        let mut result = Vec::new();
        for (identifier, (name, total_balance)) in map {
            result.push((identifier, name, total_balance));
        }
        // Sort by token name, for example
        result.sort_by(|a, b| a.1.cmp(&b.1));
        result
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

    // ─────────────────────────────────────────────────────────────────
    // Rendering
    // ─────────────────────────────────────────────────────────────────

    /// Renders the top-level token list (one row per unique token).
    /// When the user clicks on a token, we set `selected_token_id`.
    fn render_token_list(&mut self, ui: &mut Ui) {
        // Allocate space for refreshing indicator
        let refreshing_height = 33.0;
        let mut max_scroll_height = if let RefreshingStatus::Refreshing(_) = self.refreshing_status
        {
            ui.available_height() - refreshing_height
        } else {
            ui.available_height()
        };

        // Allocate space for backend message
        let backend_message_height = 40.0;
        if let Some((_, _, _)) = self.backend_message.clone() {
            max_scroll_height -= backend_message_height;
        }

        // A simple table with columns: [Token Name | Token ID | Total Balance]
        egui::ScrollArea::vertical()
            .max_height(max_scroll_height)
            .show(ui, |ui| {
                Frame::group(ui.style())
                    .fill(ui.visuals().panel_fill)
                    .stroke(egui::Stroke::new(
                        1.0,
                        ui.visuals().widgets.inactive.bg_stroke.color,
                    ))
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(Align::Center))
                            .column(Column::initial(150.0).resizable(true)) // Token Name
                            .column(Column::initial(200.0).resizable(true)) // Token ID
                            .column(Column::initial(80.0).resizable(true)) // Description
                            .column(Column::initial(80.0).resizable(true)) // Actions
                            // .column(Column::initial(80.0).resizable(true)) // Token Info
                            .header(30.0, |mut header| {
                                header.col(|ui| {
                                    ui.label("Token Name");
                                });
                                header.col(|ui| {
                                    ui.label("Token ID");
                                });
                                header.col(|ui| {
                                    ui.label("Description");
                                });
                                header.col(|ui| {
                                    ui.label("Actions");
                                });
                            })
                            .body(|mut body| {
                                for TokenInfo {
                                    token_id,
                                    token_name,
                                    description,
                                    ..
                                } in self.all_known_tokens.values()
                                {
                                    body.row(25.0, |mut row| {
                                        row.col(|ui| {
                                            // By making the label into a button or using `ui.selectable_label`,
                                            // we can respond to clicks.
                                            if ui.button(token_name).clicked() {
                                                self.selected_token_id = Some(token_id.clone());
                                            }
                                        });
                                        row.col(|ui| {
                                            ui.label(token_id.to_string(Encoding::Base58));
                                        });
                                        row.col(|ui| {
                                            ui.label(
                                                description.as_ref().unwrap_or(&String::new()),
                                            );
                                        });
                                        row.col(|ui| {
                                            // Remove
                                            if ui
                                                .button("X")
                                                .on_hover_text("Remove token from DET")
                                                .clicked()
                                            {
                                                self.confirm_remove_token_popup = true;
                                                self.token_to_remove = Some(token_id.clone());
                                            }
                                        });
                                    });
                                }
                            });
                    });
            });
    }

    /// Renders details for the selected token_id: a row per identity that holds that token.
    fn render_token_details(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        let Some(token_id) = self.selected_token_id.as_ref() else {
            return action;
        };

        let Some(token_info) = self.all_known_tokens.get(token_id) else {
            return action;
        };

        let identities = &self.identities;

        let mut detail_list: Vec<IdentityTokenMaybeBalance> = vec![];

        for (identity_id, identity) in identities {
            let record = if let Some(known_token_balance) =
                self.my_tokens.get(&IdentityTokenIdentifier {
                    identity_id: *identity_id,
                    token_id: *token_id,
                }) {
                IdentityTokenMaybeBalance {
                    token_id: *token_id,
                    token_name: token_info.token_name.clone(),
                    identity_id: *identity_id,
                    identity_alias: identity.alias.clone(),
                    balance: Some(known_token_balance.clone()),
                }
            } else {
                IdentityTokenMaybeBalance {
                    token_id: *token_id,
                    token_name: token_info.token_name.clone(),
                    identity_id: *identity_id,
                    identity_alias: identity.alias.clone(),
                    balance: None,
                }
            };
            detail_list.push(record);
        }

        // if !self.use_custom_order {
        //     self.sort_vec(&mut detail_list);
        // }

        // This is basically your old `render_table_my_token_balances` logic, but
        // limited to just the single token.
        // Allocate space for refreshing indicator
        let refreshing_height = 33.0;
        let mut max_scroll_height = if let RefreshingStatus::Refreshing(_) = self.refreshing_status
        {
            ui.available_height() - refreshing_height
        } else {
            ui.available_height()
        };

        // Allocate space for backend message
        let backend_message_height = 40.0;
        if let Some((_, _, _)) = self.backend_message.clone() {
            max_scroll_height -= backend_message_height;
        }

        // A simple table with columns: [Token Name | Token ID | Total Balance]
        egui::ScrollArea::vertical()
            .max_height(max_scroll_height)
            .show(ui, |ui| {
                Frame::group(ui.style())
                    .fill(ui.visuals().panel_fill)
                    .stroke(egui::Stroke::new(
                        1.0,
                        ui.visuals().widgets.inactive.bg_stroke.color,
                    ))
                    .inner_margin(Margin::same(8))
                    .show(ui, |ui| {
                        TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(Align::Center))
                            .column(Column::initial(60.0).resizable(true)) // Identity Alias
                            .column(Column::initial(200.0).resizable(true)) // Identity ID
                            .column(Column::initial(60.0).resizable(true)) // Balance
                            .column(Column::initial(60.0).resizable(true)) // Estimated Rewards
                            .column(Column::initial(200.0).resizable(true)) // Actions
                            .header(30.0, |mut header| {
                                header.col(|ui| {
                                    if ui.button("Identity Alias").clicked() {
                                        self.toggle_sort(SortColumn::OwnerIdentityAlias);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Identity ID").clicked() {
                                        self.toggle_sort(SortColumn::OwnerIdentity);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Balance").clicked() {
                                        self.toggle_sort(SortColumn::Balance);
                                    }
                                });
                                header.col(|ui| {
                                    ui.label("Estimated Unclaimed Rewards");
                                });
                                header.col(|ui| {
                                    ui.label("Actions");
                                });
                            })
                            .body(|mut body| {
                                for itb in &detail_list {
                                    body.row(25.0, |mut row| {
                                        row.col(|ui| {
                                            // Show identity alias or ID
                                            if let Some(alias) = self
                                                .app_context
                                                .get_alias(&itb.identity_id)
                                                .expect("Expected to get alias")
                                            {
                                                ui.label(alias);
                                            } else {
                                                ui.label("-");
                                            }
                                        });
                                        row.col(|ui| {
                                            ui.label(itb.identity_id.to_string(Encoding::Base58));
                                        });
                                        row.col(|ui| {
                                            if let Some(balance) = itb.balance.as_ref().map(|balance| balance.balance.to_string()) {
                                                ui.label(balance);
                                            } else {
                                                if ui.button("Check").clicked() {
                                                    action = AppAction::BackendTask(BackendTask::TokenTask(TokenTask::QueryIdentityTokenBalance(itb.clone().into())));
                                                }
                                            }
                                        });
                                        row.col(|ui| {
                                            if let Some(known_rewards) = itb.balance.as_ref().map(|itb| itb.estimated_unclaimed_rewards) {
                                                if let Some(known_rewards) = known_rewards {
                                                    ui.horizontal(|ui| {
                                                        ui.label(known_rewards.to_string());
                                                        if ui.button("Estimate").clicked() {
                                                            action = AppAction::BackendTask(BackendTask::TokenTask(TokenTask::EstimatePerpetualTokenRewards {
                                                                identity_id: itb.identity_id,
                                                                token_id: itb.token_id,
                                                            }));
                                                            self.refreshing_status = RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
                                                        }
                                                    });
                                                } else {
                                                    if ui.button("Estimate").clicked() {
                                                        action = AppAction::BackendTask(BackendTask::TokenTask(TokenTask::EstimatePerpetualTokenRewards {
                                                            identity_id: itb.identity_id,
                                                            token_id: itb.token_id,
                                                        }));
                                                        self.refreshing_status = RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
                                                    }
                                                }
                                            }
                                        });
                                        row.col(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = 3.0;

                                                if let Some(itb) = itb.balance.as_ref() {
                                                    // Transfer
                                                    if ui.button("Transfer").clicked() {
                                                        action = AppAction::AddScreen(
                                                            Screen::TransferTokensScreen(
                                                                TransferTokensScreen::new(
                                                                    itb.clone(),
                                                                    &self.app_context,
                                                                ),
                                                            ),
                                                        );
                                                    }

                                                    // Claim
                                                    if ui.button("Claim").clicked() {
                                                        action = AppAction::AddScreen(
                                                            Screen::ClaimTokensScreen(
                                                                ClaimTokensScreen::new(
                                                                    itb.clone(),
                                                                    &self.app_context,
                                                                ),
                                                            ),
                                                        );
                                                        ui.close_menu();
                                                    }

                                                    // Expandable advanced actions menu
                                                    ui.menu_button("...", |ui| {
                                                        if ui.button("Mint").clicked() {
                                                            action = AppAction::AddScreen(
                                                                Screen::MintTokensScreen(
                                                                    MintTokensScreen::new(
                                                                        itb.clone(),
                                                                        &self.app_context,
                                                                    ),
                                                                ),
                                                            );
                                                            ui.close_menu();
                                                        }
                                                        if ui.button("Burn").clicked() {
                                                            action = AppAction::AddScreen(
                                                                Screen::BurnTokensScreen(
                                                                    BurnTokensScreen::new(
                                                                        itb.clone(),
                                                                        &self.app_context,
                                                                    ),
                                                                ),
                                                            );
                                                            ui.close_menu();
                                                        }
                                                        if ui.button("Freeze").clicked() {
                                                            action = AppAction::AddScreen(
                                                                Screen::FreezeTokensScreen(
                                                                    FreezeTokensScreen::new(
                                                                        itb.clone(),
                                                                        &self.app_context,
                                                                    ),
                                                                ),
                                                            );
                                                            ui.close_menu();
                                                        }
                                                        if ui.button("Destroy").clicked() {
                                                            action = AppAction::AddScreen(
                                                                Screen::DestroyFrozenFundsScreen(
                                                                    DestroyFrozenFundsScreen::new(
                                                                        itb.clone(),
                                                                        &self.app_context,
                                                                    ),
                                                                ),
                                                            );
                                                            ui.close_menu();
                                                        }
                                                        if ui.button("Unfreeze").clicked() {
                                                            action = AppAction::AddScreen(
                                                                Screen::UnfreezeTokensScreen(
                                                                    UnfreezeTokensScreen::new(
                                                                        itb.clone(),
                                                                        &self.app_context,
                                                                    ),
                                                                ),
                                                            );
                                                            ui.close_menu();
                                                        }
                                                        if ui.button("Pause").clicked() {
                                                            action = AppAction::AddScreen(
                                                                Screen::PauseTokensScreen(
                                                                    PauseTokensScreen::new(
                                                                        itb.clone(),
                                                                        &self.app_context,
                                                                    ),
                                                                ),
                                                            );
                                                            ui.close_menu();
                                                        }
                                                        if ui.button("Resume").clicked() {
                                                            action = AppAction::AddScreen(
                                                                Screen::ResumeTokensScreen(
                                                                    ResumeTokensScreen::new(
                                                                        itb.clone(),
                                                                        &self.app_context,
                                                                    ),
                                                                ),
                                                            );
                                                            ui.close_menu();
                                                        }
                                                        if ui.button("View Claims").clicked() {
                                                            action = AppAction::AddScreen(
                                                                Screen::ViewTokenClaimsScreen(
                                                                    ViewTokenClaimsScreen::new(
                                                                        itb.clone(),
                                                                        &self.app_context,
                                                                    ),
                                                                ),
                                                            );
                                                            ui.close_menu();
                                                        }
                                                    });

                                                    // Remove
                                                    if ui
                                                        .button("X")
                                                        .on_hover_text(
                                                            "Remove identity token balance from DET",
                                                        )
                                                        .clicked()
                                                    {
                                                        self.confirm_remove_identity_token_balance_popup = true;
                                                        self.identity_token_balance_to_remove = Some(itb.clone());
                                                    }
                                                }
                                            });
                                        });
                                    });
                                }
                            });
                    });
            });

        action
    }

    /// Renders details for the selected_contract_id.
    fn render_contract_details(&mut self, ui: &mut Ui, contract_id: &Identifier) -> AppAction {
        let mut action = AppAction::None;

        if let Some(description) = &self.selected_contract_description {
            ui.heading("Contract Description:");
            ui.label(description.description.clone());
        }

        ui.add_space(10.0);

        ui.heading("Tokens:");
        let token_infos = self
            .selected_token_infos
            .iter()
            .filter(|token| token.data_contract_id == *contract_id)
            .cloned()
            .collect::<Vec<_>>();
        for token in token_infos {
            if token.data_contract_id == *contract_id {
                ui.heading(token.token_name.clone());
                ui.label(format!(
                    "ID: {}",
                    token.token_id.to_string(Encoding::Base58)
                ));
                ui.label(format!(
                    "Description: {}",
                    token
                        .description
                        .clone()
                        .unwrap_or("No description".to_string())
                ));
            }

            ui.add_space(5.0);

            // Add button to add token to my tokens
            if ui.button("Add to My Tokens").clicked() {
                // Add token to my tokens
                action |= self.add_token_to_my_tokens(token.clone());
            }

            ui.add_space(10.0);
        }

        action
    }

    fn render_keyword_search(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // 1) Input & “Go” button
        ui.heading("Search Tokens by Keyword");
        ui.add_space(5.0);

        ui.horizontal(|ui| {
            ui.label("Enter Keyword:");
            let query_ref = self
                .token_search_query
                .get_or_insert_with(|| "".to_string());
            ui.text_edit_singleline(query_ref);

            if ui.button("Go").clicked() {
                // Clear old results, set status
                self.search_results.lock().unwrap().clear();
                let now = Utc::now().timestamp() as u64;
                self.contract_search_status = ContractSearchStatus::WaitingForResult(now);
                self.search_current_page = 1;
                self.next_cursors.clear();
                self.previous_cursors.clear();
                self.search_has_next_page = false;

                // Dispatch a backend task to do the actual keyword => token retrieval
                let keyword = query_ref.clone();
                action = AppAction::BackendTask(BackendTask::TokenTask(
                    TokenTask::QueryDescriptionsByKeyword(keyword, None),
                ));
            }
        });

        ui.add_space(10.0);

        // 2) Display status
        match &self.contract_search_status {
            ContractSearchStatus::NotStarted => {
                ui.label("Enter a keyword above and click Go.");
            }
            ContractSearchStatus::WaitingForResult(start_time) => {
                let now = Utc::now().timestamp() as u64;
                let elapsed = now - start_time;
                ui.horizontal(|ui| {
                    ui.label(format!("Searching... {} seconds", elapsed));
                    ui.add(egui::widgets::Spinner::default().color(Color32::from_rgb(0, 128, 255)));
                });
            }
            ContractSearchStatus::Complete => {
                // Show the results
                let results = self.search_results.lock().unwrap().clone();
                if results.is_empty() {
                    ui.label("No tokens match your keyword.");
                } else {
                    action |= self.render_search_results_table(ui, &results);
                }

                // Pagination controls
                ui.horizontal(|ui| {
                    if self.search_current_page > 1 {
                        if ui.button("Previous").clicked() {
                            // Go to previous page
                            action = self.goto_previous_search_page();
                        }
                    }

                    if !(self.next_cursors.is_empty() && self.previous_cursors.is_empty()) {
                        ui.label(format!("Page {}", self.search_current_page));
                    }

                    if self.search_has_next_page {
                        if ui.button("Next").clicked() {
                            // Go to next page
                            action = self.goto_next_search_page();
                        }
                    }
                });
            }
            ContractSearchStatus::ErrorMessage(e) => {
                ui.colored_label(Color32::RED, format!("Error: {}", e));
            }
        }

        action
    }

    fn render_search_results_table(
        &mut self,
        ui: &mut Ui,
        search_results: &[ContractDescriptionInfo],
    ) -> AppAction {
        let mut action = AppAction::None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            Frame::group(ui.style())
                .fill(ui.visuals().panel_fill)
                .stroke(egui::Stroke::new(
                    1.0,
                    ui.visuals().widgets.inactive.bg_stroke.color,
                ))
                .inner_margin(Margin::same(8))
                .show(ui, |ui| {
                    TableBuilder::new(ui)
                        .striped(true)
                        .resizable(true)
                        .cell_layout(egui::Layout::left_to_right(Align::Center))
                        .column(Column::initial(60.0).resizable(true)) // Contract ID
                        .column(Column::initial(200.0).resizable(true)) // Contract Description
                        .column(Column::initial(80.0).resizable(true)) // Action
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                ui.label("Contract ID");
                            });
                            header.col(|ui| {
                                ui.label("Contract Description");
                            });
                            header.col(|ui| {
                                ui.label("Action");
                            });
                        })
                        .body(|mut body| {
                            for contract in search_results {
                                body.row(25.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(
                                            contract.data_contract_id.to_string(Encoding::Base58),
                                        );
                                    });
                                    row.col(|ui| {
                                        ui.label(contract.description.clone());
                                    });
                                    row.col(|ui| {
                                        // Example "Add" button
                                        if ui.button("More Info").clicked() {
                                            // Show more info about the token
                                            self.selected_contract_id =
                                                Some(contract.data_contract_id.clone());
                                            action =
                                                AppAction::BackendTask(BackendTask::ContractTask(
                                                    ContractTask::FetchContractsWithDescriptions(
                                                        vec![contract.data_contract_id.clone()],
                                                    ),
                                                ));

                                            // // Add to MyTokens or do something with it
                                            // // Note this is implemented, but we will add it back later!
                                            // // We changed to searching contracts instead of tokens for now
                                            // self.add_token_to_my_tokens(token.clone());
                                        }
                                    });
                                });
                            }
                        });
                });
        });

        action
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
                "Record transfers",
            );
            sub_checkbox(
                ui,
                &mut self.token_advanced_keeps_history.keeps_freezing_history,
                "Record freezes / unfreezes",
            );
            sub_checkbox(
                ui,
                &mut self.token_advanced_keeps_history.keeps_minting_history,
                "Record mints",
            );
            sub_checkbox(
                ui,
                &mut self.token_advanced_keeps_history.keeps_burning_history,
                "Record burns",
            );
            sub_checkbox(
                ui,
                &mut self
                    .token_advanced_keeps_history
                    .keeps_direct_pricing_history,
                "Record direct-pricing changes",
            );
            sub_checkbox(
                ui,
                &mut self
                    .token_advanced_keeps_history
                    .keeps_direct_purchase_history,
                "Record direct purchases",
            );
        }
    }

    pub fn render_token_creator(&mut self, context: &Context, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;

        // 1) If we've successfully completed contract creation, show a success UI
        if self.token_creator_status == TokenCreatorStatus::Complete {
            self.render_token_creator_success_screen(ui);
            return action;
        }

        // Allocate space for refreshing indicator
        let refreshing_height = 33.0;
        let mut max_scroll_height =
            if let TokenCreatorStatus::WaitingForResult(_) = self.token_creator_status {
                ui.available_height() - refreshing_height
            } else {
                ui.available_height()
            };

        // Allocate space for backend message
        let backend_message_height = 40.0;
        if let Some(_) = self.token_creator_error_message.clone() {
            max_scroll_height -= backend_message_height;
        }

        egui::ScrollArea::vertical()
            .max_height(max_scroll_height)
            .show(ui, |ui| {
                Frame::group(ui.style())
                .fill(ui.visuals().panel_fill)
                .stroke(egui::Stroke::new(
                    1.0,
                    ui.visuals().widgets.inactive.bg_stroke.color,
                ))
                .show(ui, |ui| {
                    // Identity selection
                    ui.add_space(10.0);
                    let all_identities = match self.app_context.load_local_qualified_identities() {
                        Ok(ids) => ids,
                        Err(_) => {
                            ui.colored_label(Color32::RED, "Error loading identities from local DB");
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

                    ui.horizontal(|ui| {
                        ui.label("Identity:");
                        egui::ComboBox::from_id_salt("token_creator_identity_selector")
                            .selected_text(
                                self.selected_identity
                                    .as_ref()
                                    .map(|qi| {
                                        qi.alias
                                            .clone()
                                            .unwrap_or_else(|| qi.identity.id().to_string(Encoding::Base58))
                                    })
                                    .unwrap_or_else(|| "Select Identity".to_owned()),
                            )
                            .show_ui(ui, |ui| {
                                for identity in all_identities.iter() {
                                    let display = identity
                                        .alias
                                        .clone()
                                        .unwrap_or_else(|| identity.identity.id().to_string(Encoding::Base58));
                                    if ui
                                        .selectable_label(
                                            Some(identity) == self.selected_identity.as_ref(),
                                            display,
                                        )
                                        .clicked()
                                    {
                                        // On select, store it
                                        self.selected_identity = Some(identity.clone());
                                        // Clear the selected key & wallet
                                        self.selected_key = None;
                                        self.selected_wallet = None;
                                        self.token_creator_error_message = None;
                                    }
                                }
                            });
                    });

                    // Key selection
                    ui.add_space(3.0);
                    if let Some(ref qid) = self.selected_identity {
                        // Attempt to list available keys (only auth keys in normal mode)
                        let keys = if self.app_context.developer_mode {
                            qid.identity
                                .public_keys()
                                .values()
                                .cloned()
                                .collect::<Vec<_>>()
                        } else {
                            qid.available_authentication_keys()
                                .into_iter()
                                .filter_map(|k| {
                                    if k.identity_public_key.security_level() == SecurityLevel::CRITICAL
                                        || k.identity_public_key.security_level() == SecurityLevel::HIGH
                                    {
                                        Some(k.identity_public_key.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect()
                        };

                        ui.horizontal(|ui| {
                            ui.label("Key:");
                            egui::ComboBox::from_id_salt("token_creator_key_selector")
                                .selected_text(match &self.selected_key {
                                    Some(k) => format!(
                                        "Key {} (Purpose: {:?}, Security Level: {:?})",
                                        k.id(),
                                        k.purpose(),
                                        k.security_level()
                                    ),
                                    None => "Select Key".to_owned(),
                                })
                                .show_ui(ui, |ui| {
                                    for k in keys {
                                        let label = format!(
                                            "Key {} (Purpose: {:?}, Security Level: {:?})",
                                            k.id(),
                                            k.purpose(),
                                            k.security_level()
                                        );
                                        if ui
                                            .selectable_label(
                                                Some(k.id()) == self.selected_key.as_ref().map(|kk| kk.id()),
                                                label,
                                            )
                                            .clicked()
                                        {
                                            self.selected_key = Some(k.clone());

                                            // If the key belongs to a wallet, set that wallet reference:
                                            self.selected_wallet = crate::ui::identities::get_selected_wallet(
                                                qid,
                                                None,
                                                Some(&k),
                                                &mut self.token_creator_error_message,
                                            );
                                        }
                                    }
                                });
                        });
                    } else {
                        ui.horizontal(|ui| {
                            ui.label("Key:");
                            egui::ComboBox::from_id_salt("token_creator_key_selector_empty")
                                .selected_text("Select Identity First")
                                .show_ui(ui, |_| {
                                });
                        });
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

                    if let Some(_) = self.selected_wallet {
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
                                ui.label("Token Name (singular):");
                                ui.text_edit_singleline(&mut self.token_names_input[i].0);
                                egui::ComboBox::from_id_salt(format!("token_name_language_selector_{}", i))
                                    .selected_text(format!(
                                        "{}",
                                        self.token_names_input[i].1.to_string()
                                    ))
                                    .show_ui(ui, |ui| {

                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::English, "English");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::French, "French");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Spanish, "Spanish");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Portuguese, "Portuguese");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::German, "German");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Polish, "Polish");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Russian, "Russian");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Mandarin, "Mandarin");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Japanese, "Japanese");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Vietnamese, "Vietnamese");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Korean, "Korean");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Javanese, "Javanese");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Malay, "Malay");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Telugu, "Telugu");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Arabic, "Arabic");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Bengali, "Bengali");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Punjabi, "Punjabi");
                                        ui.selectable_value(&mut self.token_names_input[i].1, TokenNameLanguage::Hindi, "Hindi");
                                    });
                                ui.horizontal(|ui| {
                                    if ui.button("+").clicked() {
                                        // Add a new token name input
                                        self.token_names_input.push((String::new(), TokenNameLanguage::English));
                                    }
                                    if ui.button("-").clicked() {
                                        token_to_remove = Some(i.try_into().expect("Failed to convert index"));
                                    }
                                });
                                ui.end_row();
                            }

                            if let Some(token) = token_to_remove {
                                self.token_names_input.remove(token.into());
                            }

                            // Row 2: Base Supply
                            ui.label("Base Supply:");
                            ui.text_edit_singleline(&mut self.base_supply_input);
                            ui.end_row();

                            // Row 3: Max Supply
                            ui.label("Max Supply:");
                            ui.text_edit_singleline(&mut self.max_supply_input);
                            ui.end_row();

                            // Row 4: Contract Keywords
                            ui.label("Contract Keywords (comma separated):");
                            ui.text_edit_singleline(&mut self.contract_keywords_input);
                            ui.end_row();

                            // Row 5: Token Description
                            ui.label("Token Description (max 100 chars):");
                            ui.text_edit_multiline(&mut self.token_description_input);
                            ui.end_row();
                        });

                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    // 5) Advanced settings toggle
                    ui.collapsing("Advanced", |ui| {
                        ui.add_space(3.0);

                        // Use `Grid` to align labels and text edits
                        egui::Grid::new("advanced_token_info_grid")
                            .num_columns(2)
                            .spacing([16.0, 8.0]) // Horizontal, vertical spacing
                            .show(ui, |ui| {

                                // Start as paused
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut self.start_as_paused_input, "Start as paused");

                                    // Information icon with tooltip
                                    if ui
                                        .add(Label::new(RichText::new("ℹ").monospace()).sense(Sense::hover()))
                                        .on_hover_text(
                                            "When enabled, the token will be created in a paused state, meaning transfers will be \
             disabled by default. All other token features—such as distributions and manual minting—\
             remain fully functional. To allow transfers in the future, the token must be unpaused \
             via an emergency action. It is strongly recommended to enable emergency actions if this \
             option is selected, unless the intention is to permanently disable transfers.",
                                        )
                                        .hovered()
                                    {
                                        // Optional: visual feedback or styling if hovered
                                    }
                                });
                                ui.end_row();

                                self.history_row(ui);
                                ui.end_row();

                                // Name should be capitalized
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut self.should_capitalize_input, "Name should be capitalized");

                                    // Information icon with tooltip
                                    if ui
                                        .add(Label::new(RichText::new("ℹ").monospace()).sense(Sense::hover()))
                                        .on_hover_text(
                                            "This is used only as helper information to client applications that will use \
                                            token. This informs them on whether to capitalize the token name or not by default.",
                                        )
                                        .hovered()
                                    {
                                    }
                                });
                                ui.end_row();

                                // Decimals
                                ui.horizontal(|ui| {
                                    ui.label("Decimals:");
                                    ui.text_edit_singleline(&mut self.decimals_input);
                                    if ui
                                        .add(Label::new(RichText::new("ℹ").monospace()).sense(Sense::hover()))
                                        .on_hover_text(
                                            "The decimal places of the token, for example Dash and Bitcoin use 8. \
                                            The minimum indivisible amount is a Duff or a Satoshi respectively. \
                                            If you put a value greater than 0 this means that it is indicated that the \
                                            consensus is that 10^(number entered) is what represents 1 full unit of the token.",
                                        )
                                        .hovered()
                                    {
                                    }
                                });
                                ui.end_row();
                            });
                    });

                    ui.add_space(5.0);

                    ui.collapsing("Action Rules", |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Preset:");

                            egui::ComboBox::from_id_salt("preset_selector")
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
                                    use TokenConfigurationPresetFeatures::*;

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

                        self.manual_minting_rules.render_control_change_rules_ui(ui, "Manual Mint");
                        self.manual_burning_rules.render_control_change_rules_ui(ui, "Manual Burn");
                        self.freeze_rules.render_control_change_rules_ui(ui, "Freeze");
                        self.unfreeze_rules.render_control_change_rules_ui(ui, "Unfreeze");
                        self.destroy_frozen_funds_rules.render_control_change_rules_ui(ui, "Destroy Frozen Funds");
                        self.emergency_action_rules.render_control_change_rules_ui(ui, "Emergency Action");
                        self.max_supply_change_rules.render_control_change_rules_ui(ui, "Max Supply Change");
                        self.conventions_change_rules.render_control_change_rules_ui(ui, "Conventions Change");

                        // Main control group change is slightly different so do this one manually.
                        ui.collapsing("Main Control Group Change", |ui| {
                            ui.add_space(3.0);

                            // A) authorized_to_make_change
                            ui.horizontal(|ui| {
                                ui.label("Allow main control group change:");
                                egui::ComboBox::from_id_salt("main_control_group_change_selector")
                                    .selected_text(format!(
                                        "{}",
                                        self.authorized_main_control_group_change.to_string()
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
                                            ui.add(egui::TextEdit::singleline(id).hint_text("base58 id"));
                                        }
                                    }
                                    AuthorizedActionTakers::Group(_) => {
                                        if self.main_control_group_change_authorized_group.is_none() {
                                            self.main_control_group_change_authorized_group = Some("0".to_string());
                                        }
                                        if let Some(ref mut group) = self.main_control_group_change_authorized_group {
                                            ui.add(egui::TextEdit::singleline(group).hint_text("group contract position"));
                                        }
                                    }
                                    _ => {}
                                }
                            });
                        });
                    });

                    ui.add_space(5.0);

                    ui.collapsing("Distribution", |ui| {
                        ui.add_space(3.0);

                        // PERPETUAL DISTRIBUTION SETTINGS
                        if ui.checkbox(
                            &mut self.enable_perpetual_distribution,
                            "Enable Perpetual Distribution",
                        ).clicked() {
                            self.perpetual_dist_type = PerpetualDistributionIntervalTypeUI::BlockBased;
                        };
                        if self.enable_perpetual_distribution {
                            ui.add_space(5.0);

                            // 2) Select the distribution type
                            ui.horizontal(|ui| {
                                ui.label("     Type:");
                                egui::ComboBox::from_id_salt("perpetual_dist_type_selector")
                                    .selected_text(format!("{:?}", self.perpetual_dist_type))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_type,
                                            PerpetualDistributionIntervalTypeUI::BlockBased,
                                            "Block-Based",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_type,
                                            PerpetualDistributionIntervalTypeUI::TimeBased,
                                            "Time-Based",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_type,
                                            PerpetualDistributionIntervalTypeUI::EpochBased,
                                            "Epoch-Based",
                                        );
                                    });
                            });

                            // If user picked a real distribution type:
                            match self.perpetual_dist_type {
                                PerpetualDistributionIntervalTypeUI::BlockBased => {
                                    ui.add_space(2.0);
                                    ui.horizontal(|ui| {
                                        ui.label("        - Block Interval:");
                                        ui.text_edit_singleline(&mut self.perpetual_dist_interval_input);
                                    });
                                }
                                PerpetualDistributionIntervalTypeUI::TimeBased => {
                                    ui.add_space(2.0);
                                    ui.horizontal(|ui| {
                                        ui.label("        - Time Interval (ms):");
                                        ui.text_edit_singleline(&mut self.perpetual_dist_interval_input);
                                    });
                                }
                                PerpetualDistributionIntervalTypeUI::EpochBased => {
                                    ui.add_space(2.0);
                                    ui.horizontal(|ui| {
                                        ui.label("        - Epoch Interval:");
                                        ui.text_edit_singleline(&mut self.perpetual_dist_interval_input);
                                    });
                                }
                                PerpetualDistributionIntervalTypeUI::None => {
                                    // Do nothing
                                }
                            }

                            ui.add_space(10.0);

                            // 3) Select the distribution function
                            ui.horizontal(|ui| {
                                ui.label("     Function:");
                                egui::ComboBox::from_id_salt("perpetual_dist_function_selector")
                                    .selected_text(format!("{:?}", self.perpetual_dist_function))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::FixedAmount,
                                            "FixedAmount",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::Random,
                                            "Random",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::StepDecreasingAmount,
                                            "StepDecreasing",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::Stepwise,
                                            "Stepwise",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::Linear,
                                            "Linear",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::Polynomial,
                                            "Polynomial",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::Exponential,
                                            "Exponential",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::Logarithmic,
                                            "Logarithmic",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::InvertedLogarithmic,
                                            "InvertedLogarithmic",
                                        );
                                    });

                                    let info_icon = Label::new("ℹ").sense(Sense::click());
                                    let response = ui.add(info_icon).on_hover_text("Info about distribution types");

                                    // Check if the label was clicked
                                    if response.clicked() {
                                        self.show_pop_up_info = Some(r#"
# FixedAmount

Emits a constant (fixed) number of tokens for every period.

### Formula
For any period `x`, the emitted tokens are:

`f(x) = n`

### Use Case
- When a predictable, unchanging reward is desired.
- Simplicity and stable emissions.

### Example
- If `n = 5` tokens per block, then after 3 blocks the total emission is 15 tokens.

---

# StepDecreasingAmount

Emits a random number of tokens within a specified range.

### Description
- This function selects a **random** token emission amount between `min` and `max`.
- The value is drawn **uniformly** between the bounds.
- The randomness uses a Pseudo Random Function (PRF) from x.

### Formula
For any period `x`, the emitted tokens follow:

`f(x) ∈ [min, max]`

### Parameters
- `min`: The **minimum** possible number of tokens emitted.
- `max`: The **maximum** possible number of tokens emitted.

### Use Cases
- **Stochastic Rewards**: Introduces randomness into rewards to incentivize unpredictability.
- **Lottery-Based Systems**: Used for randomized emissions, such as block rewards with probabilistic payouts.

### Example
Suppose a system emits **between 10 and 100 tokens per period**.

`Random { min: 10, max: 100 }`

| Period (x) | Emitted Tokens (Random) |
|------------|------------------------|
| 1          | 27                     |
| 2          | 94                     |
| 3          | 63                     |
| 4          | 12                     |

- Each period, the function emits a **random number of tokens** between `min = 10` and `max = 100`.
- Over time, the **average reward trends toward the midpoint** `(min + max) / 2`.

### Constraints
- **`min` must be ≤ `max`**, otherwise the function is invalid.
- If `min == max`, this behaves like a `FixedAmount` function with a constant emission.

---

### LinearInteger

A linear function using integer precision.

- **Formula:** f(x) = a * x + b  
- **Description:**  
    - a > 0 -> tokens increase over time  
    - a < 0 -> tokens decrease over time  
    - b is the initial value  
- **Use Case:** Incentivize early or match ecosystem growth  
- **Example:** f(x) = 10x + 50

---

### LinearFloat

A linear function with fractional (floating-point) rates.

- **Formula:** f(x) = a * x + b  
- **Description:** Similar to LinearInteger, but with fractional slope  
- **Use Case:** Gradual fractional increases/decreases over time  
- **Example:** f(x) = 0.5x + 50

---

### PolynomialInteger

A polynomial function (e.g. quadratic, cubic) using integer precision.

- **Formula:** f(x) = a * x^n + b  
- **Description:** Flexible curves (growth/decay) beyond simple linear.  
- **Use Case:** Diminishing or accelerating returns as time progresses  
- **Example:** f(x) = 2x^2 + 20

---

### PolynomialFloat

A polynomial function supporting fractional exponents or coefficients.

- **Formula:** f(x) = a * x^n + b  
- **Description:** Similar to PolynomialInteger, but with floats  
- **Example:** f(x) = 0.5x^3 + 20

---

### Exponential

Exponential growth or decay of tokens.

- **Formula:** f(x) = a * e^(b * x) + c  
- **Description:**  
    - b > 0 -> rapid growth  
    - b < 0 -> rapid decay  
- **Use Case:** Early contributor boosts or quick emission tapering  
- **Example:** f(x) = 100 * e^(-0.693 * x) + 5

---

### Logarithmic

Logarithmic growth of token emissions.

- **Formula:** f(x) = a * log_b(x) + c  
- **Description:** Growth slows as x increases.  
- **Use Case:** Sustainable long-term emission tapering  
- **Example:** f(x) = 20 * log_2(x) + 5

---

### Stepwise

Emits tokens in fixed amounts for specific intervals.

- **Description:** Emissions remain constant within each step.  
- **Use Case:** Adjust rewards at specific milestones  
- **Example:** 100 tokens per block for first 1000 blocks, then 50 tokens thereafter.
"#
                                        .to_string())};
                            });

                            ui.add_space(2.0);

                            if let Some(texture) = self.function_textures.get(&self.perpetual_dist_function) {
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(50.0); // Shift image right
                                    ui.image(texture);
                                });
                                ui.add_space(10.0);
                            } else if let Some(image) = self.function_images.get(&self.perpetual_dist_function) {
                                let texture = context.load_texture(self.perpetual_dist_function.name(), image.clone(), Default::default());
                                self.function_textures.insert(self.perpetual_dist_function.clone(), texture.clone());
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(50.0); // Shift image right
                                    ui.image(&texture);
                                });
                                ui.add_space(10.0);
                            }

                            // Based on the user’s chosen function, display relevant fields:
                            match self.perpetual_dist_function {
                                DistributionFunctionUI::FixedAmount => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Fixed Amount per Interval:");
                                        ui.text_edit_singleline(&mut self.fixed_amount_input);
                                    });
                                }

                                DistributionFunctionUI::Random => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Min Amount (n):");
                                        ui.text_edit_singleline(&mut self.random_min_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Max Amount (n):");
                                        ui.text_edit_singleline(&mut self.random_max_input);
                                    });
                                }

                                DistributionFunctionUI::StepDecreasingAmount => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Step Count (u64):");
                                        ui.text_edit_singleline(&mut self.step_count_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Decrease per Interval Numerator:");
                                        ui.text_edit_singleline(&mut self.decrease_per_interval_numerator_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Decrease per Interval Denominator:");
                                        ui.text_edit_singleline(&mut self.decrease_per_interval_denominator_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Start Period Offset (optional):");
                                        ui.text_edit_singleline(&mut self.step_decreasing_start_period_offset_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Initial Token Emission (n):");
                                        ui.text_edit_singleline(&mut self.step_decreasing_initial_emission_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Minimum Emission Value (optional):");
                                        ui.text_edit_singleline(&mut self.step_decreasing_min_value_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Maximum Interval Count (optional):");
                                        ui.text_edit_singleline(&mut self.step_decreasing_max_interval_count_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Trailing Distribution Interval Amount:");
                                        ui.text_edit_singleline(&mut self.step_decreasing_trailing_distribution_interval_amount_input);
                                    });
                                }

                                DistributionFunctionUI::Stepwise => {
                                    // Example: multiple steps (u64 block => some token amount).
                                    // Each element in `stepwise_steps` is (String, String) = (block, amount).
                                    // You can show them in a loop and let users edit each pair.
                                    let mut i = 0;
                                    while i < self.stepwise_steps.len() {
                                        let (mut block_str, mut amount_str) = self.stepwise_steps[i].clone();

                                        ui.horizontal(|ui| {
                                            ui.label(format!("        - Step #{}:", i));
                                            ui.label("Interval (u64):");
                                            ui.text_edit_singleline(&mut block_str);

                                            ui.label("Amount (n):");
                                            ui.text_edit_singleline(&mut amount_str);

                                            // If remove is clicked, remove the step at index i
                                            // and *do not* increment i, because the next element
                                            // now “shifts” into this index.
                                            if ui.button("Remove").clicked() {
                                                self.stepwise_steps.remove(i);
                                            } else {
                                                // Otherwise, update the vector with any edits and move to the next step
                                                self.stepwise_steps[i] = (block_str, amount_str);
                                                i += 1;
                                            }
                                        });
                                    }

                                    // A button to add new steps
                                    ui.horizontal(|ui| {
                                        ui.label("     ");
                                        if ui.button("Add Step").clicked() {
                                            self.stepwise_steps.push(("0".to_owned(), "0".to_owned()));
                                        }
                                    });
                                }

                                DistributionFunctionUI::Linear => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Slope Numerator (a, i64):");
                                        ui.text_edit_singleline(&mut self.linear_int_a_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Slope Divisor (d, i64):");
                                        ui.text_edit_singleline(&mut self.linear_int_d_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Start Step (s, i64):");
                                        ui.text_edit_singleline(&mut self.linear_int_start_step_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Starting Amount (b, i64):");
                                        ui.text_edit_singleline(&mut self.linear_int_starting_amount_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Minimum Emission Value (optional):");
                                        ui.text_edit_singleline(&mut self.linear_int_min_value_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Maximum Emission Value (optional):");
                                        ui.text_edit_singleline(&mut self.linear_int_max_value_input);
                                    });
                                }

                                DistributionFunctionUI::Polynomial => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Scaling Factor (a, i64):");
                                        ui.text_edit_singleline(&mut self.poly_int_a_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Exponent Numerator (m, i64):");
                                        ui.text_edit_singleline(&mut self.poly_int_m_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Exponent Denominator (n, i64):");
                                        ui.text_edit_singleline(&mut self.poly_int_n_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Divisor (d, i64):");
                                        ui.text_edit_singleline(&mut self.poly_int_d_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Start Period Offset (s, i64):");
                                        ui.text_edit_singleline(&mut self.poly_int_s_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Offset (o, i64):");
                                        ui.text_edit_singleline(&mut self.poly_int_o_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Initial Token Emission (b, i64):");
                                        ui.text_edit_singleline(&mut self.poly_int_b_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Minimum Emission Value (optional):");
                                        ui.text_edit_singleline(&mut self.poly_int_min_value_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Maximum Emission Value (optional):");
                                        ui.text_edit_singleline(&mut self.poly_int_max_value_input);
                                    });
                                }

                                DistributionFunctionUI::Exponential => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Scaling Factor (a, { 0 < a ≤ 256) }):");
                                        ui.text_edit_singleline(&mut self.exp_a_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Exponent Rate (m, { -8 ≤ m ≤ 8 ; m ≠ 0 }):");
                                        ui.text_edit_singleline(&mut self.exp_m_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Exponent Rate (n, 0 < a ≤ 32):");
                                        ui.text_edit_singleline(&mut self.exp_n_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Divisor (d, {u64 ; d ≠ 0 }):");
                                        ui.text_edit_singleline(&mut self.exp_d_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Start Period Offset (s, i64):");
                                        ui.text_edit_singleline(&mut self.exp_s_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Offset (o, i64):");
                                        ui.text_edit_singleline(&mut self.exp_o_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Offset (b, i64):");
                                        ui.text_edit_singleline(&mut self.exp_b_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Minimum Emission Value (optional):");
                                        ui.text_edit_singleline(&mut self.exp_min_value_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Maximum Emission Value (optional):");
                                        ui.text_edit_singleline(&mut self.exp_max_value_input);
                                    });
                                }

                                DistributionFunctionUI::Logarithmic => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Scaling Factor (a, i64):");
                                        ui.text_edit_singleline(&mut self.log_a_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Divisor (d, i64):");
                                        ui.text_edit_singleline(&mut self.log_d_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Exponent Numerator (m, i64):");
                                        ui.text_edit_singleline(&mut self.log_m_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Exponent Denominator (n, i64):");
                                        ui.text_edit_singleline(&mut self.log_n_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Start Period Offset (s, i64):");
                                        ui.text_edit_singleline(&mut self.log_s_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Offset (o, i64):");
                                        ui.text_edit_singleline(&mut self.log_o_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Initial Token Emission (b, i64):");
                                        ui.text_edit_singleline(&mut self.log_b_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Minimum Emission Value (optional):");
                                        ui.text_edit_singleline(&mut self.log_min_value_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Maximum Emission Value (optional):");
                                        ui.text_edit_singleline(&mut self.log_max_value_input);
                                    });
                                }

                                DistributionFunctionUI::InvertedLogarithmic => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Scaling Factor (a, i64):");
                                        ui.text_edit_singleline(&mut self.inv_log_a_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Divisor (d, i64):");
                                        ui.text_edit_singleline(&mut self.inv_log_d_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Exponent Numerator (m, i64):");
                                        ui.text_edit_singleline(&mut self.inv_log_m_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Exponent Denominator (n, i64):");
                                        ui.text_edit_singleline(&mut self.inv_log_n_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Start Period Offset (s, i64):");
                                        ui.text_edit_singleline(&mut self.inv_log_s_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Offset (o, i64):");
                                        ui.text_edit_singleline(&mut self.inv_log_o_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Initial Token Emission (b, i64):");
                                        ui.text_edit_singleline(&mut self.inv_log_b_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Minimum Emission Value (optional):");
                                        ui.text_edit_singleline(&mut self.inv_log_min_value_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Maximum Emission Value (optional):");
                                        ui.text_edit_singleline(&mut self.inv_log_max_value_input);
                                    });
                                }
                            }

                            ui.add_space(10.0);

                            // 4) Choose the distribution recipient
                            ui.horizontal(|ui| {
                                ui.label("     Recipient:");
                                egui::ComboBox::from_id_salt("perpetual_dist_recipient_selector")
                                    .selected_text(format!("{:?}", self.perpetual_dist_recipient))
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_recipient,
                                            TokenDistributionRecipientUI::ContractOwner,
                                            "Contract Owner",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_recipient,
                                            TokenDistributionRecipientUI::Identity,
                                            "Specific Identity",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_recipient,
                                            TokenDistributionRecipientUI::EvonodesByParticipation,
                                            "Evonodes",
                                        );
                                    });

                                // If user selected Identity or Group, show extra text edit
                                match &mut self.perpetual_dist_recipient {
                                    TokenDistributionRecipientUI::Identity => {
                                        if self.perpetual_dist_recipient_identity_input.is_none() {
                                            self.perpetual_dist_recipient_identity_input = Some(String::new());
                                        }
                                        if let Some(ref mut id) = self.perpetual_dist_recipient_identity_input {
                                            ui.add(egui::TextEdit::singleline(id).hint_text("Enter base58 id"));
                                        }
                                    }
                                    _ => {}
                                }
                            });

                            ui.add_space(10.0);

                            ui.horizontal(|ui| {
                                ui.label(" ");
                                self.perpetual_distribution_rules.render_control_change_rules_ui(ui, "Perpetual Distribution Rules");
                            });

                            ui.add_space(5.0);
                        } else {
                            self.perpetual_dist_type = PerpetualDistributionIntervalTypeUI::None;
                        }

                        ui.separator();

                        // PRE-PROGRAMMED DISTRIBUTION
                        ui.checkbox(
                            &mut self.enable_pre_programmed_distribution,
                            "Enable Pre-Programmed Distribution",
                        );

                        if self.enable_pre_programmed_distribution {
                            ui.add_space(2.0);

                            let mut i = 0;
                            while i < self.pre_programmed_distributions.len() {
                                // Clone the current entry
                                let mut entry = self.pre_programmed_distributions[i].clone();

                                // Render row
                                ui.horizontal(|ui| {
                                    ui.label(format!("Timestamp #{}:", i + 1));

                                    // Replace text-edit timestamp with days/hours/minutes
                                    ui.add(
                                        egui::DragValue::new(&mut entry.days)
                                            .prefix("Days: ")
                                            .range(0..=3650),
                                    );
                                    ui.add(
                                        egui::DragValue::new(&mut entry.hours)
                                            .prefix("Hours: ")
                                            .range(0..=23),
                                    );
                                    ui.add(
                                        egui::DragValue::new(&mut entry.minutes)
                                            .prefix("Minutes: ")
                                            .range(0..=59),
                                    );

                                    ui.label("Identity:");
                                    ui.text_edit_singleline(&mut entry.identity_str);

                                    ui.label("Amount:");
                                    ui.text_edit_singleline(&mut entry.amount_str);

                                    // Remove button
                                    if ui.button("Remove").clicked() {
                                        self.pre_programmed_distributions.remove(i);
                                    } else {
                                        self.pre_programmed_distributions[i] = entry;
                                    }
                                });

                                i += 1;
                            }

                            ui.add_space(2.0);

                            // Add a button to insert a blank row
                            ui.horizontal(|ui| {
                                ui.label("   ");
                                if ui.button("Add New Distribution Entry").clicked() {
                                    self.pre_programmed_distributions.push(DistributionEntry::default());
                                }
                            });

                            ui.add_space(2.0);
                        }

                        ui.separator();

                        // NEW TOKENS DESTINATION IDENTITY
                        ui.checkbox(
                            &mut self.new_tokens_destination_identity_enabled,
                            "Use a default identity to receive newly minted tokens",
                        );
                        if self.new_tokens_destination_identity_enabled {
                            ui.add_space(2.0);

                            // Show text field for ID
                            ui.horizontal(|ui| {
                                ui.label("       Default Destination Identity (Base58):");
                                ui.text_edit_singleline(&mut self.new_tokens_destination_identity);
                            });

                            ui.horizontal(|ui| {
                                ui.label("   ");
                                self.new_tokens_destination_identity_rules.render_control_change_rules_ui(ui, "New Tokens Destination Identity Rules");
                            });
                        }

                        ui.separator();

                        // MINTING ALLOW CHOOSING DESTINATION
                        ui.checkbox(
                            &mut self.minting_allow_choosing_destination,
                            "Allow user to pick a destination identity on each mint",
                        );
                        if self.minting_allow_choosing_destination {
                            ui.horizontal(|ui| {
                                ui.label("   ");
                                self.minting_allow_choosing_destination_rules.render_control_change_rules_ui(ui, "Minting Allow Choosing Destination Rules");
                            });
                        }
                    });

                    ui.add_space(5.0);

                    ui.collapsing("Groups", |ui| {
                        ui.add_space(3.0);
                        ui.label("Define one or more groups for multi-party control of the contract.");
                        ui.add_space(2.0);

                        // Add main group selection input
                        ui.horizontal(|ui| {
                            ui.label("Main Control Group Position:");
                            ui.text_edit_singleline(&mut self.main_control_group_input);
                        });

                        ui.add_space(2.0);

                        // Draw each group in a loop
                        let mut i = 0;
                        while i < self.groups_ui.len() {
                            // We’ll clone it so we can safely mutate it
                            let mut group_ui = self.groups_ui[i].clone();

                            // Create a collapsible for the group: “Group #i”
                            // or use the group_position_str to label it
                            egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                format!("group_header_{}", i).into(),
                                true,
                            )
                            .show_header(ui, |ui| {
                                ui.label(format!("Group {}", group_ui.group_position_str));
                            })
                            .body(|ui| {
                                ui.add_space(3.0);

                                ui.horizontal(|ui| {
                                    ui.label("Group Position (u16):");
                                    ui.text_edit_singleline(&mut group_ui.group_position_str);
                                });

                                ui.horizontal(|ui| {
                                    ui.label("Required Power:");
                                    ui.text_edit_singleline(&mut group_ui.required_power_str);
                                });

                                ui.label("Members:");
                                ui.add_space(3.0);

                                let mut j = 0;
                                while j < group_ui.members.len() {
                                    let mut member = group_ui.members[j].clone();

                                    ui.horizontal(|ui| {
                                        ui.label(format!("Member {}:", j + 1));

                                        ui.label("Identity (base58):");
                                        ui.text_edit_singleline(&mut member.identity_str);

                                        ui.label("Power (u32):");
                                        ui.text_edit_singleline(&mut member.power_str);

                                        if ui.button("Remove Member").clicked() {
                                            group_ui.members.remove(j);
                                            return; // return so we skip the assignment at the end
                                        } else {
                                            // Only assign back if we didn’t remove
                                            group_ui.members[j] = member;
                                        }
                                    });

                                    j += 1;
                                }

                                ui.add_space(3.0);
                                if ui.button("Add Member").clicked() {
                                    group_ui.members.push(GroupMemberUI {
                                        identity_str: "".to_owned(),
                                        power_str: "1".to_owned(),
                                    });
                                }

                                ui.add_space(3.0);

                                // A remove button for the entire group
                                if ui.button("Remove This Group").clicked() {
                                    self.groups_ui.remove(i);
                                    return;
                                } else {
                                    self.groups_ui[i] = group_ui;
                                }
                            });

                            i += 1;
                        }

                        ui.add_space(5.0);
                        if ui.button("Add New Group").clicked() {
                            self.groups_ui.push(GroupConfigUI {
                                group_position_str: (self.groups_ui.len() as u16).to_string(),
                                required_power_str: "1".to_owned(),
                                members: vec![],
                            });
                        }
                    });

                    // 6) "Register Token Contract" button
                    ui.add_space(10.0);
                    let mut new_style = (**ui.style()).clone();
                    new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                    ui.set_style(new_style);
                    ui.horizontal(|ui| {
                        let register_button =
                            egui::Button::new(RichText::new("Register Token Contract").color(Color32::WHITE))
                                .fill(Color32::from_rgb(0, 128, 255))
                                .frame(true)
                                .corner_radius(3.0);
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
                        let view_json_button = egui::Button::new(RichText::new("View JSON").color(Color32::WHITE))
                            .fill(Color32::from_rgb(0, 128, 255))
                            .frame(true)
                            .corner_radius(3.0);
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
                                    ) {
                                        Ok(dc) => dc,
                                        Err(e) => {
                                            self.token_creator_error_message = Some(format!("Error building contract V1: {e}"));
                                            return;
                                        }
                                    };

                                    let data_contract_json = data_contract.to_json(self.app_context.platform_version).expect("Expected to map contract to json");
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
        });

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
            ui.colored_label(Color32::RED, format!("{err_msg}"));
            ui.add_space(10.0);
        }

        action
    }

    /// Renders a popup window displaying the data contract JSON.
    pub fn render_data_contract_json_popup(&mut self, ui: &mut Ui) {
        if self.show_json_popup {
            let mut is_open = true;
            egui::Window::new("Data Contract JSON")
                .collapsible(false)
                .resizable(true)
                .max_height(600.0)
                .max_width(800.0)
                .scroll(true)
                .open(&mut is_open)
                .show(ui.ctx(), |ui| {
                    // Display the JSON in a multiline text box
                    ui.add_space(4.0);
                    ui.label("Below is the data contract JSON:");
                    ui.add_space(4.0);

                    egui::Resize::default()
                        .id_salt("json_resize_area_for_contract")
                        .default_size([750.0, 550.0])
                        .max_height(ui.available_height() - 50.0)
                        .max_width(ui.available_height() - 20.0)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    ui.monospace(&mut self.json_popup_text);
                                });
                        });

                    ui.add_space(10.0);

                    // A button to close
                    if ui.button("Close").clicked() {
                        self.show_json_popup = false;
                    }
                });

            // If the user closed the window via the "x" in the corner
            // we should reflect that in `show_json_popup`.
            if !is_open {
                self.show_json_popup = false;
            }
        }
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
                let max_supply_display = if self.max_supply_input.len() == 0 {
                    "None".to_string()
                } else {
                    self.max_supply_input.clone()
                };
                ui.label(format!(
                    "Name: {}\nBase Supply: {}\nMax Supply: {}",
                    self.token_names_input[0].0, self.base_supply_input, max_supply_display,
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
                        BackendTask::TokenTask(TokenTask::RegisterTokenContract {
                            identity: self.selected_identity.clone().unwrap(),
                            signing_key: self.selected_key.clone().unwrap(),

                            token_names: args.token_names,
                            contract_keywords: args.contract_keywords,
                            token_description: args.token_description,
                            should_capitalize: args.should_capitalize,
                            decimals: args.decimals,
                            base_supply: args.base_supply,
                            max_supply: args.max_supply,
                            start_paused: args.start_paused,
                            keeps_history: args.keeps_history,
                            main_control_group: args.main_control_group,

                            manual_minting_rules: args.manual_minting_rules,
                            manual_burning_rules: args.manual_burning_rules,
                            freeze_rules: args.freeze_rules,
                            unfreeze_rules: args.unfreeze_rules,
                            destroy_frozen_funds_rules: args.destroy_frozen_funds_rules,
                            emergency_action_rules: args.emergency_action_rules,
                            max_supply_change_rules: args.max_supply_change_rules,
                            conventions_change_rules: args.conventions_change_rules,
                            main_control_group_change_authorized: args
                                .main_control_group_change_authorized,
                            distribution_rules: args.distribution_rules,
                            groups: args.groups,
                        }),
                        BackendTask::TokenTask(TokenTask::QueryMyTokenBalances),
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

    /// Gathers user input and produces the arguments needed by
    /// `build_data_contract_v1_with_one_token`.
    /// Returns Err(error_msg) on invalid input.
    pub fn parse_token_build_args(&mut self) -> Result<TokenBuildArgs, String> {
        // 1) We must have a selected identity
        let identity = self
            .selected_identity
            .clone()
            .ok_or_else(|| "Please select an identity".to_string())?;
        let identity_id = identity.identity.id().clone();

        // 2) Basic fields
        if self.token_names_input.is_empty() {
            return Err("Please enter a token name".to_string());
        }
        // If any name languages are duplicated, return an error
        let mut seen_languages = HashSet::new();
        for name_with_language in self.token_names_input.iter() {
            if seen_languages.contains(&name_with_language.1) {
                return Err(format!(
                    "Duplicate token name language: {:?}",
                    name_with_language.1
                ));
            }
            seen_languages.insert(name_with_language.1);
        }
        let mut token_names: Vec<(String, String)> = Vec::new();
        for name_with_language in self.token_names_input.iter() {
            let language = match name_with_language.1 {
                TokenNameLanguage::English => "en".to_string(),
                TokenNameLanguage::Mandarin => "zh".to_string(),
                TokenNameLanguage::Hindi => "hi".to_string(),
                TokenNameLanguage::Russian => "ru".to_string(),
                TokenNameLanguage::Spanish => "es".to_string(),
                TokenNameLanguage::Arabic => "ar".to_string(),
                TokenNameLanguage::Bengali => "bn".to_string(),
                TokenNameLanguage::Portuguese => "pt".to_string(),
                TokenNameLanguage::Japanese => "ja".to_string(),
                TokenNameLanguage::Punjabi => "pa".to_string(),
                TokenNameLanguage::German => "de".to_string(),
                TokenNameLanguage::Javanese => "jv".to_string(),
                TokenNameLanguage::Malay => "ms".to_string(),
                TokenNameLanguage::Telugu => "te".to_string(),
                TokenNameLanguage::Vietnamese => "vi".to_string(),
                TokenNameLanguage::Korean => "ko".to_string(),
                TokenNameLanguage::French => "fr".to_string(),
                TokenNameLanguage::Polish => "pl".to_string(),
            };

            token_names.push((name_with_language.0.clone(), language));
        }

        // Remove whitespace and parse the comma separated string into a vec
        let contract_keywords = if self.contract_keywords_input.trim().is_empty() {
            Vec::new()
        } else {
            self.contract_keywords_input
                .split(',')
                .map(|s| s.trim().to_string())
                .collect::<Vec<String>>()
        };
        let token_description = if self.token_description_input.len() > 0 {
            Some(self.token_description_input.clone())
        } else {
            None
        };
        let decimals = self.decimals_input.parse::<u16>().unwrap_or(8);
        let base_supply = self.base_supply_input.parse::<u64>().unwrap_or(1000000);
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
                Ok(other.clone())
            }
        }
    }

    fn build_distribution_rules(&mut self) -> Result<TokenDistributionRulesV0, String> {
        // 1) Validate distribution input, parse numeric fields, etc.
        let distribution_function = match self.perpetual_dist_function {
            DistributionFunctionUI::FixedAmount => DistributionFunction::FixedAmount {
                amount: self.fixed_amount_input.parse::<u64>().unwrap_or(0),
            },
            DistributionFunctionUI::Random => DistributionFunction::Random {
                min: self.random_min_input.parse::<u64>().unwrap_or(0),
                max: self.random_max_input.parse::<u64>().unwrap_or(0),
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
                d: self.linear_int_d_input.parse::<u64>().unwrap_or(0),
                start_step: Some(self.linear_int_start_step_input.parse::<u64>().unwrap_or(0)),
                starting_amount: self
                    .linear_int_starting_amount_input
                    .parse::<u64>()
                    .unwrap_or(0),
                min_value: Some(self.linear_int_min_value_input.parse::<u64>().unwrap_or(0)),
                max_value: Some(self.linear_int_max_value_input.parse::<u64>().unwrap_or(0)),
            },
            DistributionFunctionUI::Polynomial => DistributionFunction::Polynomial {
                a: self.poly_int_a_input.parse::<i64>().unwrap_or(0),
                m: self.poly_int_m_input.parse::<i64>().unwrap_or(0),
                n: self.poly_int_n_input.parse::<u64>().unwrap_or(0),
                d: self.poly_int_d_input.parse::<u64>().unwrap_or(0),
                start_moment: Some(self.poly_int_s_input.parse::<u64>().unwrap_or(0)),
                o: self.poly_int_o_input.parse::<i64>().unwrap_or(0),
                b: self.poly_int_b_input.parse::<u64>().unwrap_or(0),
                min_value: Some(self.poly_int_min_value_input.parse::<u64>().unwrap_or(0)),
                max_value: Some(self.poly_int_max_value_input.parse::<u64>().unwrap_or(0)),
            },
            DistributionFunctionUI::Exponential => DistributionFunction::Exponential {
                a: self.exp_a_input.parse::<u64>().unwrap_or(0),
                d: self.exp_d_input.parse::<u64>().unwrap_or(0),
                m: self.exp_m_input.parse::<i64>().unwrap_or(0),
                n: self.exp_n_input.parse::<u64>().unwrap_or(0),
                o: self.exp_o_input.parse::<i64>().unwrap_or(0),
                start_moment: Some(self.exp_s_input.parse::<u64>().unwrap_or(0)),
                b: self.exp_b_input.parse::<u64>().unwrap_or(0),
                min_value: Some(self.exp_min_value_input.parse::<u64>().unwrap_or(0)),
                max_value: Some(self.exp_max_value_input.parse::<u64>().unwrap_or(0)),
            },
            DistributionFunctionUI::Logarithmic => DistributionFunction::Logarithmic {
                a: self.log_a_input.parse::<i64>().unwrap_or(0),
                d: self.log_d_input.parse::<u64>().unwrap_or(0),
                m: self.log_m_input.parse::<u64>().unwrap_or(0),
                n: self.log_n_input.parse::<u64>().unwrap_or(0),
                start_moment: Some(self.log_s_input.parse::<u64>().unwrap_or(0)),
                o: self.log_o_input.parse::<i64>().unwrap_or(0),
                b: self.log_b_input.parse::<u64>().unwrap_or(0),
                min_value: Some(self.log_min_value_input.parse::<u64>().unwrap_or(0)),
                max_value: Some(self.log_max_value_input.parse::<u64>().unwrap_or(0)),
            },
            DistributionFunctionUI::InvertedLogarithmic => {
                DistributionFunction::InvertedLogarithmic {
                    a: self.inv_log_a_input.parse::<i64>().unwrap_or(0),
                    d: self.inv_log_d_input.parse::<u64>().unwrap_or(0),
                    m: self.inv_log_m_input.parse::<u64>().unwrap_or(0),
                    n: self.inv_log_n_input.parse::<u64>().unwrap_or(0),
                    start_moment: Some(self.inv_log_s_input.parse::<u64>().unwrap_or(0)),
                    o: self.inv_log_o_input.parse::<i64>().unwrap_or(0),
                    b: self.inv_log_b_input.parse::<u64>().unwrap_or(0),
                    min_value: Some(self.inv_log_min_value_input.parse::<u64>().unwrap_or(0)),
                    max_value: Some(self.inv_log_max_value_input.parse::<u64>().unwrap_or(0)),
                }
            }
        };
        let maybe_perpetual_distribution = if self.enable_perpetual_distribution {
            // Construct the `TokenPerpetualDistributionV0` from your selected type + function
            let dist_type = match self.perpetual_dist_type {
                PerpetualDistributionIntervalTypeUI::BlockBased => {
                    // parse interval, parse emission
                    // parse distribution function
                    RewardDistributionType::BlockBasedDistribution {
                        interval: self
                            .perpetual_dist_interval_input
                            .parse::<u64>()
                            .unwrap_or(0),
                        function: distribution_function,
                    }
                }
                PerpetualDistributionIntervalTypeUI::EpochBased => {
                    RewardDistributionType::EpochBasedDistribution {
                        interval: self
                            .perpetual_dist_interval_input
                            .parse::<u16>()
                            .unwrap_or(0),
                        function: distribution_function,
                    }
                }
                PerpetualDistributionIntervalTypeUI::TimeBased => {
                    RewardDistributionType::TimeBasedDistribution {
                        interval: self
                            .perpetual_dist_interval_input
                            .parse::<u64>()
                            .unwrap_or(0),
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
            new_tokens_destination_identity: if self.new_tokens_destination_identity_enabled {
                Some(
                    Identifier::from_string(
                        &self.new_tokens_destination_identity,
                        Encoding::Base58,
                    )
                    .unwrap_or_default(),
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

    /// Attempt to parse all group UI data into a BTreeMap<u16, Group>.
    /// Returns an error if any row fails or duplicates a position, etc.
    pub fn parse_groups(&self) -> Result<BTreeMap<u16, Group>, String> {
        let mut map = BTreeMap::new();
        for (i, g) in self.groups_ui.iter().enumerate() {
            let (pos, group) = g
                .parse_into_group()
                .map_err(|e| format!("Error in Group #{}: {e}", i + 1))?;

            // Check for duplicates
            if map.contains_key(&pos) {
                return Err(format!(
                    "Duplicate group position {pos} in Group #{}",
                    i + 1
                ));
            }

            map.insert(pos, group);
        }
        Ok(map)
    }

    fn reset_token_creator(&mut self) {
        self.selected_identity = None;
        self.selected_key = None;
        self.token_creator_status = TokenCreatorStatus::NotStarted;
        self.token_names_input = vec![(String::new(), TokenNameLanguage::English)];
        self.contract_keywords_input = "".to_string();
        self.token_description_input = "".to_string();
        self.decimals_input = "8".to_string();
        self.base_supply_input = "100000".to_string();
        self.max_supply_input = "".to_string();
        self.start_as_paused_input = false;
        self.should_capitalize_input = false;
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
        self.new_tokens_destination_identity_enabled = false;
        self.new_tokens_destination_identity_rules = ChangeControlRulesUI::default();
        self.new_tokens_destination_identity = "".to_string();
        self.minting_allow_choosing_destination = false;
        self.minting_allow_choosing_destination_rules = ChangeControlRulesUI::default();

        self.show_token_creator_confirmation_popup = false;
        self.token_creator_error_message = None;
    }

    fn render_no_owned_tokens(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        ui.vertical_centered(|ui| {
            ui.add_space(20.0);
            match self.tokens_subscreen {
                TokensSubscreen::MyTokens => {
                    ui.label(
                        RichText::new("No tracked tokens.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                TokensSubscreen::SearchTokens => {
                    ui.label(
                        RichText::new("No matching tokens found.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                TokensSubscreen::TokenCreator => {
                    ui.label(
                        RichText::new("Cannot render token creator for some reason")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
            }
            ui.add_space(10.0);

            ui.label("Please check back later or try refreshing the list.");
            ui.add_space(20.0);
            if ui.button("Refresh").clicked() {
                if let RefreshingStatus::Refreshing(_) = self.refreshing_status {
                    app_action = AppAction::None;
                } else {
                    let now = Utc::now().timestamp() as u64;
                    self.refreshing_status = RefreshingStatus::Refreshing(now);
                    match self.tokens_subscreen {
                        TokensSubscreen::MyTokens => {
                            app_action = AppAction::BackendTask(BackendTask::TokenTask(
                                TokenTask::QueryMyTokenBalances,
                            ));
                        }
                        TokensSubscreen::SearchTokens => {
                            app_action = AppAction::Refresh;
                        }
                        TokensSubscreen::TokenCreator => {
                            app_action = AppAction::Refresh;
                        }
                    }
                }
            }
        });

        app_action
    }

    fn add_token_to_my_tokens(&mut self, token_info: TokenInfo) -> AppAction {
        let mut action = AppAction::None;
        let mut tokens = Vec::new();
        for identity in self
            .app_context
            .load_local_qualified_identities()
            .expect("Expected to load identities")
        {
            let identity_token_balance = IdentityTokenBalance {
                token_id: token_info.token_id,
                token_name: token_info.token_name.clone(),
                identity_id: identity.identity.id(),
                balance: 0,
                estimated_unclaimed_rewards: None,
                data_contract_id: token_info.data_contract_id,
                token_position: token_info.token_position,
            };

            tokens.push(identity_token_balance);
        }
        let my_tokens_clone = self.my_tokens.clone();

        // Prevent duplicates
        for itb in tokens {
            if !my_tokens_clone
                .values()
                .any(|t| t.token_id == itb.token_id && t.identity_id == itb.identity_id)
            {
                let _ = self.app_context.insert_token_identity_balance(
                    &itb.token_id,
                    &itb.identity_id,
                    0,
                );
                action |=
                    AppAction::BackendTask(BackendTask::TokenTask(TokenTask::QueryMyTokenBalances));
                self.display_message("Added token", MessageType::Success);
            } else {
                self.display_message("Token already added", MessageType::Error);
            }
        }

        // Save the new order
        self.save_current_order();

        action
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
            let query_string = self
                .token_search_query
                .as_ref()
                .map(|s| s.clone())
                .unwrap_or_default();

            return AppAction::BackendTask(BackendTask::TokenTask(
                TokenTask::QueryDescriptionsByKeyword(query_string, Some(next_cursor)),
            ));
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
                let query_string = self
                    .token_search_query
                    .as_ref()
                    .map(|s| s.clone())
                    .unwrap_or_default();
                return AppAction::BackendTask(BackendTask::TokenTask(
                    TokenTask::QueryDescriptionsByKeyword(query_string, Some(prev_cursor)),
                ));
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
                    token_to_remove.token_name,
                    token_to_remove.identity_id.to_string(Encoding::Base58)
                ));

                // Confirm button
                if ui.button("Confirm").clicked() {
                    if let Err(e) = self.app_context.remove_token_balance(
                        token_to_remove.token_id,
                        token_to_remove.identity_id.clone(),
                    ) {
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
            Some(token) => token.clone(),
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
                    "Are you sure you want to stop tracking the token \"{}\"? You can re-add it later. Your actual token balance will not change with this action",
                    token_name,
                ));

                // Confirm button
                if ui.button("Confirm").clicked() {
                    for identity in self
                        .app_context
                        .load_local_qualified_identities()
                        .expect("Expected to load local qualified identities")
                    {
                        if let Err(e) = self.app_context.remove_token_balance(
                            token_to_remove.clone(),
                            identity.identity.id().clone(),
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
                }

                // Cancel button
                if ui.button("Cancel").clicked() {
                    self.confirm_remove_token_popup = false;
                    self.token_to_remove = None;
                }
            });

        // If user closes the popup window (the [x] button), also reset state
        if !is_open {
            self.confirm_remove_identity_token_balance_popup = false;
            self.identity_token_balance_to_remove = None;
        }
    }
}

// ─────────────────────────────────────────────────────────────────
// ScreenLike implementation
// ─────────────────────────────────────────────────────────────────
impl ScreenLike for TokensScreen {
    fn refresh(&mut self) {
        self.my_tokens = self
            .app_context
            .identity_token_balances()
            .unwrap_or_default();

        self.all_known_tokens = self
            .app_context
            .db
            .get_all_known_tokens(&self.app_context)
            .unwrap_or_default();

        self.identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .map(|qi| (qi.identity.id(), qi))
            .collect();

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
        self.selected_token_id = None;
        self.my_tokens = self
            .app_context
            .identity_token_balances()
            .unwrap_or_default();

        self.all_known_tokens = self
            .app_context
            .db
            .get_all_known_tokens(&self.app_context)
            .unwrap_or_default();
        self.identities = self
            .app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .map(|qi| (qi.identity.id(), qi))
            .collect();
    }

    fn display_message(&mut self, msg: &str, msg_type: MessageType) {
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
                    | msg.contains("Failed to fetch token balances")
                    | msg.contains("Failed to get estimated rewards")
                {
                    self.backend_message = Some((msg.to_string(), msg_type, Utc::now()));
                    self.refreshing_status = RefreshingStatus::NotRefreshing;
                } else {
                    return;
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
            _ => {}
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;

        self.check_error_expiration();

        // Build top-right buttons
        let right_buttons = match self.tokens_subscreen {
            TokensSubscreen::MyTokens => vec![
                (
                    "Add Token",
                    DesiredAppAction::AddScreenType(ScreenType::AddTokenById),
                ),
                (
                    "Refresh",
                    DesiredAppAction::BackendTask(BackendTask::TokenTask(
                        TokenTask::QueryMyTokenBalances,
                    )),
                ),
            ],
            TokensSubscreen::SearchTokens => vec![],
            TokensSubscreen::TokenCreator => vec![],
        };

        // Top panel
        if let Some(token_id) = self.selected_token_id {
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
                    (&format!("{token_name}"), AppAction::None),
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
        CentralPanel::default().show(ctx, |ui| {
            match self.tokens_subscreen {
                TokensSubscreen::MyTokens => {
                    if self.all_known_tokens.is_empty() {
                        // If no tokens, show a “no tokens found” message
                        action |= self.render_no_owned_tokens(ui);
                    } else {
                        // Are we showing details for a selected token?
                        if self.selected_token_id.is_some() {
                            // Render detail view for one token
                            action |= self.render_token_details(ui);
                        } else {
                            // Otherwise, show the list of all tokens
                            self.render_token_list(ui);
                        }
                    }
                }
                TokensSubscreen::SearchTokens => {
                    if self.selected_contract_id.is_some() {
                        action |=
                            self.render_contract_details(ui, &self.selected_contract_id.unwrap());
                    } else {
                        action |= self.render_keyword_search(ui);
                    }
                }
                TokensSubscreen::TokenCreator => {
                    action |= self.render_token_creator(ctx, ui);
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
                        egui::ScrollArea::vertical()
                            .max_height(600.0)
                            .show(ui, |ui| {
                                let mut cache = CommonMarkCache::default();
                                CommonMarkViewer::new().show(ui, &mut cache, &info_text);
                            });

                        if ui.button("Close").clicked() {
                            self.show_pop_up_info = None;
                        }
                    });
            }
        });

        // Post-processing on user actions
        match action {
            AppAction::BackendTask(BackendTask::TokenTask(TokenTask::QueryMyTokenBalances)) => {
                self.refreshing_status =
                    RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
            }
            AppAction::SetMainScreen(_) => {
                self.refreshing_status = RefreshingStatus::NotRefreshing;

                // should put these in a fn
                self.selected_token_id = None;
                self.selected_contract_id = None;
                self.token_search_query = None;
                self.search_current_page = 1;
                self.search_has_next_page = false;
                self.search_results = Arc::new(Mutex::new(vec![]));
                self.selected_contract_id = None;
                self.selected_contract_description = None;

                self.reset_token_creator();
            }
            AppAction::Custom(ref s) if s == "Back to tokens" => {
                self.selected_token_id = None;
            }
            AppAction::Custom(ref s) if s == "Back to tokens from contract" => {
                self.selected_contract_id = None;
            }
            _ => {}
        }

        if action == AppAction::None {
            if let Some(bt) = self.pending_backend_task.take() {
                action = AppAction::BackendTask(bt);
            }
        }
        action
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
            self.authorized_group = None;

            self.rules.admin_action_takers =
                AuthorizedActionTakers::Identity(Identifier::default());
            self.admin_identity = Some("CCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9".to_owned());
            self.admin_group = None;

            self.rules
                .changing_authorized_action_takers_to_no_one_allowed = true;
            self.rules.changing_admin_action_takers_to_no_one_allowed = true;
            self.rules.self_changing_admin_action_takers_allowed = true;
        }
    }

    #[test]
    fn test_token_creator_ui_builds_correct_contract() {
        let db_file_path = "test_db";
        let db = Arc::new(Database::new(&db_file_path).unwrap());
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
        let mock = Identity::create_basic_identity(test_identity_id, app_context.platform_version)
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
        let mock_key = IdentityPublicKey::random_key(0, None, app_context.platform_version);
        token_creator_ui.selected_key = Some(mock_key);

        // Basic token info
        token_creator_ui.token_names_input =
            vec![("AcmeCoin".to_string(), TokenNameLanguage::English)];
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
        token_creator_ui.new_tokens_destination_identity_enabled = true;
        token_creator_ui.new_tokens_destination_identity =
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
                group_position_str: "2".to_string(),
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
                group_position_str: "7".to_string(),
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
        assert_eq!(*token_pos as u16, 0, "Should be at position 0 by default");

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
        assert_eq!(keeps_history_v0.keeps_transfer_history, true);
        assert_eq!(keeps_history_v0.keeps_freezing_history, true);
        assert_eq!(token_v0.base_supply, 5_000_000);
        assert_eq!(token_v0.max_supply, Some(10_000_000));
        assert_eq!(token_v0.start_as_paused, true);
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
        assert_eq!(dist_rules_v0.minting_allow_choosing_destination, true);

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
        let db = Arc::new(Database::new(&db_file_path).unwrap());
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
        let mock = Identity::create_basic_identity(test_identity_id, app_context.platform_version)
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
        let mock_key = IdentityPublicKey::random_key(0, None, app_context.platform_version);
        token_creator_ui.selected_key = Some(mock_key);

        token_creator_ui.token_names_input =
            vec![("TestToken".to_owned(), TokenNameLanguage::English)];

        // Enable perpetual distribution, select Random
        token_creator_ui.enable_perpetual_distribution = true;
        token_creator_ui.perpetual_dist_type = PerpetualDistributionIntervalTypeUI::TimeBased;
        token_creator_ui.perpetual_dist_interval_input = "60000".to_string();
        token_creator_ui.perpetual_dist_function = DistributionFunctionUI::Random;
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

        let TokenConfiguration::V0(ref token_v0) = contract_v1.tokens[&(0u16.into())];
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
        let db = Arc::new(Database::new(&db_file_path).unwrap());
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
        let mock = Identity::create_basic_identity(test_identity_id, app_context.platform_version)
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
        let mock_key = IdentityPublicKey::random_key(0, None, app_context.platform_version);
        token_creator_ui.selected_key = Some(mock_key);

        // Intentionally leave token_name_input empty
        token_creator_ui.token_names_input = vec![];

        let err = token_creator_ui
            .parse_token_build_args()
            .expect_err("Should fail if token name is empty");
        assert_eq!(err, "Please enter a token name");
    }
}
