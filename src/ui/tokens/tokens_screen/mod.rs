mod contract_details;
mod creator;
mod data_contract_json_pop_up;
mod keyword_search;
mod my_tokens;
mod structs;

pub use structs::*;

use crate::wallet_backend::poison::MutexRecover;
use std::sync::{Arc, Mutex, RwLock};
use tracing::error;

use chrono::{DateTime, Utc};
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_function::evaluate_interval::IntervalEvaluationExplanation;
use dash_sdk::dpp::data_contract::associated_token::token_perpetual_distribution::distribution_recipient::TokenDistributionRecipient;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::identity::identity_public_key::accessors::v0::IdentityPublicKeyGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::proto::get_documents_request::get_documents_request_v0::Start;
use dash_sdk::platform::{Identifier, IdentityPublicKey};
use dash_sdk::query_types::IndexMap;
use eframe::egui::{self, Ui};
use enum_iterator::Sequence;
use crate::app::BackendTasksExecutionMode;
use crate::backend_task::contract::ContractTask;
use crate::backend_task::error::TaskError;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskContext};

use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::model::user_role::UserRole;
use crate::model::wallet::Wallet;
use crate::ui::components::Component;
use crate::ui::components::MessageBanner;
use crate::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
use crate::ui::components::info_popup::InfoPopup;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::message_banner::{BannerHandle, OptionBannerExt};
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::tokens_subscreen_chooser_panel::add_tokens_subscreen_chooser_panel;
use crate::ui::components::top_panel::{
    add_top_panel, add_top_panel_with_global_nav, subdued_everyday_spec,
};
use crate::ui::components::wallet_unlock_popup::{WalletUnlockPopup, WalletUnlockResult};
use crate::ui::helpers::ModalOpeningGuard;
use crate::ui::{BackendTaskSuccessResult, MessageType, RootScreenType, ScreenLike, ScreenType};

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

#[derive(Debug, PartialEq)]
pub enum RefreshingStatus {
    Refreshing,
    NotRefreshing,
}

impl RefreshingStatus {
    fn stop_on_failure(
        &mut self,
        expected: Option<&BackendTaskContext>,
        failed: &BackendTaskContext,
    ) {
        if *self == Self::Refreshing && expected == Some(failed) {
            *self = Self::NotRefreshing;
        }
    }
}

fn is_duplicate_balance_refresh(
    status: &RefreshingStatus,
    pending_context: Option<&BackendTaskContext>,
    action: &AppAction,
) -> bool {
    *status == RefreshingStatus::Refreshing
        && pending_context.map(BackendTaskContext::operation)
            == Some(&BackendTaskContext::TokenBalanceRefresh)
        && matches!(
            action,
            AppAction::BackendTask(BackendTask::TokenTask(task))
                if matches!(task.as_ref(), TokenTask::QueryMyTokenBalances)
        )
}

fn completes_pending_operation(
    pending_context: Option<&BackendTaskContext>,
    result_context: &BackendTaskContext,
    result: &BackendTaskSuccessResult,
) -> bool {
    if pending_context != Some(result_context) {
        return false;
    }

    match result {
        BackendTaskSuccessResult::FetchedTokenBalances
        | BackendTaskSuccessResult::TokenBalanceRefreshAlreadyInFlight => {
            *result_context.operation() == BackendTaskContext::TokenBalanceRefresh
        }
        BackendTaskSuccessResult::TokenEstimatedNonClaimedPerpetualDistributionAmountWithExplanation(
            identity_token_id,
            ..
        ) => {
            *result_context.operation() == BackendTaskContext::TokenRewardEstimate(*identity_token_id)
        }
        _ => false,
    }
}

/// Represents the status of the user's search
#[derive(PartialEq, Eq, Clone)]
pub enum ContractSearchStatus {
    NotStarted,
    WaitingForResult,
    Complete,
    Error,
}

#[cfg(feature = "testing")]
#[derive(Debug, PartialEq, Default)]
pub enum TokenCreatorStatus {
    #[default]
    NotStarted,
    WaitingForResult,
    Complete,
    Error,
}

/// Sorting columns
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    OwnerIdentity,
    OwnerIdentityName,
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

/// The main, combined TokensScreen:
/// - Displays token balances or a search UI
/// - Allows reordering of tokens if desired
///
// TODO(det): this god-struct still mixes state for every subscreen (My Tokens,
// Search, Token Creator). Split per-subscreen state into dedicated structs behind
// a thin shell; the kittest reads `selected_identity` directly, so update it too.
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
    pending_backend_task: Option<BackendTask>,
    refreshing_status: RefreshingStatus,
    pending_operation_context: Option<BackendTaskContext>,
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

    creator_workspace: crate::ui::state::token_creator::CreatorWorkspace,
    pub selected_identity: Option<QualifiedIdentity>,
    selected_key: Option<IdentityPublicKey>,
    selected_wallet: Option<Arc<RwLock<Wallet>>>,
    wallet_open_attempted: bool,
    wallet_unlock_popup: WalletUnlockPopup,
    show_json_popup: bool,
    json_popup_text: String,
    json_popup_opening_guard: ModalOpeningGuard,
    token_info_popup_opening_guard: ModalOpeningGuard,
    explanation_popup_opening_guard: ModalOpeningGuard,
    show_pop_up_info: Option<String>,
    adding_token_start_time: Option<DateTime<Utc>>,
    adding_token_name: Option<String>,
    operation_banner: Option<BannerHandle>,
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
    let in_dev_mode = app_context.user_role().at_least(UserRole::Power);

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
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn begin_token_registration_for_test(&mut self) -> BackendTaskContext {
        let task = BackendTask::TokenTask(Box::new(TokenTask::CheckTokenCreation));
        let context = BackendTaskContext::for_dispatch_on(&task, self.app_context.network);
        self.creator_workspace.pending = Some(context.clone());
        context
    }
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn token_creator_status_for_test(&self) -> TokenCreatorStatus {
        if self.creator_workspace.pending.is_some() {
            TokenCreatorStatus::WaitingForResult
        } else if self.creator_workspace.notice.is_some() {
            TokenCreatorStatus::Error
        } else if self.creator_workspace.outcome.is_some() {
            TokenCreatorStatus::Complete
        } else {
            TokenCreatorStatus::NotStarted
        }
    }
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn token_registration_pending_for_test(&self) -> bool {
        self.creator_workspace.pending.is_some()
    }

    pub fn new(app_context: &Arc<AppContext>, tokens_subscreen: TokensSubscreen) -> Self {
        let identities = app_context
            .load_local_qualified_identities()
            .unwrap_or_default()
            .into_iter()
            .map(|qi| (qi.identity.id(), qi))
            .collect();
        let all_known_tokens = app_context
            .get_all_known_tokens_with_data_contract()
            .unwrap_or_default();

        let my_tokens = my_tokens(
            app_context,
            &identities,
            &all_known_tokens,
            &IndexMap::new(),
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
            sort_column: SortColumn::OwnerIdentityName,
            sort_order: SortOrder::Ascending,
            use_custom_order: false,
            pending_backend_task: None,
            tokens_subscreen,
            refreshing_status: RefreshingStatus::NotRefreshing,
            pending_operation_context: None,

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

            creator_workspace: Default::default(),
            selected_identity: None,
            selected_key: None,
            selected_wallet: None,
            wallet_open_attempted: false,
            wallet_unlock_popup: WalletUnlockPopup::new(),
            show_json_popup: false,
            json_popup_text: String::new(),
            json_popup_opening_guard: ModalOpeningGuard::default(),
            token_info_popup_opening_guard: ModalOpeningGuard::default(),
            explanation_popup_opening_guard: ModalOpeningGuard::default(),
            show_pop_up_info: None,
            adding_token_start_time: None,
            adding_token_name: None,
            operation_banner: None,
        };

        if let Ok(saved_ids) = screen.app_context.load_token_order() {
            screen.reorder_vec_to(saved_ids);
            screen.use_custom_order = true;
        }

        // Seed selected_identity from the app-scoped selection (W4 SYNC); fall back to
        // the first identity the creator offers.
        if let Some(preferred_id) = screen.app_context.selected_identity_id()
            && let Some(qi) = screen
                .identities
                .get(&preferred_id)
                .filter(|qi| offered_by_token_creator(qi))
        {
            screen.selected_identity = Some(qi.clone());
        }
        if screen.selected_identity.is_none()
            && let Some((_, qi)) = screen
                .identities
                .iter()
                .find(|(_, qi)| offered_by_token_creator(qi))
        {
            screen.selected_identity = Some(qi.clone());
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
            .save_token_order(all_ids)
            .map_err(|e| {
                error!("Error saving token order: {}", e);
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

    // Message display is handled by the global MessageBanner

    /// Re-reads every stored identity. Only a successful read refreshes the Token
    /// Creator's selection, so a failed one does not drop what the user picked.
    fn reload_identities(&mut self) {
        match self.app_context.load_local_qualified_identities() {
            Ok(identities) => {
                self.identities = identities
                    .into_iter()
                    .map(|qi| (qi.identity.id(), qi))
                    .collect();
                self.refresh_token_creator_identity();
            }
            Err(error) => {
                tracing::warn!(?error, "Tokens screen could not reload stored identities");
                self.identities.clear();
            }
        }
    }

    /// Replaces the creator's copy of its selected identity with the one just read
    /// into `self.identities`, so keys and balance changed elsewhere show here. A
    /// selected key the identity no longer holds is dropped; an identity that is
    /// removed, or that the creator does not offer, is deselected so the selector
    /// picks one it offers.
    fn refresh_token_creator_identity(&mut self) {
        let Some(id) = self.selected_identity.as_ref().map(|qi| qi.identity.id()) else {
            return;
        };
        let Some(fresh) = self
            .identities
            .get(&id)
            .filter(|qi| offered_by_token_creator(qi))
        else {
            self.selected_identity = None;
            self.selected_key = None;
            return;
        };
        self.selected_key = self.selected_key.take().and_then(|selected| {
            fresh
                .private_keys
                .identity_public_keys()
                .iter()
                .map(|key_ref| &key_ref.1.identity_public_key)
                .find(|key| key.id() == selected.id() && key.data() == selected.data())
                .cloned()
        });
        self.selected_identity = Some(fresh.clone());
    }

    /// A network switch drops only this screen's draft and signing choices.
    /// The network-bound durable registration journal is retained.
    pub(crate) fn reset_for_network_switch(&mut self) {
        self.creator_workspace = Default::default();
        self.selected_identity = None;
        self.selected_key = None;
        self.selected_wallet = None;
        self.wallet_open_attempted = false;
        self.wallet_unlock_popup.close();
        self.show_json_popup = false;
        self.json_popup_text.clear();
        self.json_popup_opening_guard = ModalOpeningGuard::default();
    }

    fn add_token_to_tracked_tokens(&mut self, token_info: TokenInfo) -> Result<AppAction, String> {
        // Check if token is already added
        if self.all_known_tokens.contains_key(&token_info.token_id) {
            MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Token already in My Tokens",
                MessageType::Error,
            );
            return Ok(AppAction::None);
        }

        // Set adding status
        self.adding_token_start_time = Some(Utc::now());
        self.adding_token_name = Some(token_info.token_name.clone());
        self.operation_banner.take_and_clear();
        let handle = MessageBanner::set_global(
            self.app_context.egui_ctx(),
            "Adding token...",
            MessageType::Info,
        );
        handle.with_elapsed();
        self.operation_banner = Some(handle);

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
            self.contract_search_status = ContractSearchStatus::WaitingForResult;
            self.operation_banner.take_and_clear();
            let handle = MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Searching contracts...",
                MessageType::Info,
            );
            handle.with_elapsed();
            self.operation_banner = Some(handle);

            // push the current one onto "previous" so we can go back
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
            self.contract_search_status = ContractSearchStatus::WaitingForResult;
            self.operation_banner.take_and_clear();
            let handle = MessageBanner::set_global(
                self.app_context.egui_ctx(),
                "Searching contracts...",
                MessageType::Info,
            );
            handle.with_elapsed();
            self.operation_banner = Some(handle);

            // The "last" previous_cursors item is the new page's state
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

    fn show_remove_identity_token_balance_popup(&mut self, ui: &mut Ui) -> AppAction {
        // If no token is set, nothing to confirm
        let token_to_remove = match &self.identity_token_balance_to_remove {
            Some(token) => token.clone(),
            None => {
                self.confirm_remove_identity_token_balance_popup = false;
                return AppAction::None;
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

        let mut action = AppAction::None;
        if let Some(status) = response.dialog_response {
            if let ConfirmationStatus::Confirmed = status {
                action = AppAction::BackendTask(BackendTask::TokenTask(Box::new(
                    TokenTask::StopTrackingTokenBalance(IdentityTokenIdentifier {
                        identity_id: token_to_remove.identity_id,
                        token_id: token_to_remove.token_id,
                    }),
                )));
            }
            self.confirm_remove_identity_token_balance_popup = false;
            self.identity_token_balance_to_remove = None;
            self.remove_identity_token_balance_confirmation_dialog = None;
        }
        action
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
                    if let Err(error) = self.app_context.remove_token(&token_to_remove) {
                        MessageBanner::set_global(
                            self.app_context.egui_ctx(),
                            "The token balance could not be removed. Refresh and try again.",
                            MessageType::Error,
                        )
                        .with_details(error);
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

pub(super) fn offered_by_token_creator(identity: &QualifiedIdentity) -> bool {
    identity.identity_type == IdentityType::User && !identity.private_keys.is_empty()
}

// ─────────────────────────────────────────────────────────────────
// ScreenLike implementation
// ─────────────────────────────────────────────────────────────────
impl ScreenLike for TokensScreen {
    fn cached_data_changed(&mut self, update: crate::backend_task::CacheUpdate) {
        use crate::backend_task::CacheDomain;
        if update.network != self.app_context.network {
            return;
        }
        let tokens_changed = matches!(update.domain, CacheDomain::Tokens | CacheDomain::All);
        let identities_changed =
            matches!(update.domain, CacheDomain::Identities | CacheDomain::All);
        if !tokens_changed && !identities_changed {
            return;
        }
        if tokens_changed {
            self.all_known_tokens = self
                .app_context
                .get_all_known_tokens_with_data_contract()
                .unwrap_or_default();
        }
        if identities_changed {
            self.reload_identities();
        }
        // My Tokens joins both: one recomputation after whichever side changed.
        self.my_tokens = my_tokens(
            &self.app_context,
            &self.identities,
            &self.all_known_tokens,
            &self.token_pricing_data,
        );
    }
    fn refresh(&mut self) {
        self.all_known_tokens = self
            .app_context
            .get_all_known_tokens_with_data_contract()
            .unwrap_or_default();

        self.reload_identities();

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

        match self.app_context.load_token_order() {
            Ok(saved_ids) => {
                self.reorder_vec_to(saved_ids);

                self.use_custom_order = true;
            }
            Err(e) => {
                error!("Error loading token order: {}", e);
            }
        }
    }

    fn refresh_on_arrival(&mut self) {
        self.selected_token = None;

        self.all_known_tokens = self
            .app_context
            .get_all_known_tokens_with_data_contract()
            .unwrap_or_default();
        self.reload_identities();

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

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        let mut action = AppAction::None;

        // Build top-right buttons
        let right_buttons = match self.tokens_subscreen {
            TokensSubscreen::MyTokens => vec![
                (
                    "Create Token",
                    DesiredAppAction::SetMainScreen(RootScreenType::RootScreenTokenCreator),
                ),
                (
                    "Import Token",
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
                ui,
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
                ui,
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
            action |= add_top_panel_with_global_nav(
                ui,
                &self.app_context,
                subdued_everyday_spec("Tokens", RootScreenType::RootScreenMyTokenBalances),
                right_buttons.clone(),
            );
        }

        // Left panel
        match self.tokens_subscreen {
            TokensSubscreen::MyTokens => {
                action |= add_left_panel(
                    ui,
                    &self.app_context,
                    RootScreenType::RootScreenMyTokenBalances,
                );
            }
            TokensSubscreen::SearchTokens => {
                action |=
                    add_left_panel(ui, &self.app_context, RootScreenType::RootScreenTokenSearch);
            }
            TokensSubscreen::TokenCreator => {
                action |= add_left_panel(
                    ui,
                    &self.app_context,
                    RootScreenType::RootScreenTokenCreator,
                );
            }
        }

        // Subscreen chooser
        action |= add_tokens_subscreen_chooser_panel(ui, self.app_context.as_ref());

        // Main panel
        action |= island_central_panel(ui, |ui| {
            if self.tokens_subscreen == TokensSubscreen::TokenCreator {
                return self.render_token_workspace(ctx, ui);
            }
            egui::ScrollArea::vertical()
                .show(ui, |ui| {
                    let mut inner_action = AppAction::None;

                    match self.tokens_subscreen {
                        TokensSubscreen::MyTokens => {
                            inner_action |= self.render_my_tokens_subscreen(ui);
                        }
                        TokensSubscreen::SearchTokens => {
                            if let Some(contract_id) = self.selected_contract_id {
                                inner_action |= self.render_contract_details(ui, &contract_id);
                                // Render the JSON popup if needed
                                if self.show_json_popup {
                                    self.render_data_contract_json_popup(ui);
                                }
                            } else {
                                inner_action |= self.render_keyword_search(ui);
                            }
                        }
                        TokensSubscreen::TokenCreator => {}
                    }

                    // Elapsed display for refreshing is handled by the global MessageBanner

                    if self.confirm_remove_identity_token_balance_popup {
                        inner_action |= self.show_remove_identity_token_balance_popup(ui);
                    }
                    if self.confirm_remove_token_popup {
                        self.show_remove_token_popup(ui);
                    }

                    // If we have info text, open a pop-up window to show it
                    if let Some(info_text) = self.show_pop_up_info.clone() {
                        let mut popup = InfoPopup::new(
                            egui::Id::new("tokens_screen_info_popup"),
                            "Information",
                            &info_text,
                        );
                        if popup.show(ui).inner {
                            self.show_pop_up_info = None;
                        }
                    }

                    inner_action
                })
                .inner
        });

        let duplicate_balance_refresh = is_duplicate_balance_refresh(
            &self.refreshing_status,
            self.pending_operation_context.as_ref(),
            &action,
        );
        // A tracked operation is sent under a dispatch context of its own, so
        // only its own result or failure settles it.
        let mut dispatch_context = None;
        if duplicate_balance_refresh {
            action = AppAction::None;
        } else if let AppAction::BackendTask(task) = &action
            && matches!(
                BackendTaskContext::from(task),
                BackendTaskContext::TokenBalanceRefresh
                    | BackendTaskContext::TokenRewardEstimate(_)
            )
        {
            let context = BackendTaskContext::for_dispatch_on(task, self.app_context.network);
            self.pending_operation_context = Some(context.clone());
            dispatch_context = Some(context);
        }

        // Post-processing on user actions
        match action {
            AppAction::BackendTask(BackendTask::TokenTask(ref token_task))
                if matches!(token_task.as_ref(), TokenTask::QueryMyTokenBalances) =>
            {
                self.refreshing_status = RefreshingStatus::Refreshing;
                self.operation_banner.take_and_clear();
                let handle =
                    MessageBanner::set_global(ctx, "Refreshing tokens...", MessageType::Info);
                handle.with_elapsed();
                self.operation_banner = Some(handle);
            }
            AppAction::SetMainScreenThenGoToMainScreen(_) => {
                self.refreshing_status = RefreshingStatus::NotRefreshing;
                self.pending_operation_context = None;

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

        if let Some(context) = dispatch_context
            && let AppAction::BackendTask(task) = std::mem::replace(&mut action, AppAction::None)
        {
            action = AppAction::BackendTaskWithContext { task, context };
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
        // Banner display is handled globally by AppState; this is only for side-effects.

        // Uncorrelated failures must not clear a genuine token refresh banner.
        if matches!(msg_type, MessageType::Error | MessageType::Warning)
            && self.refreshing_status != RefreshingStatus::Refreshing
        {
            self.operation_banner.take_and_clear();
        }

        // Reset contract details loading on any error/warning
        if matches!(msg_type, MessageType::Error | MessageType::Warning)
            && self.contract_details_loading
        {
            self.contract_details_loading = false;
        }

        match self.tokens_subscreen {
            // Registration failures arrive through `display_backend_task_error`.
            TokensSubscreen::TokenCreator => {}
            TokensSubscreen::MyTokens => {
                tracing::debug!(
                    msg = msg,
                    ?msg_type,
                    "unsupported message received in token screen"
                );
            }
            TokensSubscreen::SearchTokens => {
                if msg_type == MessageType::Error {
                    self.contract_search_status = ContractSearchStatus::Error;
                    // Clear adding status on error
                    self.adding_token_start_time = None;
                    self.adding_token_name = None;
                } else if msg.contains("Added token")
                    | msg.contains("Token already added")
                    | msg.contains("Saved token to db")
                {
                    // Clear adding status (success message shown by global banner)
                    self.adding_token_start_time = None;
                    self.adding_token_name = None;
                }
            }
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        self.workspace_error(context, _error);
        if self.tokens_subscreen == TokensSubscreen::MyTokens {
            let was_stopped = self.refreshing_status == RefreshingStatus::Refreshing
                && self.pending_operation_context.as_ref() == Some(context);
            let expected = self.pending_operation_context.clone();
            self.refreshing_status
                .stop_on_failure(expected.as_ref(), context);
            if !was_stopped {
                return;
            }
            self.pending_operation_context = None;
            self.operation_banner.take_and_clear();
        }
    }

    fn display_task_error(&mut self, _error: &TaskError) -> bool {
        false
    }

    fn display_backend_task_result(
        &mut self,
        context: &BackendTaskContext,
        backend_task_success_result: BackendTaskSuccessResult,
    ) {
        self.workspace_result(context, &backend_task_success_result);
        let completes_pending = completes_pending_operation(
            self.pending_operation_context.as_ref(),
            context,
            &backend_task_success_result,
        );
        match backend_task_success_result {
            BackendTaskSuccessResult::DescriptionsByKeyword(descriptions, next_cursor) => {
                let mut sr = self.search_results.lock_recover();
                *sr = descriptions;
                self.search_has_next_page = next_cursor.is_some();
                if let Some(cursor) = next_cursor {
                    self.next_cursors.push(cursor);
                }
                self.contract_search_status = ContractSearchStatus::Complete;
            }
            BackendTaskSuccessResult::ContractsWithDescriptions(contracts_with_descriptions) => {
                let default_info = (None, vec![]);
                let info = self
                    .selected_contract_id
                    .and_then(|id| contracts_with_descriptions.get(&id))
                    .unwrap_or(&default_info);

                self.selected_contract_description = info.0.clone();
                self.selected_token_infos = info.1.clone();
                self.contract_details_loading = false;
            }
            BackendTaskSuccessResult::TokenEstimatedNonClaimedPerpetualDistributionAmountWithExplanation(
                identity_token_id,
                amount,
                explanation,
            ) => {
                if completes_pending {
                    self.refreshing_status = RefreshingStatus::NotRefreshing;
                    self.pending_operation_context = None;
                    self.operation_banner.take_and_clear();
                }
                if let Some(itb) = self.my_tokens.get_mut(&identity_token_id) {
                    itb.estimated_unclaimed_rewards = Some(amount);
                }
                self.reward_explanations.insert(identity_token_id, explanation);
            }
            BackendTaskSuccessResult::TokenPricing { token_id, prices } => {
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
            }
            BackendTaskSuccessResult::FetchedTokenBalances
            | BackendTaskSuccessResult::TokenBalanceRefreshAlreadyInFlight => {
                if completes_pending {
                    self.refreshing_status = RefreshingStatus::NotRefreshing;
                    self.pending_operation_context = None;
                    self.operation_banner.take_and_clear();
                }
                // Refresh my_tokens to show updated balances
                self.my_tokens = my_tokens(
                    &self.app_context,
                    &self.identities,
                    &self.all_known_tokens,
                    &self.token_pricing_data,
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::qualified_identity::IdentityStatus;
    use crate::model::qualified_identity::encrypted_key_storage::KeyStorage;
    use dash_sdk::dpp::dashcore::Network;
    use dash_sdk::dpp::data_contract::TokenConfiguration;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;
    use dash_sdk::dpp::identity::{KeyID, KeyType, Purpose, SecurityLevel};
    use dash_sdk::platform::DataContract;
    use dash_sdk::platform::Identity;
    use std::collections::BTreeMap;

    #[test]
    fn in_flight_token_refresh_failure_requires_the_matching_backend_task() {
        let expected = BackendTaskContext::TokenBalanceRefresh;
        let mut status = RefreshingStatus::Refreshing;
        status.stop_on_failure(Some(&expected), &BackendTaskContext::Other);
        status.stop_on_failure(Some(&expected), &BackendTaskContext::Unknown);
        assert_eq!(status, RefreshingStatus::Refreshing);

        status.stop_on_failure(Some(&expected), &expected);
        assert_eq!(status, RefreshingStatus::NotRefreshing);

        status.stop_on_failure(Some(&expected), &expected);
        assert_eq!(status, RefreshingStatus::NotRefreshing);
    }

    #[test]
    fn duplicate_balance_refresh_is_suppressed_only_while_the_first_is_pending() {
        let action = AppAction::BackendTask(BackendTask::TokenTask(Box::new(
            TokenTask::QueryMyTokenBalances,
        )));

        assert!(is_duplicate_balance_refresh(
            &RefreshingStatus::Refreshing,
            Some(&BackendTaskContext::TokenBalanceRefresh),
            &action,
        ));
        assert!(!is_duplicate_balance_refresh(
            &RefreshingStatus::NotRefreshing,
            None,
            &action,
        ));
    }

    #[test]
    fn token_balance_success_requires_the_refresh_task_context() {
        let pending = BackendTaskContext::TokenBalanceRefresh;
        let result = BackendTaskSuccessResult::FetchedTokenBalances;

        assert!(completes_pending_operation(
            Some(&pending),
            &BackendTaskContext::TokenBalanceRefresh,
            &result,
        ));
        assert!(!completes_pending_operation(
            Some(&pending),
            &BackendTaskContext::Other,
            &result,
        ));
    }

    #[test]
    fn in_flight_reward_estimate_failure_requires_the_matching_pair() {
        let expected = BackendTaskContext::TokenRewardEstimate(IdentityTokenIdentifier {
            identity_id: Identifier::from([1; 32]),
            token_id: Identifier::from([2; 32]),
        });
        let other_pair = BackendTaskContext::TokenRewardEstimate(IdentityTokenIdentifier {
            identity_id: Identifier::from([1; 32]),
            token_id: Identifier::from([3; 32]),
        });
        let mut status = RefreshingStatus::Refreshing;

        status.stop_on_failure(Some(&expected), &other_pair);
        assert_eq!(status, RefreshingStatus::Refreshing);

        status.stop_on_failure(Some(&expected), &expected);
        assert_eq!(status, RefreshingStatus::NotRefreshing);
    }
    fn known_token(token_id: Identifier, context: &AppContext) -> TokenInfoWithDataContract {
        use dash_sdk::dpp::data_contract::config::DataContractConfig;
        use dash_sdk::dpp::data_contract::v1::DataContractV1;
        let token_configuration =
            TokenConfiguration::V0(TokenConfigurationV0::default_most_restrictive());
        let data_contract = DataContract::V1(DataContractV1 {
            id: Identifier::from([0x33; 32]),
            version: 1,
            owner_id: Identifier::from([0x34; 32]),
            document_types: BTreeMap::new(),
            config: DataContractConfig::default_for_version(context.platform_version())
                .expect("contract config"),
            schema_defs: None,
            groups: BTreeMap::new(),
            tokens: BTreeMap::from([(0, token_configuration.clone())]),
            keywords: vec![],
            created_at: None,
            updated_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            created_at_epoch: None,
            updated_at_epoch: None,
            description: None,
        });
        TokenInfoWithDataContract {
            token_id,
            token_name: "marker".to_owned(),
            data_contract,
            token_position: 0,
            token_configuration,
            description: None,
        }
    }
    fn screen_with_unstored_entries()
    -> (TokensScreen, Identifier, Arc<AppContext>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(tmp.path());
        let mut screen = TokensScreen::new(&context, TokensSubscreen::TokenCreator);
        let marker = Identifier::from([0x32; 32]);
        screen.token_pricing_data.insert(marker, None);
        screen.selected_token = Some(marker);
        let identity = QualifiedIdentity {
            identity: Identity::create_basic_identity(marker, context.platform_version()).unwrap(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: crate::model::qualified_identity::IdentityType::User,
            alias: None,
            private_keys: KeyStorage::default(),
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: context.network,
        };
        screen.identities.insert(marker, identity);
        screen
            .all_known_tokens
            .insert(marker, known_token(marker, &context));
        (screen, marker, context, tmp)
    }
    fn other_network(network: Network) -> Network {
        if network == Network::Mainnet {
            Network::Testnet
        } else {
            Network::Mainnet
        }
    }

    #[test]
    fn token_cache_updates_reload_only_the_matching_network_and_keep_form_state() {
        use crate::backend_task::{CacheDomain, CacheUpdate};
        let (mut screen, marker, context, _tmp) = screen_with_unstored_entries();
        screen.cached_data_changed(CacheUpdate {
            network: other_network(context.network),
            domain: CacheDomain::All,
        });
        assert!(screen.identities.contains_key(&marker));
        assert!(screen.all_known_tokens.contains_key(&marker));

        screen.cached_data_changed(CacheUpdate {
            network: context.network,
            domain: CacheDomain::All,
        });
        assert!(!screen.identities.contains_key(&marker));
        assert!(!screen.all_known_tokens.contains_key(&marker));
        assert_eq!(screen.selected_token, Some(marker));
        assert!(screen.token_pricing_data.contains_key(&marker));
    }

    #[test]
    fn a_cache_update_re_reads_only_the_domain_it_names() {
        use crate::backend_task::{CacheDomain, CacheUpdate};
        for (domain, identities_kept, tokens_kept) in [
            (CacheDomain::Identities, false, true),
            (CacheDomain::Tokens, true, false),
            (CacheDomain::Dpns, true, true),
        ] {
            let (mut screen, marker, context, _tmp) = screen_with_unstored_entries();
            screen.cached_data_changed(CacheUpdate {
                network: context.network,
                domain,
            });
            assert_eq!(
                screen.identities.contains_key(&marker),
                identities_kept,
                "{domain:?} re-reads identities"
            );
            assert_eq!(
                screen.all_known_tokens.contains_key(&marker),
                tokens_kept,
                "{domain:?} re-reads tokens"
            );
            assert_eq!(screen.selected_token, Some(marker), "{domain:?}");
            assert!(
                screen.token_pricing_data.contains_key(&marker),
                "{domain:?}"
            );
        }
    }

    #[test]
    fn my_tokens_is_recomputed_after_a_change_to_a_domain_it_joins() {
        use crate::backend_task::{CacheDomain, CacheUpdate};
        for (domain, recomputed) in [
            (CacheDomain::Identities, true),
            (CacheDomain::Tokens, true),
            (CacheDomain::All, true),
            (CacheDomain::Dpns, false),
        ] {
            let (mut screen, marker, context, _tmp) = screen_with_unstored_entries();
            // A stale entry: the store holds no balance, so a recomputation
            // drops it.
            let identity = screen.identities.get(&marker).unwrap().clone();
            let token = screen.all_known_tokens.get(&marker).unwrap().clone();
            let stale = IdentityTokenBalance {
                token_id: marker,
                token_alias: token.token_name.clone(),
                token_config: token.token_configuration.clone(),
                identity_id: marker,
                balance: 7,
                estimated_unclaimed_rewards: None,
                data_contract_id: Identifier::from([0x33; 32]),
                token_position: 0,
            };
            let key = IdentityTokenIdentifier {
                identity_id: marker,
                token_id: marker,
            };
            screen.my_tokens.insert(
                key,
                stale.into_with_actions(&identity, &token.data_contract, false, None),
            );

            screen.cached_data_changed(CacheUpdate {
                network: context.network,
                domain,
            });
            assert_eq!(
                !screen.my_tokens.contains_key(&key),
                recomputed,
                "{domain:?} recomputes My Tokens"
            );
        }
    }
    fn creator_identity(
        context: &AppContext,
        keys: &[(KeyID, Purpose, SecurityLevel)],
    ) -> QualifiedIdentity {
        identity_holding(context, 0x41, IdentityType::User, keys)
    }
    fn identity_holding(
        context: &AppContext,
        id: u8,
        identity_type: IdentityType,
        keys: &[(KeyID, Purpose, SecurityLevel)],
    ) -> QualifiedIdentity {
        use crate::context::test_support::identity_public_key;
        use crate::model::qualified_identity::PrivateKeyTarget;
        use crate::model::qualified_identity::encrypted_key_storage::PrivateKeyData;
        use crate::model::qualified_identity::qualified_identity_public_key::QualifiedIdentityPublicKey;

        let mut private_keys = KeyStorage::default();
        for &(id, purpose, security_level) in keys {
            let key = identity_public_key(
                id,
                purpose,
                security_level,
                KeyType::ECDSA_HASH160,
                vec![id as u8; 20],
            );
            private_keys.insert_at(
                (PrivateKeyTarget::PrivateKeyOnMainIdentity, id),
                (
                    QualifiedIdentityPublicKey::from(key),
                    PrivateKeyData::Clear(crate::test_support::keys::secret_bytes()),
                ),
            );
        }
        QualifiedIdentity {
            identity: Identity::create_basic_identity(
                Identifier::from([id; 32]),
                context.platform_version(),
            )
            .expect("basic identity"),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type,
            alias: None,
            private_keys,
            dpns_names: vec![],
            associated_wallets: BTreeMap::new(),
            secret_access: None,
            wallet_index: None,
            top_ups: BTreeMap::new(),
            status: IdentityStatus::Active,
            network: context.network,
        }
    }
    fn selected_key_ids(screen: &TokensScreen) -> Vec<KeyID> {
        let mut ids: Vec<KeyID> = screen
            .selected_identity
            .as_ref()
            .expect("an identity stays selected")
            .private_keys
            .identity_public_keys()
            .iter()
            .map(|key_ref| key_ref.1.identity_public_key.id())
            .collect();
        ids.sort_unstable();
        ids
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn token_creator_shows_the_current_keys_of_the_selected_identity() {
        use crate::backend_task::{CacheDomain, CacheUpdate};

        for arrival in ["navigation", "cache update"] {
            let tmp = tempfile::tempdir().expect("test tmpdir");
            let context = crate::context::test_support::wired_test_app_context(tmp.path()).await;
            let stored = creator_identity(
                &context,
                &[(1, Purpose::AUTHENTICATION, SecurityLevel::MEDIUM)],
            );
            context
                .insert_local_qualified_identity(&stored, &None)
                .expect("store identity");
            let mut screen = TokensScreen::new(&context, TokensSubscreen::TokenCreator);
            assert_eq!(selected_key_ids(&screen), vec![1]);
            screen.creator_workspace.draft.description = "Kept across navigation".to_string();

            let with_new_key = creator_identity(
                &context,
                &[
                    (1, Purpose::AUTHENTICATION, SecurityLevel::MEDIUM),
                    (2, Purpose::AUTHENTICATION, SecurityLevel::HIGH),
                ],
            );
            context
                .update_local_qualified_identity(&with_new_key)
                .expect("add key");

            if arrival == "navigation" {
                screen.refresh_on_arrival();
            } else {
                screen.cached_data_changed(CacheUpdate {
                    network: context.network,
                    domain: CacheDomain::Identities,
                });
            }

            assert_eq!(selected_key_ids(&screen), vec![1, 2], "after {arrival}");
            assert_eq!(
                screen.creator_workspace.draft.description, "Kept across navigation",
                "after {arrival}"
            );
            context.wallet_backend().unwrap().shutdown().await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn token_creator_drops_a_selected_key_the_identity_no_longer_holds() {
        let tmp = tempfile::tempdir().expect("test tmpdir");
        let context = crate::context::test_support::wired_test_app_context(tmp.path()).await;
        let stored = creator_identity(
            &context,
            &[
                (1, Purpose::AUTHENTICATION, SecurityLevel::HIGH),
                (2, Purpose::AUTHENTICATION, SecurityLevel::CRITICAL),
            ],
        );
        context
            .insert_local_qualified_identity(&stored, &None)
            .expect("store identity");
        let mut screen = TokensScreen::new(&context, TokensSubscreen::TokenCreator);
        let key_of = |id: KeyID| {
            stored
                .private_keys
                .identity_public_keys()
                .iter()
                .map(|key_ref| key_ref.1.identity_public_key.clone())
                .find(|key| key.id() == id)
                .expect("stored key")
        };

        screen.selected_key = Some(key_of(1));
        screen.refresh_on_arrival();
        assert_eq!(screen.selected_key.as_ref().map(|key| key.id()), Some(1));

        screen.selected_key = Some(key_of(2));
        let without_key_2 = creator_identity(
            &context,
            &[(1, Purpose::AUTHENTICATION, SecurityLevel::HIGH)],
        );
        context
            .update_local_qualified_identity(&without_key_2)
            .expect("remove key");
        screen.refresh_on_arrival();
        assert!(screen.selected_key.is_none());
        context.wallet_backend().unwrap().shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn token_creator_deselects_an_identity_that_is_no_longer_stored() {
        let tmp = tempfile::tempdir().expect("test tmpdir");
        let app_context = crate::context::test_support::wired_test_app_context(tmp.path()).await;
        let mut screen = TokensScreen::new(&app_context, TokensSubscreen::TokenCreator);
        fill_minimal_creator_inputs(&mut screen, &app_context);

        screen.refresh_on_arrival();

        assert!(screen.selected_identity.is_none());
        assert_eq!(
            screen.creator_workspace.draft.names[0].singular,
            "PresetCoin"
        );
        app_context.wallet_backend().unwrap().shutdown().await;
    }

    #[test]
    fn token_creator_keeps_its_identity_when_identities_cannot_be_read() {
        let tmp = tempfile::tempdir().expect("test tmpdir");
        // Without a wallet backend, reading stored identities fails.
        let app_context = crate::context::test_support::test_app_context(tmp.path());
        let mut screen = TokensScreen::new(&app_context, TokensSubscreen::TokenCreator);
        fill_minimal_creator_inputs(&mut screen, &app_context);

        screen.refresh_on_arrival();

        assert_eq!(
            screen.selected_identity.map(|qi| qi.identity.id()),
            Some(Identifier::from([7; 32]))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn token_creator_never_keeps_an_identity_it_does_not_offer() {
        let key = [(1, Purpose::AUTHENTICATION, SecurityLevel::HIGH)];
        for (case, ineligible) in [
            ("masternode", IdentityType::Masternode),
            ("keyless user", IdentityType::User),
        ] {
            let tmp = tempfile::tempdir().expect("test tmpdir");
            let context = crate::context::test_support::wired_test_app_context(tmp.path()).await;
            let ineligible_keys: &[_] = if ineligible == IdentityType::User {
                &[]
            } else {
                &key
            };
            let ineligible = identity_holding(&context, 0x01, ineligible, ineligible_keys);
            let ineligible_id = ineligible.identity.id();
            context
                .insert_local_qualified_identity(&ineligible, &None)
                .expect("store ineligible identity");
            context.set_selected_identity(Some(ineligible_id));

            // Opened before any identity it offers exists: nothing is selected.
            let mut screen = TokensScreen::new(&context, TokensSubscreen::TokenCreator);
            assert!(screen.selected_identity.is_none(), "{case}: seed");

            // A selection made from an older roster is dropped on return, so the
            // selector picks an identity it offers.
            let eligible = creator_identity(&context, &key);
            context
                .insert_local_qualified_identity(&eligible, &None)
                .expect("store eligible identity");
            screen.selected_identity = Some(ineligible.clone());
            screen.refresh_on_arrival();
            assert!(screen.selected_identity.is_none(), "{case}: arrival");

            // A creator opened now starts from an identity it offers.
            let screen = TokensScreen::new(&context, TokensSubscreen::TokenCreator);
            assert_eq!(
                screen.selected_identity.map(|qi| qi.identity.id()),
                Some(eligible.identity.id()),
                "{case}: seed with an offered identity"
            );
            context.wallet_backend().unwrap().shutdown().await;
        }
    }

    #[test]
    fn network_switch_closes_the_wallet_unlock_popup() {
        let tmp = tempfile::tempdir().expect("test tmpdir");
        let app_context = crate::context::test_support::test_app_context(tmp.path());
        let mut screen = TokensScreen::new(&app_context, TokensSubscreen::TokenCreator);
        screen.wallet_unlock_popup.open();

        screen.reset_for_network_switch();

        assert!(!screen.wallet_unlock_popup.is_open());
    }

    #[test]
    fn network_switch_closes_the_contract_json_popup() {
        let tmp = tempfile::tempdir().expect("test tmpdir");
        let app_context = crate::context::test_support::test_app_context(tmp.path());
        let mut screen = TokensScreen::new(&app_context, TokensSubscreen::TokenCreator);
        screen.open_data_contract_json_popup("{\"previous\": \"network\"}".to_owned());

        screen.reset_for_network_switch();

        assert!(!screen.show_json_popup);
        assert!(screen.json_popup_text.is_empty());
    }

    #[test]
    fn creator_rejects_duplicate_group_members_without_overwriting_power() {
        use crate::model::token_creator::groups::{GroupDraft, MemberDraft};
        let mut group = GroupDraft::new(0);
        group.members = ["1", "2"]
            .map(|weight| MemberDraft {
                identity: Identifier::from([1; 32]).to_string(Encoding::Base58),
                weight: weight.into(),
            })
            .into();
        assert!(
            group
                .parse(dash_sdk::dpp::version::PlatformVersion::latest())
                .is_err()
        );
    }

    #[test]
    fn creator_rejects_unreachable_group_threshold() {
        use crate::model::token_creator::groups::{GroupDraft, MemberDraft};
        let mut group = GroupDraft::new(0);
        group.threshold = "3".into();
        group.members = [1, 2]
            .map(|id| MemberDraft {
                identity: Identifier::from([id; 32]).to_string(Encoding::Base58),
                weight: "1".into(),
            })
            .into();
        assert!(
            group
                .parse(dash_sdk::dpp::version::PlatformVersion::latest())
                .is_err()
        );
    }

    #[test]
    fn creator_rejects_malformed_recurring_amount_instead_of_zero() {
        use crate::model::token_creator::{RecurringDraft, distributions::Parameter};
        let mut recurring = RecurringDraft {
            enabled: true,
            ..Default::default()
        };
        recurring.functions[0].amounts.insert(
            Parameter::Amount,
            crate::model::token_creator::amount::TokenAmountInput::tokens("not-an-amount"),
        );
        assert!(
            recurring
                .parse(
                    2,
                    Network::Testnet,
                    dash_sdk::dpp::version::PlatformVersion::latest()
                )
                .is_err()
        );
    }

    #[test]
    fn creator_rejects_duplicate_schedule_recipient_at_same_time() {
        use crate::model::token_creator::{
            Recipient, ScheduleDraft, TokenDraft, amount::TokenAmountInput,
        };
        let mut draft = TokenDraft::example(false);
        draft.scheduled_enabled = true;
        draft.scheduled = ["10", "20"]
            .into_iter()
            .enumerate()
            .map(|(id, value)| ScheduleDraft {
                id: id as u64,
                timestamp: Some(100_000),
                recipient: Recipient::Owner,
                amount: TokenAmountInput::tokens(value),
            })
            .collect();
        assert!(
            draft
                .parse(
                    Identifier::from([7; 32]),
                    Network::Testnet,
                    dash_sdk::dpp::version::PlatformVersion::latest(),
                    1000
                )
                .is_err()
        );
    }
    fn fill_minimal_creator_inputs(screen: &mut TokensScreen, context: &Arc<AppContext>) {
        screen.creator_workspace.draft = crate::model::token_creator::TokenDraft::example(false);
        screen.creator_workspace.draft.names[0].singular = "PresetCoin".into();
        screen.selected_identity = Some(identity_holding(
            context,
            7,
            IdentityType::User,
            &[(1, Purpose::AUTHENTICATION, SecurityLevel::HIGH)],
        ));
    }
}
