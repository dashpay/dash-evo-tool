use crate::context::AppContext;
use crate::logging::initialize_logger;
use crate::platform::BackendTask;
use crate::ui::dpns_contested_names_screen::DPNSContestedNamesScreen;
use crate::ui::identities_screen::IdentitiesScreen;
use crate::ui::network_chooser_screen::NetworkChooserScreen;
use crate::ui::transition_visualizer_screen::TransitionVisualizerScreen;
use crate::ui::{RootScreenType, Screen, ScreenLike, ScreenType};
use dash_sdk::dpp::dashcore::Network;
use eframe::{egui, App};
use std::collections::BTreeMap;
use std::ops::BitOrAssign;
use std::sync::Arc;
use std::vec;

#[derive(Debug)]
pub enum TaskResult {
    Success(String), // Represents a successful task with a message
    Error(String),   // Represents a task failure with an error message
}

impl From<Result<(), String>> for TaskResult {
    fn from(value: Result<(), String>) -> Self {
        match value {
            Ok(_) => TaskResult::Success("Success".to_string()),
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
    // pub task_result_sender: mpsc::Sender<TaskResult>, // Channel sender for sending task results
    // pub task_result_receiver: mpsc::Receiver<TaskResult>, // Channel receiver for receiving task results
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
            DesiredAppAction::SwitchNetwork(network) => AppAction::SwitchNetwork(network.clone()),
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
        let mainnet_app_context =
            AppContext::new(Network::Dash).expect("expected Dash config for mainnet");
        let testnet_app_context = AppContext::new(Network::Testnet);

        let identities_screen = IdentitiesScreen::new(&mainnet_app_context);
        let dpns_contested_names_screen = DPNSContestedNamesScreen::new(&mainnet_app_context);
        let transition_visualizer_screen = TransitionVisualizerScreen::new(&mainnet_app_context);
        let network_chooser_screen = NetworkChooserScreen::new(&mainnet_app_context);

        // // Create a channel with a buffer size of 32 (adjust as needed)
        // let (task_result_sender, task_result_receiver) = mpsc::channel(256);

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
                    RootScreenType::RootScreenTransitionVisualizerScreen,
                    Screen::TransitionVisualizerScreen(transition_visualizer_screen),
                ),
                (
                    RootScreenType::RootScreenNetworkChooser,
                    Screen::NetworkChooserScreen(network_chooser_screen),
                ),
            ]
            .into(),
            selected_main_screen: RootScreenType::RootScreenIdentities,
            screen_stack: vec![],
            chosen_network: Network::Dash,
            mainnet_app_context,
            testnet_app_context,
            // task_result_sender,
            // task_result_receiver,
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
        // let sender = self.task_result_sender.clone();
        let app_context = self.current_app_context().clone();

        tokio::spawn(async move {
            let result = app_context.run_backend_task(task).await;

            // Send the result back to the main thread
            // if let Err(e) = sender.send(result.into()).await {
            //     eprintln!("Failed to send task result: {}", e);
            // }
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

impl App for AppState {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll the receiver for any new task results
        // while let Ok(task_result) = self.task_result_receiver.try_recv() {
        //     // Handle the result on the main thread
        //     match task_result {
        //         TaskResult::Success(message) => {
        //             self.visible_screen_mut()
        //                 .display_message(message, MessageType::Info);
        //         }
        //         TaskResult::Error(message) => {
        //             self.visible_screen_mut()
        //                 .display_message(message, MessageType::Error);
        //         }
        //     }
        // }

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
                    self.active_root_screen_mut().refresh();
                }
            }
            AppAction::GoToMainScreen => {
                self.screen_stack = vec![];
                self.active_root_screen_mut().refresh();
            }
            AppAction::BackendTask(task) => {
                self.handle_backend_task(task);
            }
            AppAction::SetMainScreen(root_screen_type) => {
                self.selected_main_screen = root_screen_type;
                self.active_root_screen_mut().refresh();
            }
            AppAction::SwitchNetwork(network) => self.change_network(network),
        }
    }
}
