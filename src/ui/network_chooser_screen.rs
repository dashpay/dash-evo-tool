use crate::app::AppAction;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{RootScreenType, ScreenLike};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use eframe::egui::{self, Color32, Context, Ui};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct NetworkChooserScreen {
    pub mainnet_app_context: Arc<AppContext>,
    pub testnet_app_context: Option<Arc<AppContext>>,
    pub devnet_app_context: Option<Arc<AppContext>>,
    pub local_app_context: Option<Arc<AppContext>>,
    pub current_network: Network,
    pub mainnet_core_status_online: bool,
    pub testnet_core_status_online: bool,
    pub devnet_core_status_online: bool,
    pub local_core_status_online: bool,
    pub recheck_time: Option<TimestampMillis>,
    custom_dash_qt_path: Option<String>,
    custom_dash_qt_error_message: Option<String>,
    overwrite_dash_conf: bool,
}

impl NetworkChooserScreen {
    pub fn new(
        mainnet_app_context: &Arc<AppContext>,
        testnet_app_context: Option<&Arc<AppContext>>,
        devnet_app_context: Option<&Arc<AppContext>>,
        local_app_context: Option<&Arc<AppContext>>,
        current_network: Network,
        custom_dash_qt_path: Option<String>,
        overwrite_dash_conf: bool,
    ) -> Self {
        Self {
            mainnet_app_context: mainnet_app_context.clone(),
            testnet_app_context: testnet_app_context.cloned(),
            devnet_app_context: devnet_app_context.cloned(),
            local_app_context: local_app_context.cloned(),
            current_network,
            mainnet_core_status_online: false,
            testnet_core_status_online: false,
            devnet_core_status_online: false,
            local_core_status_online: false,
            recheck_time: None,
            custom_dash_qt_path,
            custom_dash_qt_error_message: None,
            overwrite_dash_conf,
        }
    }

    pub fn context_for_network(&self, network: Network) -> &Arc<AppContext> {
        match network {
            Network::Dash => &self.mainnet_app_context,
            Network::Testnet if self.testnet_app_context.is_some() => {
                self.testnet_app_context.as_ref().unwrap()
            }
            Network::Devnet if self.devnet_app_context.is_some() => {
                self.devnet_app_context.as_ref().unwrap()
            }
            Network::Regtest if self.local_app_context.is_some() => {
                self.local_app_context.as_ref().unwrap()
            }
            _ => &self.mainnet_app_context,
        }
    }

    pub fn current_app_context(&self) -> &Arc<AppContext> {
        self.context_for_network(self.current_network)
    }

    /// Render the network selection table
    fn render_network_table(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;

        egui::Grid::new("network_grid")
            .striped(true)
            .spacing([20.0, 10.0])
            .show(ui, |ui| {
                // Header row
                ui.label(egui::RichText::new("Network").strong().underline());
                ui.label(egui::RichText::new("Status").strong().underline());
                // ui.label(egui::RichText::new("Wallet Count").strong().underline());
                // ui.label(egui::RichText::new("Add New Wallet").strong().underline());
                ui.label(egui::RichText::new("Select").strong().underline());
                ui.label(egui::RichText::new("Start").strong().underline());
                ui.end_row();

                // Render Mainnet Row
                app_action |= self.render_network_row(ui, Network::Dash, "Mainnet");

                // Render Testnet Row
                app_action |= self.render_network_row(ui, Network::Testnet, "Testnet");

                // Render Devnet Row
                app_action |= self.render_network_row(ui, Network::Devnet, "Devnet");

                // Render Local Row
                app_action |= self.render_network_row(ui, Network::Regtest, "Local");
            });

        ui.add_space(10.0);

        egui::CollapsingHeader::new("Advanced settings")
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("advanced_settings")
                    .show(ui, |ui| {
                        ui.label("Custom Dash-QT path:");

                        if ui.button("Select file").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_file() {
                                {
                                    let file_name = path.file_name().and_then(|f| f.to_str());
                                    if let Some(file_name) = file_name {
                                        self.custom_dash_qt_path = None;
                                        self.custom_dash_qt_error_message = None;
                                        let required_file_name = if cfg!(target_os = "windows") {
                                            String::from("dash-qt.exe")
                                        } else if cfg!(target_os = "macos") {
                                            String::from("dash-qt")
                                        } else { //linux
                                            String::from("dash-qt")
                                        };
                                        if file_name.ends_with(required_file_name.as_str()) {
                                            self.custom_dash_qt_path = Some(path.display().to_string());
                                            self.custom_dash_qt_error_message = None;
                                            self.current_app_context().db.update_dash_core_execution_settings(self.custom_dash_qt_path.clone(), self.overwrite_dash_conf).expect("Expected to save db settings");
                                        } else {
                                            self.custom_dash_qt_error_message = Some(format!("Invalid file: Please select a valid '{}'.", required_file_name));
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(ref file) = self.custom_dash_qt_path {
                            ui.label(format!("Selected: {}", file));
                        } else if let Some(ref error) = self.custom_dash_qt_error_message {
                            ui.colored_label(egui::Color32::RED, error);
                        } else {
                            ui.label("");
                        }
                        if self.custom_dash_qt_path.is_some() || self.custom_dash_qt_error_message.is_some() {
                            if ui.button("clear").clicked() {
                                self.custom_dash_qt_path = None;
                                self.custom_dash_qt_error_message = None;
                            }
                        }
                        ui.end_row();

                        if ui.checkbox(&mut self.overwrite_dash_conf, "Overwrite dash.conf").clicked() {
                            self.current_app_context().db.update_dash_core_execution_settings(self.custom_dash_qt_path.clone(), self.overwrite_dash_conf).expect("Expected to save db settings");
                        }
                        if !self.overwrite_dash_conf {
                            ui.end_row();
                            if self.current_network == Network::Dash {
                                ui.colored_label(egui::Color32::ORANGE, "The following lines must be included in the custom Mainnet dash.conf:");
                                ui.end_row();
                                ui.label("zmqpubrawtxlocksig=tcp://0.0.0.0:23708");
                                ui.end_row();
                                ui.label("zmqpubrawchainlock=tcp://0.0.0.0:23708");
                            } else if self.current_network == Network::Testnet {
                                ui.colored_label(egui::Color32::ORANGE, "The following lines must be included in the custom Testnet dash.conf:");
                                ui.end_row();
                                ui.label("zmqpubrawtxlocksig=tcp://0.0.0.0:23709");
                                ui.end_row();
                                ui.label("zmqpubrawchainlock=tcp://0.0.0.0:23709");
                            } else if self.current_network == Network::Devnet {
                                ui.colored_label(egui::Color32::ORANGE, "The following lines must be included in the custom Devnet dash.conf:");
                                ui.end_row();
                                ui.label("zmqpubrawtxlocksig=tcp://0.0.0.0:23710");
                                ui.end_row();
                                ui.label("zmqpubrawchainlock=tcp://0.0.0.0:23710");
                            } else if self.current_network == Network::Regtest {
                                ui.colored_label(egui::Color32::ORANGE, "The following lines must be included in the custom Regtest dash.conf:");
                                ui.end_row();
                                ui.label("zmqpubrawtxlocksig=tcp://0.0.0.0:20302");
                            }
                        }
                    });
            });
        app_action
    }

    /// Render a single row for the network table
    fn render_network_row(&mut self, ui: &mut Ui, network: Network, name: &str) -> AppAction {
        let mut app_action = AppAction::None;
        ui.label(name);

        // Check network status
        let is_working = self.check_network_status(network);
        let status_color = if is_working {
            Color32::DARK_GREEN // Green if working
        } else {
            Color32::RED // Red if not working
        };

        // Display status indicator
        ui.colored_label(status_color, if is_working { "Online" } else { "Offline" });

        if network == Network::Testnet && self.testnet_app_context.is_none() {
            ui.label("(No configs for testnet loaded)");
            ui.end_row();
            return AppAction::None;
        }
        if network == Network::Devnet && self.devnet_app_context.is_none() {
            ui.label("(No configs for devnet loaded)");
            ui.end_row();
            return AppAction::None;
        }
        if network == Network::Regtest && self.local_app_context.is_none() {
            ui.label("(No configs for local loaded)");
            ui.end_row();
            return AppAction::None;
        }

        // // Display wallet count
        // let wallet_count = format!(
        //     "{}",
        //     self.context_for_network(network)
        //         .wallets
        //         .read()
        //         .unwrap()
        //         .len()
        // );
        // ui.label(wallet_count);

        // // Add a button to add a wallet
        // if ui.button("+").clicked() {
        //     let context = if network == Network::Dash || self.testnet_app_context.is_none() {
        //         &self.mainnet_app_context
        //     } else {
        //         &self.testnet_app_context.as_ref().unwrap()
        //     };
        //     app_action =
        //         AppAction::AddScreen(Screen::AddNewWalletScreen(AddNewWalletScreen::new(context)));
        // }

        // Network selection
        let mut is_selected = self.current_network == network;
        if ui.checkbox(&mut is_selected, "").clicked() && is_selected {
            self.current_network = network;
            app_action = AppAction::SwitchNetwork(network);
            // Recheck in 1 second
            self.recheck_time = Some(
                (SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    + Duration::from_secs(1))
                .as_millis() as u64,
            );
        }

        // Add a button to start the network
        if !(network == Network::Regtest) {
            if ui.button("Start").clicked() {
                app_action = AppAction::BackendTask(BackendTask::CoreTask(CoreTask::StartDashQT(
                    network,
                    self.custom_dash_qt_path.clone(),
                    self.overwrite_dash_conf,
                )));
            }
        }

        ui.end_row();
        app_action
    }

    /// Check if the network is working
    fn check_network_status(&self, network: Network) -> bool {
        match network {
            Network::Dash => self.mainnet_core_status_online,
            Network::Testnet => self.testnet_core_status_online,
            Network::Devnet => self.devnet_core_status_online,
            Network::Regtest => self.local_core_status_online,
            _ => false,
        }
    }
}

impl ScreenLike for NetworkChooserScreen {
    fn display_message(&mut self, message: &str, _message_type: super::MessageType) {
        if message.contains("Failed to get best chain lock for mainnet, testnet, devnet, and local")
        {
            self.mainnet_core_status_online = false;
            self.testnet_core_status_online = false;
            self.devnet_core_status_online = false;
            self.local_core_status_online = false;
        }
    }

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        match backend_task_success_result {
            BackendTaskSuccessResult::CoreItem(CoreItem::ChainLocks(
                mainnet_chainlock,
                testnet_chainlock,
                devnet_chainlock,
                local_chainlock,
            )) => {
                match mainnet_chainlock {
                    Some(_) => self.mainnet_core_status_online = true,
                    None => self.mainnet_core_status_online = false,
                }
                match testnet_chainlock {
                    Some(_) => self.testnet_core_status_online = true,
                    None => self.testnet_core_status_online = false,
                }
                match devnet_chainlock {
                    Some(_) => self.devnet_core_status_online = true,
                    None => self.devnet_core_status_online = false,
                }
                match local_chainlock {
                    Some(_) => self.local_core_status_online = true,
                    None => self.local_core_status_online = false,
                }
            }
            _ => {}
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            self.current_app_context(),
            vec![("Networks", AppAction::None)],
            vec![],
        );

        action |= add_left_panel(
            ctx,
            self.current_app_context(),
            RootScreenType::RootScreenNetworkChooser,
        );

        egui::CentralPanel::default().show(ctx, |ui| {
            action |= self.render_network_table(ui);
        });

        // Recheck both network status every 3 seconds
        let recheck_time = Duration::from_secs(3);
        if action == AppAction::None {
            let current_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards");
            if let Some(time) = self.recheck_time {
                if current_time.as_millis() as u64 >= time {
                    action =
                        AppAction::BackendTask(BackendTask::CoreTask(CoreTask::GetBestChainLocks));
                    self.recheck_time = Some((current_time + recheck_time).as_millis() as u64);
                }
            } else {
                self.recheck_time = Some((current_time + recheck_time).as_millis() as u64);
            }
        }

        action
    }
}
