#[cfg(not(feature = "testing"))]
use crate::app_dir::app_user_data_file_path;
use crate::app_dir::{copy_env_file_if_not_exists, create_app_user_data_directory_if_not_exists};
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::core::CoreItem;
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::components::core_zmq_listener::{CoreZMQListener, ZMQMessage};
use crate::context::AppContext;
use crate::context::connection_status::{ConnectionStatus, OverallConnectionState};
use crate::database::Database;
#[cfg(not(feature = "testing"))]
use crate::logging::initialize_logger;
use crate::model::settings::Settings;
use crate::spv::CoreBackendMode;
use crate::ui::components::selection_dialog::{SelectionDialog, SelectionStatus};
use crate::ui::components::{BannerHandle, MessageBanner};
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
    Error(TaskError),
}

impl From<Result<BackendTaskSuccessResult, TaskError>> for TaskResult {
    fn from(value: Result<BackendTaskSuccessResult, TaskError>) -> Self {
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
    /// Previous connection state, used to detect transitions and update banners.
    /// `None` on startup / after network switch to force the first evaluation.
    previous_connection_state: Option<OverallConnectionState>,
    /// Handle to the current connection status banner, if one is displayed
    connection_banner_handle: Option<BannerHandle>,
    /// Dialog for selecting which Dash Core wallet to use
    core_wallet_dialog: Option<SelectionDialog>,
    /// Wallet names from Core, stored while dialog is open
    core_wallet_names: Option<Vec<String>>,
    /// Async shutdown receiver. `Some` while a graceful shutdown is in progress;
    /// the viewport is closed once the receiver resolves.
    shutdown_receiver: Option<tokio::sync::oneshot::Receiver<()>>,
    /// Timestamp when the async shutdown was initiated, used as a hard deadline
    /// to force-close the viewport if the shutdown task stalls.
    shutdown_started: Option<std::time::Instant>,
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
    /// Creates a new `AppState` using the production database.
    ///
    /// This constructor is hidden when the `testing` feature is active to prevent
    /// tests from accidentally using the production database. Use the `testing`
    /// feature-gated `new()` variant instead.
    #[cfg(not(feature = "testing"))]
    pub fn new(ctx: egui::Context) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        create_app_user_data_directory_if_not_exists()?;
        copy_env_file_if_not_exists();
        initialize_logger();
        let db_file_path = app_user_data_file_path("data.db")?;
        let db = Arc::new(Database::new(&db_file_path)?);
        db.initialize(&db_file_path)?;
        Self::new_inner(ctx, db)
    }

    /// Creates a new `AppState` using an in-memory database for testing.
    ///
    /// Available only when the `testing` feature is active. This prevents tests
    /// from reading or writing the production database.
    #[cfg(feature = "testing")]
    pub fn new(ctx: egui::Context) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        create_app_user_data_directory_if_not_exists()?;
        copy_env_file_if_not_exists();
        let db = Arc::new(
            crate::database::test_helpers::create_test_database()
                .map_err(|e| format!("Failed to create test database: {}", e))?,
        );
        Self::new_inner(ctx, db)
    }

    fn new_inner(
        ctx: egui::Context,
        db: Arc<Database>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let settings = db.get_settings()?.map(Settings::from).unwrap_or_default();
        let password_info = settings.password_info;
        let theme_preference = settings.theme_mode;
        let overwrite_dash_conf = settings.overwrite_dash_conf;
        let onboarding_completed = settings.onboarding_completed;

        let subtasks = Arc::new(TaskManager::new());
        let connection_status = Arc::new(ConnectionStatus::new());
        let mainnet_app_context = AppContext::new(
            Network::Dash,
            db.clone(),
            password_info.clone(),
            subtasks.clone(),
            connection_status.clone(),
            ctx.clone(),
        )
        .ok_or("Failed to create AppContext for mainnet. Check your Dash configuration.")?;
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
        // Validate that the saved network has an available context.
        // We fail fast instead of silently routing user actions to a different network.
        let chosen_network = match settings.network {
            Network::Dash => Network::Dash,
            Network::Testnet => {
                assert!(
                    testnet_app_context.is_some(),
                    "Saved network is Testnet but no Testnet AppContext is configured"
                );
                Network::Testnet
            }
            Network::Devnet => {
                assert!(
                    devnet_app_context.is_some(),
                    "Saved network is Devnet but no Devnet AppContext is configured"
                );
                Network::Devnet
            }
            Network::Regtest => {
                assert!(
                    local_app_context.is_some(),
                    "Saved network is Regtest but no Regtest AppContext is configured"
                );
                Network::Regtest
            }
            unsupported_network => {
                panic!(
                    "Saved network {:?} is unsupported. Refusing automatic fallback.",
                    unsupported_network
                );
            }
        };
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
            match CoreZMQListener::spawn_listener(
                Network::Dash,
                &mainnet_core_zmq_endpoint,
                core_message_sender.clone(),
                Some(mainnet_app_context.sx_zmq_status.clone()),
            ) {
                Ok(listener) => Some(listener),
                Err(e) => {
                    tracing::error!(
                        "Failed to create mainnet ZMQ listener: {}. ZMQ features will be unavailable for mainnet.",
                        e
                    );
                    None
                }
            }
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
            match CoreZMQListener::spawn_listener(
                Network::Testnet,
                &testnet_core_zmq_endpoint,
                core_message_sender.clone(),
                testnet_tx_zmq_status_option,
            ) {
                Ok(listener) => Some(listener),
                Err(e) => {
                    tracing::error!(
                        "Failed to create testnet ZMQ listener: {}. ZMQ features will be unavailable for testnet.",
                        e
                    );
                    None
                }
            }
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
            match CoreZMQListener::spawn_listener(
                Network::Devnet,
                &devnet_core_zmq_endpoint,
                core_message_sender.clone(),
                devnet_tx_zmq_status_option,
            ) {
                Ok(listener) => Some(listener),
                Err(e) => {
                    tracing::error!(
                        "Failed to create devnet ZMQ listener: {}. ZMQ features will be unavailable for devnet.",
                        e
                    );
                    None
                }
            }
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
            match CoreZMQListener::spawn_listener(
                Network::Regtest,
                &local_core_zmq_endpoint,
                core_message_sender,
                local_tx_zmq_status_option,
            ) {
                Ok(listener) => Some(listener),
                Err(e) => {
                    tracing::error!(
                        "Failed to create local ZMQ listener: {}. ZMQ features will be unavailable for local/regtest.",
                        e
                    );
                    None
                }
            }
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
            previous_connection_state: None,
            connection_banner_handle: None,
            core_wallet_dialog: None,
            core_wallet_names: None,
            shutdown_receiver: None,
            shutdown_started: None,
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

        Ok(app_state)
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
        // Invariant: chosen_network must always have a corresponding context.
        // Fail fast on violations to avoid silently routing operations to mainnet.
        match self.chosen_network {
            Network::Dash => &self.mainnet_app_context,
            Network::Testnet => self.testnet_app_context.as_ref().unwrap_or_else(|| {
                panic!(
                    "BUG: chosen network is Testnet but testnet_app_context is missing; refusing silent mainnet fallback"
                )
            }),
            Network::Devnet => self.devnet_app_context.as_ref().unwrap_or_else(|| {
                panic!(
                    "BUG: chosen network is Devnet but devnet_app_context is missing; refusing silent mainnet fallback"
                )
            }),
            Network::Regtest => self.local_app_context.as_ref().unwrap_or_else(|| {
                panic!(
                    "BUG: chosen network is Regtest but local_app_context is missing; refusing silent mainnet fallback"
                )
            }),
            unsupported_network => panic!(
                "BUG: unsupported network variant {:?} in current_app_context; refusing silent mainnet fallback",
                unsupported_network
            ),
        }
    }

    fn context_available_for_network(&self, network: Network) -> bool {
        match network {
            Network::Dash => true, // Mainnet is always available
            Network::Testnet => self.testnet_app_context.is_some(),
            Network::Devnet => self.devnet_app_context.is_some(),
            Network::Regtest => self.local_app_context.is_some(),
            _ => false,
        }
    }

    fn enforce_network_context_invariant(&mut self) {
        if self.context_available_for_network(self.chosen_network) {
            return;
        }

        panic!(
            "BUG: selected network {:?} has no AppContext. Refusing to auto-switch networks.",
            self.chosen_network
        );
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
        if !self.context_available_for_network(network) {
            tracing::error!(
                "Cannot switch to {:?}: network context not available. Staying on current network.",
                network
            );
            return;
        }

        self.chosen_network = network;
        let app_context = self.current_app_context().clone();

        // INTENTIONAL(SEC-004): Clear stale banners from the previous network context.
        // A backend task completing after the switch could set a new banner in the new
        // network context — accepted risk for a local desktop app (cosmetic only).
        MessageBanner::clear_all_global(app_context.egui_ctx());

        for screen in self.main_screens.values_mut() {
            screen.change_context(app_context.clone())
        }

        self.connection_status
            .reset(app_context.core_backend_mode());

        // Reset connection banner tracking so the next frame re-evaluates
        // the new network's state (even if it matches the old state).
        if let Some(handle) = self.connection_banner_handle.take() {
            handle.clear();
        }
        self.previous_connection_state = None;
    }

    /// Update the connection status banner when the overall connection state
    /// transitions between Disconnected, Connecting, Syncing, and Synced.
    ///
    /// Also re-evaluates the banner text while in `Connecting` state each frame
    /// because the degraded-peer timeout can fire without a state transition.
    fn update_connection_banner(&mut self, ctx: &egui::Context, app_context: &Arc<AppContext>) {
        let connection_status = app_context.connection_status();
        let current_state = connection_status.overall_state();
        let state_changed = self.previous_connection_state != Some(current_state);

        // In Connecting state the banner text can change (normal → degraded)
        // without a state transition, so we must re-evaluate every frame.
        // For all other states, skip if nothing changed.
        if !state_changed && current_state != OverallConnectionState::Connecting {
            return;
        }

        // Clear old banner on state transitions
        if state_changed && let Some(handle) = self.connection_banner_handle.take() {
            handle.clear();
        }

        // Display new banner based on current state
        let backend_mode = connection_status.backend_mode();
        match current_state {
            OverallConnectionState::Disconnected => {
                let msg = match backend_mode {
                    CoreBackendMode::Rpc => "Disconnected — check that Dash Core is running",
                    CoreBackendMode::Spv => "Disconnected — check your internet connection",
                };
                self.connection_banner_handle =
                    Some(MessageBanner::set_global(ctx, msg, MessageType::Error));
            }
            OverallConnectionState::Connecting => {
                // SPV active but no peers connected yet. The degraded flag
                // flips after 30 s — `set_global` is idempotent for same text,
                // so calling it every frame while Connecting is cheap.
                let msg = if connection_status.spv_peer_degraded() {
                    "Having trouble finding peers. Check your connection."
                } else {
                    "Looking for peers…"
                };
                // Replace the banner when the text changes (normal → degraded).
                if let Some(handle) = &self.connection_banner_handle {
                    handle.set_message(msg);
                } else {
                    self.connection_banner_handle =
                        Some(MessageBanner::set_global(ctx, msg, MessageType::Warning));
                }
            }
            OverallConnectionState::Syncing => {
                let msg = match backend_mode {
                    CoreBackendMode::Rpc => "Syncing with Dash Core…",
                    CoreBackendMode::Spv => "SPV sync in progress…",
                };
                self.connection_banner_handle =
                    Some(MessageBanner::set_global(ctx, msg, MessageType::Warning));
            }
            OverallConnectionState::Error => {
                self.connection_banner_handle = Some(MessageBanner::set_global(
                    ctx,
                    "SPV sync error — check connection status for details",
                    MessageType::Error,
                ));
            }
            OverallConnectionState::Synced => {
                // No banner needed for fully synced state
            }
        }
        self.previous_connection_state = Some(current_state);
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
        // ── Graceful shutdown: intercept window close so the UI stays responsive ──
        // When the user closes the window we cancel the native close, show a banner,
        // and start an async shutdown. Once all tasks have finished (or timed out)
        // we issue Close ourselves.
        if let Some(rx) = &mut self.shutdown_receiver {
            // Shutdown already in progress — check if it's done.
            let should_close = match rx.try_recv() {
                Ok(()) => {
                    tracing::debug!("Async shutdown finished, closing viewport");
                    true
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    // Sender dropped without sending — shutdown task likely panicked.
                    tracing::warn!("Shutdown channel closed unexpectedly (possible panic)");
                    true
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    // Still waiting — check hard deadline to prevent infinite loop.
                    if let Some(started) = self.shutdown_started {
                        let grace = crate::utils::tasks::SHUTDOWN_TIMEOUT
                            + std::time::Duration::from_secs(5);
                        if started.elapsed() > grace {
                            tracing::warn!(
                                "Shutdown hard deadline exceeded, force-closing viewport"
                            );
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
            };
            if should_close {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                ctx.request_repaint();
            }
            // Render a minimal UI that shows the shutdown banner.
            crate::ui::theme::apply_theme(ctx, self.theme_preference);
            crate::ui::components::styled::island_central_panel(ctx, |_ui| {});
            return;
        }

        if ctx.input(|i| i.viewport().close_requested()) {
            // Prevent the window from closing immediately.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            MessageBanner::set_global(
                ctx,
                "Shutting down background tasks — please wait…",
                MessageType::Warning,
            );
            tracing::debug!("Close requested, starting async shutdown");
            self.shutdown_receiver = Some(self.subtasks.shutdown_async());
            self.shutdown_started = Some(std::time::Instant::now());
            ctx.request_repaint();
            return;
        }

        // Apply Dash theme with user preference
        crate::ui::theme::apply_theme(ctx, self.theme_preference);

        self.enforce_network_context_invariant();
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
                        BackendTaskSuccessResult::Message(ref msg) => {
                            // TODO(RUST-002): Some screens inspect Message text for error
                            // keywords and may override with an Error banner, causing a
                            // brief green-then-red flash. Refactor to pass structured error
                            // types through task results instead of string messages.
                            // See https://github.com/dashpay/dash-evo-tool/issues/660 .
                            MessageBanner::set_global(ctx, msg, MessageType::Success);
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
                        BackendTaskSuccessResult::CoreWalletSelectionNeeded(wallets) => {
                            // Dismiss any in-progress banner on the current screen
                            self.visible_screen_mut().display_message(
                                "Dash Core wallet selection required",
                                MessageType::Info,
                            );
                            let dialog = SelectionDialog::new(
                                "Select Dash Core Wallet",
                                "Your Dash Core node has multiple wallets loaded.\nSelect which wallet to use for RPC operations:",
                                wallets.clone(),
                            );
                            self.core_wallet_dialog = Some(dialog);
                            self.core_wallet_names = Some(wallets);
                        }
                        _ => {
                            // For all other success results, let the screen decide how to display
                            // the outcome without showing a generic global success banner.
                            self.visible_screen_mut()
                                .display_task_result(unboxed_message);
                        }
                    }
                }
                TaskResult::Error(err) => {
                    let msg = err.to_string();
                    let handle = MessageBanner::set_global(ctx, &msg, MessageType::Error);
                    if self.current_app_context().is_developer_mode() {
                        // INTENTIONAL(SEC-003): TaskError Debug output is shown to users
                        // in developer mode. This is a local UI app — no third parties
                        // see this output. Ensure inner error types don't expose secrets
                        // (see #667).
                        handle.with_details(&err);
                    }
                    self.visible_screen_mut()
                        .display_message(&msg, MessageType::Error);
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

        self.update_connection_banner(ctx, &active_context);

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

        // Show Core wallet selection dialog if active
        if self.core_wallet_dialog.is_some() {
            use crate::ui::components::component_trait::{Component, ComponentResponse};

            // Block all input behind the dialog by consuming clicks on a full-screen area
            egui::Area::new(egui::Id::new("core_wallet_dialog_blocker"))
                .fixed_pos(egui::Pos2::ZERO)
                .order(egui::Order::Middle)
                .interactable(true)
                .show(ctx, |ui| {
                    let content_rect = ctx.content_rect();
                    ui.allocate_response(content_rect.size(), egui::Sense::click_and_drag());
                });

            // Render the dialog in a Foreground-order Area so the Window appears above everything
            egui::Area::new(egui::Id::new("core_wallet_dialog_area"))
                .fixed_pos(egui::Pos2::ZERO)
                .order(egui::Order::Foreground)
                .interactable(true)
                .show(ctx, |ui| {
                    ui.set_min_size(ctx.content_rect().size());
                    let mut dialog = self.core_wallet_dialog.take().unwrap();
                    let response = dialog.show(ui);
                    if let Some(status) = response.inner.changed_value() {
                        match status {
                            SelectionStatus::Selected(idx) => {
                                if let Some(wallets) = self.core_wallet_names.take()
                                    && let Some(wallet_name) = wallets.get(*idx).cloned()
                                {
                                    match self
                                        .current_app_context()
                                        .clone()
                                        .set_core_wallet_name(wallet_name.clone())
                                    {
                                        Ok(()) => {
                                            MessageBanner::set_global(
                                                ctx,
                                                format!(
                                                    "Dash Core wallet '{}' selected",
                                                    wallet_name
                                                ),
                                                MessageType::Success,
                                            );
                                        }
                                        Err(e) => {
                                            MessageBanner::set_global(
                                                ctx,
                                                "Failed to set Dash Core wallet",
                                                MessageType::Error,
                                            )
                                            .with_details(&e);
                                        }
                                    }
                                }
                            }
                            SelectionStatus::Canceled => {
                                self.core_wallet_names = None;
                                MessageBanner::set_global(
                                    ctx,
                                    "Dash Core wallet not selected. Set manually in .env with {NETWORK}_core_wallet_name=<wallet>",
                                    MessageType::Info,
                                );
                            }
                        }
                    } else {
                        self.core_wallet_dialog = Some(dialog);
                    }
                });
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // If shutdown_receiver is Some, the async shutdown was already initiated
        // in update(). Skip the blocking fallback to avoid double-shutdown.
        // The blocking path only runs when the window was force-closed without
        // going through update() (e.g., OS-level kill, alt-F4 on some platforms).
        if self.shutdown_receiver.is_some() {
            tracing::debug!("on_exit: async shutdown was initiated, skipping blocking fallback");
            return;
        }
        tracing::debug!("on_exit: fallback blocking shutdown");
        if let Err(e) = self.subtasks.shutdown() {
            tracing::error!("Error during task shutdown: {}", e);
        }
        tracing::debug!("App shutdown complete");
    }
}
