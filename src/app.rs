use crate::app_dir::{
    app_user_data_file_path, copy_env_file_if_not_exists,
    create_app_user_data_directory_if_not_exists,
};
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::core::CoreItem;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::components::core_zmq_listener::{CoreZMQListener, ZMQMessage};
use crate::context::AppContext;
use crate::context::connection_status::ConnectionStatus;
use crate::database::Database;
use crate::logging::initialize_logger;
use crate::model::settings::Settings;
use crate::ui::components::MessageBanner;
use crate::ui::contracts_documents::contracts_documents_screen::DocumentQueryScreen;
use crate::ui::dashpay::{DashPayScreen, DashPaySubscreen, ProfileSearchScreen};
use crate::ui::dpns::dpns_contested_names_screen::{
    DPNSScreen, DPNSSubscreen, ScheduledVoteCastingStatus,
};
use crate::ui::identities::identities_screen::IdentitiesScreen;
use crate::ui::network_chooser_screen::NetworkChooserScreen;
use crate::ui::theme::ThemeMode;
use crate::ui::tokens::tokens_screen::{TokensScreen, TokensSubscreen};
use crate::ui::tools::address_balance_screen::AddressBalanceScreen;
use crate::ui::tools::contract_visualizer_screen::ContractVisualizerScreen;
use crate::ui::tools::document_visualizer_screen::DocumentVisualizerScreen;
use crate::ui::tools::grovestark_screen::GroveSTARKScreen;
use crate::ui::tools::masternode_list_diff_screen::MasternodeListDiffScreen;
use crate::ui::tools::platform_info_screen::PlatformInfoScreen;
use crate::ui::tools::proof_log_screen::ProofLogScreen;
use crate::ui::tools::proof_visualizer_screen::ProofVisualizerScreen;
use crate::ui::tools::transition_visualizer_screen::TransitionVisualizerScreen;
use crate::ui::wallets::wallets_screen::WalletsBalancesScreen;
use crate::ui::welcome_screen::WelcomeScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike, ScreenType};
use crate::utils::egui_mpsc::{self, EguiMpscAsync, EguiMpscSync};
use crate::utils::tasks::TaskManager;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use derive_more::From;
use eframe::{App, egui};
use std::collections::BTreeMap;
use std::ops::BitOrAssign;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime};
use std::vec;
use tokio::sync::mpsc as tokiompsc;

#[derive(Debug, From)]
pub enum TaskResult {
    Refresh,
    Success(Box<BackendTaskSuccessResult>),
    Error(String),
}

impl From<Result<BackendTaskSuccessResult, String>> for TaskResult {
    fn from(value: Result<BackendTaskSuccessResult, String>) -> Self {
        match value {
            Ok(value) => TaskResult::Success(Box::new(value)),
            Err(e) => TaskResult::Error(e),
        }
    }
}

pub struct AppState {
    pub main_screens: BTreeMap<RootScreenType, Screen>,
    pub selected_main_screen: RootScreenType,
    pub screen_stack: Vec<Screen>,
    pub chosen_network: Network,
    pub connection_status: Arc<ConnectionStatus>,
    pub mainnet_app_context: Arc<AppContext>,
    pub testnet_app_context: Option<Arc<AppContext>>,
    pub devnet_app_context: Option<Arc<AppContext>>,
    pub local_app_context: Option<Arc<AppContext>>,
    #[allow(dead_code)] // Kept alive for the lifetime of the app
    pub mainnet_core_zmq_listener: Option<CoreZMQListener>,
    #[allow(dead_code)] // Kept alive for the lifetime of the app
    pub testnet_core_zmq_listener: Option<CoreZMQListener>,
    #[allow(dead_code)] // Kept alive for the lifetime of the app
    pub devnet_core_zmq_listener: Option<CoreZMQListener>,
    #[allow(dead_code)] // Kept alive for the lifetime of the app
    pub local_core_zmq_listener: Option<CoreZMQListener>,
    pub core_message_receiver: mpsc::Receiver<(ZMQMessage, Network)>,
    pub task_result_sender: egui_mpsc::SenderAsync<TaskResult>, // Channel sender for sending task results
    pub task_result_receiver: tokiompsc::Receiver<TaskResult>, // Channel receiver for receiving task results
    pub theme_preference: ThemeMode,                           // Current theme preference
    last_scheduled_vote_check: Instant, // Last time we checked if there are scheduled masternode votes to cast
    last_repaint_request: Instant,      // Throttle periodic repaint scheduling to once per second
    pub subtasks: Arc<TaskManager>,     // Subtasks manager for graceful shutdown
    /// Whether to show the welcome/onboarding screen
    pub show_welcome_screen: bool,
    /// The welcome screen instance (only created if needed)
    pub welcome_screen: Option<WelcomeScreen>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DesiredAppAction {
    None,
    #[allow(dead_code)] // May be used in future for explicit refresh actions
    Refresh,
    AddScreenType(Box<ScreenType>),
    BackendTask(Box<BackendTask>),
    BackendTasks(Vec<BackendTask>, BackendTasksExecutionMode),
    Custom(String),
}

impl DesiredAppAction {
    pub fn create_action(&self, app_context: &Arc<AppContext>) -> AppAction {
        match self {
            DesiredAppAction::None => AppAction::None,
            DesiredAppAction::Refresh => AppAction::Refresh,
            DesiredAppAction::Custom(message) => AppAction::Custom(message.clone()),
            DesiredAppAction::AddScreenType(screen_type) => {
                AppAction::AddScreen(screen_type.create_screen(app_context))
            }
            DesiredAppAction::BackendTask(backend_task) => {
                AppAction::BackendTask((**backend_task).clone())
            }
            DesiredAppAction::BackendTasks(tasks, mode) => {
                AppAction::BackendTasks(tasks.clone(), mode.clone())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackendTasksExecutionMode {
    Sequential,
    Concurrent,
}

#[derive(Debug, PartialEq)]
pub enum AppAction {
    None,
    Refresh,
    PopScreen,
    PopScreenAndRefresh,
    GoToMainScreen,
    SwitchNetwork(Network),
    SetMainScreen(RootScreenType),
    SetMainScreenThenPopScreen(RootScreenType),
    SetMainScreenThenGoToMainScreen(RootScreenType),
    AddScreen(Screen),
    PopThenAddScreenToMainScreen(RootScreenType, Screen),
    BackendTask(BackendTask),
    BackendTasks(Vec<BackendTask>, BackendTasksExecutionMode),
    Custom(String),
    /// Mark onboarding as complete, hide welcome screen, and optionally navigate
    OnboardingComplete {
        /// The main screen to show
        main_screen: RootScreenType,
        /// Optional sub-screen to push onto the stack
        add_screen: Option<Box<crate::ui::ScreenType>>,
    },
}

impl BitOrAssign for AppAction {
    fn bitor_assign(&mut self, rhs: Self) {
        if matches!(rhs, AppAction::None) {
            // If rhs is None, keep the current value.
            return;
        }

        // Otherwise, assign rhs to self.
        *self = rhs;
    }
}
impl AppState {
    pub fn new(ctx: egui::Context) -> Self {
        create_app_user_data_directory_if_not_exists()
            .expect("Failed to create app user_data directory");
        copy_env_file_if_not_exists();
        initialize_logger();
        let db_file_path = app_user_data_file_path("data.db").expect("should create db file path");
        let db = Arc::new(Database::new(&db_file_path).unwrap());
        db.initialize(&db_file_path).unwrap();

        let settings = db
            .get_settings()
            .expect("expected to get settings")
            .map(Settings::from)
            .unwrap_or_default();
        let password_info = settings.password_info;
        let theme_preference = settings.theme_mode;
        let overwrite_dash_conf = settings.overwrite_dash_conf;
        let onboarding_completed = settings.onboarding_completed;

        let subtasks = Arc::new(TaskManager::new());
        let connection_status = Arc::new(ConnectionStatus::new());
        let mainnet_app_context = match AppContext::new(
            Network::Dash,
            db.clone(),
            password_info.clone(),
            subtasks.clone(),
            connection_status.clone(),
            ctx.clone(),
        ) {
            Some(context) => context,
            None => {
                tracing::error!(
                    "Failed to create the AppContext. Expected Dash config for mainnet."
                );
                std::process::exit(1);
            }
        };
        let testnet_app_context = AppContext::new(
            Network::Testnet,
            db.clone(),
            password_info.clone(),
            subtasks.clone(),
            connection_status.clone(),
            ctx.clone(),
        );
        let devnet_app_context = AppContext::new(
            Network::Devnet,
            db.clone(),
            password_info.clone(),
            subtasks.clone(),
            connection_status.clone(),
            ctx.clone(),
        );
        let local_app_context = AppContext::new(
            Network::Regtest,
            db.clone(),
            password_info,
            subtasks.clone(),
            connection_status.clone(),
            ctx.clone(),
        );

        // load fonts
        ctx.set_fonts(crate::bundled::fonts().expect("failed to load fonts"));

        // create screens
        let mut identities_screen = IdentitiesScreen::new(&mainnet_app_context);
        let mut dpns_active_contests_screen =
            DPNSScreen::new(&mainnet_app_context, DPNSSubscreen::Active);
        let mut dpns_past_contests_screen =
            DPNSScreen::new(&mainnet_app_context, DPNSSubscreen::Past);
        let mut dpns_my_usernames_screen =
            DPNSScreen::new(&mainnet_app_context, DPNSSubscreen::Owned);
        let mut dpns_scheduled_votes_screen =
            DPNSScreen::new(&mainnet_app_context, DPNSSubscreen::ScheduledVotes);
        let mut transition_visualizer_screen =
            TransitionVisualizerScreen::new(&mainnet_app_context);
        let mut proof_visualizer_screen = ProofVisualizerScreen::new(&mainnet_app_context);
        let mut document_visualizer_screen = DocumentVisualizerScreen::new(&mainnet_app_context);
        let mut contract_visualizer_screen = ContractVisualizerScreen::new(&mainnet_app_context);
        let mut proof_log_screen = ProofLogScreen::new(&mainnet_app_context);
        let mut platform_info_screen = PlatformInfoScreen::new(&mainnet_app_context);
        let mut address_balance_screen = AddressBalanceScreen::new(&mainnet_app_context);
        let mut grovestark_screen = GroveSTARKScreen::new(&mainnet_app_context);
        let mut document_query_screen = DocumentQueryScreen::new(&mainnet_app_context);
        let mut tokens_balances_screen =
            TokensScreen::new(&mainnet_app_context, TokensSubscreen::MyTokens);
        let mut token_search_screen =
            TokensScreen::new(&mainnet_app_context, TokensSubscreen::SearchTokens);
        let mut token_creator_screen =
            TokensScreen::new(&mainnet_app_context, TokensSubscreen::TokenCreator);
        let mut contracts_dashpay_screen =
            DashPayScreen::new(&mainnet_app_context, DashPaySubscreen::Profile);

        // Create DashPay screens
        let mut dashpay_contacts_screen =
            DashPayScreen::new(&mainnet_app_context, DashPaySubscreen::Contacts);
        let mut dashpay_profile_screen =
            DashPayScreen::new(&mainnet_app_context, DashPaySubscreen::Profile);
        let mut dashpay_payments_screen =
            DashPayScreen::new(&mainnet_app_context, DashPaySubscreen::Payments);
        let mut dashpay_profile_search_screen =
            ProfileSearchScreen::new(mainnet_app_context.clone());

        let mut network_chooser_screen = NetworkChooserScreen::new(
            &mainnet_app_context,
            testnet_app_context.as_ref(),
            devnet_app_context.as_ref(),
            local_app_context.as_ref(),
            Network::Dash,
            overwrite_dash_conf,
        );

        let mut masternode_list_diff_screen = MasternodeListDiffScreen::new(&mainnet_app_context);

        let mut wallets_balances_screen = WalletsBalancesScreen::new(&mainnet_app_context);

        let selected_main_screen = settings.root_screen_type;
        let chosen_network = settings.network;
        network_chooser_screen.current_network = chosen_network;

        if let (Network::Testnet, Some(testnet_app_context)) =
            (chosen_network, testnet_app_context.as_ref())
        {
            identities_screen = IdentitiesScreen::new(testnet_app_context);
            dpns_active_contests_screen =
                DPNSScreen::new(testnet_app_context, DPNSSubscreen::Active);
            dpns_past_contests_screen = DPNSScreen::new(testnet_app_context, DPNSSubscreen::Past);
            dpns_my_usernames_screen = DPNSScreen::new(testnet_app_context, DPNSSubscreen::Owned);
            dpns_scheduled_votes_screen =
                DPNSScreen::new(testnet_app_context, DPNSSubscreen::ScheduledVotes);
            transition_visualizer_screen = TransitionVisualizerScreen::new(testnet_app_context);
            proof_visualizer_screen = ProofVisualizerScreen::new(testnet_app_context);
            document_visualizer_screen = DocumentVisualizerScreen::new(testnet_app_context);
            contract_visualizer_screen = ContractVisualizerScreen::new(testnet_app_context);
            document_query_screen = DocumentQueryScreen::new(testnet_app_context);
            grovestark_screen = GroveSTARKScreen::new(testnet_app_context);
            wallets_balances_screen = WalletsBalancesScreen::new(testnet_app_context);
            proof_log_screen = ProofLogScreen::new(testnet_app_context);
            platform_info_screen = PlatformInfoScreen::new(testnet_app_context);
            address_balance_screen = AddressBalanceScreen::new(testnet_app_context);
            masternode_list_diff_screen = MasternodeListDiffScreen::new(testnet_app_context);
            contracts_dashpay_screen =
                DashPayScreen::new(testnet_app_context, DashPaySubscreen::Profile);
            tokens_balances_screen =
                TokensScreen::new(testnet_app_context, TokensSubscreen::MyTokens);
            token_search_screen =
                TokensScreen::new(testnet_app_context, TokensSubscreen::SearchTokens);
            token_creator_screen =
                TokensScreen::new(testnet_app_context, TokensSubscreen::TokenCreator);
            dashpay_contacts_screen =
                DashPayScreen::new(testnet_app_context, DashPaySubscreen::Contacts);
            dashpay_profile_screen =
                DashPayScreen::new(testnet_app_context, DashPaySubscreen::Profile);
            dashpay_payments_screen =
                DashPayScreen::new(testnet_app_context, DashPaySubscreen::Payments);
            dashpay_profile_search_screen = ProfileSearchScreen::new(testnet_app_context.clone());
        } else if let (Network::Devnet, Some(devnet_app_context)) =
            (chosen_network, devnet_app_context.as_ref())
        {
            identities_screen = IdentitiesScreen::new(devnet_app_context);
            dpns_active_contests_screen =
                DPNSScreen::new(devnet_app_context, DPNSSubscreen::Active);
            dpns_past_contests_screen = DPNSScreen::new(devnet_app_context, DPNSSubscreen::Past);
            dpns_my_usernames_screen = DPNSScreen::new(devnet_app_context, DPNSSubscreen::Owned);
            dpns_scheduled_votes_screen =
                DPNSScreen::new(devnet_app_context, DPNSSubscreen::ScheduledVotes);
            transition_visualizer_screen = TransitionVisualizerScreen::new(devnet_app_context);
            proof_visualizer_screen = ProofVisualizerScreen::new(devnet_app_context);
            document_visualizer_screen = DocumentVisualizerScreen::new(devnet_app_context);
            document_query_screen = DocumentQueryScreen::new(devnet_app_context);
            masternode_list_diff_screen = MasternodeListDiffScreen::new(devnet_app_context);
            contract_visualizer_screen = ContractVisualizerScreen::new(devnet_app_context);
            grovestark_screen = GroveSTARKScreen::new(devnet_app_context);
            wallets_balances_screen = WalletsBalancesScreen::new(devnet_app_context);
            proof_log_screen = ProofLogScreen::new(devnet_app_context);
            platform_info_screen = PlatformInfoScreen::new(devnet_app_context);
            address_balance_screen = AddressBalanceScreen::new(devnet_app_context);
            tokens_balances_screen =
                TokensScreen::new(devnet_app_context, TokensSubscreen::MyTokens);
            token_search_screen =
                TokensScreen::new(devnet_app_context, TokensSubscreen::SearchTokens);
            token_creator_screen =
                TokensScreen::new(devnet_app_context, TokensSubscreen::TokenCreator);
            dashpay_contacts_screen =
                DashPayScreen::new(devnet_app_context, DashPaySubscreen::Contacts);
            dashpay_profile_screen =
                DashPayScreen::new(devnet_app_context, DashPaySubscreen::Profile);
            dashpay_payments_screen =
                DashPayScreen::new(devnet_app_context, DashPaySubscreen::Payments);
            dashpay_profile_search_screen = ProfileSearchScreen::new(devnet_app_context.clone());
        } else if let (Network::Regtest, Some(local_app_context)) =
            (chosen_network, local_app_context.as_ref())
        {
            identities_screen = IdentitiesScreen::new(local_app_context);
            dpns_active_contests_screen = DPNSScreen::new(local_app_context, DPNSSubscreen::Active);
            dpns_past_contests_screen = DPNSScreen::new(local_app_context, DPNSSubscreen::Past);
            dpns_my_usernames_screen = DPNSScreen::new(local_app_context, DPNSSubscreen::Owned);
            dpns_scheduled_votes_screen =
                DPNSScreen::new(local_app_context, DPNSSubscreen::ScheduledVotes);
            transition_visualizer_screen = TransitionVisualizerScreen::new(local_app_context);
            proof_visualizer_screen = ProofVisualizerScreen::new(local_app_context);
            document_visualizer_screen = DocumentVisualizerScreen::new(local_app_context);
            contract_visualizer_screen = ContractVisualizerScreen::new(local_app_context);
            document_query_screen = DocumentQueryScreen::new(local_app_context);
            grovestark_screen = GroveSTARKScreen::new(local_app_context);
            wallets_balances_screen = WalletsBalancesScreen::new(local_app_context);
            masternode_list_diff_screen = MasternodeListDiffScreen::new(local_app_context);
            proof_log_screen = ProofLogScreen::new(local_app_context);
            platform_info_screen = PlatformInfoScreen::new(local_app_context);
            address_balance_screen = AddressBalanceScreen::new(local_app_context);
            contracts_dashpay_screen =
                DashPayScreen::new(local_app_context, DashPaySubscreen::Profile);
            tokens_balances_screen =
                TokensScreen::new(local_app_context, TokensSubscreen::MyTokens);
            token_search_screen =
                TokensScreen::new(local_app_context, TokensSubscreen::SearchTokens);
            token_creator_screen =
                TokensScreen::new(local_app_context, TokensSubscreen::TokenCreator);
            dashpay_contacts_screen =
                DashPayScreen::new(local_app_context, DashPaySubscreen::Contacts);
            dashpay_profile_screen =
                DashPayScreen::new(local_app_context, DashPaySubscreen::Profile);
            dashpay_payments_screen =
                DashPayScreen::new(local_app_context, DashPaySubscreen::Payments);
            dashpay_profile_search_screen = ProfileSearchScreen::new(local_app_context.clone());
        }

        // // Create a channel with a buffer size of 32 (adjust as needed)
        let (task_result_sender, task_result_receiver) =
            tokiompsc::channel(256).with_egui_ctx(ctx.clone());

        // Create a channel for communication with the InstantSendListener
        let (core_message_sender, core_message_receiver) =
            mpsc::channel().with_egui_ctx(ctx.clone());

        let mainnet_core_zmq_endpoint = mainnet_app_context
            .config
            .read()
            .unwrap()
            .core_zmq_endpoint
            .clone()
            .unwrap_or_else(|| "tcp://127.0.0.1:23708".to_string());
        let mainnet_disable_zmq = mainnet_app_context
            .get_settings()
            .ok()
            .flatten()
            .map(|s| s.disable_zmq)
            .unwrap_or(false);
        let mainnet_core_zmq_listener = if !mainnet_disable_zmq {
            Some(
                CoreZMQListener::spawn_listener(
                    Network::Dash,
                    &mainnet_core_zmq_endpoint,
                    core_message_sender.clone(), // Clone the sender for each listener
                    Some(mainnet_app_context.sx_zmq_status.clone()),
                )
                .expect("Failed to create mainnet InstantSend listener"),
            )
        } else {
            None
        };

        let testnet_tx_zmq_status_option = testnet_app_context
            .as_ref()
            .map(|context| context.sx_zmq_status.clone());

        let testnet_core_zmq_endpoint = testnet_app_context
            .as_ref()
            .and_then(|ctx| ctx.config.read().unwrap().core_zmq_endpoint.clone())
            .unwrap_or_else(|| "tcp://127.0.0.1:23709".to_string());
        let testnet_disable_zmq = testnet_app_context
            .as_ref()
            .and_then(|ctx| ctx.get_settings().ok().flatten())
            .map(|s| s.disable_zmq)
            .unwrap_or(false);
        let testnet_core_zmq_listener = if !testnet_disable_zmq {
            Some(
                CoreZMQListener::spawn_listener(
                    Network::Testnet,
                    &testnet_core_zmq_endpoint,
                    core_message_sender.clone(), // Use the original sender or create a new one if needed
                    testnet_tx_zmq_status_option,
                )
                .expect("Failed to create testnet InstantSend listener"),
            )
        } else {
            None
        };

        let devnet_tx_zmq_status_option = devnet_app_context
            .as_ref()
            .map(|context| context.sx_zmq_status.clone());

        let devnet_core_zmq_endpoint = devnet_app_context
            .as_ref()
            .and_then(|ctx| ctx.config.read().unwrap().core_zmq_endpoint.clone())
            .unwrap_or_else(|| "tcp://127.0.0.1:23710".to_string());
        let devnet_disable_zmq = devnet_app_context
            .as_ref()
            .and_then(|ctx| ctx.get_settings().ok().flatten())
            .map(|s| s.disable_zmq)
            .unwrap_or(false);
        let devnet_core_zmq_listener = if !devnet_disable_zmq {
            Some(
                CoreZMQListener::spawn_listener(
                    Network::Devnet,
                    &devnet_core_zmq_endpoint,
                    core_message_sender.clone(),
                    devnet_tx_zmq_status_option,
                )
                .expect("Failed to create devnet InstantSend listener"),
            )
        } else {
            None
        };

        let local_tx_zmq_status_option = local_app_context
            .as_ref()
            .map(|context| context.sx_zmq_status.clone());

        let local_core_zmq_endpoint = local_app_context
            .as_ref()
            .and_then(|ctx| ctx.config.read().unwrap().core_zmq_endpoint.clone())
            .unwrap_or_else(|| "tcp://127.0.0.1:20302".to_string());
        let local_disable_zmq = local_app_context
            .as_ref()
            .and_then(|ctx| ctx.get_settings().ok().flatten())
            .map(|s| s.disable_zmq)
            .unwrap_or(false);
        let local_core_zmq_listener = if !local_disable_zmq {
            Some(
                CoreZMQListener::spawn_listener(
                    Network::Regtest,
                    &local_core_zmq_endpoint,
                    core_message_sender,
                    local_tx_zmq_status_option,
                )
                .expect("Failed to create local InstantSend listener"),
            )
        } else {
            None
        };

        let mut app_state = Self {
            main_screens: [
                (
                    RootScreenType::RootScreenIdentities,
                    Screen::IdentitiesScreen(identities_screen),
                ),
                (
                    RootScreenType::RootScreenDPNSActiveContests,
                    Screen::DPNSScreen(dpns_active_contests_screen),
                ),
                (
                    RootScreenType::RootScreenDPNSPastContests,
                    Screen::DPNSScreen(dpns_past_contests_screen),
                ),
                (
                    RootScreenType::RootScreenDPNSOwnedNames,
                    Screen::DPNSScreen(dpns_my_usernames_screen),
                ),
                (
                    RootScreenType::RootScreenDPNSScheduledVotes,
                    Screen::DPNSScreen(dpns_scheduled_votes_screen),
                ),
                (
                    RootScreenType::RootScreenWalletsBalances,
                    Screen::WalletsBalancesScreen(wallets_balances_screen),
                ),
                (
                    RootScreenType::RootScreenToolsTransitionVisualizerScreen,
                    Screen::TransitionVisualizerScreen(transition_visualizer_screen),
                ),
                (
                    RootScreenType::RootScreenToolsProofVisualizerScreen,
                    Screen::ProofVisualizerScreen(proof_visualizer_screen),
                ),
                (
                    RootScreenType::RootScreenToolsDocumentVisualizerScreen,
                    Screen::DocumentVisualizerScreen(document_visualizer_screen),
                ),
                (
                    RootScreenType::RootScreenToolsContractVisualizerScreen,
                    Screen::ContractVisualizerScreen(contract_visualizer_screen),
                ),
                (
                    RootScreenType::RootScreenToolsProofLogScreen,
                    Screen::ProofLogScreen(proof_log_screen),
                ),
                (
                    RootScreenType::RootScreenToolsPlatformInfoScreen,
                    Screen::PlatformInfoScreen(platform_info_screen),
                ),
                (
                    RootScreenType::RootScreenToolsAddressBalanceScreen,
                    Screen::AddressBalanceScreen(address_balance_screen),
                ),
                (
                    RootScreenType::RootScreenToolsGroveSTARKScreen,
                    Screen::GroveSTARKScreen(grovestark_screen),
                ),
                (
                    RootScreenType::RootScreenDocumentQuery,
                    Screen::DocumentQueryScreen(document_query_screen),
                ),
                (
                    RootScreenType::RootScreenDashpay,
                    Screen::DashPayScreen(contracts_dashpay_screen),
                ),
                (
                    RootScreenType::RootScreenNetworkChooser,
                    Screen::NetworkChooserScreen(network_chooser_screen),
                ),
                (
                    RootScreenType::RootScreenToolsMasternodeListDiffScreen,
                    Screen::MasternodeListDiffScreen(masternode_list_diff_screen),
                ),
                (
                    RootScreenType::RootScreenMyTokenBalances,
                    Screen::TokensScreen(Box::new(tokens_balances_screen)),
                ),
                (
                    RootScreenType::RootScreenTokenSearch,
                    Screen::TokensScreen(Box::new(token_search_screen)),
                ),
                (
                    RootScreenType::RootScreenTokenCreator,
                    Screen::TokensScreen(Box::new(token_creator_screen)),
                ),
                (
                    RootScreenType::RootScreenDashPayContacts,
                    Screen::DashPayScreen(dashpay_contacts_screen),
                ),
                (
                    RootScreenType::RootScreenDashPayProfile,
                    Screen::DashPayScreen(dashpay_profile_screen),
                ),
                (
                    RootScreenType::RootScreenDashPayPayments,
                    Screen::DashPayScreen(dashpay_payments_screen),
                ),
                (
                    RootScreenType::RootScreenDashPayProfileSearch,
                    Screen::DashPayProfileSearchScreen(dashpay_profile_search_screen),
                ),
            ]
            .into(),
            selected_main_screen,
            screen_stack: vec![],
            chosen_network,
            connection_status,
            mainnet_app_context,
            testnet_app_context,
            devnet_app_context,
            local_app_context,
            mainnet_core_zmq_listener,
            testnet_core_zmq_listener,
            devnet_core_zmq_listener,
            local_core_zmq_listener,
            core_message_receiver,
            task_result_sender,
            task_result_receiver,
            theme_preference,
            last_scheduled_vote_check: Instant::now(),
            last_repaint_request: Instant::now(),
            subtasks,
            show_welcome_screen: !onboarding_completed,
            welcome_screen: None,
        };

        // Initialize welcome screen if needed (after mainnet_app_context is owned by the struct)
        if app_state.show_welcome_screen {
            app_state.welcome_screen =
                Some(WelcomeScreen::new(app_state.mainnet_app_context.clone()));
        } else {
            // Auto-start SPV sync if onboarding is completed, backend mode is SPV, auto-start is enabled,
            // and developer mode is enabled.
            // TODO: SPV auto-start is gated behind developer mode while SPV is in development.
            // Remove the is_developer_mode() check once SPV is production-ready.
            let current_context = app_state.current_app_context();
            let auto_start_spv = db.get_auto_start_spv().unwrap_or(false);
            if auto_start_spv
                && current_context.is_developer_mode()
                && current_context.core_backend_mode() == crate::spv::CoreBackendMode::Spv
            {
                if let Err(e) = current_context.start_spv() {
                    tracing::warn!("Failed to auto-start SPV sync: {}", e);
                } else {
                    tracing::info!("SPV sync started automatically for {:?}", chosen_network);
                }
            }

            // Refresh ALL main screens so they load data properly
            // This ensures screens like DashPay Profile have identities loaded
            // even if they're not the initially selected screen
            for screen in app_state.main_screens.values_mut() {
                screen.refresh_on_arrival();
            }
        }

        app_state
    }

    /// Allows enabling or disabling animations globally for the app.
    ///
    /// Default is enabled.
    pub fn with_animations(self, enabled: bool) -> Self {
        self.mainnet_app_context.enable_animations(enabled);
        if let Some(context) = self.devnet_app_context.as_ref() {
            context.enable_animations(enabled)
        }
        if let Some(context) = self.testnet_app_context.as_ref() {
            context.enable_animations(enabled)
        }
        if let Some(context) = self.local_app_context.as_ref() {
            context.enable_animations(enabled)
        }

        self
    }

    pub fn current_app_context(&self) -> &Arc<AppContext> {
        match self.chosen_network {
            Network::Dash => &self.mainnet_app_context,
            Network::Testnet => self.testnet_app_context.as_ref().expect("expected testnet"),
            Network::Devnet => self.devnet_app_context.as_ref().expect("expected devnet"),
            Network::Regtest => self.local_app_context.as_ref().expect("expected local"),
            _ => todo!(),
        }
    }

    // Handle the backend task and send the result through the channel
    fn handle_backend_task(&self, task: BackendTask) {
        let sender = self.task_result_sender.clone();
        let app_context = self.current_app_context().clone();
        tokio::spawn(async move {
            let result = app_context.run_backend_task(task, sender.clone()).await;

            // Send the result back to the main thread
            if let Err(e) = sender.send(result.into()).await {
                tracing::error!("Failed to send task result: {}", e);
            }
        });
    }

    /// Handle the backend tasks and send the results through the channel
    fn handle_backend_tasks(&self, tasks: Vec<BackendTask>, mode: BackendTasksExecutionMode) {
        let sender = self.task_result_sender.clone();
        let app_context = self.current_app_context().clone();

        tokio::spawn(async move {
            let results = match mode {
                BackendTasksExecutionMode::Sequential => {
                    app_context
                        .run_backend_tasks_sequential(tasks, sender.clone())
                        .await
                }
                BackendTasksExecutionMode::Concurrent => {
                    app_context
                        .run_backend_tasks_concurrent(tasks, sender.clone())
                        .await
                }
            };

            // Send the results back to the main thread
            for result in results {
                if let Err(e) = sender.send(result.into()).await {
                    tracing::error!("Failed to send task result: {}", e);
                }
            }
        });
    }

    pub fn active_root_screen_mut(&mut self) -> &mut Screen {
        self.main_screens
            .get_mut(&self.selected_main_screen)
            .expect("expected to get screen")
    }

    pub fn change_network(&mut self, network: Network) {
        self.chosen_network = network;
        let app_context = self.current_app_context().clone();

        for screen in self.main_screens.values_mut() {
            screen.change_context(app_context.clone())
        }

        self.connection_status
            .reset(app_context.core_backend_mode());
    }

    pub fn visible_screen_mut(&mut self) -> &mut Screen {
        if self.screen_stack.is_empty() {
            self.active_root_screen_mut()
        } else {
            self.screen_stack.last_mut().unwrap()
        }
    }
}

impl AppState {
    // /// This function continuously listens for asset locks and updates the wallets accordingly.
    // fn start_listening_for_asset_locks(&mut self) {
    //     let instant_send_receiver = self.instant_send_receiver.clone(); // Clone the receiver
    //     let mainnet_app_context = self.mainnet_app_context.clone();
    //     let testnet_app_context = self.testnet_app_context.clone();
    //
    //     // Spawn a new task to listen asynchronously for asset locks
    //     task::spawn_blocking(move || {
    //         while let Ok((tx, islock, network)) = instant_send_receiver.recv() {
    //             let app_context = match network {
    //                 Network::Dash => &mainnet_app_context,
    //                 Network::Testnet => {
    //                     if let Some(context) = testnet_app_context.as_ref() {
    //                         context
    //                     } else {
    //                         // Handle the case when testnet_app_context is None
    //                         eprintln!("No testnet app context available for Testnet");
    //                         continue; // Skip this iteration or handle as needed
    //                     }
    //                 }
    //                 _ => continue,
    //             };
    //             // Store the asset lock transaction in the database
    //             if let Err(e) = app_context.store_asset_lock_in_db(&tx, islock) {
    //                 eprintln!("Failed to store asset lock: {}", e);
    //             }
    //
    //             // Sleep briefly to avoid busy-waiting
    //             std::thread::sleep(Duration::from_millis(50));
    //         }
    //     });
    // }
}

impl App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply Dash theme with user preference
        crate::ui::theme::apply_theme(ctx, self.theme_preference);

        let active_context = self.current_app_context().clone();

        // Poll the receiver for any new task results
        while let Ok(task_result) = self.task_result_receiver.try_recv() {
            active_context
                .connection_status()
                .handle_task_result(&task_result, active_context.network);

            // Handle the result on the main thread
            match task_result {
                TaskResult::Success(message) => {
                    let unboxed_message = *message;
                    match unboxed_message {
                        BackendTaskSuccessResult::None => {}
                        BackendTaskSuccessResult::Refresh => {
                            self.visible_screen_mut().refresh();
                        }
                        BackendTaskSuccessResult::Message(ref _msg) => {
                            // Let the screen handle Message via display_task_result
                            // so it can do custom handling (like clearing spinners)
                            self.visible_screen_mut()
                                .display_task_result(unboxed_message);
                        }
                        BackendTaskSuccessResult::UpdatedThemePreference(new_theme) => {
                            self.theme_preference = new_theme;
                            MessageBanner::set_global(
                                ctx,
                                "Theme preference updated successfully",
                                MessageType::Success,
                            );
                            self.visible_screen_mut().display_message(
                                "Theme preference updated successfully",
                                MessageType::Success,
                            );
                        }
                        BackendTaskSuccessResult::CastScheduledVote(ref vote) => {
                            let _ = self.current_app_context().mark_vote_executed(
                                vote.voter_id.as_slice(),
                                vote.contested_name.clone(),
                            );
                            MessageBanner::set_global(
                                ctx,
                                "Successfully cast scheduled vote",
                                MessageType::Success,
                            );
                            self.visible_screen_mut().display_message(
                                "Successfully cast scheduled vote",
                                MessageType::Success,
                            );
                            self.visible_screen_mut().refresh();
                        }
                        _ => {
                            MessageBanner::set_global(ctx, "Success", MessageType::Success);
                            self.visible_screen_mut()
                                .display_task_result(unboxed_message);
                        }
                    }
                }
                TaskResult::Error(message) => {
                    MessageBanner::set_global(ctx, &message, MessageType::Error);
                    self.visible_screen_mut()
                        .display_message(&message, MessageType::Error);
                }
                TaskResult::Refresh => {
                    self.visible_screen_mut().refresh();
                }
            }
        }

        // Schedule a periodic repaint every ~1 second so timed messages update
        // their countdown and other periodic UI elements stay current.
        // Throttled so we don't re-schedule on every frame during user interaction.
        if self.last_repaint_request.elapsed() >= Duration::from_secs(1) {
            ctx.request_repaint_after(Duration::from_secs(1));
            self.last_repaint_request = Instant::now();
        }

        // **Poll the instant_send_receiver for any new InstantSend messages**
        while let Ok((message, network)) = self.core_message_receiver.try_recv() {
            let app_context = match network {
                Network::Dash => &self.mainnet_app_context,
                Network::Testnet => {
                    if let Some(context) = self.testnet_app_context.as_ref() {
                        context
                    } else {
                        tracing::error!("No testnet app context available for Testnet");
                        continue;
                    }
                }
                Network::Devnet => {
                    if let Some(context) = self.devnet_app_context.as_ref() {
                        context
                    } else {
                        tracing::error!("No devnet app context available");
                        continue;
                    }
                }
                Network::Regtest => {
                    if let Some(context) = self.local_app_context.as_ref() {
                        context
                    } else {
                        tracing::error!("No local app context available");
                        continue;
                    }
                }
                _ => continue,
            };
            match message {
                ZMQMessage::ISLockedTransaction(tx, is_lock) => {
                    // Store the asset lock transaction in the database
                    match app_context.received_transaction_finality(
                        &tx,
                        Some(is_lock.clone()),
                        None,
                    ) {
                        Ok(utxos) => {
                            let core_item =
                                CoreItem::InstantLockedTransaction(tx.clone(), utxos, is_lock);
                            self.visible_screen_mut()
                                .display_task_result(BackendTaskSuccessResult::CoreItem(core_item));
                        }
                        Err(e) => {
                            tracing::error!("Failed to store asset lock: {}", e);
                        }
                    }
                }
                ZMQMessage::ChainLockedLockedTransaction(tx, height) => {
                    if let Err(e) =
                        app_context.received_transaction_finality(&tx, None, Some(height))
                    {
                        tracing::error!("Failed to store asset lock: {}", e);
                    }
                }
                ZMQMessage::ChainLockedBlock(block, chain_lock) => {
                    self.visible_screen_mut().display_task_result(
                        BackendTaskSuccessResult::CoreItem(CoreItem::ChainLockedBlock(
                            block, chain_lock,
                        )),
                    );
                }
            }
        }

        // Check if there are scheduled masternode votes to cast and if so, cast them
        let now = Instant::now();
        if now.duration_since(self.last_scheduled_vote_check) > Duration::from_secs(60) {
            self.last_scheduled_vote_check = now;
            let app_context = self.current_app_context();

            // Query the database
            let db_votes = match app_context.get_scheduled_votes() {
                Ok(votes) => votes,
                Err(e) => {
                    tracing::error!("Error querying scheduled votes: {}", e);
                    return;
                }
            };

            // Filter due votes
            let current_time = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            let due_votes: Vec<_> = db_votes
                .into_iter()
                .filter(|v| {
                    v.unix_timestamp <= current_time
                        && !v.executed_successfully
                        && (v.unix_timestamp + 120000 >= current_time) // Don't cast votes more than 2 minutes behind current time
                })
                .collect();

            // For each due vote, construct a BackendTask and handle it
            if !due_votes.is_empty() {
                let local_identities = match app_context.load_local_voting_identities() {
                    Ok(identities) => identities,
                    Err(e) => {
                        tracing::error!("Error querying local voting identities: {}", e);
                        return;
                    }
                };

                for vote in due_votes {
                    if let Some(voter) = local_identities
                        .iter()
                        .find(|i| i.identity.id() == vote.voter_id)
                    {
                        let dpns_screen = self
                            .main_screens
                            .get_mut(&RootScreenType::RootScreenDPNSScheduledVotes)
                            .unwrap();
                        if let Screen::DPNSScreen(screen) = dpns_screen {
                            screen.scheduled_vote_cast_in_progress = true;
                            if let Some((_, s)) = screen
                                .scheduled_votes
                                .lock()
                                .unwrap()
                                .iter_mut()
                                .find(|(v, _)| v == &vote)
                            {
                                *s = ScheduledVoteCastingStatus::InProgress;
                            }
                        }
                        let task = BackendTask::ContestedResourceTask(
                            ContestedResourceTask::CastScheduledVote(vote, Box::new(voter.clone())),
                        );
                        self.handle_backend_task(task);
                    } else {
                        tracing::warn!("Voter not found for scheduled vote: {:?}", vote);
                    }
                }
            }
        }

        // Show welcome screen if onboarding not completed
        let mut actions = Vec::new();
        if self.show_welcome_screen
            && let Some(welcome_screen) = &mut self.welcome_screen
        {
            actions.push(welcome_screen.ui(ctx));
        } else {
            actions.push(self.visible_screen_mut().ui(ctx));
        };

        // Schedule connection status refresh
        actions.push(
            active_context
                .connection_status()
                .trigger_refresh(active_context.as_ref()),
        );

        for action in actions {
            match action {
                AppAction::None => {}
                AppAction::AddScreen(screen) => self.screen_stack.push(screen),
                AppAction::Refresh => self.visible_screen_mut().refresh(),
                AppAction::PopScreen => {
                    if !self.screen_stack.is_empty() {
                        self.screen_stack.pop();
                    }
                }
                AppAction::PopScreenAndRefresh => {
                    if !self.screen_stack.is_empty() {
                        self.screen_stack.pop();
                    }
                    if let Some(screen) = self.screen_stack.last_mut() {
                        screen.refresh();
                    } else {
                        self.active_root_screen_mut().refresh_on_arrival();
                    }
                }
                AppAction::GoToMainScreen => {
                    self.screen_stack = vec![];
                    self.active_root_screen_mut().refresh_on_arrival();
                }
                AppAction::BackendTask(task) => {
                    self.handle_backend_task(task);
                }
                AppAction::BackendTasks(tasks, mode) => {
                    self.handle_backend_tasks(tasks, mode);
                }
                AppAction::SetMainScreen(root_screen_type) => {
                    self.selected_main_screen = root_screen_type;
                    self.active_root_screen_mut().refresh_on_arrival();
                    self.current_app_context()
                        .update_settings(root_screen_type)
                        .ok();
                }
                AppAction::SetMainScreenThenGoToMainScreen(root_screen_type) => {
                    self.selected_main_screen = root_screen_type;
                    self.active_root_screen_mut().refresh_on_arrival();
                    self.current_app_context()
                        .update_settings(root_screen_type)
                        .ok();
                    self.screen_stack = vec![];
                }
                AppAction::SetMainScreenThenPopScreen(root_screen_type) => {
                    self.selected_main_screen = root_screen_type;
                    self.active_root_screen_mut().refresh_on_arrival();
                    self.current_app_context()
                        .update_settings(root_screen_type)
                        .ok();
                    if !self.screen_stack.is_empty() {
                        self.screen_stack.pop();
                    }
                }
                AppAction::SwitchNetwork(network) => {
                    self.change_network(network);
                    self.current_app_context()
                        .update_settings(RootScreenType::RootScreenNetworkChooser)
                        .ok();
                }
                AppAction::PopThenAddScreenToMainScreen(root_screen_type, screen) => {
                    self.screen_stack = vec![screen];
                    self.selected_main_screen = root_screen_type;
                    self.active_root_screen_mut().refresh_on_arrival();
                    self.current_app_context()
                        .update_settings(root_screen_type)
                        .ok();
                }
                AppAction::Custom(_) => {}
                AppAction::OnboardingComplete {
                    main_screen,
                    add_screen,
                } => {
                    self.show_welcome_screen = false;
                    self.welcome_screen = None;
                    self.selected_main_screen = main_screen;
                    self.active_root_screen_mut().refresh_on_arrival();
                    self.current_app_context().update_settings(main_screen).ok();
                    // If there's an additional screen to push, create and push it
                    if let Some(screen_type) = add_screen {
                        let screen = screen_type.create_screen(self.current_app_context());
                        self.screen_stack.push(screen);
                    }
                    // Start SPV sync after onboarding completes (if auto-start is enabled and developer mode is on)
                    // TODO: SPV auto-start is gated behind developer mode while SPV is in development.
                    // Remove the is_developer_mode() check once SPV is production-ready.
                    let current_context = self.current_app_context();
                    let auto_start_spv = current_context.db.get_auto_start_spv().unwrap_or(false);
                    if auto_start_spv
                        && current_context.is_developer_mode()
                        && current_context.core_backend_mode() == crate::spv::CoreBackendMode::Spv
                    {
                        if let Err(e) = current_context.start_spv() {
                            tracing::warn!("Failed to start SPV sync after onboarding: {}", e);
                        } else {
                            tracing::info!("SPV sync started after onboarding");
                        }
                    }
                }
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Gracefully shutdown all background tasks, waiting for them to complete
        // This ensures tasks like the dash-qt handler have time to check their settings
        // and decide whether to terminate the process or leave it running
        tracing::debug!("App received on_exit event, initiating graceful shutdown");
        if let Err(e) = self.subtasks.shutdown() {
            tracing::error!("Error during task shutdown: {}", e);
        }
        tracing::debug!("App shutdown complete");
    }
}
