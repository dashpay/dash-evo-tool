mod contract_details;
mod control_rules;
mod data_contract_json_pop_up;
mod distributions;
mod groups;
mod keyword_search;
mod my_tokens;
mod structs;
mod token_creator;

pub use control_rules::{ChangeControlRulesUI, MintExtras};
pub use distributions::{
    DistributionEntry, DistributionFunctionUI, IntervalTimeUnit,
    PerpetualDistributionIntervalTypeUI, TokenDistributionRecipientUI,
    validate_perpetual_distribution_recipient,
};
pub use structs::*;
pub use token_creator::TokenBuildArgs;

pub use groups::*;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use crate::lock_helper::MutexExt;

use serde_json;

use chrono::{DateTime, Utc};
use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationPresetFeatures;
use dash_sdk::dpp::data_contract::associated_token::token_keeps_history_rules::v0::TokenKeepsHistoryRulesV0;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::evaluate_interval::IntervalEvaluationExplanation;
use dash_sdk::dpp::data_contract::change_control_rules::authorized_action_takers::AuthorizedActionTakers;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use dash_sdk::query_types::IndexMap;
use eframe::egui::{self, Color32, Context, Ui};
use crate::ui::theme::DashColors;
use egui::{ColorImage, TextureHandle};
use enum_iterator::Sequence;
use crate::app::BackendTasksExecutionMode;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::tokens::{TokenResult, TokenTask};
use crate::backend_task::{BackendTask, NO_IDENTITIES_FOUND};

use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::wallet::Wallet;
use crate::ui::components::Component;
use crate::ui::components::amount_input::AmountInput;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::components::wallet_unlock_popup::{WalletUnlockPopup, WalletUnlockResult};
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, ScreenLike, ScreenType};

use token_creator::{
    DEFAULT_DECIMALS, EXP_FORMULA_PNG, INV_LOG_FORMULA_PNG, LINEAR_FORMULA_PNG, LOG_FORMULA_PNG,
    POLYNOMIAL_FORMULA_PNG, load_formula_image,
};
use token_creator::{sanitize_i64, sanitize_u64};

#[derive(Debug, Clone, PartialEq)]
pub struct ContractDescriptionInfo {
    pub data_contract_id: Identifier,
    pub description: String,
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

#[derive(Debug, PartialEq, Default)]
pub enum TokenCreatorStatus {
    #[default]
    NotStarted,
    WaitingForResult(u64),
    Complete,
    ErrorMessage(String),
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

impl TokenNameLanguage {
    pub fn iso_code(self) -> &'static str {
        match self {
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
            TokenNameLanguage::Mandarin => "zh",
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
        }
    }

    pub fn ui_label(self) -> &'static str {
        match self {
            TokenNameLanguage::English => "English",
            TokenNameLanguage::Arabic => "Arabic",
            TokenNameLanguage::Bengali => "Bengali",
            TokenNameLanguage::Burmese => "Burmese",
            TokenNameLanguage::Chinese => "Chinese",
            TokenNameLanguage::Czech => "Czech",
            TokenNameLanguage::Dutch => "Dutch",
            TokenNameLanguage::Farsi => "Farsi (Persian)",
            TokenNameLanguage::Filipino => "Filipino (Tagalog)",
            TokenNameLanguage::French => "French",
            TokenNameLanguage::German => "German",
            TokenNameLanguage::Greek => "Greek",
            TokenNameLanguage::Gujarati => "Gujarati",
            TokenNameLanguage::Hausa => "Hausa",
            TokenNameLanguage::Hebrew => "Hebrew",
            TokenNameLanguage::Hindi => "Hindi",
            TokenNameLanguage::Hungarian => "Hungarian",
            TokenNameLanguage::Igbo => "Igbo",
            TokenNameLanguage::Indonesian => "Indonesian",
            TokenNameLanguage::Italian => "Italian",
            TokenNameLanguage::Japanese => "Japanese",
            TokenNameLanguage::Javanese => "Javanese",
            TokenNameLanguage::Kannada => "Kannada",
            TokenNameLanguage::Khmer => "Khmer",
            TokenNameLanguage::Korean => "Korean",
            TokenNameLanguage::Malay => "Malay",
            TokenNameLanguage::Malayalam => "Malayalam",
            TokenNameLanguage::Mandarin => "Mandarin Chinese",
            TokenNameLanguage::Marathi => "Marathi",
            TokenNameLanguage::Nepali => "Nepali",
            TokenNameLanguage::Oriya => "Oriya",
            TokenNameLanguage::Pashto => "Pashto",
            TokenNameLanguage::Polish => "Polish",
            TokenNameLanguage::Portuguese => "Portuguese",
            TokenNameLanguage::Punjabi => "Punjabi",
            TokenNameLanguage::Romanian => "Romanian",
            TokenNameLanguage::Russian => "Russian",
            TokenNameLanguage::Serbian => "Serbian",
            TokenNameLanguage::Sindhi => "Sindhi",
            TokenNameLanguage::Sinhala => "Sinhala",
            TokenNameLanguage::Somali => "Somali",
            TokenNameLanguage::Spanish => "Spanish",
            TokenNameLanguage::Swahili => "Swahili",
            TokenNameLanguage::Swedish => "Swedish",
            TokenNameLanguage::Tamil => "Tamil",
            TokenNameLanguage::Telugu => "Telugu",
            TokenNameLanguage::Thai => "Thai",
            TokenNameLanguage::Turkish => "Turkish",
            TokenNameLanguage::Ukrainian => "Ukrainian",
            TokenNameLanguage::Urdu => "Urdu",
            TokenNameLanguage::Vietnamese => "Vietnamese",
            TokenNameLanguage::Yoruba => "Yoruba",
        }
    }

    pub fn selection_order() -> &'static [TokenNameLanguage] {
        &[
            TokenNameLanguage::English,
            TokenNameLanguage::Arabic,
            TokenNameLanguage::Bengali,
            TokenNameLanguage::Burmese,
            TokenNameLanguage::Chinese,
            TokenNameLanguage::Czech,
            TokenNameLanguage::Dutch,
            TokenNameLanguage::Farsi,
            TokenNameLanguage::Filipino,
            TokenNameLanguage::French,
            TokenNameLanguage::German,
            TokenNameLanguage::Greek,
            TokenNameLanguage::Gujarati,
            TokenNameLanguage::Hausa,
            TokenNameLanguage::Hebrew,
            TokenNameLanguage::Hindi,
            TokenNameLanguage::Hungarian,
            TokenNameLanguage::Igbo,
            TokenNameLanguage::Indonesian,
            TokenNameLanguage::Italian,
            TokenNameLanguage::Japanese,
            TokenNameLanguage::Javanese,
            TokenNameLanguage::Kannada,
            TokenNameLanguage::Khmer,
            TokenNameLanguage::Korean,
            TokenNameLanguage::Malay,
            TokenNameLanguage::Malayalam,
            TokenNameLanguage::Mandarin,
            TokenNameLanguage::Marathi,
            TokenNameLanguage::Nepali,
            TokenNameLanguage::Oriya,
            TokenNameLanguage::Pashto,
            TokenNameLanguage::Polish,
            TokenNameLanguage::Portuguese,
            TokenNameLanguage::Punjabi,
            TokenNameLanguage::Romanian,
            TokenNameLanguage::Russian,
            TokenNameLanguage::Serbian,
            TokenNameLanguage::Sindhi,
            TokenNameLanguage::Sinhala,
            TokenNameLanguage::Somali,
            TokenNameLanguage::Spanish,
            TokenNameLanguage::Swahili,
            TokenNameLanguage::Swedish,
            TokenNameLanguage::Tamil,
            TokenNameLanguage::Telugu,
            TokenNameLanguage::Thai,
            TokenNameLanguage::Turkish,
            TokenNameLanguage::Ukrainian,
            TokenNameLanguage::Urdu,
            TokenNameLanguage::Vietnamese,
            TokenNameLanguage::Yoruba,
        ]
    }
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
    // Token Creator expanded sections
    token_creator_advanced_expanded: bool,
    token_creator_action_rules_expanded: bool,
    token_creator_main_control_expanded: bool,
    token_creator_distribution_expanded: bool,
    token_creator_groups_expanded: bool,
    token_creator_groups_items_expanded: std::collections::HashSet<String>,
    token_creator_document_schemas_expanded: bool,
    // Individual action rules expanded states
    token_creator_manual_mint_expanded: bool,
    token_creator_manual_burn_expanded: bool,
    token_creator_freeze_expanded: bool,
    token_creator_unfreeze_expanded: bool,
    token_creator_destroy_frozen_expanded: bool,
    token_creator_emergency_action_expanded: bool,
    token_creator_max_supply_change_expanded: bool,
    token_creator_conventions_change_expanded: bool,
    token_creator_marketplace_expanded: bool,
    token_creator_direct_purchase_pricing_expanded: bool,
    // Nested rules expanded states
    token_creator_new_tokens_destination_expanded: bool,
    token_creator_minting_allow_choosing_expanded: bool,
    token_creator_perpetual_distribution_rules_expanded: bool,

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
    remove_identity_token_balance_confirmation_dialog: Option<ConfirmationDialog>,
    confirm_remove_token_popup: bool,
    token_to_remove: Option<Identifier>,
    remove_token_confirmation_dialog: Option<ConfirmationDialog>,

    // Reward explanations
    reward_explanations: IndexMap<IdentityTokenIdentifier, IntervalEvaluationExplanation>,
    show_explanation_popup: Option<IdentityTokenIdentifier>,

    // Token info popup
    show_token_info_popup: Option<Identifier>,

    // ====================================
    //           Token Creator
    // ====================================
    show_advanced_token_creator: bool,
    selected_token_preset: Option<TokenConfigurationPresetFeatures>,
    show_pop_up_info: Option<String>,
    identity_id_string: String,
    selected_identity: Option<QualifiedIdentity>,
    selected_key: Option<IdentityPublicKey>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_unlock_popup: WalletUnlockPopup,
    token_names_input: Vec<(String, String, TokenNameLanguage, TokenSearchable)>,
    contract_keywords_input: String,
    token_description_input: String,
    should_capitalize_input: bool,
    decimals_input: String,
    base_supply_amount: Option<Amount>,
    base_supply_input: Option<AmountInput>,
    max_supply_amount: Option<Amount>,
    max_supply_input: Option<AmountInput>,
    start_as_paused_input: bool,
    main_control_group_input: String,
    show_token_creator_confirmation_popup: bool,
    token_creator_confirmation_dialog: Option<ConfirmationDialog>,
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

    // Marketplace rules
    marketplace_trade_mode: u8, // 0 = NotTradeable, future values for other modes
    marketplace_rules: ChangeControlRulesUI,
    change_direct_purchase_pricing_rules: ChangeControlRulesUI,

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

    // --- Random ---  -  not supported
    // pub random_min_input: String,
    // pub random_max_input: String,

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

    // Token adding status
    adding_token_start_time: Option<DateTime<Utc>>,
    adding_token_name: Option<String>,

    // Document Schemas
    document_schemas_input: String,
    parsed_document_schemas: Option<BTreeMap<String, serde_json::Value>>,
    document_schemas_error: Option<String>,
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
    let in_dev_mode = app_context.is_developer_mode();

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
            remove_identity_token_balance_confirmation_dialog: None,
            confirm_remove_token_popup: false,
            token_to_remove: None,
            remove_token_confirmation_dialog: None,

            // Reward explanations
            reward_explanations: IndexMap::new(),
            show_explanation_popup: None,
            show_token_info_popup: None,

            // Token Creator
            show_advanced_token_creator: false,
            selected_token_preset: None,
            show_pop_up_info: None,
            identity_id_string: String::new(),
            selected_identity: None,
            selected_key: None,
            selected_wallet: None,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            show_token_creator_confirmation_popup: false,
            token_creator_confirmation_dialog: None,
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
            decimals_input: DEFAULT_DECIMALS.to_string(),
            base_supply_amount: None,
            base_supply_input: None,
            max_supply_amount: None,
            max_supply_input: None,
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

            // Marketplace rules
            marketplace_trade_mode: 0, // NotTradeable
            marketplace_rules: ChangeControlRulesUI::default(),
            change_direct_purchase_pricing_rules: ChangeControlRulesUI::default(),

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
            // random_min_input: String::new(),
            // random_max_input: String::new(),
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
            // Token Creator expanded sections
            token_creator_advanced_expanded: false,
            token_creator_action_rules_expanded: false,
            token_creator_main_control_expanded: false,
            token_creator_distribution_expanded: false,
            token_creator_groups_expanded: false,
            token_creator_groups_items_expanded: std::collections::HashSet::new(),
            token_creator_document_schemas_expanded: false,
            // Individual action rules expanded states
            token_creator_manual_mint_expanded: false,
            token_creator_manual_burn_expanded: false,
            token_creator_freeze_expanded: false,
            token_creator_unfreeze_expanded: false,
            token_creator_destroy_frozen_expanded: false,
            token_creator_emergency_action_expanded: false,
            token_creator_max_supply_change_expanded: false,
            token_creator_conventions_change_expanded: false,
            token_creator_marketplace_expanded: false,
            token_creator_direct_purchase_pricing_expanded: false,
            // Nested rules expanded states
            token_creator_new_tokens_destination_expanded: false,
            token_creator_minting_allow_choosing_expanded: false,
            token_creator_perpetual_distribution_rules_expanded: false,

            // Token adding status
            adding_token_start_time: None,
            adding_token_name: None,

            // Document Schemas
            document_schemas_input: String::new(),
            parsed_document_schemas: None,
            document_schemas_error: None,
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

        // Append any tokens not present in the saved order (e.g., newly added tokens)
        for (key, value) in &self.my_tokens {
            if !reordered.contains_key(key) {
                reordered.insert(*key, value.clone());
            }
        }

        // Replace the original with the reordered map
        self.my_tokens = reordered;
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

    fn add_token_to_tracked_tokens(&mut self, token_info: TokenInfo) -> Result<AppAction, String> {
        // Check if token is already added
        if self.all_known_tokens.contains_key(&token_info.token_id) {
            self.backend_message = Some((
                "Token already in My Tokens".to_string(),
                MessageType::Error,
                Utc::now(),
            ));
            return Ok(AppAction::None);
        }

        // Set adding status with timestamp for elapsed time display
        self.adding_token_start_time = Some(Utc::now());
        self.adding_token_name = Some(token_info.token_name.clone());
        self.backend_message = Some(("Adding token...".to_string(), MessageType::Info, Utc::now()));

        // Always save the token locally and refresh balances
        // The contract will be fetched automatically when needed
        Ok(AppAction::BackendTasks(
            vec![
                BackendTask::ContractTask(Box::new(ContractTask::FetchContracts(vec![
                    token_info.data_contract_id,
                ]))),
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

        // Lazy initialization of the confirmation dialog
        let confirmation_dialog = self
            .remove_identity_token_balance_confirmation_dialog
            .get_or_insert_with(|| {
                ConfirmationDialog::new(
                "Confirm Stop Tracking Balance",
                format!(
                    "Are you sure you want to stop tracking the token \"{}\" for identity \"{}\"?",
                    token_to_remove.token_alias,
                    token_to_remove.identity_id.to_string(Encoding::Base58)
                ),
            )
            .confirm_text(Some("Confirm"))
            .cancel_text(Some("Cancel"))
            });

        // Show the dialog and handle the response
        let response = confirmation_dialog.show(ui).inner;

        if let Some(status) = response.dialog_response {
            match status {
                ConfirmationStatus::Confirmed => {
                    if let Err(e) = self
                        .app_context
                        .remove_token_balance(token_to_remove.token_id, token_to_remove.identity_id)
                    {
                        self.backend_message = Some((
                            format!("Error removing token balance: {}", e),
                            MessageType::Error,
                            Utc::now(),
                        ));
                    } else {
                        self.refresh();
                    }
                    self.confirm_remove_identity_token_balance_popup = false;
                    self.identity_token_balance_to_remove = None;
                    self.remove_identity_token_balance_confirmation_dialog = None;
                }
                ConfirmationStatus::Canceled => {
                    self.confirm_remove_identity_token_balance_popup = false;
                    self.identity_token_balance_to_remove = None;
                    self.remove_identity_token_balance_confirmation_dialog = None;
                }
            }
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

        // Lazy initialization of the confirmation dialog
        let confirmation_dialog = self.remove_token_confirmation_dialog.get_or_insert_with(|| {
            ConfirmationDialog::new(
                "Confirm Remove Token",
                format!(
                    "Are you sure you want to stop tracking the token \"{}\"? You can re-add it later. Your actual token balance will not change with this action.",
                    token_name,
                ),
            )
            .confirm_text(Some("Confirm"))
            .cancel_text(Some("Cancel"))
        });

        // Show the dialog and handle the response
        let response = confirmation_dialog.show(ui).inner;

        if let Some(status) = response.dialog_response {
            match status {
                ConfirmationStatus::Confirmed => {
                    if let Err(e) = self
                        .app_context
                        .db
                        .remove_token(&token_to_remove, &self.app_context)
                    {
                        self.backend_message = Some((
                            format!("Error removing token balance: {}", e),
                            MessageType::Error,
                            Utc::now(),
                        ));
                    } else {
                        self.refresh();
                    }
                    self.confirm_remove_token_popup = false;
                    self.token_to_remove = None;
                    self.remove_token_confirmation_dialog = None;
                }
                ConfirmationStatus::Canceled => {
                    self.confirm_remove_token_popup = false;
                    self.token_to_remove = None;
                    self.remove_token_confirmation_dialog = None;
                }
            }
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

        // Clear pricing data to force re-fetching when tokens are selected
        // This ensures we get updated pricing after changes like SetPrice
        self.token_pricing_data.clear();
        self.pricing_loading_state.clear();

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

        // Clear pricing data to force re-fetching when tokens are selected
        // This ensures we get updated pricing after changes like SetPrice
        self.token_pricing_data.clear();
        self.pricing_loading_state.clear();

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
        let right_buttons = match self.tokens_subscreen {
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
            egui::ScrollArea::vertical()
                .show(ui, |ui| {
                    let mut inner_action = AppAction::None;

                    match self.tokens_subscreen {
                        TokensSubscreen::MyTokens => {
                            inner_action |= self.render_my_tokens_subscreen(ui);
                        }
                        TokensSubscreen::SearchTokens => {
                            if self.selected_contract_id.is_some() {
                                inner_action |= self.render_contract_details(
                                    ui,
                                    &self.selected_contract_id.unwrap(),
                                );
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

                    // Show either refreshing indicator or message, but not both
                    if let RefreshingStatus::Refreshing(start_time) = self.refreshing_status {
                        ui.add_space(25.0); // Space above
                        let now = Utc::now().timestamp() as u64;
                        let elapsed = now - start_time;
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.label(format!("Refreshing... Time so far: {}", elapsed));
                            ui.add(egui::widgets::Spinner::default().color(DashColors::DASH_BLUE));
                        });
                        ui.add_space(2.0); // Space below
                    } else if let Some((msg, msg_type, timestamp)) = self.backend_message.clone() {
                        ui.add_space(25.0); // Same space as refreshing indicator
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        let color = match msg_type {
                            MessageType::Error => Color32::DARK_RED,
                            MessageType::Info => DashColors::text_primary(dark_mode),
                            MessageType::Success => Color32::DARK_GREEN,
                        };
                        ui.horizontal(|ui| {
                            // Calculate remaining seconds
                            let now = Utc::now();
                            let elapsed = now.signed_duration_since(timestamp);
                            let remaining = (10 - elapsed.num_seconds()).max(0);

                            // Add the message with auto-dismiss countdown
                            let full_msg = format!("{} ({}s)", msg, remaining);
                            ui.label(egui::RichText::new(full_msg).color(color));
                        });
                        ui.add_space(2.0); // Same space below as refreshing indicator
                    }

                    if self.confirm_remove_identity_token_balance_popup {
                        self.show_remove_identity_token_balance_popup(ui);
                    }
                    if self.confirm_remove_token_popup {
                        self.show_remove_token_popup(ui);
                    }

                    // If we have info text, open a pop-up window to show it
                    if let Some(info_text) = self.show_pop_up_info.clone() {
                        let mut popup = InfoPopup::new("Information", &info_text);
                        if popup.show(ui).inner {
                            self.show_pop_up_info = None;
                        }
                    }

                    inner_action
                })
                .inner
        });

        // Post-processing on user actions
        match action {
            AppAction::BackendTask(BackendTask::TokenTask(ref token_task))
                if matches!(token_task.as_ref(), TokenTask::QueryMyTokenBalances) =>
            {
                self.refreshing_status =
                    RefreshingStatus::Refreshing(Utc::now().timestamp() as u64);
                self.backend_message = None; // Clear any existing message
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

        if action == AppAction::None
            && let Some(bt) = self.pending_backend_task.take()
        {
            action = AppAction::BackendTask(bt);
        }

        // Show wallet unlock popup if open
        if self.wallet_unlock_popup.is_open()
            && let Some(wallet) = &self.selected_wallet
        {
            let result = self
                .wallet_unlock_popup
                .show(ctx, wallet, &self.app_context);
            if result == WalletUnlockResult::Unlocked {
                // Wallet unlocked successfully
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
                }
            }
            TokensSubscreen::MyTokens => {
                if msg.contains("Successfully fetched token balances")
                    || msg.contains("Failed to fetch token balances")
                    || msg.contains("Failed to get estimated rewards")
                    || msg.eq(NO_IDENTITIES_FOUND)
                {
                    // Clear adding status on any error
                    if msg.contains("Failed") {
                        self.adding_token_start_time = None;
                        self.adding_token_name = None;
                    }
                    self.backend_message = Some((msg.to_string(), msg_type, Utc::now()));
                    self.refreshing_status = RefreshingStatus::NotRefreshing;
                } else if msg.contains("Failed to query token pricing") {
                    self.backend_message = Some((msg.to_string(), MessageType::Error, Utc::now()));
                } else {
                    tracing::debug!(
                        ?msg,
                        ?msg_type,
                        "unsupported message received in token screen"
                    );
                }
            }
            TokensSubscreen::SearchTokens => {
                if msg_type == MessageType::Error {
                    self.contract_search_status =
                        ContractSearchStatus::ErrorMessage(msg.to_string());
                    // Clear adding status on error
                    self.adding_token_start_time = None;
                    self.adding_token_name = None;
                } else if msg.contains("Added token")
                    | msg.contains("Token already added")
                    | msg.contains("Saved token to db")
                {
                    // Clear adding status and show success message
                    self.adding_token_start_time = None;
                    self.adding_token_name = None;
                    self.backend_message = Some((
                        "Token added successfully!".to_string(),
                        MessageType::Success,
                        Utc::now(),
                    ));
                }
            }
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::Token(TokenResult::DescriptionsByKeyword(
                descriptions,
                next_cursor,
            )) => {
                let mut sr = self.search_results.lock_or_recover();
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
            BackendTaskSuccessResult::Token(TokenResult::EstimatedDistributionRewards(
                identity_token_id,
                amount,
                explanation,
            )) => {
                self.refreshing_status = RefreshingStatus::NotRefreshing;
                if let Some(itb) = self.my_tokens.get_mut(&identity_token_id) {
                    itb.estimated_unclaimed_rewards = Some(amount);
                }
                self.reward_explanations
                    .insert(identity_token_id, explanation);
            }
            BackendTaskSuccessResult::Token(TokenResult::TokenPricing { token_id, prices }) => {
                // Store the pricing data
                self.token_pricing_data.insert(token_id, prices);
                // Clear loading state
                self.pricing_loading_state.insert(token_id, false);
                // Refresh my_tokens to update available actions with new pricing data
                self.my_tokens = my_tokens(
                    &self.app_context,
                    &self.identities,
                    &self.all_known_tokens,
                    &self.token_pricing_data,
                );
                // Refresh display
                self.refreshing_status = RefreshingStatus::NotRefreshing;
            }
            BackendTaskSuccessResult::Token(TokenResult::FetchedTokenBalances) => {
                // Refresh my_tokens to show updated balances
                self.my_tokens = my_tokens(
                    &self.app_context,
                    &self.identities,
                    &self.all_known_tokens,
                    &self.token_pricing_data,
                );
                self.refreshing_status = RefreshingStatus::NotRefreshing;
            }
            BackendTaskSuccessResult::Token(TokenResult::RegisteredTokenContract) => {
                self.token_creator_status = TokenCreatorStatus::Complete;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Once;

    use crate::app_dir::copy_env_file_if_not_exists;
    use crate::database::Database;
    use crate::model::qualified_identity::IdentityStatus;
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;

    use super::*;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::TokenConfigurationConvention;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration_localization::accessors::v0::TokenConfigurationLocalizationV0Getters;
    use dash_sdk::dpp::data_contract::associated_token::token_distribution_rules::TokenDistributionRules;
    use dash_sdk::dpp::data_contract::associated_token::token_keeps_history_rules::TokenKeepsHistoryRules;
    use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::DistributionFunction;
    use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_recipient::TokenDistributionRecipient;
    use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::reward_distribution_type::RewardDistributionType;
    use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::TokenPerpetualDistribution;
    use dash_sdk::dpp::data_contract::group::accessors::v0::GroupV0Getters;
    use dash_sdk::dpp::data_contract::TokenConfiguration;
    use dash_sdk::dpp::identifier::Identifier;
    use dash_sdk::platform::{DataContract, Identity};

    fn ensure_test_env() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            copy_env_file_if_not_exists(); // required by AppContext::new()

            // Ensure minimum required configs exist even if .env isn't loaded.
            // Safety: tests set env vars once to ensure deterministic config.
            // No other test mutates these values.
            unsafe {
                std::env::set_var("MAINNET_dapi_addresses", "http://127.0.0.1:1443");
                std::env::set_var("MAINNET_core_host", "127.0.0.1");
                std::env::set_var("MAINNET_core_rpc_port", "9998");
                std::env::set_var("MAINNET_core_rpc_user", "dashrpc");
                std::env::set_var("MAINNET_core_rpc_password", "password");
                std::env::set_var("MAINNET_insight_api_url", "http://127.0.0.1:3001");
                std::env::set_var("MAINNET_show_in_ui", "true");

                std::env::set_var("LOCAL_dapi_addresses", "http://127.0.0.1:2443");
                std::env::set_var("LOCAL_core_host", "127.0.0.1");
                std::env::set_var("LOCAL_core_rpc_port", "20302");
                std::env::set_var("LOCAL_core_rpc_user", "dashmate");
                std::env::set_var("LOCAL_core_rpc_password", "password");
                std::env::set_var("LOCAL_insight_api_url", "http://127.0.0.1:3001");
                std::env::set_var("LOCAL_show_in_ui", "true");
            }
        });
    }

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
        let db_file_path = "test_db_token_creator";
        let _ = std::fs::remove_file(db_file_path); // Clean up from previous runs
        let db = Arc::new(Database::new(db_file_path).unwrap());
        db.initialize(Path::new(&db_file_path)).unwrap();

        ensure_test_env();
        let app_context = AppContext::new(Network::Regtest, db, None, Default::default())
            .expect("Expected to create AppContext");
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
            status: IdentityStatus::Active,
            network: Network::Dash,
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
        token_creator_ui.base_supply_input = None;
        token_creator_ui.base_supply_amount = Some(Amount::new(5000000, 8));
        token_creator_ui.max_supply_input = None;
        token_creator_ui.max_supply_amount = Some(Amount::new(10000000, 8));
        token_creator_ui.decimals_input = DEFAULT_DECIMALS.to_string();
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
        // We'll define 2 groups for testing: positions 0 (main) and 1
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
                build_args.document_schemas,
                build_args.marketplace_trade_mode,
                build_args.marketplace_rules,
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
            "BCMnPwQZcH3RP9atgkmvtmN45QrVcYvh5cmUYARHBTu9"
        );
        assert!(dist_rules_v0.minting_allow_choosing_destination);

        // F) Check the Groups
        //    (Positions 0 and 1, from above)
        assert_eq!(contract_v1.groups.len(), 2, "We added two groups in the UI");

        let group0 = contract_v1.groups.get(&0).expect("Expected group pos=0");
        assert_eq!(
            group0.required_power(),
            2,
            "Group #0 required_power mismatch"
        );
        let members = &group0.members();
        assert_eq!(members.len(), 2);

        let group1 = contract_v1.groups.get(&1).expect("Expected group pos=1");
        assert_eq!(group1.required_power(), 1);
        assert_eq!(group1.members().len(), 0);
    }

    #[test]
    fn test_distribution_function_random() {
        let db_file_path = "test_db_distribution_random";
        let _ = std::fs::remove_file(db_file_path); // Clean up from previous runs
        let db = Arc::new(Database::new(db_file_path).unwrap());
        db.initialize(Path::new(&db_file_path)).unwrap();

        ensure_test_env();
        let app_context = AppContext::new(Network::Regtest, db, None, Default::default())
            .expect("Expected to create AppContext");
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
            status: IdentityStatus::Active,
            network: Network::Dash,
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

        // Set base supply
        token_creator_ui.base_supply_amount = Some(Amount::new(1000000, 8));

        // Enable perpetual distribution, select Random
        token_creator_ui.enable_perpetual_distribution = true;
        token_creator_ui.perpetual_dist_type = PerpetualDistributionIntervalTypeUI::TimeBased;
        token_creator_ui.perpetual_dist_function = DistributionFunctionUI::FixedAmount;
        token_creator_ui.perpetual_dist_interval_input = "60".to_string();
        token_creator_ui.perpetual_dist_interval_unit = IntervalTimeUnit::Second;
        token_creator_ui.fixed_amount_input = "100".to_string();

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
                build_args.document_schemas,
                build_args.marketplace_trade_mode,
                build_args.marketplace_rules,
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
                    DistributionFunction::FixedAmount { amount } => {
                        assert_eq!(*amount, 100);
                    }
                    _ => panic!("Expected DistributionFunction::FixedAmount"),
                }
            }
            _ => panic!("Expected TimeBasedDistribution"),
        }
    }

    #[test]
    fn test_parse_token_build_args_fails_with_empty_token_name() {
        let db_file_path = "test_db_empty_token_name";
        let _ = std::fs::remove_file(db_file_path); // Clean up from previous runs
        let db = Arc::new(Database::new(db_file_path).unwrap());
        db.initialize(Path::new(&db_file_path)).unwrap();

        ensure_test_env();
        let app_context = AppContext::new(Network::Regtest, db, None, Default::default())
            .expect("Expected to create AppContext");
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
            status: IdentityStatus::Active,
            network: Network::Dash,
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
