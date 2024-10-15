use crate::app::AppAction;
use crate::context::AppContext;
use crate::platform::core::{CoreItem, CoreTask};
use crate::platform::{BackendTask, BackendTaskSuccessResult};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{RootScreenType, ScreenLike};
use dash_sdk::dashcore_rpc::RpcApi;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use eframe::egui::{self, Color32, Context, Ui};
use std::env;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::runtime::Runtime;

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

    pub fn current_app_context(&self) -> &Arc<AppContext> {
        match self.current_network {
            Network::Dash => &self.mainnet_app_context,
            Network::Testnet if self.testnet_app_context.is_some() => {
                self.testnet_app_context.as_ref().unwrap()
            }
            _ => &self.mainnet_app_context,
        }
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
    fn start_dash_qt(&self, network: Network) -> std::io::Result<()> {
        // Determine the path based on the operating system
        let dash_qt_path = if cfg!(target_os = "macos") {
            "/Applications/Dash-Qt.app/Contents/MacOS/Dash-Qt"
        } else if cfg!(target_os = "windows") {
            "C:\\Program Files\\Dash\\dash-qt.exe" // Replace with the correct path for Windows
        } else {
            "/usr/local/bin/dash-qt" // Linux path, update accordingly
        };

        // Determine the config file based on the network
        let config_file = match network {
            Network::Dash => "dash_core_configs/mainnet.conf",
            Network::Testnet => "dash_core_configs/testnet.conf",
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Unsupported network",
                ))
            }
        };

        // Construct the full path for the config file
        let current_dir = env::current_dir()?;
        let config_path = current_dir.join(config_file);

        Command::new(dash_qt_path)
            .arg(format!("-conf={}", config_path.display()))
            .spawn()?; // Spawn the Dash QT process

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
