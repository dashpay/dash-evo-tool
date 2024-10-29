use crate::app::AppAction;
use crate::context::AppContext;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::wallet::add_new_wallet_screen::AddNewWalletScreen;
use crate::ui::{RootScreenType, Screen, ScreenLike};
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use eframe::egui::{self, Color32, Context, Ui};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{env, io};

pub struct NetworkChooserScreen {
    pub mainnet_app_context: Arc<AppContext>,
    pub testnet_app_context: Option<Arc<AppContext>>,
    pub current_network: Network,
    pub mainnet_core_status_online: bool,
    pub testnet_core_status_online: bool,
    status_checked: bool,
    pub recheck_time: Option<TimestampMillis>,
}

impl NetworkChooserScreen {
    pub fn new(
        mainnet_app_context: &Arc<AppContext>,
        testnet_app_context: Option<&Arc<AppContext>>,
        current_network: Network,
    ) -> Self {
        Self {
            mainnet_app_context: mainnet_app_context.clone(),
            testnet_app_context: testnet_app_context.cloned(),
            current_network,
            mainnet_core_status_online: false,
            testnet_core_status_online: false,
            status_checked: false,
            recheck_time: None,
        }
    }

    pub fn context_for_network(&self, network: Network) -> &Arc<AppContext> {
        match network {
            Network::Dash => &self.mainnet_app_context,
            Network::Testnet if self.testnet_app_context.is_some() => {
                self.testnet_app_context.as_ref().unwrap()
            }
            _ => &self.mainnet_app_context,
        }
    }

    pub fn current_app_context(&self) -> &Arc<AppContext> {
        self.context_for_network(self.current_network)
    }

    /// Function to check the status of Dash Core for a given network
    async fn check_core_status(app_context: &Arc<AppContext>) -> bool {
        app_context.core_client.get_best_chain_lock().is_ok()
    }

    /// Render the network selection table
    fn render_network_table(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        ui.heading("Choose Network");

        egui::Grid::new("network_grid")
            .striped(true)
            .show(ui, |ui| {
                // Header row
                ui.label("Network");
                ui.label("Status");
                ui.label("Wallet Count");
                ui.label("Add New Wallet");
                ui.label("Select");
                ui.label("Start");
                ui.end_row();

                // Render Mainnet
                app_action |= self.render_network_row(ui, Network::Dash, "Mainnet");

                // Render Testnet
                app_action |= self.render_network_row(ui, Network::Testnet, "Testnet");
            });
        app_action
    }

    /// Render a single row for the network table
    fn render_network_row(&mut self, ui: &mut Ui, network: Network, name: &str) -> AppAction {
        let mut app_action = AppAction::None;
        ui.label(name);

        // Simulate checking network status
        let is_working = self.check_network_status(network);
        let status_color = if is_working {
            Color32::from_rgb(0, 255, 0) // Green if working
        } else {
            if let Some(recheck_time) = self.recheck_time {
                let current_time = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards");
                let current_time_ms = current_time.as_millis() as u64;
                if current_time_ms >= recheck_time {
                    app_action |=
                        AppAction::BackendTask(BackendTask::CoreTask(CoreTask::GetBestChainLock));
                    self.recheck_time =
                        Some((current_time + Duration::from_secs(5)).as_millis() as u64);
                }
            }
            Color32::from_rgb(255, 0, 0) // Red if not working
        };

        // Display status indicator
        ui.colored_label(status_color, if is_working { "Online" } else { "Offline" });

        if network == Network::Testnet && self.testnet_app_context.is_none() {
            ui.label("(No configs for testnet loaded)");
            return AppAction::None;
        }

        // Display wallet count
        let wallet_count = format!(
            "{}",
            self.context_for_network(network)
                .wallets
                .read()
                .unwrap()
                .len()
        );
        ui.label(wallet_count);

        // Add a button to start the network
        if ui.button("+").clicked() {
            let context = if network == Network::Dash || self.testnet_app_context.is_none() {
                &self.mainnet_app_context
            } else {
                &self.testnet_app_context.as_ref().unwrap()
            };
            app_action |=
                AppAction::AddScreen(Screen::AddNewWalletScreen(AddNewWalletScreen::new(context)));
        }

        // Network selection
        let mut is_selected = self.current_network == network;
        if ui.checkbox(&mut is_selected, "Select").clicked() && is_selected {
            self.current_network = network;
            app_action = AppAction::SwitchNetwork(network);
        }

        // Add a button to start the network
        if ui.button("Start").clicked() {
            match self.start_dash_qt(network) {
                Ok(_) => println!("Dash QT started successfully!"),
                Err(e) => eprintln!("Failed to start Dash QT: {}", e),
            }
            // in 5 seconds
            self.recheck_time = Some(
                (SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    + Duration::from_secs(5))
                .as_millis() as u64,
            );
        }

        ui.end_row();
        app_action
    }

    /// Function to start Dash QT based on the selected network
    fn start_dash_qt(&self, network: Network) -> io::Result<()> {
        // Determine the path to Dash-Qt based on the operating system
        let dash_qt_path: PathBuf = if cfg!(target_os = "macos") {
            PathBuf::from("/Applications/Dash-Qt.app/Contents/MacOS/Dash-Qt")
        } else if cfg!(target_os = "windows") {
            // Retrieve the PROGRAMFILES environment variable and construct the path
            let program_files = env::var("PROGRAMFILES")
                .map(PathBuf::from)
                .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;

            program_files.join("DashCore\\dash-qt.exe")
        } else {
            PathBuf::from("/usr/local/bin/dash-qt") // Linux path
        };

        // Ensure the Dash-Qt binary path exists
        if !dash_qt_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("Dash-Qt not found at: {:?}", dash_qt_path),
            ));
        }

        // Determine the config file based on the network
        let config_file: &str = match network {
            Network::Dash => "dash_core_configs/mainnet.conf",
            Network::Testnet => "dash_core_configs/testnet.conf",
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Unsupported network",
                ))
            }
        };

        // Construct the full path to the config file
        let current_dir = env::current_dir()?;
        let config_path = current_dir.join(config_file);

        // Start Dash-Qt with the appropriate config
        Command::new(&dash_qt_path)
            .arg(format!("-conf={}", config_path.display()))
            .stdout(Stdio::null()) // Optional: Suppress output
            .stderr(Stdio::null())
            .spawn()?; // Spawn the Dash-Qt process

        Ok(())
    }

    /// Simulate a function to check if the network is working
    fn check_network_status(&self, network: Network) -> bool {
        match network {
            Network::Dash => self.mainnet_core_status_online,
            Network::Testnet => self.testnet_core_status_online,
            _ => false,
        }
    }
}

impl ScreenLike for NetworkChooserScreen {
    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::CoreItem(CoreItem::ChainLock(_, network)) =
            backend_task_success_result
        {
            match network {
                Network::Dash => {
                    self.mainnet_core_status_online = true;
                }
                Network::Testnet => {
                    self.testnet_core_status_online = true;
                }
                _ => {}
            }
        }
    }
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            self.current_app_context(),
            vec![("Dash Evo Tool", AppAction::None)],
            vec![],
        );

        if !self.status_checked {
            self.status_checked = true;
            action |= AppAction::BackendTask(BackendTask::CoreTask(CoreTask::GetBestChainLock));
        }

        action |= add_left_panel(
            ctx,
            self.current_app_context(),
            RootScreenType::RootScreenNetworkChooser,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            action |= self.render_network_table(ui);
        });

        action
    }
}
