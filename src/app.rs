use crate::context::AppContext;
use crate::database::Database;
use crate::logging::initialize_logger;
use crate::platform::{BackendTask, BackendTaskSuccessResult};
use crate::ui::document_query_screen::DocumentQueryScreen;
use crate::ui::wallet::wallets_screen::WalletsBalancesScreen;
use crate::ui::dpns_contested_names_screen::DPNSContestedNamesScreen;
use crate::ui::identities::identities_screen::IdentitiesScreen;
use crate::ui::network_chooser_screen::NetworkChooserScreen;
use crate::ui::transition_visualizer_screen::TransitionVisualizerScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike, ScreenType};
use dash_sdk::dpp::dashcore::Network;
use derive_more::From;
use eframe::{egui, App};
use std::collections::BTreeMap;
use std::ops::BitOrAssign;
use std::sync::Arc;
use std::time::Instant;
use std::vec;
use egui::Widget;
use tokio::sync::mpsc;

#[derive(Debug, From)]
pub enum TaskResult {
    Refresh,
    Success(BackendTaskSuccessResult),
    Error(String),
}

impl From<Result<BackendTaskSuccessResult, String>> for TaskResult {
    fn from(value: Result<BackendTaskSuccessResult, String>) -> Self {
        match value {
            Ok(value) => TaskResult::Success(value),
            Err(e) => TaskResult::Error(e),
        }
    }
}

pub struct AppState {
    pub main_screens: BTreeMap<RootScreenType, Screen>,
    pub selected_main_screen: RootScreenType,
    pub screen_stack: Vec<Screen>,
    pub chosen_network: Network,
    pub mainnet_app_context: Arc<AppContext>,
    pub testnet_app_context: Option<Arc<AppContext>>,
    pub task_result_sender: mpsc::Sender<TaskResult>, // Channel sender for sending task results
    pub task_result_receiver: mpsc::Receiver<TaskResult>, // Channel receiver for receiving task results
    last_repaint: Instant, // Track the last time we requested a repaint
}

#[derive(Debug, PartialEq)]
pub enum DesiredAppAction {
    None,
    PopScreen,
    GoToMainScreen,
    SwitchNetwork(Network),
    AddScreenType(ScreenType),
    BackendTask(BackendTask),
}

impl DesiredAppAction {
    pub fn create_action(&self, app_context: &Arc<AppContext>) -> AppAction {
        match self {
            DesiredAppAction::None => AppAction::None,
            DesiredAppAction::PopScreen => AppAction::PopScreen,
            DesiredAppAction::GoToMainScreen => AppAction::GoToMainScreen,
            DesiredAppAction::AddScreenType(screen_type) => {
                AppAction::AddScreen(screen_type.create_screen(app_context))
            }
            DesiredAppAction::BackendTask(backend_task) => {
                AppAction::BackendTask(backend_task.clone())
            }
            DesiredAppAction::SwitchNetwork(network) => AppAction::SwitchNetwork(*network),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum AppAction {
    None,
    PopScreen,
    PopScreenAndRefresh,
    GoToMainScreen,
    SwitchNetwork(Network),
    SetMainScreen(RootScreenType),
    AddScreen(Screen),
    BackendTask(BackendTask),
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
    pub fn new() -> Self {
        initialize_logger();
        let db = Arc::new(Database::new("identities.db").unwrap());
        db.initialize().unwrap();

        let settings = db.get_settings().expect("expected to get settings");

        let mainnet_app_context = match AppContext::new(Network::Dash, db.clone()) {
            Some(context) => context,
            None => {
                eprintln!(
                    "Error: Failed to create the AppContext. Expected Dash config for mainnet."
                );
                std::process::exit(1);
            }
        };
        let testnet_app_context = AppContext::new(Network::Testnet, db.clone());

        let mut identities_screen = IdentitiesScreen::new(&mainnet_app_context);
        let mut dpns_contested_names_screen = DPNSContestedNamesScreen::new(&mainnet_app_context);
        let mut transition_visualizer_screen =
            TransitionVisualizerScreen::new(&mainnet_app_context);
        let mut document_query_screen = DocumentQueryScreen::new(&mainnet_app_context);
        let mut network_chooser_screen = NetworkChooserScreen::new(
            &mainnet_app_context,
            testnet_app_context.as_ref(),
            Network::Dash,
        );

        let mut wallets_balances_screen = WalletsBalancesScreen::new(&mainnet_app_context);

        let mut selected_main_screen = RootScreenType::RootScreenIdentities;

        let mut chosen_network = Network::Dash;

        if let Some((network, screen_type)) = settings {
            selected_main_screen = screen_type;
            chosen_network = network;
            if network == Network::Testnet && testnet_app_context.is_some() {
                let testnet_app_context = testnet_app_context.as_ref().unwrap();
                identities_screen = IdentitiesScreen::new(testnet_app_context);
                dpns_contested_names_screen = DPNSContestedNamesScreen::new(testnet_app_context);
                transition_visualizer_screen = TransitionVisualizerScreen::new(testnet_app_context);
                document_query_screen = DocumentQueryScreen::new(testnet_app_context);
                wallets_balances_screen = WalletsBalancesScreen::new(testnet_app_context);
            }
            network_chooser_screen.current_network = chosen_network;
        }

        // // Create a channel with a buffer size of 32 (adjust as needed)
        let (task_result_sender, task_result_receiver) = mpsc::channel(256);

        // Initialize the last repaint time to the current instant
        let last_repaint = Instant::now();

        Self {
            main_screens: [
                (
                    RootScreenType::RootScreenIdentities,
                    Screen::IdentitiesScreen(identities_screen),
                ),
                (
                    RootScreenType::RootScreenDPNSContestedNames,
                    Screen::DPNSContestedNamesScreen(dpns_contested_names_screen),
                ),
                (
                    RootScreenType::RootScreenWalletsBalances,
                    Screen::WalletsBalancesScreen(wallets_balances_screen),
                ),
                (
                    RootScreenType::RootScreenTransitionVisualizerScreen,
                    Screen::TransitionVisualizerScreen(transition_visualizer_screen),
                ),
                (
                    RootScreenType::RootScreenDocumentQuery,
                    Screen::DocumentQueryScreen(document_query_screen),
                ),
                (
                    RootScreenType::RootScreenNetworkChooser,
                    Screen::NetworkChooserScreen(network_chooser_screen),
                ),
            ]
            .into(),
            selected_main_screen,
            screen_stack: vec![],
            chosen_network,
            mainnet_app_context,
            testnet_app_context,
            task_result_sender,
            task_result_receiver,
            last_repaint,
        }
    }

    pub fn current_app_context(&self) -> &Arc<AppContext> {
        match self.chosen_network {
            Network::Dash => &self.mainnet_app_context,
            Network::Testnet => self.testnet_app_context.as_ref().expect("expected testnet"),
            Network::Devnet => todo!(),
            Network::Regtest => todo!(),
            _ => todo!(),
        }
    }

    // Handle the backend task and send the result through the channel
    pub fn handle_backend_task(&self, task: BackendTask) {
        let sender = self.task_result_sender.clone();
        let app_context = self.current_app_context().clone();

        tokio::spawn(async move {
            let result = app_context.run_backend_task(task, sender.clone()).await;

            // Send the result back to the main thread
            if let Err(e) = sender.send(result.into()).await {
                eprintln!("Failed to send task result: {}", e);
            }
        });
    }

    pub fn active_root_screen(&self) -> &Screen {
        self.main_screens
            .get(&self.selected_main_screen)
            .expect("expected to get screen")
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
    }

    pub fn visible_screen(&self) -> &Screen {
        if let Some(last_screen) = self.screen_stack.last() {
            last_screen
        } else {
            self.active_root_screen()
        }
    }

    pub fn visible_screen_mut(&mut self) -> &mut Screen {
        if self.screen_stack.is_empty() {
            self.active_root_screen_mut()
        } else {
            self.screen_stack.last_mut().unwrap()
        }
    }

    pub fn visible_screen_type(&self) -> ScreenType {
        if let Some(last_screen) = self.screen_stack.last() {
            last_screen.screen_type()
        } else {
            self.selected_main_screen.into()
        }
    }
}

impl AppState {}

impl App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll the receiver for any new task results
        while let Ok(task_result) = self.task_result_receiver.try_recv() {
            // Handle the result on the main thread
            match task_result {
                TaskResult::Success(message) => match message {
                    BackendTaskSuccessResult::None => {
                        self.visible_screen_mut().pop_on_success();
                    }
                    BackendTaskSuccessResult::Message(message) => {
                        self.visible_screen_mut()
                            .display_message(&message, MessageType::Info);
                    }
                    BackendTaskSuccessResult::Documents(_) => {
                        self.visible_screen_mut().display_task_result(message);
                    }
                    BackendTaskSuccessResult::CoreItem(_) => {
                        self.visible_screen_mut().display_task_result(message);
                    }
                    BackendTaskSuccessResult::SuccessfulVotes(_) => {
                        self.visible_screen_mut().refresh();
                    }
                },
                TaskResult::Error(message) => {
                    self.visible_screen_mut()
                        .display_message(&message, MessageType::Error);
                }
                TaskResult::Refresh => {
                    self.visible_screen_mut().refresh();
                }
            }
        }

        // Use a timer to repaint the UI every 0.05 seconds
        ctx.request_repaint_after(std::time::Duration::from_millis(50));

        let action = self.visible_screen_mut().ui(ctx);

        match action {
            AppAction::AddScreen(screen) => self.screen_stack.push(screen),
            AppAction::None => {}
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
            AppAction::SetMainScreen(root_screen_type) => {
                self.selected_main_screen = root_screen_type;
                self.active_root_screen_mut().refresh_on_arrival();
                self.current_app_context()
                    .update_settings(root_screen_type)
                    .ok();
            }
            AppAction::SwitchNetwork(network) => self.change_network(network),
        }
    }
}
