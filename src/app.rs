#[cfg(not(feature = "testing"))]
use crate::app_dir::data_file_path;
use crate::app_dir::{app_user_data_dir_path, ensure_data_dir_exists, ensure_env_file};
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
use crate::model::password_info::PasswordInfo;
use crate::model::settings::Settings;
use crate::spv::CoreBackendMode;
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
use std::path::PathBuf;
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

/// Parameters needed for lazy `AppContext` creation when the user switches
/// to a network whose context was deferred at startup.
struct LazyContextParams {
    data_dir: PathBuf,
    db: Arc<Database>,
    password_info: Option<PasswordInfo>,
    subtasks: Arc<TaskManager>,
    connection_status: Arc<ConnectionStatus>,
    egui_ctx: egui::Context,
}

struct ThemeState {
    preference: ThemeMode,
    resolved: ThemeMode,
    last_applied: Option<ThemeMode>,
    last_checked: Instant,
}

impl ThemeState {
    fn new(preference: ThemeMode) -> Self {
        Self {
            resolved: crate::ui::theme::resolve_theme_mode(preference),
            last_applied: None,
            last_checked: Instant::now(),
            preference,
        }
    }

    /// Polls the OS for system theme changes (throttled to every 2s) and
    /// applies the theme if it changed. Returns `true` if the theme was applied.
    fn poll_and_apply(&mut self, ctx: &egui::Context) -> bool {
        if self.preference == ThemeMode::System {
            let now = Instant::now();
            if now.duration_since(self.last_checked) >= Duration::from_secs(2) {
                self.last_checked = now;
                if let Some(detected) = crate::ui::theme::try_detect_system_theme()
                    && detected != self.resolved
                {
                    self.resolved = detected;
                }
            }
        }
        if self.last_applied != Some(self.resolved) {
            crate::ui::theme::apply_theme(ctx, self.resolved);
            self.last_applied = Some(self.resolved);
            true
        } else {
            false
        }
    }

    fn apply_new_preference(&mut self, ctx: &egui::Context, new_theme: ThemeMode) -> bool {
        self.preference = new_theme;
        let mut detection_failed = false;
        self.resolved = if new_theme == ThemeMode::System {
            match crate::ui::theme::try_detect_system_theme() {
                Some(detected) => detected,
                None => {
                    detection_failed = true;
                    self.resolved
                }
            }
        } else {
            new_theme
        };
        self.last_checked = Instant::now();
        crate::ui::theme::apply_theme(ctx, self.resolved);
        self.last_applied = Some(self.resolved);
        detection_failed
    }
}

pub struct AppState {
    pub main_screens: BTreeMap<RootScreenType, Screen>,
    pub selected_main_screen: RootScreenType,
    pub screen_stack: Vec<Screen>,
    pub chosen_network: Network,
    pub connection_status: Arc<ConnectionStatus>,
    pub network_contexts: BTreeMap<Network, Arc<AppContext>>,
    /// Params kept for lazy AppContext creation when switching networks.
    lazy_ctx_params: Option<LazyContextParams>,
    #[allow(dead_code)] // Kept alive for the lifetime of the app
    zmq_listeners: BTreeMap<Network, CoreZMQListener>,
    pub core_message_receiver: mpsc::Receiver<(ZMQMessage, Network)>,
    pub task_result_sender: egui_mpsc::SenderAsync<TaskResult>, // Channel sender for sending task results
    pub task_result_receiver: tokiompsc::Receiver<TaskResult>, // Channel receiver for receiving task results
    theme: ThemeState,
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
    /// Async shutdown receiver. `Some` while a graceful shutdown is in progress;
    /// the viewport is closed once the receiver resolves.
    shutdown_receiver: Option<tokio::sync::oneshot::Receiver<()>>,
    /// Timestamp when the async shutdown was initiated, used as a hard deadline
    /// to force-close the viewport if the shutdown task stalls.
    shutdown_started: Option<std::time::Instant>,
    /// Whether accessibility is force-enabled (DASH_EVO_TOOL_ACCESSIBILITY=1). When unset, accessibility still works normally via VoiceOver or other assistive technology — this flag forces it on unconditionally.
    accessibility_enforced: bool,
    /// Whether we have already triggered platform-level accessibility activation.
    accessibility_activated: bool,
    /// How many frames we have attempted accessibility activation.
    accessibility_retries: u32,
    /// Shared MCP context -- follows network switches via `ArcSwap`.
    #[cfg(feature = "mcp")]
    pub mcp_app_context: Option<Arc<arc_swap::ArcSwap<AppContext>>>,
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
        let data_dir = app_user_data_dir_path()?;
        ensure_data_dir_exists(&data_dir)?;
        ensure_env_file(&data_dir);
        crate::config::Config::migrate_env_file_if_needed();
        initialize_logger();
        let db_file_path = data_file_path(&data_dir, "data.db")?;
        let db = Arc::new(Database::new(&db_file_path)?);
        db.initialize(&db_file_path)?;
        Self::new_inner(ctx, db, data_dir)
    }

    /// Creates a new `AppState` using an in-memory database for testing.
    ///
    /// Available only when the `testing` feature is active. This prevents tests
    /// from reading or writing the production database.
    #[cfg(feature = "testing")]
    pub fn new(ctx: egui::Context) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let data_dir = app_user_data_dir_path()?;
        ensure_data_dir_exists(&data_dir)?;
        ensure_env_file(&data_dir);
        crate::config::Config::migrate_env_file_if_needed();
        let db = Arc::new(
            crate::database::test_helpers::create_test_database()
                .map_err(|e| format!("Failed to create test database: {}", e))?,
        );
        Self::new_inner(ctx, db, data_dir)
    }

    fn new_inner(
        ctx: egui::Context,
        db: Arc<Database>,
        data_dir: PathBuf,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let settings = db.get_settings()?.map(Settings::from).unwrap_or_default();
        let password_info = settings.password_info;
        let theme_preference = settings.theme_mode;
        let overwrite_dash_conf = settings.overwrite_dash_conf;
        let onboarding_completed = settings.onboarding_completed;

        let subtasks = Arc::new(TaskManager::new());
        let connection_status = Arc::new(ConnectionStatus::new());

        let saved_network = settings.network;

        // Build a helper to create AppContext for a given network.
        let make_context = |network: Network| -> Option<Arc<AppContext>> {
            AppContext::new(
                data_dir.clone(),
                network,
                db.clone(),
                password_info.clone(),
                subtasks.clone(),
                connection_status.clone(),
                ctx.clone(),
            )
        };

        // Only create the saved/active network eagerly; defer ALL others
        // (including mainnet) until the user switches to them. This avoids
        // DAPI discovery + SDK init for networks the user may never use.
        let active_context = make_context(saved_network).ok_or_else(|| {
            format!(
                "Failed to create AppContext for {:?}. Check your Dash configuration.",
                saved_network
            )
        })?;

        let mut network_contexts = BTreeMap::new();
        network_contexts.insert(saved_network, active_context.clone());

        // Store params for lazy context creation when switching networks later.
        let lazy_ctx_params = Some(LazyContextParams {
            data_dir,
            db: db.clone(),
            password_info,
            subtasks: subtasks.clone(),
            connection_status: connection_status.clone(),
            egui_ctx: ctx.clone(),
        });

        // load fonts
        ctx.set_fonts(crate::bundled::fonts().expect("failed to load fonts"));

        // Force-enable AccessKit so the accessibility tree is populated every
        // frame, even without VoiceOver or other assistive technology running.
        // Without this flag, AccessKit activates lazily when a real assistive
        // client connects (which is the normal behavior).
        // Gated behind DASH_EVO_TOOL_ACCESSIBILITY=1 to avoid per-frame cost
        // when not needed for automation tooling.
        let accessibility_enforced =
            std::env::var("DASH_EVO_TOOL_ACCESSIBILITY").unwrap_or_default() == "1";
        if accessibility_enforced {
            ctx.enable_accesskit();
        }

        // All screens are initialized with the active context (chosen_network).
        // They will get the right context via change_context() on network switch.
        let identities_screen = IdentitiesScreen::new(&active_context);
        let dpns_active_contests_screen = DPNSScreen::new(&active_context, DPNSSubscreen::Active);
        let dpns_past_contests_screen = DPNSScreen::new(&active_context, DPNSSubscreen::Past);
        let dpns_my_usernames_screen = DPNSScreen::new(&active_context, DPNSSubscreen::Owned);
        let dpns_scheduled_votes_screen =
            DPNSScreen::new(&active_context, DPNSSubscreen::ScheduledVotes);
        let transition_visualizer_screen = TransitionVisualizerScreen::new(&active_context);
        let proof_visualizer_screen = ProofVisualizerScreen::new(&active_context);
        let document_visualizer_screen = DocumentVisualizerScreen::new(&active_context);
        let contract_visualizer_screen = ContractVisualizerScreen::new(&active_context);
        let proof_log_screen = ProofLogScreen::new(&active_context);
        let platform_info_screen = PlatformInfoScreen::new(&active_context);
        let address_balance_screen = AddressBalanceScreen::new(&active_context);
        let grovestark_screen = GroveSTARKScreen::new(&active_context);
        let document_query_screen = DocumentQueryScreen::new(&active_context);
        let tokens_balances_screen = TokensScreen::new(&active_context, TokensSubscreen::MyTokens);
        let token_search_screen = TokensScreen::new(&active_context, TokensSubscreen::SearchTokens);
        let token_creator_screen =
            TokensScreen::new(&active_context, TokensSubscreen::TokenCreator);
        let contracts_dashpay_screen =
            DashPayScreen::new(&active_context, DashPaySubscreen::Profile);
        let dashpay_contacts_screen =
            DashPayScreen::new(&active_context, DashPaySubscreen::Contacts);
        let dashpay_profile_screen = DashPayScreen::new(&active_context, DashPaySubscreen::Profile);
        let dashpay_payments_screen =
            DashPayScreen::new(&active_context, DashPaySubscreen::Payments);
        let dashpay_profile_search_screen = ProfileSearchScreen::new(active_context.clone());

        let network_chooser_screen =
            NetworkChooserScreen::new(&network_contexts, saved_network, overwrite_dash_conf);

        let masternode_list_diff_screen = MasternodeListDiffScreen::new(&active_context);

        let wallets_balances_screen = WalletsBalancesScreen::new(&active_context);

        let selected_main_screen = settings.root_screen_type;
        let chosen_network = saved_network;

        // // Create a channel with a buffer size of 32 (adjust as needed)
        let (task_result_sender, task_result_receiver) =
            tokiompsc::channel(256).with_egui_ctx(ctx.clone());

        // Create a channel for communication with the InstantSendListener
        let (core_message_sender, core_message_receiver) =
            mpsc::channel().with_egui_ctx(ctx.clone());

        let zmq_listeners: BTreeMap<Network, CoreZMQListener> = network_contexts
            .iter()
            .filter_map(|(&network, ctx)| {
                Self::spawn_zmq_listener(ctx, network, &core_message_sender)
                    .map(|listener| (network, listener))
            })
            .collect();

        // MCP server (feature-gated, opt-in via MCP_API_KEY env var)
        #[cfg(feature = "mcp")]
        let mcp_app_context = {
            if let Some(mcp_config) = crate::mcp::McpConfig::from_env() {
                let initial_ctx = active_context.clone();
                let mcp_ctx = Arc::new(arc_swap::ArcSwap::new(initial_ctx));
                let ctx_for_server = mcp_ctx.clone();
                let cancel = subtasks.cancellation_token.clone();
                subtasks.spawn_sync("mcp-server", async move {
                    if let Err(e) =
                        crate::mcp::start_http_server(ctx_for_server, mcp_config, cancel).await
                    {
                        tracing::error!("MCP server failed: {e}");
                    }
                });
                tracing::debug!("MCP server enabled");
                Some(mcp_ctx)
            } else {
                let reason = match std::env::var("MCP_API_KEY") {
                    Ok(ref k) if !k.is_empty() => "MCP_API_KEY is set but invalid (too short)",
                    _ => "MCP_API_KEY not set",
                };
                tracing::debug!("MCP server disabled ({reason})");
                None
            }
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
            network_contexts,
            lazy_ctx_params,
            zmq_listeners,
            core_message_receiver,
            task_result_sender,
            task_result_receiver,
            theme: ThemeState::new(theme_preference),
            last_scheduled_vote_check: Instant::now(),
            last_repaint_request: Instant::now(),
            subtasks,
            show_welcome_screen: !onboarding_completed,
            welcome_screen: None,
            previous_connection_state: None,
            connection_banner_handle: None,
            shutdown_receiver: None,
            shutdown_started: None,
            accessibility_enforced,
            accessibility_activated: false,
            accessibility_retries: 0,
            #[cfg(feature = "mcp")]
            mcp_app_context,
        };

        // Initialize welcome screen if needed (uses whichever context is active)
        if app_state.show_welcome_screen {
            app_state.welcome_screen =
                Some(WelcomeScreen::new(app_state.current_app_context().clone()));
        } else {
            app_state.try_auto_start_spv();

            // Refresh ALL main screens so they load data properly
            // This ensures screens like DashPay Profile have identities loaded
            // even if they're not the initially selected screen
            for screen in app_state.main_screens.values_mut() {
                screen.refresh_on_arrival();
            }
        }

        // Warm up the Halo 2 ProvingKey in a background thread (~30s build).
        // This ensures the key is ready for the user's first shielded operation.
        #[cfg(not(feature = "testing"))]
        std::thread::spawn(|| {
            let _ = crate::context::shielded::get_proving_key();
            tracing::info!("Halo 2 ProvingKey built and cached");
        });

        Ok(app_state)
    }

    /// Allows enabling or disabling animations globally for the app.
    ///
    /// Default is enabled.
    pub fn with_animations(self, enabled: bool) -> Self {
        for context in self.network_contexts.values() {
            context.enable_animations(enabled);
        }
        self
    }

    pub fn current_app_context(&self) -> &Arc<AppContext> {
        self.network_contexts
            .get(&self.chosen_network)
            .unwrap_or_else(|| {
                panic!(
                    "BUG: chosen network is {:?} but its AppContext is missing",
                    self.chosen_network
                )
            })
    }

    fn context_available_for_network(&self, network: Network) -> bool {
        self.network_contexts.contains_key(&network)
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

    // Handle the backend task and send the result through the channel.
    //
    // Uses spawn_blocking + block_on to avoid Send bound issues with platform
    // SDK types (DataContract/Sdk references across await points).
    fn handle_backend_task(&self, task: BackendTask) {
        let sender = self.task_result_sender.clone();
        let app_context = self.current_app_context().clone();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            handle.block_on(async move {
                let result = app_context.run_backend_task(task, sender.clone()).await;
                if let Err(e) = sender.send(result.into()).await {
                    tracing::error!("Failed to send task result: {}", e);
                }
            });
        });
    }

    /// Handle the backend tasks and send the results through the channel
    fn handle_backend_tasks(&self, tasks: Vec<BackendTask>, mode: BackendTasksExecutionMode) {
        let sender = self.task_result_sender.clone();
        let app_context = self.current_app_context().clone();
        let handle = tokio::runtime::Handle::current();

        tokio::task::spawn_blocking(move || {
            handle.block_on(async move {
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

                for result in results {
                    if let Err(e) = sender.send(result.into()).await {
                        tracing::error!("Failed to send task result: {}", e);
                    }
                }
            });
        });
    }

    fn spawn_zmq_listener(
        ctx: &Arc<AppContext>,
        network: Network,
        sender: &egui_mpsc::SenderSync<(ZMQMessage, Network)>,
    ) -> Option<CoreZMQListener> {
        let default_endpoint = match network {
            Network::Mainnet => "tcp://127.0.0.1:23708",
            Network::Testnet => "tcp://127.0.0.1:23709",
            Network::Devnet => "tcp://127.0.0.1:23710",
            Network::Regtest => "tcp://127.0.0.1:20302",
            _ => return None,
        };
        let endpoint = ctx
            .config
            .read()
            .unwrap()
            .core_zmq_endpoint
            .clone()
            .unwrap_or_else(|| default_endpoint.to_string());
        let disable = ctx
            .get_settings()
            .ok()
            .flatten()
            .map(|s| s.disable_zmq)
            .unwrap_or(false);
        if disable {
            return None;
        }
        CoreZMQListener::spawn_listener(
            network,
            &endpoint,
            sender.clone(),
            Some(ctx.sx_zmq_status.clone()),
        )
        .inspect_err(|e| tracing::error!("Failed to create {network:?} ZMQ listener: {e}"))
        .ok()
    }

    pub fn active_root_screen_mut(&mut self) -> &mut Screen {
        self.main_screens
            .get_mut(&self.selected_main_screen)
            .expect("expected to get screen")
    }

    pub fn change_network(&mut self, network: Network) {
        // Lazily create the AppContext if this network was deferred at startup.
        if !self.context_available_for_network(network) {
            if let Some(ref params) = self.lazy_ctx_params
                && let Some(ctx) = AppContext::new(
                    params.data_dir.clone(),
                    network,
                    params.db.clone(),
                    params.password_info.clone(),
                    params.subtasks.clone(),
                    params.connection_status.clone(),
                    params.egui_ctx.clone(),
                )
            {
                self.network_contexts.insert(network, ctx);
            }

            if !self.context_available_for_network(network) {
                tracing::error!(
                    "Cannot switch to {:?}: network context not available. Staying on current network.",
                    network
                );
                return;
            }
        }

        self.chosen_network = network;

        let app_context = self.current_app_context().clone();

        // Update MCP server's context to follow network switch
        #[cfg(feature = "mcp")]
        if let Some(ref mcp_ctx) = self.mcp_app_context {
            mcp_ctx.store(app_context.clone());
            tracing::debug!("MCP context switched to {:?}", network);
        }

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
                let handle = MessageBanner::set_global(
                    ctx,
                    "SPV sync failed. Go to Settings for connection details.",
                    MessageType::Error,
                );
                if let Some(detail) = connection_status.spv_last_error() {
                    handle.with_details(detail);
                }
                self.connection_banner_handle = Some(handle);
            }
            OverallConnectionState::Synced => {
                // No banner needed for fully synced state.
                // Fetch epoch info on first sync to populate protocol version
                // and fee multiplier — needed for feature gating (e.g., shielded
                // tab requires protocol version >= 12).
                if state_changed {
                    let task = BackendTask::PlatformInfo(
                        crate::backend_task::platform_info::PlatformInfoTaskRequestType::CurrentEpochInfo,
                    );
                    self.handle_backend_task(task);
                }
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

    fn set_main_screen(&mut self, root_screen_type: RootScreenType) {
        self.selected_main_screen = root_screen_type;
        self.active_root_screen_mut().refresh_on_arrival();
        self.current_app_context()
            .update_settings(root_screen_type)
            .ok();
    }

    /// Auto-start SPV sync if the conditions are met: auto-start enabled,
    /// developer mode on, and backend mode is SPV.
    // TODO: SPV auto-start is gated behind developer mode while SPV is in development.
    // Remove the is_developer_mode() check once SPV is production-ready.
    fn try_auto_start_spv(&self) {
        let ctx = self.current_app_context();
        let auto_start = ctx.db.get_auto_start_spv().unwrap_or(false);
        if auto_start && ctx.is_developer_mode() && ctx.core_backend_mode() == CoreBackendMode::Spv
        {
            if let Err(e) = ctx.start_spv() {
                tracing::warn!("Failed to auto-start SPV sync: {e}");
            } else {
                tracing::info!("SPV sync started automatically for {:?}", ctx.network);
            }
        }
    }
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
            self.theme.poll_and_apply(ctx);
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

        // On the first frame, trigger platform-level accessibility activation
        // so tools like Peekaboo can see the AccessKit tree without VoiceOver.
        // Retries up to 60 frames, then gives up to avoid indefinite repaints.
        const MAX_ACCESSIBILITY_RETRIES: u32 = 60;
        if self.accessibility_enforced
            && !self.accessibility_activated
            && self.accessibility_retries < MAX_ACCESSIBILITY_RETRIES
        {
            self.accessibility_retries += 1;
            self.accessibility_activated = crate::platform::force_accessibility_activation();
            if !self.accessibility_activated {
                if self.accessibility_retries >= MAX_ACCESSIBILITY_RETRIES {
                    tracing::warn!(
                        "Accessibility activation failed after {} frames, giving up",
                        MAX_ACCESSIBILITY_RETRIES
                    );
                } else {
                    // Ensure we get another frame to retry, even if egui would otherwise go idle.
                    ctx.request_repaint();
                }
            }
        }

        self.theme.poll_and_apply(ctx);

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
                        BackendTaskSuccessResult::Progress { .. } => {
                            // Progress updates only go to the screen — no global banner.
                            // The screen updates its existing banner handle in-place.
                            // TODO: Routes via visible_screen_mut(), so if the user
                            // navigates away from the originating screen, progress
                            // updates land on the wrong screen. Adding task-to-screen
                            // affinity would fix this (same limitation as Message).
                            self.visible_screen_mut()
                                .display_task_result(unboxed_message);
                        }
                        BackendTaskSuccessResult::UpdatedThemePreference(new_theme) => {
                            let detection_failed = self.theme.apply_new_preference(ctx, new_theme);
                            if detection_failed {
                                MessageBanner::set_global(
                                    ctx,
                                    "Could not detect your system theme. Using the previous theme for now — it will update automatically when detection succeeds.",
                                    MessageType::Warning,
                                );
                            } else {
                                MessageBanner::set_global(
                                    ctx,
                                    "Theme preference updated successfully",
                                    MessageType::Success,
                                );
                            }
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
                            // For all other success results, let the screen decide how to display
                            // the outcome without showing a generic global success banner.
                            self.visible_screen_mut()
                                .display_task_result(unboxed_message);
                        }
                    }
                }
                TaskResult::Error(TaskError::MustRetry(msg)) => {
                    MessageBanner::set_global(ctx, &msg, MessageType::Success);
                    self.visible_screen_mut()
                        .display_message(&msg, MessageType::Success);
                    self.visible_screen_mut().refresh();
                }
                TaskResult::Error(err) => {
                    // Let the screen handle specific error types first.
                    // If handled, skip the generic error banner.
                    let handled = self.visible_screen_mut().display_task_error(&err);

                    if !handled {
                        let msg = err.to_string();
                        let handle = MessageBanner::set_global(ctx, &msg, MessageType::Error);
                        // Show technical details only in developer mode.
                        // All user-facing information is in the Display string.
                        if self.current_app_context().is_developer_mode() {
                            // INTENTIONAL(SEC-003): TaskError Debug output is shown to users
                            // in developer mode. This is a local UI app —
                            // no third parties see this output. Ensure inner error types
                            // don't expose secrets (see #667).
                            handle.with_details(&err);
                        }
                        self.visible_screen_mut()
                            .display_message(&msg, MessageType::Error);
                    }
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
            let Some(app_context) = self.network_contexts.get(&network) else {
                tracing::error!("No app context available for {:?}", network);
                continue;
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
                    self.set_main_screen(root_screen_type);
                }
                AppAction::SetMainScreenThenGoToMainScreen(root_screen_type) => {
                    self.set_main_screen(root_screen_type);
                    self.screen_stack = vec![];
                }
                AppAction::SetMainScreenThenPopScreen(root_screen_type) => {
                    self.set_main_screen(root_screen_type);
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
                    self.set_main_screen(root_screen_type);
                }
                AppAction::Custom(_) => {}
                AppAction::OnboardingComplete {
                    main_screen,
                    add_screen,
                } => {
                    self.show_welcome_screen = false;
                    self.welcome_screen = None;
                    self.set_main_screen(main_screen);
                    if let Some(screen_type) = add_screen {
                        let screen = screen_type.create_screen(self.current_app_context());
                        self.screen_stack.push(screen);
                    }
                    self.try_auto_start_spv();
                }
            }
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
