use crate::app_dir::{
    app_user_data_file_path, copy_env_file_if_not_exists,
    create_app_user_data_directory_if_not_exists,
};
use crate::backend_task::contested_names::ContestedResourceTask;
use crate::backend_task::core::CoreItem;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::components::core_zmq_listener::{CoreZMQListener, ZMQMessage};
use crate::context::AppContext;
use crate::database::Database;
use crate::logging::initialize_logger;
use crate::ui::contracts_documents::document_query_screen::DocumentQueryScreen;
use crate::ui::dpns::dpns_contested_names_screen::{
    DPNSScreen, DPNSSubscreen, ScheduledVoteCastingStatus,
};
use crate::ui::identities::identities_screen::IdentitiesScreen;
use crate::ui::network_chooser_screen::NetworkChooserScreen;
use crate::ui::tools::proof_log_screen::ProofLogScreen;
use crate::ui::tools::proof_visualizer_screen::ProofVisualizerScreen;
use crate::ui::tools::transition_visualizer_screen::TransitionVisualizerScreen;
use crate::ui::wallets::wallets_screen::WalletsBalancesScreen;
use crate::ui::{MessageType, RootScreenType, Screen, ScreenLike, ScreenType};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use derive_more::From;
use eframe::{egui, App};
use std::collections::BTreeMap;
use std::ops::BitOrAssign;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime};
use std::vec;
use tokio::sync::mpsc as tokiompsc;

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
    pub mainnet_core_zmq_listener: CoreZMQListener,
    pub testnet_core_zmq_listener: CoreZMQListener,
    pub core_message_receiver: mpsc::Receiver<(ZMQMessage, Network)>,
    pub task_result_sender: tokiompsc::Sender<TaskResult>, // Channel sender for sending task results
    pub task_result_receiver: tokiompsc::Receiver<TaskResult>, // Channel receiver for receiving task results
    last_repaint: Instant, // Track the last time we requested a repaint
    last_scheduled_vote_check: Instant, // Last time we checked if there are scheduled masternode votes to cast
}

#[derive(Debug, Clone, PartialEq)]
pub enum DesiredAppAction {
    None,
    Refresh,
    PopScreen,
    GoToMainScreen,
    SwitchNetwork(Network),
    AddScreenType(ScreenType),
    BackendTask(BackendTask),
    BackendTasks(Vec<BackendTask>, BackendTasksExecutionMode),
    Custom(String),
}

impl DesiredAppAction {
    pub fn create_action(&self, app_context: &Arc<AppContext>) -> AppAction {
        match self {
            DesiredAppAction::None => AppAction::None,
            DesiredAppAction::Refresh => AppAction::Refresh,
            DesiredAppAction::Custom(message) => AppAction::Custom(message.clone()),
            DesiredAppAction::PopScreen => AppAction::PopScreen,
            DesiredAppAction::GoToMainScreen => AppAction::GoToMainScreen,
            DesiredAppAction::AddScreenType(screen_type) => {
                AppAction::AddScreen(screen_type.create_screen(app_context))
            }
            DesiredAppAction::BackendTask(backend_task) => {
                AppAction::BackendTask(backend_task.clone())
            }
            DesiredAppAction::BackendTasks(tasks, mode) => {
                AppAction::BackendTasks(tasks.clone(), mode.clone())
            }
            DesiredAppAction::SwitchNetwork(network) => AppAction::SwitchNetwork(*network),
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
    AddScreen(Screen),
    PopThenAddScreenToMainScreen(RootScreenType, Screen),
    BackendTask(BackendTask),
    BackendTasks(Vec<BackendTask>, BackendTasksExecutionMode),
    Custom(String),
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
        create_app_user_data_directory_if_not_exists()
            .expect("Failed to create app user_data directory");
        copy_env_file_if_not_exists();
        initialize_logger();
        let db_file_path = app_user_data_file_path("data.db").expect("should create db file path");
        let db = Arc::new(Database::new(&db_file_path).unwrap());
        db.initialize(&db_file_path).unwrap();

        let settings = db.get_settings().expect("expected to get settings");

        let password_info = settings
            .clone()
            .map(|(_, _, password_info, _, _)| password_info)
            .flatten();

        let mainnet_app_context =
            match AppContext::new(Network::Dash, db.clone(), password_info.clone()) {
                Some(context) => context,
                None => {
                    eprintln!(
                        "Error: Failed to create the AppContext. Expected Dash config for mainnet."
                    );
                    std::process::exit(1);
                }
            };
        let testnet_app_context = AppContext::new(Network::Testnet, db.clone(), password_info);

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
        let mut proof_log_screen = ProofLogScreen::new(&mainnet_app_context);
        let mut document_query_screen = DocumentQueryScreen::new(&mainnet_app_context);

        let (custom_dash_qt_path, overwrite_dash_conf) = match settings.clone() {
            Some((.., db_custom_dash_qt_path, db_overwrite_dash_qt)) => {
                (db_custom_dash_qt_path, db_overwrite_dash_qt)
            }
            _ => {
                // Default values: Use system default path and overwrite conf
                (None, true)
            }
        };

        let mut network_chooser_screen = NetworkChooserScreen::new(
            &mainnet_app_context,
            testnet_app_context.as_ref(),
            Network::Dash,
            custom_dash_qt_path,
            overwrite_dash_conf,
        );

        let mut wallets_balances_screen = WalletsBalancesScreen::new(&mainnet_app_context);

        let mut selected_main_screen = RootScreenType::RootScreenIdentities;

        let mut chosen_network = Network::Dash;

        if let Some((network, screen_type, _, _, _)) = settings {
            selected_main_screen = screen_type;
            chosen_network = network;
            if chosen_network == Network::Testnet && testnet_app_context.is_some() {
                let testnet_app_context = testnet_app_context.as_ref().unwrap();
                identities_screen = IdentitiesScreen::new(testnet_app_context);
                dpns_active_contests_screen =
                    DPNSScreen::new(&testnet_app_context, DPNSSubscreen::Active);
                dpns_past_contests_screen =
                    DPNSScreen::new(&testnet_app_context, DPNSSubscreen::Past);
                dpns_my_usernames_screen =
                    DPNSScreen::new(&testnet_app_context, DPNSSubscreen::Owned);
                dpns_scheduled_votes_screen =
                    DPNSScreen::new(&testnet_app_context, DPNSSubscreen::ScheduledVotes);
                transition_visualizer_screen = TransitionVisualizerScreen::new(testnet_app_context);
                proof_visualizer_screen = ProofVisualizerScreen::new(testnet_app_context);
                document_query_screen = DocumentQueryScreen::new(testnet_app_context);
                wallets_balances_screen = WalletsBalancesScreen::new(testnet_app_context);
                proof_log_screen = ProofLogScreen::new(testnet_app_context);
            }
            network_chooser_screen.current_network = chosen_network;
        }

        // // Create a channel with a buffer size of 32 (adjust as needed)
        let (task_result_sender, task_result_receiver) = tokiompsc::channel(256);

        // Initialize the last repaint time to the current instant
        let last_repaint = Instant::now();

        // Create a channel for communication with the InstantSendListener
        let (core_message_sender, core_message_receiver) = mpsc::channel();

        let mainnet_core_zmq_listener = CoreZMQListener::spawn_listener(
            Network::Dash,
            "tcp://127.0.0.1:23708",
            core_message_sender.clone(), // Clone the sender for each listener
            Some(mainnet_app_context.sx_zmq_status.clone()),
        )
        .expect("Failed to create mainnet InstantSend listener");

        let tx_zmq_status_option = match testnet_app_context {
            Some(ref context) => Some(context.sx_zmq_status.clone()),
            None => None,
        };

        let testnet_core_zmq_listener = CoreZMQListener::spawn_listener(
            Network::Testnet,
            "tcp://127.0.0.1:23709",
            core_message_sender, // Use the original sender or create a new one if needed
            tx_zmq_status_option,
        )
        .expect("Failed to create testnet InstantSend listener");

        Self {
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
                    RootScreenType::RootScreenToolsProofLogScreen,
                    Screen::ProofLogScreen(proof_log_screen),
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
            mainnet_core_zmq_listener,
            testnet_core_zmq_listener,
            core_message_receiver,
            task_result_sender,
            task_result_receiver,
            last_repaint,
            last_scheduled_vote_check: Instant::now(),
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

    /// Handle the backend tasks and send the results through the channel
    pub fn handle_backend_tasks(&self, tasks: Vec<BackendTask>, mode: BackendTasksExecutionMode) {
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
                    eprintln!("Failed to send task result: {}", e);
                }
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
        if let Ok(event) = self.current_app_context().rx_zmq_status.try_recv() {
            if let Ok(mut status) = self.current_app_context().zmq_connection_status.lock() {
                *status = event;
            }
        }

        // Poll the receiver for any new task results
        while let Ok(task_result) = self.task_result_receiver.try_recv() {
            // Handle the result on the main thread
            match task_result {
                TaskResult::Success(message) => match message {
                    BackendTaskSuccessResult::None => {
                        self.visible_screen_mut().pop_on_success();
                    }
                    BackendTaskSuccessResult::Refresh => {
                        self.visible_screen_mut().refresh();
                    }
                    BackendTaskSuccessResult::Message(message) => {
                        self.visible_screen_mut()
                            .display_message(&message, MessageType::Success);
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
                    BackendTaskSuccessResult::DPNSVoteResults(_) => {
                        self.visible_screen_mut().display_task_result(message);
                    }
                    BackendTaskSuccessResult::CastScheduledVote(vote) => {
                        let _ = self
                            .current_app_context()
                            .mark_vote_executed(vote.voter_id.as_slice(), vote.contested_name);
                        self.visible_screen_mut().display_message(
                            "Successfully cast scheduled vote",
                            MessageType::Success,
                        );
                        self.visible_screen_mut().refresh();
                    }
                    BackendTaskSuccessResult::RegisteredIdentity(_) => {
                        self.visible_screen_mut().display_task_result(message);
                    }
                    BackendTaskSuccessResult::ToppedUpIdentity(_) => {
                        self.visible_screen_mut().display_task_result(message);
                    }
                    BackendTaskSuccessResult::FetchedContract(_) => {
                        self.visible_screen_mut().display_task_result(message);
                    }
                    BackendTaskSuccessResult::FetchedContracts(_) => {
                        self.visible_screen_mut().display_task_result(message);
                    }
                    BackendTaskSuccessResult::PageDocuments(_, _) => {
                        self.visible_screen_mut().display_task_result(message);
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

        // **Poll the instant_send_receiver for any new InstantSend messages**
        while let Ok((message, network)) = self.core_message_receiver.try_recv() {
            let app_context = match network {
                Network::Dash => &self.mainnet_app_context,
                Network::Testnet => {
                    if let Some(context) = self.testnet_app_context.as_ref() {
                        context
                    } else {
                        // Handle the case when testnet_app_context is None
                        eprintln!("No testnet app context available for Testnet");
                        continue; // Skip this iteration or handle as needed
                    }
                }
                _ => continue,
            };
            match message {
                ZMQMessage::ISLockedTransaction(tx, is_lock) => {
                    // Store the asset lock transaction in the database
                    match app_context.received_transaction_finality(&tx, Some(is_lock), None) {
                        Ok(utxos) => {
                            let core_item =
                                CoreItem::ReceivedAvailableUTXOTransaction(tx.clone(), utxos);
                            self.visible_screen_mut()
                                .display_task_result(BackendTaskSuccessResult::CoreItem(core_item));
                        }
                        Err(e) => {
                            eprintln!("Failed to store asset lock: {}", e);
                        }
                    }
                }
                ZMQMessage::ChainLockedLockedTransaction(tx, height) => {
                    if let Err(e) =
                        app_context.received_transaction_finality(&tx, None, Some(height))
                    {
                        eprintln!("Failed to store asset lock: {}", e);
                    }
                }
                ZMQMessage::ChainLockedBlock(_) => {}
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
                    eprintln!("Error querying scheduled votes: {}", e);
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
                        && !(v.unix_timestamp + 120000 < current_time) // Don't cast votes more than 2 minutes behind current time
                })
                .collect();

            // For each due vote, construct a BackendTask and handle it
            if !due_votes.is_empty() {
                let local_identities = match app_context.load_local_voting_identities() {
                    Ok(identities) => identities,
                    Err(e) => {
                        eprintln!("Error querying local voting identities: {}", e);
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
                            screen
                                .scheduled_votes
                                .lock()
                                .unwrap()
                                .iter_mut()
                                .find(|(v, _)| v == &vote)
                                .map(|(_, s)| *s = ScheduledVoteCastingStatus::InProgress);
                        }
                        let task = BackendTask::ContestedResourceTask(
                            ContestedResourceTask::CastScheduledVote(vote, voter.clone()),
                        );
                        self.handle_backend_task(task);
                    } else {
                        eprintln!("Voter not found for scheduled vote: {:?}", vote);
                    }
                }
            }
        }

        // Use a timer to repaint the UI every 0.05 seconds
        ctx.request_repaint_after(std::time::Duration::from_millis(50));

        let action = self.visible_screen_mut().ui(ctx);

        match action {
            AppAction::AddScreen(screen) => self.screen_stack.push(screen),
            AppAction::None => {}
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
        }
    }
}
