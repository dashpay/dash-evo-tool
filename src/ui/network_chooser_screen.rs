use crate::app::AppAction;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::system_task::SystemTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::config::Config;
use crate::context::AppContext;
use crate::spv::{CoreBackendMode, SpvStatus, SpvStatusSnapshot};
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::{StyledCard, StyledCheckbox, island_central_panel};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::{DashColors, ThemeMode};
use crate::ui::{RootScreenType, ScreenLike};
use crate::utils::path::format_path_for_display;
use dash_sdk::dash_spv::types::{DetailedSyncProgress, SyncStage};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use eframe::egui::{self, Context, Ui};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct NetworkChooserScreen {
    pub mainnet_app_context: Arc<AppContext>,
    pub testnet_app_context: Option<Arc<AppContext>>,
    pub devnet_app_context: Option<Arc<AppContext>>,
    pub local_app_context: Option<Arc<AppContext>>,
    pub local_network_dashmate_password: String,
    pub current_network: Network,
    pub mainnet_core_status_online: bool,
    pub testnet_core_status_online: bool,
    pub devnet_core_status_online: bool,
    pub local_core_status_online: bool,
    pub recheck_time: Option<TimestampMillis>,
    custom_dash_qt_path: Option<PathBuf>,
    custom_dash_qt_error_message: Option<String>,
    overwrite_dash_conf: bool,
    developer_mode: bool,
    theme_preference: ThemeMode,
    should_reset_collapsing_states: bool,
    backend_modes: HashMap<Network, CoreBackendMode>,
}

impl NetworkChooserScreen {
    pub fn new(
        mainnet_app_context: &Arc<AppContext>,
        testnet_app_context: Option<&Arc<AppContext>>,
        devnet_app_context: Option<&Arc<AppContext>>,
        local_app_context: Option<&Arc<AppContext>>,
        current_network: Network,
        overwrite_dash_conf: bool,
    ) -> Self {
        let local_network_dashmate_password = if let Ok(config) = Config::load() {
            if let Some(local_config) = config.config_for_network(Network::Regtest) {
                local_config.core_rpc_password.clone()
            } else {
                "".to_string()
            }
        } else {
            "".to_string()
        };

        let current_context = match current_network {
            Network::Dash => mainnet_app_context,
            Network::Testnet => testnet_app_context.unwrap_or(mainnet_app_context),
            Network::Devnet => devnet_app_context.unwrap_or(mainnet_app_context),
            Network::Regtest => local_app_context.unwrap_or(mainnet_app_context),
            _ => mainnet_app_context,
        };
        let developer_mode = current_context.is_developer_mode();

        // Load settings including theme preference and dash_qt_path
        let settings = current_context
            .get_settings()
            .ok()
            .flatten()
            .unwrap_or_default();
        let theme_preference = settings.theme_mode;
        let custom_dash_qt_path = settings.dash_qt_path;

        let mut backend_modes = HashMap::new();
        backend_modes.insert(Network::Dash, mainnet_app_context.core_backend_mode());
        backend_modes.insert(
            Network::Testnet,
            testnet_app_context
                .map(|ctx| ctx.core_backend_mode())
                .unwrap_or_default(),
        );
        backend_modes.insert(
            Network::Devnet,
            devnet_app_context
                .map(|ctx| ctx.core_backend_mode())
                .unwrap_or_default(),
        );
        backend_modes.insert(
            Network::Regtest,
            local_app_context
                .map(|ctx| ctx.core_backend_mode())
                .unwrap_or_default(),
        );

        Self {
            mainnet_app_context: mainnet_app_context.clone(),
            testnet_app_context: testnet_app_context.cloned(),
            devnet_app_context: devnet_app_context.cloned(),
            local_app_context: local_app_context.cloned(),
            local_network_dashmate_password,
            current_network,
            mainnet_core_status_online: false,
            testnet_core_status_online: false,
            devnet_core_status_online: false,
            local_core_status_online: false,
            recheck_time: None,
            custom_dash_qt_path,
            custom_dash_qt_error_message: None,
            overwrite_dash_conf,
            developer_mode,
            theme_preference,
            should_reset_collapsing_states: true, // Start with collapsed state
            backend_modes,
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

    /// Save the current settings to the database
    ///
    /// TODO: doesn't save local network settings like password yet.
    fn save(&self) -> Result<(), String> {
        self.current_app_context()
            .update_dash_core_execution_settings(
                self.custom_dash_qt_path.clone(),
                self.overwrite_dash_conf,
            )
            .map_err(|e| e.to_string())
    }
    /// Render the network selection table
    fn render_network_table(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        egui::Grid::new("network_grid")
            .striped(false)
            .spacing([20.0, 10.0])
            .show(ui, |ui| {
                // Header row
                ui.label(
                    egui::RichText::new("Network")
                        .strong()
                        .underline()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.label(
                    egui::RichText::new("Status")
                        .strong()
                        .underline()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.label(
                    egui::RichText::new("Backend")
                        .strong()
                        .underline()
                        .color(DashColors::text_primary(dark_mode)),
                );
                // ui.label(egui::RichText::new("Wallet Count").strong().underline());
                // ui.label(egui::RichText::new("Add New Wallet").strong().underline());
                ui.label(
                    egui::RichText::new("Select")
                        .strong()
                        .underline()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.label(
                    egui::RichText::new("Start")
                        .strong()
                        .underline()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.label(
                    egui::RichText::new("Dashmate Password")
                        .strong()
                        .underline()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.label(
                    egui::RichText::new("Actions")
                        .strong()
                        .underline()
                        .color(DashColors::text_primary(dark_mode)),
                );
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

        ui.add_space(20.0);

        // Advanced Settings - Collapsible
        let mut collapsing_state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.make_persistent_id("advanced_settings_header"),
            false,
        );

        // Force close if we need to reset
        if self.should_reset_collapsing_states {
            collapsing_state.set_open(false);
            self.should_reset_collapsing_states = false;
        }

        collapsing_state
            .show_header(ui, |ui| {
                ui.label("Advanced Settings");
            })
            .body(|ui| {
                // Advanced Settings Card Content
                StyledCard::new().padding(20.0).show(ui, |ui| {
                    ui.vertical(|ui| {
                        // Dash-QT Path Section
                        ui.group(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new("Custom Dash-QT Path")
                                        .strong()
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                                ui.add_space(8.0);

                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::new("Select File")
                                                .fill(DashColors::DASH_BLUE)
                                                .stroke(egui::Stroke::NONE)
                                                .corner_radius(egui::CornerRadius::same(6))
                                                .min_size(egui::vec2(120.0, 32.0)),
                                        )
                                        .clicked()
                                        && let Some(path) = rfd::FileDialog::new().pick_file() {
                                            let file_name =
                                                path.file_name().and_then(|f| f.to_str());
                                            if let Some(file_name) = file_name {
                                                self.custom_dash_qt_path = None;
                                                self.custom_dash_qt_error_message = None;

                                                // Handle macOS .app bundles
                                                let resolved_path = if cfg!(target_os = "macos") && path.extension().and_then(|s| s.to_str()) == Some("app") {
                                                    // For .app bundles, resolve to the actual executable inside
                                                    path.join("Contents").join("MacOS").join("Dash-Qt")
                                                } else {
                                                    path.clone()
                                                };

                                                // Check if the resolved path exists and is valid
                                                let is_valid = if cfg!(target_os = "windows") {
                                                    file_name.to_ascii_lowercase().ends_with("dash-qt.exe")
                                                } else if cfg!(target_os = "macos") {
                                                    // Accept both direct executable and .app bundle
                                                    file_name.eq_ignore_ascii_case("dash-qt") || 
                                                    (file_name.to_ascii_lowercase().ends_with(".app") && resolved_path.exists())
                                                } else {
                                                    // Linux
                                                    file_name.eq_ignore_ascii_case("dash-qt")
                                                };

                                                if is_valid {
                                                    self.custom_dash_qt_path = Some(resolved_path);
                                                    self.custom_dash_qt_error_message = None;
                                                    self.save()
                                                        .expect("Expected to save db settings");
                                                } else {
                                                    let required_file_name = if cfg!(target_os = "windows") {
                                                        "dash-qt.exe"
                                                    } else if cfg!(target_os = "macos") {
                                                        "Dash-Qt or Dash-Qt.app"
                                                    } else {
                                                        "dash-qt"
                                                    };
                                                    self.custom_dash_qt_error_message = Some(format!(
                                                        "Invalid file: Please select a valid '{}'.",
                                                        required_file_name
                                                    ));
                                                }
                                            }
                                        }

                                    if (self.custom_dash_qt_path.is_some()
                                        || self.custom_dash_qt_error_message.is_some())
                                        && ui
                                            .add(
                                                egui::Button::new("Clear")
                                                    .fill(DashColors::ERROR.linear_multiply(0.8))
                                                    .stroke(egui::Stroke::NONE)
                                                    .corner_radius(egui::CornerRadius::same(6))
                                                    .min_size(egui::vec2(80.0, 32.0)),
                                            )
                                            .clicked()
                                    {
                                        self.custom_dash_qt_path = Some(PathBuf::new()); // Reset to empty to avoid auto-detection
                                        self.custom_dash_qt_error_message = None;
                                        self.save().expect("Expected to save db settings");
                                    }
                                });

                                ui.add_space(8.0);

                                if let Some(ref file) = self.custom_dash_qt_path {
                                    ui.horizontal(|ui| {
                                        ui.label("Selected:");
                                        ui.label(
                                            egui::RichText::new(format_path_for_display(file)).color(DashColors::SUCCESS),
                                        )
                                        .on_hover_text(format!("Full path: {}", file.display()));
                                    });
                                } else if let Some(ref error) = self.custom_dash_qt_error_message {
                                    ui.horizontal(|ui| {
                                        ui.label("Error:");
                                        ui.colored_label(DashColors::ERROR, error);
                                    });
                                } else {
                                    ui.label(
                                        egui::RichText::new(
                                            "dash-qt not found, click 'Select File' to choose.",
                                        )
                                        .color(DashColors::TEXT_SECONDARY)
                                        .italics(),
                                    );
                                }
                            });
                        });

                        ui.add_space(16.0);

                        // Configuration Options Section
                        ui.group(|ui| {
                            ui.vertical(|ui| {
                                ui.label(
                                    egui::RichText::new("Configuration Options")
                                        .strong()
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                                ui.add_space(8.0);

                                // Overwrite dash.conf checkbox
                                ui.horizontal(|ui| {
                                    if StyledCheckbox::new(
                                        &mut self.overwrite_dash_conf,
                                        "Overwrite dash.conf",
                                    )
                                    .show(ui)
                                    .clicked()
                                    {
                                        self.save().expect("Expected to save db settings");
                                    }
                                    ui.label(
                                    egui::RichText::new(
                                        "Automatically configure dash.conf with required settings",
                                    )
                                    .color(DashColors::TEXT_SECONDARY),
                                );
                                });

                                ui.add_space(8.0);

                                // Developer mode checkbox
                                ui.horizontal(|ui| {
                                    if StyledCheckbox::new(
                                        &mut self.developer_mode,
                                        "Enable developer mode",
                                    )
                                    .show(ui)
                                    .clicked()
                                    {
                                        // Update the global developer mode in config
                                        if let Ok(mut config) = Config::load() {
                                            config.developer_mode = Some(self.developer_mode);
                                            if let Err(e) = config.save() {
                                                eprintln!("Failed to save config to .env: {e}");
                                            }

                                            // Update developer mode for all contexts
                                            self.mainnet_app_context
                                                .enable_developer_mode(self.developer_mode);

                                            if let Some(ref testnet_ctx) = self.testnet_app_context
                                            {
                                                testnet_ctx
                                                    .enable_developer_mode(self.developer_mode);
                                            }

                                            if let Some(ref devnet_ctx) = self.devnet_app_context {
                                                devnet_ctx
                                                    .enable_developer_mode(self.developer_mode);
                                            }

                                            if let Some(ref local_ctx) = self.local_app_context {
                                                local_ctx
                                                    .enable_developer_mode(self.developer_mode);
                                            }
                                        }
                                    }
                                    ui.label(
                                        egui::RichText::new(
                                            "Enables advanced features and less strict validation",
                                        )
                                        .color(DashColors::TEXT_SECONDARY),
                                    );
                                });
                            });
                        });

                        // Theme Selection Section
                        ui.add_space(16.0);
                        ui.group(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new("Theme:")
                                            .strong()
                                            .color(DashColors::text_primary(dark_mode)),
                                    );

                                    egui::ComboBox::from_id_salt("theme_selection")
                                        .selected_text(match self.theme_preference {
                                            ThemeMode::Light => "Light",
                                            ThemeMode::Dark => "Dark",
                                            ThemeMode::System => "System",
                                        })
                                        .show_ui(ui, |ui| {
                                            if ui.selectable_value(&mut self.theme_preference, ThemeMode::System, "System").clicked() {
                                                app_action |= AppAction::BackendTask(BackendTask::SystemTask(
                                                    SystemTask::UpdateThemePreference(ThemeMode::System)
                                                ));
                                            }
                                            if ui.selectable_value(&mut self.theme_preference, ThemeMode::Light, "Light").clicked() {
                                                app_action |= AppAction::BackendTask(BackendTask::SystemTask(
                                                    SystemTask::UpdateThemePreference(ThemeMode::Light)
                                                ));
                                            }
                                            if ui.selectable_value(&mut self.theme_preference, ThemeMode::Dark, "Dark").clicked() {
                                                app_action |= AppAction::BackendTask(BackendTask::SystemTask(
                                                    SystemTask::UpdateThemePreference(ThemeMode::Dark)
                                                ));
                                            }
                                        });
                                });
                                ui.label(
                                    egui::RichText::new(
                                        "System: follows your OS theme • Light/Dark: force specific theme",
                                    )
                                    .color(DashColors::TEXT_SECONDARY),
                                );
                            });
                        });

                        // Configuration Requirements Section (only show if not overwriting dash.conf)
                        if !self.overwrite_dash_conf {
                            ui.add_space(16.0);

                            ui.group(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new("Manual Configuration Required")
                                            .strong()
                                            .color(DashColors::WARNING),
                                    );
                                    ui.add_space(8.0);

                                    let (network_name, zmq_ports) = match self.current_network {
                                        Network::Dash => ("Mainnet", ("23708", "23708")),
                                        Network::Testnet => ("Testnet", ("23709", "23709")),
                                        Network::Devnet => ("Devnet", ("23710", "23710")),
                                        Network::Regtest => ("Regtest", ("20302", "20302")),
                                        _ => ("Unknown", ("0", "0")),
                                    };

                                    ui.label(
                                        egui::RichText::new(format!(
                                            "Add these lines to your {} dash.conf:",
                                            network_name
                                        ))
                                        .color(DashColors::TEXT_PRIMARY),
                                    );

                                    ui.add_space(8.0);

                                    // Configuration code block
                                    egui::Frame::new()
                                        .fill(DashColors::INPUT_BACKGROUND)
                                        .stroke(egui::Stroke::new(1.0, DashColors::BORDER))
                                        .corner_radius(egui::CornerRadius::same(6))
                                        .inner_margin(egui::Margin::same(12))
                                        .show(ui, |ui| {
                                            ui.vertical(|ui| {
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "zmqpubrawtxlocksig=tcp://0.0.0.0:{}",
                                                        zmq_ports.0
                                                    ))
                                                    .monospace()
                                                    .color(DashColors::TEXT_PRIMARY),
                                                );
                                                if self.current_network != Network::Regtest {
                                                    ui.label(
                                                        egui::RichText::new(format!(
                                                            "zmqpubrawchainlock=tcp://0.0.0.0:{}",
                                                            zmq_ports.1
                                                        ))
                                                        .monospace()
                                                        .color(DashColors::TEXT_PRIMARY),
                                                    );
                                                }
                                            });
                                        });
                                });
                            });
                        }
                    });
                });
            });

        app_action
    }

    /// Render a single row for the network table
    fn render_network_row(&mut self, ui: &mut Ui, network: Network, name: &str) -> AppAction {
        let mut app_action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.label(name);

        let current_mode = *self
            .backend_modes
            .entry(network)
            .or_insert(CoreBackendMode::Rpc);

        if !self.has_context_for(network) {
            ui.colored_label(DashColors::error_color(dark_mode), "No config");
            self.render_backend_selector(ui, network, current_mode, true, None);
            ui.label("(No configs loaded)");
            ui.label("");
            ui.label("");
            ui.label("");
            ui.end_row();
            return app_action;
        }

        let context = Arc::clone(self.context_for_network(network));

        let mut spv_snapshot: Option<SpvStatusSnapshot> = None;
        match current_mode {
            CoreBackendMode::Rpc => {
                let is_working = self.check_network_status(network);
                let status_color = if is_working {
                    DashColors::success_color(dark_mode)
                } else {
                    DashColors::error_color(dark_mode)
                };
                let status_label = if is_working { "Online" } else { "Offline" };
                ui.colored_label(status_color, status_label);
            }
            CoreBackendMode::Spv => {
                let snapshot = context.spv_manager().status();
                self.render_spv_status(ui, dark_mode, &snapshot, &context);
                spv_snapshot = Some(snapshot);
            }
        }

        let backend_mode =
            self.render_backend_selector(ui, network, current_mode, false, Some(&context));

        if backend_mode == CoreBackendMode::Rpc && current_mode != CoreBackendMode::Rpc {
            self.recheck_time = None;
        }

        let mut is_selected = self.current_network == network;
        if StyledCheckbox::new(&mut is_selected, "").show(ui).clicked() && is_selected {
            self.current_network = network;
            app_action = AppAction::SwitchNetwork(network);
            self.recheck_time = Some(
                (SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    + Duration::from_secs(1))
                .as_millis() as u64,
            );
        }

        match backend_mode {
            CoreBackendMode::Rpc => {
                let start_enabled = if let Some(path) = self.custom_dash_qt_path.as_ref() {
                    !path.as_os_str().is_empty() && path.is_file()
                } else {
                    false
                };

                if network != Network::Regtest {
                    ui.add_enabled_ui(start_enabled, |ui| {
                        if ui
                            .button("Start")
                            .on_disabled_hover_text(
                                "Please select path to dash-qt binary in Advanced Settings",
                            )
                            .clicked()
                        {
                            app_action = AppAction::BackendTask(BackendTask::CoreTask(
                                CoreTask::StartDashQT(
                                    network,
                                    self.custom_dash_qt_path
                                        .clone()
                                        .expect("Some() checked above"),
                                    self.overwrite_dash_conf,
                                ),
                            ));
                        }
                    });
                } else {
                    ui.label("");
                }
            }
            CoreBackendMode::Spv => {
                let snapshot = spv_snapshot.unwrap_or_else(|| context.spv_manager().status());
                match snapshot.status {
                    SpvStatus::Idle | SpvStatus::Stopped | SpvStatus::Error => {
                        if ui.button("Connect").clicked() {
                            if let Err(err) = context.start_spv() {
                                app_action =
                                    AppAction::Custom(format!("Failed to start SPV: {}", err));
                            }
                        }
                    }
                    SpvStatus::Starting | SpvStatus::Syncing | SpvStatus::Running => {
                        if ui.button("Stop").clicked() {
                            context.stop_spv();
                        }
                    }
                    SpvStatus::Stopping => {
                        ui.add_enabled_ui(false, |ui| {
                            let _ = ui.button("Stopping…");
                        });
                    }
                }
            }
        }

        if network == Network::Regtest {
            if backend_mode == CoreBackendMode::Rpc {
                ui.spacing_mut().item_spacing.x = 5.0;
                let text_color = DashColors::text_primary(dark_mode);
                let background = DashColors::input_background(dark_mode);
                ui.add(
                    egui::TextEdit::singleline(&mut self.local_network_dashmate_password)
                        .desired_width(100.0)
                        .text_color(text_color)
                        .background_color(background),
                );
                if ui.button("Save Password").clicked() {
                    if let Ok(mut config) = Config::load()
                        && let Some(local_cfg) = config.config_for_network(Network::Regtest).clone()
                    {
                        let updated_local_config = local_cfg
                            .update_core_rpc_password(self.local_network_dashmate_password.clone());
                        config.update_config_for_network(
                            Network::Regtest,
                            updated_local_config.clone(),
                        );
                        if let Err(e) = config.save() {
                            eprintln!("Failed to save config to .env: {e}");
                        }

                        if let Some(local_app_context) = &self.local_app_context {
                            {
                                let mut cfg_lock = local_app_context.config.write().unwrap();
                                *cfg_lock = updated_local_config;
                            }

                            if let Err(e) =
                                Arc::clone(local_app_context).reinit_core_client_and_sdk()
                            {
                                eprintln!("Failed to re-init local RPC client and sdk: {}", e);
                            } else {
                                app_action = AppAction::SwitchNetwork(Network::Regtest);
                            }
                        }
                    }
                }
            } else {
                ui.label("-");
            }
        } else {
            ui.label("");
        }

        if network == Network::Devnet {
            if ui.button("Clear local Platform data").clicked() {
                app_action =
                    AppAction::BackendTask(BackendTask::SystemTask(SystemTask::WipePlatformData));
            }
        } else {
            ui.label("");
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

    fn any_rpc_backend(&self) -> bool {
        self.backend_modes
            .iter()
            .any(|(network, mode)| *mode == CoreBackendMode::Rpc && self.has_context_for(*network))
    }

    fn has_context_for(&self, network: Network) -> bool {
        match network {
            Network::Dash => true,
            Network::Testnet => self.testnet_app_context.is_some(),
            Network::Devnet => self.devnet_app_context.is_some(),
            Network::Regtest => self.local_app_context.is_some(),
            _ => false,
        }
    }

    fn backend_mode_label(mode: CoreBackendMode) -> &'static str {
        match mode {
            CoreBackendMode::Rpc => "Dash Core RPC",
            CoreBackendMode::Spv => "Dash SPV",
        }
    }

    fn render_backend_selector(
        &mut self,
        ui: &mut Ui,
        network: Network,
        current_mode: CoreBackendMode,
        disabled: bool,
        context: Option<&Arc<AppContext>>,
    ) -> CoreBackendMode {
        let mut mode = current_mode;

        if disabled {
            ui.add_enabled_ui(false, |ui| {
                egui::ComboBox::from_id_salt(format!("backend_{:?}", network))
                    .selected_text(Self::backend_mode_label(mode))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut mode,
                            CoreBackendMode::Rpc,
                            Self::backend_mode_label(CoreBackendMode::Rpc),
                        );
                        ui.selectable_value(
                            &mut mode,
                            CoreBackendMode::Spv,
                            Self::backend_mode_label(CoreBackendMode::Spv),
                        );
                    });
            });
            return current_mode;
        }

        let mut changed = false;
        egui::ComboBox::from_id_salt(format!("backend_{:?}", network))
            .selected_text(Self::backend_mode_label(mode))
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(
                        &mut mode,
                        CoreBackendMode::Rpc,
                        Self::backend_mode_label(CoreBackendMode::Rpc),
                    )
                    .changed();
                changed |= ui
                    .selectable_value(
                        &mut mode,
                        CoreBackendMode::Spv,
                        Self::backend_mode_label(CoreBackendMode::Spv),
                    )
                    .changed();
            });

        if changed {
            self.backend_modes.insert(network, mode);
            if let Some(ctx) = context {
                ctx.set_core_backend_mode(mode);
                if mode == CoreBackendMode::Rpc {
                    ctx.stop_spv();
                }
            }
            mode
        } else {
            current_mode
        }
    }

    fn render_spv_status(
        &self,
        ui: &mut Ui,
        dark_mode: bool,
        snapshot: &SpvStatusSnapshot,
        context: &Arc<AppContext>,
    ) {
        let (label, color) = self.spv_status_label(snapshot, dark_mode);
        ui.vertical(|ui| {
            ui.colored_label(color, label);
            if let Some(detail) = self.spv_status_detail(snapshot) {
                ui.label(egui::RichText::new(detail).color(DashColors::text_secondary(dark_mode)));
            }
        });

        if snapshot.status.is_active() {
            context.repaint_animation(ui.ctx());
        }
    }

    fn spv_status_label(
        &self,
        snapshot: &SpvStatusSnapshot,
        dark_mode: bool,
    ) -> (String, egui::Color32) {
        match snapshot.status {
            SpvStatus::Idle => ("Idle".to_string(), DashColors::muted_color(dark_mode)),
            SpvStatus::Starting => (
                "Starting...".to_string(),
                DashColors::warning_color(dark_mode),
            ),
            SpvStatus::Syncing => (
                "Syncing...".to_string(),
                DashColors::warning_color(dark_mode),
            ),
            SpvStatus::Running => ("Synced".to_string(), DashColors::success_color(dark_mode)),
            SpvStatus::Stopping => (
                "Stopping...".to_string(),
                DashColors::warning_color(dark_mode),
            ),
            SpvStatus::Stopped => ("Stopped".to_string(), DashColors::muted_color(dark_mode)),
            SpvStatus::Error => ("Error".to_string(), DashColors::error_color(dark_mode)),
        }
    }

    fn spv_status_detail(&self, snapshot: &SpvStatusSnapshot) -> Option<String> {
        if let SpvStatus::Error = snapshot.status {
            if let Some(err) = &snapshot.last_error {
                return Some(err.clone());
            }
        }

        if let Some(progress) = snapshot.detailed_progress.as_ref() {
            return Some(Self::format_detailed_progress(progress));
        }

        if let Some(progress) = snapshot.sync_progress.as_ref() {
            return Some(format!(
                "Headers: {} | Filters: {} | Peers: {}",
                progress.header_height, progress.filter_header_height, progress.peer_count
            ));
        }

        snapshot.last_error.clone()
    }

    fn format_detailed_progress(progress: &DetailedSyncProgress) -> String {
        let mut message = match &progress.sync_stage {
            SyncStage::Connecting => "Connecting to peers".to_string(),
            SyncStage::QueryingPeerHeight => "Querying peer heights".to_string(),
            SyncStage::DownloadingHeaders { start, end } => {
                format!("Headers {start} -> {end}")
            }
            SyncStage::ValidatingHeaders { batch_size } => {
                format!("Validating headers (batch {batch_size})")
            }
            SyncStage::StoringHeaders { batch_size } => {
                format!("Storing headers (batch {batch_size})")
            }
            SyncStage::Complete => "Sync complete".to_string(),
            SyncStage::Failed(reason) => format!("Failed: {reason}"),
        };

        let percent = progress.calculate_percentage();
        if percent.is_finite() && percent > 0.0 {
            message = format!("{message} ({percent:.1}%)");
        }

        if progress.connected_peers > 0 {
            message = format!("{message} | peers: {}", progress.connected_peers);
        }

        if let Some(eta) = progress.calculate_eta() {
            if eta.as_secs() > 0 {
                message = format!("{message} | ETA: {}s", eta.as_secs());
            }
        }

        message
    }
}

impl ScreenLike for NetworkChooserScreen {
    fn refresh_on_arrival(&mut self) {
        // Reset collapsing states when arriving at this screen
        // This ensures dropdowns are closed when navigating back
        self.should_reset_collapsing_states = true;

        // Reload settings from database to ensure we have the latest values
        if let Ok(Some(settings)) = self.current_app_context().get_settings() {
            self.custom_dash_qt_path = settings.dash_qt_path;
            self.overwrite_dash_conf = settings.overwrite_dash_conf;
            self.theme_preference = settings.theme_mode;
        }

        self.backend_modes
            .insert(Network::Dash, self.mainnet_app_context.core_backend_mode());
        if let Some(ctx) = &self.testnet_app_context {
            self.backend_modes
                .insert(Network::Testnet, ctx.core_backend_mode());
        }
        if let Some(ctx) = &self.devnet_app_context {
            self.backend_modes
                .insert(Network::Devnet, ctx.core_backend_mode());
        }
        if let Some(ctx) = &self.local_app_context {
            self.backend_modes
                .insert(Network::Regtest, ctx.core_backend_mode());
        }
    }

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
        if let BackendTaskSuccessResult::CoreItem(CoreItem::ChainLocks(
            mainnet_chainlock,
            testnet_chainlock,
            devnet_chainlock,
            local_chainlock,
        )) = backend_task_success_result
        {
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

        action |= island_central_panel(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| self.render_network_table(ui))
                .inner
        });

        // Recheck both network status every 3 seconds
        let recheck_time = Duration::from_secs(3);
        if action == AppAction::None {
            if self.any_rpc_backend() {
                let current_time = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards");
                if let Some(time) = self.recheck_time {
                    if current_time.as_millis() as u64 >= time {
                        action = AppAction::BackendTask(BackendTask::CoreTask(
                            CoreTask::GetBestChainLocks,
                        ));
                        self.recheck_time = Some((current_time + recheck_time).as_millis() as u64);
                    }
                } else {
                    self.recheck_time = Some((current_time + recheck_time).as_millis() as u64);
                }
            } else {
                self.recheck_time = None;
            }
        }

        action
    }
}
