use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{DateTime, Utc};
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::v0::TokenDistributionRulesV0;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
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
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::identity::SecurityLevel;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use eframe::egui::{self, CentralPanel, Color32, Context, Frame, Margin, Ui};
use egui::{Align, RichText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use egui_extras::{Column, TableBuilder};
use crate::app::BackendTasksExecutionMode;
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
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, Screen, ScreenLike};
use ordered_float::NotNan;

use super::burn_tokens_screen::BurnTokensScreen;
use super::destroy_frozen_funds_screen::DestroyFrozenFundsScreen;
use super::freeze_tokens_screen::FreezeTokensScreen;
use super::mint_tokens_screen::MintTokensScreen;
use super::pause_tokens_screen::PauseTokensScreen;
use super::resume_tokens_screen::ResumeTokensScreen;
use super::transfer_tokens_screen::TransferTokensScreen;
use super::unfreeze_tokens_screen::UnfreezeTokensScreen;

/// A token owned by an identity.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityTokenBalance {
    pub token_identifier: Identifier,
    pub token_name: String,
    pub identity_id: Identifier,
    pub balance: u64,
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
pub enum TokenSearchStatus {
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
    pub authorized: AuthorizedActionTakers,
    pub authorized_identity: Option<String>,
    pub authorized_group: Option<String>,

    pub admin_action_takers: AuthorizedActionTakers,
    pub admin_identity: Option<String>,
    pub admin_group: Option<String>,

    pub changing_authorized_action_takers_to_no_one_allowed: bool,
    pub changing_admin_action_takers_to_no_one_allowed: bool,
    pub self_changing_admin_action_takers_allowed: bool,
}

impl ChangeControlRulesUI {
    /// Renders the UI for a single action’s configuration (mint, burn, freeze, etc.)
    pub fn render_control_change_rules_ui(&mut self, ui: &mut egui::Ui, action_name: &str) {
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
                            .selected_text(self.authorized.to_string())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.authorized,
                                    AuthorizedActionTakers::NoOne,
                                    "No One",
                                );
                                ui.selectable_value(
                                    &mut self.authorized,
                                    AuthorizedActionTakers::ContractOwner,
                                    "Contract Owner",
                                );
                                ui.selectable_value(
                                    &mut self.authorized,
                                    AuthorizedActionTakers::Identity(Identifier::default()),
                                    "Identity",
                                );
                                ui.selectable_value(
                                    &mut self.authorized,
                                    AuthorizedActionTakers::MainGroup,
                                    "Main Group",
                                );
                                ui.selectable_value(
                                    &mut self.authorized,
                                    AuthorizedActionTakers::Group(0),
                                    "Group",
                                );
                            });

                        // If user selected Identity or Group, show text edit
                        match &mut self.authorized {
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
                            .selected_text(self.admin_action_takers.to_string())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.admin_action_takers,
                                    AuthorizedActionTakers::NoOne,
                                    "No One",
                                );
                                ui.selectable_value(
                                    &mut self.admin_action_takers,
                                    AuthorizedActionTakers::ContractOwner,
                                    "Contract Owner",
                                );
                                ui.selectable_value(
                                    &mut self.admin_action_takers,
                                    AuthorizedActionTakers::Identity(Identifier::default()),
                                    "Identity",
                                );
                                ui.selectable_value(
                                    &mut self.admin_action_takers,
                                    AuthorizedActionTakers::MainGroup,
                                    "Main Group",
                                );
                                ui.selectable_value(
                                    &mut self.admin_action_takers,
                                    AuthorizedActionTakers::Group(0),
                                    "Group",
                                );
                            });

                        match &mut self.admin_action_takers {
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
                        &mut self.changing_authorized_action_takers_to_no_one_allowed,
                        "Changing authorized action takers to no one allowed",
                    );
                    ui.end_row();

                    ui.checkbox(
                        &mut self.changing_admin_action_takers_to_no_one_allowed,
                        "Changing admin action takers to no one allowed",
                    );
                    ui.end_row();

                    ui.checkbox(
                        &mut self.self_changing_admin_action_takers_allowed,
                        "Self-changing admin action takers allowed",
                    );
                    ui.end_row();
                });

            ui.add_space(3.0);
        });
    }

    pub fn to_change_control_rules(
        &mut self,
        action_name: &str,
    ) -> Result<ChangeControlRules, String> {
        // 1) Update self.authorized if it’s Identity or Group
        match self.authorized {
            AuthorizedActionTakers::Identity(_) => {
                if let Some(ref id_str) = self.authorized_identity {
                    let parsed =
                        Identifier::from_string(id_str, Encoding::Base58).map_err(|_| {
                            format!(
                                "Invalid base58 identifier for {} authorized identity",
                                action_name
                            )
                        })?;
                    self.authorized = AuthorizedActionTakers::Identity(parsed);
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
                    self.authorized = AuthorizedActionTakers::Group(parsed);
                }
            }
            _ => {}
        }

        // 2) Update self.admin_action_takers if it’s Identity or Group
        match self.admin_action_takers {
            AuthorizedActionTakers::Identity(_) => {
                if let Some(ref id_str) = self.admin_identity {
                    let parsed =
                        Identifier::from_string(id_str, Encoding::Base58).map_err(|_| {
                            format!(
                                "Invalid base58 identifier for {} admin identity",
                                action_name
                            )
                        })?;
                    self.admin_action_takers = AuthorizedActionTakers::Identity(parsed);
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
                    self.admin_action_takers = AuthorizedActionTakers::Group(parsed);
                }
            }
            _ => {}
        }

        // 3) Construct the ChangeControlRules
        let rules = ChangeControlRules::V0(ChangeControlRulesV0 {
            authorized_to_make_change: self.authorized.clone(),
            admin_action_takers: self.admin_action_takers.clone(),
            changing_authorized_action_takers_to_no_one_allowed: self
                .changing_authorized_action_takers_to_no_one_allowed,
            changing_admin_action_takers_to_no_one_allowed: self
                .changing_admin_action_takers_to_no_one_allowed,
            self_changing_admin_action_takers_allowed: self
                .self_changing_admin_action_takers_allowed,
        });

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
#[derive(Debug, Clone, PartialEq)]
pub enum DistributionFunctionUI {
    FixedAmount,
    StepDecreasingAmount,
    LinearInteger,
    LinearFloat,
    PolynomialInteger,
    PolynomialFloat,
    Exponential,
    Logarithmic,
    Stepwise,
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
    /// The block timestamp or block height when distribution occurs
    pub timestamp_str: String,

    /// The base58 identity to receive distribution
    pub identity_str: String,

    /// The distribution amount
    pub amount_str: String,
}

/// The main, combined TokensScreen:
/// - Displays token balances or a search UI
/// - Allows reordering of tokens if desired
pub struct TokensScreen {
    pub app_context: Arc<AppContext>,
    pub tokens_subscreen: TokensSubscreen,
    my_tokens: Arc<Mutex<Vec<IdentityTokenBalance>>>,
    pub selected_token_id: Option<Identifier>,
    show_token_info: Option<Identifier>,
    backend_message: Option<(String, MessageType, DateTime<Utc>)>,
    pending_backend_task: Option<BackendTask>,
    refreshing_status: RefreshingStatus,

    // Token Search
    token_search_query: Option<String>,
    search_results: Arc<Mutex<Vec<IdentityTokenBalance>>>,
    token_search_status: TokenSearchStatus,
    search_current_page: usize,
    search_has_next_page: bool,
    next_cursors: Vec<Identifier>,
    previous_cursors: Vec<Identifier>,

    /// Sorting
    sort_column: SortColumn,
    sort_order: SortOrder,
    use_custom_order: bool,

    // Remove token
    confirm_remove_identity_token_balance_popup: bool,
    identity_token_balance_to_remove: Option<IdentityTokenBalance>,
    confirm_remove_token_popup: bool,
    token_to_remove: Option<Identifier>,

    /// Token Creator
    show_pop_up_info: Option<String>,
    selected_identity: Option<QualifiedIdentity>,
    selected_key: Option<IdentityPublicKey>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_password: String,
    show_password: bool,
    token_name_input: String,
    should_capitalize_input: bool,
    decimals_input: String,
    base_supply_input: String,
    max_supply_input: String,
    start_as_paused_input: bool,
    main_control_group_input: String,
    show_token_creator_confirmation_popup: bool,
    token_creator_status: TokenCreatorStatus,
    token_creator_error_message: Option<String>,
    token_keeps_history: bool,

    // Action Rules
    manual_minting_rules: ChangeControlRulesUI,
    manual_burning_rules: ChangeControlRulesUI,
    freeze_rules: ChangeControlRulesUI,
    unfreeze_rules: ChangeControlRulesUI,
    destroy_frozen_funds_rules: ChangeControlRulesUI,
    emergency_action_rules: ChangeControlRulesUI,
    max_supply_change_rules: ChangeControlRulesUI,
    conventions_change_rules: ChangeControlRulesUI,

    // Main control group change rules
    authorized_main_control_group_change: AuthorizedActionTakers,
    main_control_group_change_authorized_identity: Option<String>,
    main_control_group_change_authorized_group: Option<String>,

    /// Distribution (perpetual) toggles/fields
    pub enable_perpetual_distribution: bool,
    pub perpetual_distribution_rules: ChangeControlRulesUI,

    // Which distribution interval type is selected?
    pub perpetual_dist_type: PerpetualDistributionIntervalTypeUI,

    // Block-based / time-based / epoch-based inputs
    pub perpetual_dist_interval_input: String,

    // Which distribution is currently selected?
    pub perpetual_dist_function: DistributionFunctionUI,

    // --- FixedAmount ---
    pub fixed_amount_input: String,

    // --- StepDecreasingAmount ---
    pub step_count_input: String,
    pub decrease_per_interval_input: String,
    pub step_dec_amount_input: String,

    // --- LinearInteger ---
    pub linear_int_a_input: String,
    pub linear_int_b_input: String,

    // --- LinearFloat ---
    pub linear_float_a_input: String,
    pub linear_float_b_input: String,

    // --- PolynomialInteger ---
    pub poly_int_a_input: String,
    pub poly_int_n_input: String,
    pub poly_int_b_input: String,

    // --- PolynomialFloat ---
    pub poly_float_a_input: String,
    pub poly_float_n_input: String,
    pub poly_float_b_input: String,

    // --- Exponential ---
    pub exp_a_input: String,
    pub exp_b_input: String,
    pub exp_c_input: String,

    // --- Logarithmic ---
    pub log_a_input: String,
    pub log_b_input: String,
    pub log_c_input: String,

    // --- Stepwise ---
    // If you want multiple (block, amount) pairs, store them in a Vec.
    // Each tuple in the Vec can be (String, String) for block + amount
    // or however you prefer to represent it.
    pub stepwise_steps: Vec<(String, String)>,

    // Similarly for identity recipients, you might store:
    pub perpetual_dist_recipient: TokenDistributionRecipientUI,
    pub perpetual_dist_recipient_identity_input: Option<String>,

    /// Pre-programmed distribution
    pub enable_pre_programmed_distribution: bool,
    pub pre_programmed_distributions: Vec<DistributionEntry>,

    // new_tokens_destination_identity
    pub new_tokens_destination_identity_enabled: bool,
    pub new_tokens_destination_identity: String,
    pub new_tokens_destination_identity_rules: ChangeControlRulesUI,

    // minting_allow_choosing_destination
    pub minting_allow_choosing_destination: bool,
    pub minting_allow_choosing_destination_rules: ChangeControlRulesUI,
}

impl TokensScreen {
    pub fn new(app_context: &Arc<AppContext>, tokens_subscreen: TokensSubscreen) -> Self {
        let my_tokens = Arc::new(Mutex::new(
            app_context.identity_token_balances().unwrap_or_default(),
        ));

        let mut screen = Self {
            app_context: app_context.clone(),
            my_tokens,
            selected_token_id: None,
            show_token_info: None,
            token_search_query: None,
            token_search_status: TokenSearchStatus::NotStarted,
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
            show_pop_up_info: None,
            selected_identity: None,
            selected_key: None,
            selected_wallet: None,
            wallet_password: String::new(),
            show_password: false,
            show_token_creator_confirmation_popup: false,
            token_creator_status: TokenCreatorStatus::NotStarted,
            token_creator_error_message: None,
            token_name_input: String::new(),
            should_capitalize_input: false,
            decimals_input: 8.to_string(),
            base_supply_input: TokenConfigurationV0::default_most_restrictive()
                .base_supply()
                .to_string(),
            max_supply_input: String::new(),
            start_as_paused_input: false,
            token_keeps_history: false,
            main_control_group_input: String::new(),

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
            step_count_input: String::new(),
            decrease_per_interval_input: String::new(),
            step_dec_amount_input: String::new(),
            linear_int_a_input: String::new(),
            linear_int_b_input: String::new(),
            linear_float_a_input: String::new(),
            linear_float_b_input: String::new(),
            poly_int_a_input: String::new(),
            poly_int_n_input: String::new(),
            poly_int_b_input: String::new(),
            poly_float_a_input: String::new(),
            poly_float_n_input: String::new(),
            poly_float_b_input: String::new(),
            exp_a_input: String::new(),
            exp_b_input: String::new(),
            exp_c_input: String::new(),
            log_a_input: String::new(),
            log_b_input: String::new(),
            log_c_input: String::new(),
            stepwise_steps: Vec::new(),

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
    fn reorder_vec_to(&self, new_order: Vec<(Identifier, Identifier)>) {
        let mut lock = self.my_tokens.lock().unwrap();
        for (desired_idx, (token_id, identity_id)) in new_order.iter().enumerate() {
            if let Some(current_idx) = lock
                .iter()
                .position(|t| t.token_identifier == *token_id && t.identity_id == *identity_id)
            {
                if current_idx != desired_idx && current_idx < lock.len() {
                    lock.swap(current_idx, desired_idx);
                }
            }
        }
    }

    /// Save the current vector's order of token IDs to the DB
    fn save_current_order(&self) {
        let lock = self.my_tokens.lock().unwrap();
        let all_ids = lock
            .iter()
            .map(|token| (token.token_identifier.clone(), token.identity_id.clone()))
            .collect::<Vec<_>>();
        drop(lock);
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
                SortColumn::TokenID => a.token_identifier.cmp(&b.token_identifier),
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
        tokens: &[IdentityTokenBalance],
    ) -> Vec<(Identifier, String, u64)> {
        let mut map: HashMap<Identifier, (String, u64)> = HashMap::new();
        for tb in tokens {
            let entry = map.entry(tb.token_identifier.clone()).or_insert_with(|| {
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
    fn render_token_list(&mut self, ui: &mut Ui, tokens: &[IdentityTokenBalance]) {
        let mut grouped = self.group_tokens_by_identifier(tokens);
        if !self.use_custom_order {
            self.sort_vec_of_groups(&mut grouped);
        }

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
                    .inner_margin(Margin::same(8.0))
                    .show(ui, |ui| {
                        TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(Align::Center))
                            .column(Column::initial(150.0).resizable(true)) // Token Name
                            .column(Column::initial(200.0).resizable(true)) // Token ID
                            .column(Column::initial(80.0).resizable(true)) // Total Balance
                            .column(Column::initial(80.0).resizable(true)) // Actions
                            // .column(Column::initial(80.0).resizable(true)) // Token Info
                            .header(30.0, |mut header| {
                                header.col(|ui| {
                                    if ui.button("Token Name").clicked() {
                                        self.toggle_sort(SortColumn::TokenName);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Token ID").clicked() {
                                        self.toggle_sort(SortColumn::TokenID);
                                    }
                                });
                                header.col(|ui| {
                                    if ui.button("Total Balance").clicked() {
                                        self.toggle_sort(SortColumn::Balance);
                                    }
                                });
                                header.col(|ui| {
                                    ui.label("Actions");
                                });
                                // header.col(|ui| {
                                //     ui.label("Token Info");
                                // });
                            })
                            .body(|mut body| {
                                for (token_id, token_name, total_balance) in grouped {
                                    body.row(25.0, |mut row| {
                                        row.col(|ui| {
                                            // By making the label into a button or using `ui.selectable_label`,
                                            // we can respond to clicks.
                                            if ui.button(&token_name).clicked() {
                                                self.selected_token_id = Some(token_id.clone());
                                            }
                                        });
                                        row.col(|ui| {
                                            ui.label(token_id.to_string(Encoding::Base58));
                                        });
                                        row.col(|ui| {
                                            ui.label(total_balance.to_string());
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
                                        // row.col(|ui| {
                                        //     if ui.button("Info").clicked() {
                                        //         self.show_token_info = Some(token_id.clone());
                                        //     }
                                        // });
                                    });
                                }
                            });
                    });
            });
    }

    /// Renders details for the selected token_id: a row per identity that holds that token.
    fn render_token_details(&mut self, ui: &mut Ui, tokens: &[IdentityTokenBalance]) -> AppAction {
        let mut action = AppAction::None;

        let token_id = self.selected_token_id.as_ref().unwrap();

        // Filter out only the IdentityTokenBalance for this token_id
        let mut detail_list: Vec<IdentityTokenBalance> = tokens
            .iter()
            .filter(|t| &t.token_identifier == token_id)
            .cloned()
            .collect();
        if !self.use_custom_order {
            self.sort_vec(&mut detail_list);
        }

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
                    .inner_margin(Margin::same(8.0))
                    .show(ui, |ui| {
                        TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(Align::Center))
                            .column(Column::initial(60.0).resizable(true)) // Identity Alias
                            .column(Column::initial(200.0).resizable(true)) // Identity ID
                            .column(Column::initial(60.0).resizable(true)) // Balance
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
                                            ui.label(itb.balance.to_string());
                                        });
                                        row.col(|ui| {
                                            ui.horizontal(|ui| {
                                                ui.spacing_mut().item_spacing.x = 3.0;

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
                                            });
                                        });
                                    });
                                }
                            });
                    });
            });

        action
    }

    fn render_token_search(&mut self, ui: &mut Ui) -> AppAction {
        let action = AppAction::None;

        ui.vertical_centered(|ui| {
            ui.add_space(10.0);
            ui.heading("Coming Soon");
            ui.add_space(5.0);

            //     ui.add_space(10.0);
            //     ui.label("Search for tokens by keyword, name, or ID.");
            //     ui.add_space(5.0);

            //     ui.horizontal(|ui| {
            //         ui.label("Search by keyword(s):");
            //         ui.text_edit_singleline(self.token_search_query.get_or_insert_with(String::new));
            //         if ui.button("Go").clicked() {
            //             // 1) Clear old results, set status
            //             let now = Utc::now().timestamp() as u64;
            //             self.token_search_status = TokenSearchStatus::WaitingForResult(now);
            //             {
            //                 let mut sr = self.search_results.lock().unwrap();
            //                 sr.clear();
            //             }
            //             self.search_current_page = 1;
            //             self.next_cursors.clear();
            //             self.previous_cursors.clear();
            //             self.search_has_next_page = false;

            //             // 2) Dispatch backend request
            //             let query_string = self
            //                 .token_search_query
            //                 .as_ref()
            //                 .map(|s| s.clone())
            //                 .unwrap_or_default();

            //             // Example: if you want paged results from the start:
            //             action = AppAction::BackendTask(BackendTask::TokenTask(
            //                 TokenTask::QueryTokensByKeywordPage(query_string, None),
            //             ));
            //         }
            //     });
        });

        // ui.separator();
        // ui.add_space(10.0);

        // // Show results or messages
        // match self.token_search_status {
        //     TokenSearchStatus::WaitingForResult(start_time) => {
        //         let now = Utc::now().timestamp() as u64;
        //         let elapsed = now - start_time;
        //         ui.label(format!("Searching... Time so far: {} seconds", elapsed));
        //         ui.add(egui::widgets::Spinner::default().color(Color32::from_rgb(0, 128, 255)));
        //     }
        //     TokenSearchStatus::Complete => {
        //         // Render the results table
        //         let tokens = self.search_results.lock().unwrap().clone();
        //         if tokens.is_empty() {
        //             ui.label("No tokens match your search.");
        //         } else {
        //             // Possibly add a filter input above the table, if you like
        //             action |= self.render_search_results_table(ui, &tokens);
        //         }

        //         // Then pagination controls
        //         ui.horizontal(|ui| {
        //             // If not on page 1, we can show a “Prev” button
        //             if self.search_current_page > 1 {
        //                 if ui.button("Previous Page").clicked() {
        //                     action |= self.goto_previous_search_page();
        //                 }
        //             }

        //             ui.label(format!("Page {}", self.search_current_page));

        //             // If has_next_page, show “Next Page” button
        //             if self.search_has_next_page {
        //                 if ui.button("Next Page").clicked() {
        //                     action |= self.goto_next_search_page();
        //                 }
        //             }
        //         });
        //     }
        //     TokenSearchStatus::ErrorMessage(ref e) => {
        //         ui.colored_label(Color32::DARK_RED, format!("Error: {}", e));
        //     }
        //     TokenSearchStatus::NotStarted => {
        //         ui.label("Enter keywords above and click Go to search tokens.");
        //     }
        // }

        action
    }

    fn render_search_results_table(
        &mut self,
        ui: &mut Ui,
        search_results: &[IdentityTokenBalance],
    ) -> AppAction {
        let action = AppAction::None;

        // In your DocumentQueryScreen code, you also had a ScrollArea
        egui::ScrollArea::vertical().show(ui, |ui| {
            Frame::group(ui.style())
                .fill(ui.visuals().panel_fill)
                .stroke(egui::Stroke::new(
                    1.0,
                    ui.visuals().widgets.inactive.bg_stroke.color,
                ))
                .inner_margin(Margin::same(8.0))
                .show(ui, |ui| {
                    TableBuilder::new(ui)
                        .striped(true)
                        .resizable(true)
                        .cell_layout(egui::Layout::left_to_right(Align::Center))
                        .column(Column::initial(80.0).resizable(true)) // Token Name
                        .column(Column::initial(330.0).resizable(true)) // Identity
                        .column(Column::initial(60.0).resizable(true)) // Balance
                        .column(Column::initial(80.0).resizable(true)) // Action
                        .header(30.0, |mut header| {
                            header.col(|ui| {
                                if ui.button("Token Name").clicked() {
                                    self.toggle_sort(SortColumn::TokenName);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Token ID").clicked() {
                                    self.toggle_sort(SortColumn::TokenID);
                                }
                            });
                            header.col(|ui| {
                                if ui.button("Balance").clicked() {
                                    self.toggle_sort(SortColumn::Balance);
                                }
                            });
                            header.col(|ui| {
                                ui.label("Action");
                            });
                        })
                        .body(|mut body| {
                            for token in search_results {
                                body.row(25.0, |mut row| {
                                    row.col(|ui| {
                                        ui.label(&token.token_name);
                                    });
                                    row.col(|ui| {
                                        ui.label(token.identity_id.to_string(Encoding::Base58));
                                    });
                                    row.col(|ui| {
                                        ui.label(token.balance.to_string());
                                    });
                                    row.col(|ui| {
                                        if ui.button("Add").clicked() {
                                            // Add to my_tokens
                                            self.add_token_to_my_tokens(token.clone());
                                        }
                                    });
                                });
                            }
                        });
                });
        });

        action
    }

    pub fn render_token_creator(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = AppAction::None;

        // 1) If we've successfully completed contract creation, show a success UI
        if self.token_creator_status == TokenCreatorStatus::Complete {
            self.render_token_creator_success_screen(ui);
            return action;
        }

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
                    // 2) Choose identity & key
                    //    We'll show a dropdown of local QualifiedIdentities, then a sub-dropdown of keys

                    // Identity selection
                    ui.add_space(10.0);
                    let all_identities = match self.app_context.load_local_qualified_identities() {
                        Ok(ids) => ids,
                        Err(_) => {
                            ui.colored_label(egui::Color32::RED, "Error loading identities from local DB");
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
                    ui.add_space(10.0);

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
                            ui.label("Token Name (singular):");
                            ui.text_edit_singleline(&mut self.token_name_input);
                            ui.end_row();

                            // Row 2: Base Supply
                            ui.label("Base Supply:");
                            ui.text_edit_singleline(&mut self.base_supply_input);
                            ui.end_row();

                            // Row 3: Max Supply
                            ui.label("Max Supply (optional):");
                            ui.text_edit_singleline(&mut self.max_supply_input);
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
                                ui.checkbox(&mut self.start_as_paused_input, "Start as paused");
                                ui.end_row();

                                // 1) Keep history?
                                ui.checkbox(&mut self.token_keeps_history, "Keep history");
                                ui.end_row();

                                // Name should be capitalized
                                ui.checkbox(
                                    &mut self.should_capitalize_input,
                                    "Name should be capitalized",
                                );
                                ui.end_row();

                                // Decimals
                                ui.horizontal(|ui| {
                                    ui.label("Decimals:");
                                    ui.text_edit_singleline(&mut self.decimals_input);
                                });
                                ui.end_row();

                                // Add main group selection input
                                ui.horizontal(|ui| {
                                    ui.label("Main Control Group Position:");
                                    ui.text_edit_singleline(&mut self.main_control_group_input);
                                });
                                ui.end_row();
                            });
                    });

                    ui.add_space(5.0);

                    ui.collapsing("Action Rules", |ui| {
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
                            self.enable_pre_programmed_distribution = false;
                            self.pre_programmed_distributions = Vec::new();
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
                                            DistributionFunctionUI::StepDecreasingAmount,
                                            "StepDecreasing",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::LinearInteger,
                                            "LinearInteger",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::LinearFloat,
                                            "LinearFloat",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::PolynomialInteger,
                                            "PolynomialInteger",
                                        );
                                        ui.selectable_value(
                                            &mut self.perpetual_dist_function,
                                            DistributionFunctionUI::PolynomialFloat,
                                            "PolynomialFloat",
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
                                            DistributionFunctionUI::Stepwise,
                                            "Stepwise",
                                        );
                                    });

                                    let info_icon = egui::Label::new("ℹ").sense(egui::Sense::click());
                                    let response = ui.add(info_icon).on_hover_text("Info about distribution types");

                                    // Check if the label was clicked
                                    if response.clicked() {
                                        self.show_pop_up_info = Some(r#"
### FixedAmount

A fixed amount of tokens is emitted for each period.

- **Formula:** f(x) = n  
- **Use Case:** Simplicity, stable reward emissions  
- **Example:** If we emit 5 tokens per block, and 3 blocks have passed, 15 tokens have been released.

---

### StepDecreasingAmount

The amount of tokens decreases in predefined steps at fixed intervals.

- **Formula:** f(x) = n * (1 - decrease_per_interval)^(x / step_count)  
- **Use Case:** Mimics Bitcoin/Dash models, encourages early participation  
- **Example:** Bitcoin halves every 210,000 blocks (~4 years)

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

                            // Based on the user’s chosen function, display relevant fields:
                            match self.perpetual_dist_function {
                                DistributionFunctionUI::FixedAmount => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Fixed Amount per Interval:");
                                        ui.text_edit_singleline(&mut self.fixed_amount_input);
                                    });
                                }

                                DistributionFunctionUI::StepDecreasingAmount => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Step Count (u64):");
                                        ui.text_edit_singleline(&mut self.step_count_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Decrease per Interval (float):");
                                        ui.text_edit_singleline(&mut self.decrease_per_interval_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Initial Amount (n):");
                                        ui.text_edit_singleline(&mut self.step_dec_amount_input);
                                    });
                                }

                                DistributionFunctionUI::LinearInteger => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Coefficient (a, i64):");
                                        ui.text_edit_singleline(&mut self.linear_int_a_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Initial Value (b, i64):");
                                        ui.text_edit_singleline(&mut self.linear_int_b_input);
                                    });
                                }

                                DistributionFunctionUI::LinearFloat => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Coefficient (a, float):");
                                        ui.text_edit_singleline(&mut self.linear_float_a_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Initial Value (b):");
                                        ui.text_edit_singleline(&mut self.linear_float_b_input);
                                    });
                                }

                                DistributionFunctionUI::PolynomialInteger => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Coefficient (a, i64):");
                                        ui.text_edit_singleline(&mut self.poly_int_a_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Degree (n, i64):");
                                        ui.text_edit_singleline(&mut self.poly_int_n_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Base Amount (b, i64):");
                                        ui.text_edit_singleline(&mut self.poly_int_b_input);
                                    });
                                }

                                DistributionFunctionUI::PolynomialFloat => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Coefficient (a, float):");
                                        ui.text_edit_singleline(&mut self.poly_float_a_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Degree (n, float):");
                                        ui.text_edit_singleline(&mut self.poly_float_n_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Base Amount (b):");
                                        ui.text_edit_singleline(&mut self.poly_float_b_input);
                                    });
                                }

                                DistributionFunctionUI::Exponential => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Scaling Factor (a, float):");
                                        ui.text_edit_singleline(&mut self.exp_a_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Growth/Decay Rate (b, float):");
                                        ui.text_edit_singleline(&mut self.exp_b_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Offset (c):");
                                        ui.text_edit_singleline(&mut self.exp_c_input);
                                    });
                                }

                                DistributionFunctionUI::Logarithmic => {
                                    ui.horizontal(|ui| {
                                        ui.label("        - Scaling Factor (a, float):");
                                        ui.text_edit_singleline(&mut self.log_a_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Log Base (b, float):");
                                        ui.text_edit_singleline(&mut self.log_b_input);
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("        - Offset (c):");
                                        ui.text_edit_singleline(&mut self.log_c_input);
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
                        if ui
                            .checkbox(
                                &mut self.enable_pre_programmed_distribution,
                                "Enable Pre-Programmed Distribution",
                            )
                            .clicked()
                        {
                            // If user turns this on, you could clear the other distribution type
                            self.enable_perpetual_distribution = false;
                            self.perpetual_dist_type = PerpetualDistributionIntervalTypeUI::None;
                        };

                        if self.enable_pre_programmed_distribution {
                            ui.add_space(2.0);

                            let mut i = 0;
                            while i < self.pre_programmed_distributions.len() {
                                // Clone the current entry
                                let mut entry = self.pre_programmed_distributions[i].clone();

                                // Render row
                                ui.horizontal(|ui| {
                                    ui.label(format!("      Timestamp #{}:", i + 1));
                                    ui.text_edit_singleline(&mut entry.timestamp_str);

                                    ui.label("Identity:");
                                    ui.text_edit_singleline(&mut entry.identity_str);

                                    ui.label("Amount:");
                                    ui.text_edit_singleline(&mut entry.amount_str);

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
                                    self.pre_programmed_distributions
                                        .push(DistributionEntry::default());
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

                    // 6) "Register Token Contract" button
                    ui.add_space(10.0);
                    let mut new_style = (**ui.style()).clone();
                    new_style.spacing.button_padding = egui::vec2(10.0, 5.0);
                    ui.set_style(new_style);
                    let button =
                        egui::Button::new(RichText::new("Register Token Contract").color(Color32::WHITE))
                            .fill(Color32::from_rgb(0, 128, 255))
                            .frame(true)
                            .rounding(3.0);
                    if ui.add(button).clicked() {
                        if self.selected_key.is_none() || self.selected_identity.is_none() {
                            self.token_creator_error_message = Some("Please select an identity and key".to_string());
                        } else if self.token_name_input.len() == 0 {
                            self.token_creator_error_message = Some("Please enter a token name".to_string());
                        } else {
                            // Validate input & if valid, show confirmation
                            self.token_creator_error_message = None;
                            self.show_token_creator_confirmation_popup = true;
                        }
                    };
                });
        });

        // 7) If the user pressed "Register Token Contract," show a popup confirmation
        if self.show_token_creator_confirmation_popup {
            action |= self.render_token_creator_confirmation_popup(ui);
        }

        // 8) If we are waiting, show spinner / time elapsed
        if let TokenCreatorStatus::WaitingForResult(start_time) = self.token_creator_status {
            let now = chrono::Utc::now().timestamp() as u64;
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
            ui.colored_label(egui::Color32::RED, format!("{err_msg}"));
            ui.add_space(10.0);
        }

        action
    }

    /// Shows a popup "Are you sure?" for creating the token contract
    fn render_token_creator_confirmation_popup(&mut self, ui: &mut egui::Ui) -> AppAction {
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
                    self.token_name_input,
                    self.base_supply_input,
                    max_supply_display,
                ));

                ui.add_space(10.0);

                // Confirm
                if ui.button("Confirm").clicked() {
                    // Attempt to parse fields
                    let decimals = if let Ok(dec) = self.decimals_input.parse::<u16>() {
                        dec
                    } else {
                        8 // default
                    };
                    let base_supply = if let Ok(base) = self.base_supply_input.parse::<u64>() {
                        base
                    } else {
                        TokenConfigurationV0::default_most_restrictive().base_supply
                    };
                    let max_supply = if let Ok(max) = self.max_supply_input.parse::<u64>() {
                        Some(max)
                    } else {
                        TokenConfigurationV0::default_most_restrictive().max_supply
                    };
                    let main_control_group = if let Ok(group) = self.main_control_group_input.parse::<u16>() {
                        Some(group)
                    } else {
                        TokenConfigurationV0::default_most_restrictive().main_control_group
                    };

                    // We'll switch status to "Waiting"
                    self.token_creator_status =
                        TokenCreatorStatus::WaitingForResult(chrono::Utc::now().timestamp() as u64);
                    self.show_token_creator_confirmation_popup = false;

                    // Build a new DataContract on the fly (or ask the backend task to do it).
                    // For example:
                    let identity = self.selected_identity.clone().unwrap();
                    let key = self.selected_key.clone().unwrap();

                    let start_paused = self.start_as_paused_input;
                    let token_name = self.token_name_input.clone();

                    match self.manual_minting_rules.authorized {
                        AuthorizedActionTakers::Identity(_) => {
                            if let Some(ref id_str) = self.manual_minting_rules.authorized_identity {
                                if let Ok(id) = Identifier::from_string(id_str, Encoding::Base58) {
                                    self.manual_minting_rules.authorized = AuthorizedActionTakers::Identity(id);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid base58 identifier for manual mint authorized identity".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        AuthorizedActionTakers::Group(_) => {
                            if let Some(ref group_str) = self.manual_minting_rules.authorized_group {
                                if let Ok(group) = group_str.parse::<u16>() {
                                    self.manual_minting_rules.authorized = AuthorizedActionTakers::Group(group);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid group contract position for manual mint authorized group".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }
                    match self.manual_minting_rules.admin_action_takers {
                        AuthorizedActionTakers::Identity(_) => {
                            if let Some(ref id_str) = self.manual_minting_rules.admin_identity {
                                if let Ok(id) = Identifier::from_string(id_str, Encoding::Base58) {
                                    self.manual_minting_rules.admin_action_takers = AuthorizedActionTakers::Identity(id);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid base58 identifier for manual mint admin identity".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        AuthorizedActionTakers::Group(_) => {
                            if let Some(ref group_str) = self.manual_minting_rules.admin_group {
                                if let Ok(group) = group_str.parse::<u16>() {
                                    self.manual_minting_rules.admin_action_takers = AuthorizedActionTakers::Group(group);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid group contract position for manual mint admin group".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }

                    // These rules are slightly different than the others (missing the last few fields)
                    // So we do this manually here, while for the others it's handled in the `render_control_change_rules_ui` function.
                    match self.authorized_main_control_group_change {
                        AuthorizedActionTakers::Identity(_) => {
                            if let Some(ref id_str) = self.main_control_group_change_authorized_identity {
                                if let Ok(id) = Identifier::from_string(id_str, Encoding::Base58) {
                                    self.authorized_main_control_group_change = AuthorizedActionTakers::Identity(id);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid base58 identifier for main control group change authorized identity".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        AuthorizedActionTakers::Group(_) => {
                            if let Some(ref group_str) = self.main_control_group_change_authorized_group {
                                if let Ok(group) = group_str.parse::<u16>() {
                                    self.authorized_main_control_group_change = AuthorizedActionTakers::Group(group);
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid group contract position for main control group change authorized group".to_string(),
                                    );
                                    return;
                                }
                            }
                        }
                        _ => {}
                    }

                    // 1) Validate distribution input, parse numeric fields, etc.
                    let distribution_function = match self.perpetual_dist_function {
                        DistributionFunctionUI::FixedAmount => {
                            DistributionFunction::FixedAmount {
                                n: self.fixed_amount_input.parse::<u64>().unwrap_or(0),
                            }
                        },
                        DistributionFunctionUI::StepDecreasingAmount => {
                            DistributionFunction::StepDecreasingAmount {
                                n: self.step_dec_amount_input.parse::<u64>().unwrap_or(0),
                                decrease_per_interval: NotNan::new(self.decrease_per_interval_input.parse::<f64>().unwrap_or(0.0)).unwrap(),
                                step_count: self.step_count_input.parse::<u64>().unwrap_or(0),
                            }
                        },
                        DistributionFunctionUI::LinearInteger => {
                            DistributionFunction::LinearInteger {
                                a: self.linear_int_a_input.parse::<i64>().unwrap_or(0),
                                b: self.linear_int_b_input.parse::<i64>().unwrap_or(0),
                            }
                        },
                        DistributionFunctionUI::LinearFloat => {
                            DistributionFunction::LinearFloat {
                                a: NotNan::new(self.linear_float_a_input.parse::<f64>().unwrap_or(0.0)).unwrap(),
                                b: self.linear_float_b_input.parse::<i64>().unwrap_or(0),
                            }
                        },
                        DistributionFunctionUI::PolynomialInteger => {
                            DistributionFunction::PolynomialInteger {
                                a: self.poly_int_a_input.parse::<i64>().unwrap_or(0),
                                n: self.poly_int_n_input.parse::<i64>().unwrap_or(0),
                                b: self.poly_int_b_input.parse::<i64>().unwrap_or(0),
                            }
                        },
                        DistributionFunctionUI::PolynomialFloat => {
                            DistributionFunction::PolynomialFloat {
                                a: NotNan::new(self.poly_float_a_input.parse::<f64>().unwrap_or(0.0)).unwrap(),
                                n: NotNan::new(self.poly_float_n_input.parse::<f64>().unwrap_or(0.0)).unwrap(),
                                b: self.poly_float_b_input.parse::<i64>().unwrap_or(0),
                            }
                        },
                        DistributionFunctionUI::Exponential => {
                            DistributionFunction::Exponential {
                                a: NotNan::new(self.exp_a_input.parse::<f64>().unwrap_or(0.0)).unwrap(),
                                b: NotNan::new(self.exp_b_input.parse::<f64>().unwrap_or(0.0)).unwrap(),
                                c: self.exp_c_input.parse::<i64>().unwrap_or(0),
                            }
                        },
                        DistributionFunctionUI::Logarithmic => {
                            DistributionFunction::Logarithmic {
                                a: NotNan::new(self.log_a_input.parse::<f64>().unwrap_or(0.0)).unwrap(),
                                b: NotNan::new(self.log_b_input.parse::<f64>().unwrap_or(0.0)).unwrap(),
                                c: self.log_c_input.parse::<i64>().unwrap_or(0),
                            }
                        },
                        DistributionFunctionUI::Stepwise => {
                            let mut steps = Vec::new();
                            for (block_str, amount_str) in self.stepwise_steps.iter() {
                                if let Ok(block) = block_str.parse::<u64>() {
                                    if let Ok(amount) = amount_str.parse::<u64>() {
                                        steps.push((block, amount));
                                    } else {
                                        self.token_creator_error_message = Some(
                                            "Invalid amount in stepwise distribution".to_string(),
                                        );
                                        return;
                                    }
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid block interval in stepwise distribution".to_string(),
                                    );
                                    return;
                                }
                            }
                            DistributionFunction::Stepwise(steps)
                        }
                    };
                    let maybe_perpetual_distribution = if self.enable_perpetual_distribution {
                        // Construct the `TokenPerpetualDistributionV0` from your selected type + function
                        let dist_type = match self.perpetual_dist_type {
                            PerpetualDistributionIntervalTypeUI::BlockBased => {
                                // parse interval, parse emission
                                // parse distribution function
                                RewardDistributionType::BlockBasedDistribution(
                                    self.perpetual_dist_interval_input.parse::<u64>().unwrap_or(0),
                                    0, // this field should be removed in Platform because the individual functions define it
                                    distribution_function,
                                )
                            }
                            PerpetualDistributionIntervalTypeUI::EpochBased => {
                                RewardDistributionType::EpochBasedDistribution(
                                    self.perpetual_dist_interval_input.parse::<u16>().unwrap_or(0),
                                    0, // this field should be removed in Platform because the individual functions define it
                                    distribution_function,
                                )
                            }
                            PerpetualDistributionIntervalTypeUI::TimeBased => {
                                RewardDistributionType::TimeBasedDistribution(
                                    self.perpetual_dist_interval_input.parse::<u64>().unwrap_or(0),
                                    0, // this field should be removed in Platform because the individual functions define it
                                    distribution_function,
                                )
                            }
                            _ => {
                                RewardDistributionType::BlockBasedDistribution(0, 0, DistributionFunction::FixedAmount { n: 0 })
                            }
                        };

                        let recipient = match self.perpetual_dist_recipient {
                            TokenDistributionRecipientUI::ContractOwner => TokenDistributionRecipient::ContractOwner,
                            TokenDistributionRecipientUI::Identity => {
                                if let Some(id) = self.perpetual_dist_recipient_identity_input.as_ref() {
                                    let id_res = Identifier::from_string(id, Encoding::Base58);
                                    TokenDistributionRecipient::Identity(id_res.unwrap_or_default())
                                } else {
                                    self.token_creator_error_message = Some(
                                        "Invalid base58 identifier for perpetual distribution recipient".to_string(),
                                    );
                                    return;
                                }
                            }
                            TokenDistributionRecipientUI::EvonodesByParticipation => {
                                TokenDistributionRecipient::EvonodesByParticipation
                            }
                        };

                        Some(TokenPerpetualDistribution::V0(TokenPerpetualDistributionV0 {
                            distribution_type: dist_type,
                            distribution_recipient: recipient,
                        }))
                    } else {
                        None
                    };

                    // 2) Build the distribution rules structure
                    let dist_rules_v0 = TokenDistributionRulesV0 {
                        perpetual_distribution: maybe_perpetual_distribution,
                        perpetual_distribution_rules: self.perpetual_distribution_rules.to_change_control_rules("Perpetual Distribution").unwrap(),
                        pre_programmed_distribution: if self.enable_pre_programmed_distribution {
                            let distributions: BTreeMap<u64, BTreeMap<Identifier, u64>> = match self.parse_pre_programmed_distributions() {
                                Ok(distributions) => distributions.into_iter().map(|(k, v)| (k, std::iter::once(v).collect())).collect(),
                                Err(err) => {
                                    self.token_creator_error_message = Some(err);
                                    return;
                                }
                            };

                            Some(TokenPreProgrammedDistribution::V0(
                                TokenPreProgrammedDistributionV0 {
                                    distributions,
                                }
                            ))
                        } else {
                            None
                        },
                        new_tokens_destination_identity: if self.new_tokens_destination_identity_enabled {
                            Some(Identifier::from_string(&self.new_tokens_destination_identity, Encoding::Base58).unwrap_or_default())
                        } else {
                            None
                        },
                        new_tokens_destination_identity_rules: self.new_tokens_destination_identity_rules.to_change_control_rules("New Tokens Destination Identity").unwrap(),
                        minting_allow_choosing_destination: self.minting_allow_choosing_destination,
                        minting_allow_choosing_destination_rules: self.minting_allow_choosing_destination_rules.to_change_control_rules("Minting Allow Choosing Destination").unwrap(),
                    };

                    let manual_minting_rules = match self.manual_minting_rules.to_change_control_rules("Manual Mint") {
                        Ok(rules) => rules,
                        Err(err) => {
                            self.token_creator_error_message = Some(err);
                            return;
                        }
                    };
                    let manual_burning_rules = match self.manual_burning_rules.to_change_control_rules("Manual Burn") {
                        Ok(rules) => rules,
                        Err(err) => {
                            self.token_creator_error_message = Some(err);
                            return;
                        }
                    };
                    let freeze_rules = match self.freeze_rules.to_change_control_rules("Freeze") {
                        Ok(rules) => rules,
                        Err(err) => {
                            self.token_creator_error_message = Some(err);
                            return;
                        }
                    };
                    let unfreeze_rules = match self.unfreeze_rules.to_change_control_rules("Unfreeze") {
                        Ok(rules) => rules,
                        Err(err) => {
                            self.token_creator_error_message = Some(err);
                            return;
                        }
                    };
                    let destroy_frozen_funds_rules = match self.destroy_frozen_funds_rules.to_change_control_rules("Destroy Frozen Funds") {
                        Ok(rules) => rules,
                        Err(err) => {
                            self.token_creator_error_message = Some(err);
                            return;
                        }
                    };
                    let emergency_action_rules = match self.emergency_action_rules.to_change_control_rules("Emergency Action") {
                        Ok(rules) => rules,
                        Err(err) => {
                            self.token_creator_error_message = Some(err);
                            return;
                        }
                    };
                    let max_supply_change_rules = match self.max_supply_change_rules.to_change_control_rules("Max Supply Change") {
                        Ok(rules) => rules,
                        Err(err) => {
                            self.token_creator_error_message = Some(err);
                            return;
                        }
                    };
                    let conventions_change_rules = match self.conventions_change_rules.to_change_control_rules("Conventions Change") {
                        Ok(rules) => rules,
                        Err(err) => {
                            self.token_creator_error_message = Some(err);
                            return;
                        }
                    };

                    let tasks = vec![
                        BackendTask::TokenTask(TokenTask::RegisterTokenContract {
                            identity,
                            signing_key: key,
                            token_name,
                            should_capitalize: self.should_capitalize_input,
                            decimals,
                            base_supply,
                            max_supply,
                            start_paused,
                            keeps_history: self.token_keeps_history,
                            main_control_group,

                            manual_minting_rules,
                            manual_burning_rules,
                            freeze_rules,
                            unfreeze_rules,
                            destroy_frozen_funds_rules,
                            emergency_action_rules,
                            max_supply_change_rules,
                            conventions_change_rules,
                            main_control_group_change_authorized: self
                                .authorized_main_control_group_change
                                .clone(),
                            distribution_rules: TokenDistributionRules::V0(dist_rules_v0),
                        }),
                        BackendTask::TokenTask(TokenTask::QueryMyTokenBalances),
                    ];

                    action = AppAction::BackendTasks(tasks, BackendTasksExecutionMode::Sequential);
                }

                // Cancel
                if ui.button("Cancel").clicked() {
                    self.show_token_creator_confirmation_popup = false;
                }
            });

        if !is_open {
            self.show_token_creator_confirmation_popup = false;
        }

        action
    }

    /// Once the contract creation is done (status=Complete),
    /// render a simple "Success" screen
    fn render_token_creator_success_screen(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(50.0);
            ui.heading("Token Contract Created Successfully! 🎉");
            ui.add_space(10.0);
            if ui.button("Back").clicked() {
                self.reset_token_creator();
            }
        });
    }

    /// Attempts to parse the `pre_programmed_distributions` into a BTreeMap.
    /// Returns an error string if any row fails.
    pub fn parse_pre_programmed_distributions(
        &mut self,
    ) -> Result<BTreeMap<u64, (Identifier, u64)>, String> {
        let mut map = BTreeMap::new();
        for (i, entry) in self.pre_programmed_distributions.iter().enumerate() {
            // Parse timestamp
            let timestamp = entry.timestamp_str.parse::<u64>().map_err(|_| {
                format!(
                    "Row {}: invalid timestamp (expected u64). Got '{}'",
                    i + 1,
                    entry.timestamp_str
                )
            })?;

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
        self.token_name_input = "".to_string();
        self.decimals_input = "8".to_string();
        self.base_supply_input = "100000".to_string();
        self.max_supply_input = "".to_string();
        self.start_as_paused_input = false;
        self.should_capitalize_input = false;
        self.token_keeps_history = false;
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
        self.perpetual_dist_function = DistributionFunctionUI::FixedAmount;
        self.perpetual_dist_type = PerpetualDistributionIntervalTypeUI::None;
        self.perpetual_dist_interval_input = "".to_string();
        self.fixed_amount_input = "".to_string();
        self.step_dec_amount_input = "".to_string();
        self.decrease_per_interval_input = "".to_string();
        self.step_count_input = "".to_string();
        self.linear_int_a_input = "".to_string();
        self.linear_int_b_input = "".to_string();
        self.linear_float_a_input = "".to_string();
        self.linear_float_b_input = "".to_string();
        self.poly_int_a_input = "".to_string();
        self.poly_int_n_input = "".to_string();
        self.poly_int_b_input = "".to_string();
        self.poly_float_a_input = "".to_string();
        self.poly_float_n_input = "".to_string();
        self.poly_float_b_input = "".to_string();
        self.exp_a_input = "".to_string();
        self.exp_b_input = "".to_string();
        self.exp_c_input = "".to_string();
        self.log_a_input = "".to_string();
        self.log_b_input = "".to_string();
        self.log_c_input = "".to_string();
        self.stepwise_steps = vec![(String::new(), String::new())];
        self.perpetual_dist_recipient = TokenDistributionRecipientUI::ContractOwner;
        self.perpetual_dist_recipient_identity_input = None;
        self.enable_perpetual_distribution = false;
        self.perpetual_distribution_rules = ChangeControlRulesUI::default();
        self.enable_pre_programmed_distribution = false;
        self.pre_programmed_distributions = Vec::new();
        self.new_tokens_destination_identity_enabled = false;
        self.new_tokens_destination_identity_rules = ChangeControlRulesUI::default();
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
                        egui::RichText::new("No owned tokens found.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                TokensSubscreen::SearchTokens => {
                    ui.label(
                        egui::RichText::new("No matching tokens found.")
                            .heading()
                            .strong()
                            .color(Color32::GRAY),
                    );
                }
                TokensSubscreen::TokenCreator => {
                    ui.label(
                        egui::RichText::new("Cannot render token creator for some reason")
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

    fn add_token_to_my_tokens(&self, token: IdentityTokenBalance) {
        let mut my_tokens = self.my_tokens.lock().unwrap();
        // Prevent duplicates
        if !my_tokens
            .iter()
            .any(|t| t.token_identifier == token.token_identifier)
        {
            my_tokens.push(token);
        }
        // Save the new order
        self.save_current_order();
    }

    fn goto_next_search_page(&mut self) -> AppAction {
        // If we have a next cursor:
        if let Some(next_cursor) = self.next_cursors.last().cloned() {
            // set status
            let now = Utc::now().timestamp() as u64;
            self.token_search_status = TokenSearchStatus::WaitingForResult(now);

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
                TokenTask::QueryTokensByKeywordPage(query_string, Some(next_cursor)),
            ));
        }
        AppAction::None
    }

    fn goto_previous_search_page(&mut self) -> AppAction {
        if self.search_current_page > 1 {
            // Move to (page - 1)
            self.search_current_page -= 1;
            let now = Utc::now().timestamp() as u64;
            self.token_search_status = TokenSearchStatus::WaitingForResult(now);

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
                    TokenTask::QueryTokensByKeywordPage(query_string, Some(prev_cursor)),
                ));
            }
        }
        AppAction::None
    }

    fn show_remove_identity_token_balance_popup(&mut self, ui: &mut egui::Ui) {
        // If no token is set, nothing to confirm
        let token_to_remove = match &self.identity_token_balance_to_remove {
            Some(token) => token.clone(),
            None => {
                self.confirm_remove_identity_token_balance_popup = false;
                return;
            }
        };

        let mut is_open = true;

        egui::Window::new("Confirm Remove Balance")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                ui.label(format!(
                    "Are you sure you want to remove identity token balance \"{}\" for identity \"{}\"?",
                    token_to_remove.token_name,
                    token_to_remove.identity_id.to_string(Encoding::Base58)
                ));

                // Confirm button
                if ui.button("Confirm").clicked() {
                    if let Err(e) = self.app_context.remove_token_balance(
                        token_to_remove.token_identifier,
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

    fn show_remove_token_popup(&mut self, ui: &mut egui::Ui) {
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
            .my_tokens
            .lock()
            .unwrap()
            .iter()
            .find_map(|t| {
                if t.token_identifier == token_to_remove {
                    Some(t.token_name.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| token_to_remove.to_string(Encoding::Base58));

        let mut is_open = true;

        egui::Window::new("Confirm Remove Token")
            .collapsible(false)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                ui.label(format!(
                    "Are you sure you want to remove token \"{}\" for all identities?",
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
        self.my_tokens = Arc::new(Mutex::new(
            self.app_context
                .identity_token_balances()
                .unwrap_or_default(),
        ));
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
        self.my_tokens = Arc::new(Mutex::new(
            self.app_context
                .identity_token_balances()
                .unwrap_or_default(),
        ));
    }

    fn display_message(&mut self, msg: &str, msg_type: MessageType) {
        if self.tokens_subscreen == TokensSubscreen::TokenCreator {
            // Handle messages from Token Creator
            if msg.contains("Successfully registered token contract") {
                self.token_creator_status = TokenCreatorStatus::Complete;
            } else if msg.contains("Error registering token contract") {
                self.token_creator_status = TokenCreatorStatus::ErrorMessage(msg.to_string());
                self.token_creator_error_message = Some(msg.to_string());
            } else {
                return;
            }
        }

        // Handle messages from querying My Token Balances
        if msg.contains("Successfully fetched token balances")
            | msg.contains("Failed to fetch token balances")
        {
            self.backend_message = Some((msg.to_string(), msg_type, Utc::now()));
            self.refreshing_status = RefreshingStatus::NotRefreshing;
        }

        // Handle messages from Token Search
        if msg.contains("Error fetching tokens") {
            self.token_search_status = TokenSearchStatus::ErrorMessage(msg.to_string());
            self.backend_message = Some((msg.to_string(), msg_type, Utc::now()));
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::TokensByKeyword(tokens) => {
                // This might be a “full” result (no paging).
                let mut srch = self.search_results.lock().unwrap();
                *srch = tokens;
                self.token_search_status = TokenSearchStatus::Complete;
            }
            BackendTaskSuccessResult::TokensByKeywordPage(tokens, next_cursor) => {
                // Paged result
                let mut srch = self.search_results.lock().unwrap();
                *srch = tokens;
                self.search_has_next_page = next_cursor.is_some();

                if let Some(cursor) = next_cursor {
                    // Save it for “next page” retrieval
                    self.next_cursors.push(cursor);
                }
                self.token_search_status = TokenSearchStatus::Complete;
            }
            _ => {}
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;

        self.check_error_expiration();

        // Build top-right buttons
        let right_buttons = match self.tokens_subscreen {
            TokensSubscreen::MyTokens => vec![(
                "Refresh",
                DesiredAppAction::BackendTask(BackendTask::TokenTask(
                    TokenTask::QueryMyTokenBalances,
                )),
            )],
            TokensSubscreen::SearchTokens => vec![("Refresh", DesiredAppAction::Refresh)],
            TokensSubscreen::TokenCreator => vec![],
        };

        // Top panel
        if let Some(token_id) = self.selected_token_id {
            let token_name: String = self
                .my_tokens
                .lock()
                .unwrap()
                .iter()
                .find_map(|t| {
                    if t.token_identifier == token_id {
                        Some(t.token_name.clone())
                    } else {
                        None
                    }
                })
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
                    let tokens = self.my_tokens.lock().unwrap().clone();
                    if tokens.is_empty() {
                        // If no tokens, show a “no tokens found” message
                        action |= self.render_no_owned_tokens(ui);
                    } else {
                        // Are we showing details for a selected token?
                        if self.selected_token_id.is_some() {
                            // Render detail view for one token
                            action |= self.render_token_details(ui, &tokens);
                        } else {
                            // Otherwise, show the list of all tokens
                            self.render_token_list(ui, &tokens);
                        }
                    }
                }
                TokensSubscreen::SearchTokens => {
                    action |= self.render_token_search(ui);
                }
                TokensSubscreen::TokenCreator => {
                    action |= self.render_token_creator(ui);
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
            AppAction::BackendTask(BackendTask::TokenTask(TokenTask::QueryTokensByKeyword(_))) => {
                self.refreshing_status =
                    RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
            }
            AppAction::SetMainScreen(_) => {
                self.refreshing_status = RefreshingStatus::NotRefreshing;
                self.selected_token_id = None;
                self.reset_token_creator();
            }
            AppAction::Custom(ref s) if s == "Back to tokens" => {
                self.selected_token_id = None;
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
