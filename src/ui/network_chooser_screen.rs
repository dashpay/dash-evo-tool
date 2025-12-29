use crate::app::AppAction;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::system_task::SystemTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::config::Config;
use crate::context::AppContext;
use crate::spv::{CoreBackendMode, SpvStatus, SpvStatusSnapshot};
use crate::ui::components::component_trait::Component;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::{
    ConfirmationDialog, ConfirmationStatus, StyledCard, StyledCheckbox, island_central_panel,
};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::{DashColors, Shape, ThemeMode};
use crate::ui::{RootScreenType, ScreenLike};
use crate::utils::path::format_path_for_display;
use dash_sdk::dash_spv::types::{DetailedSyncProgress, SyncStage};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use eframe::egui::{self, Color32, Context, Frame, Margin, RichText, Ui};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
enum SpvClearMessage {
    Success(String),
    Error(String),
}

#[derive(Debug, Clone)]
enum DatabaseClearMessage {
    Success(String),
    Error(String),
}

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
    disable_zmq: bool,
    developer_mode: bool,
    theme_preference: ThemeMode,
    should_reset_collapsing_states: bool,
    backend_modes: HashMap<Network, CoreBackendMode>,
    filter_headers_stage_start: Option<u32>,
    spv_clear_dialog: Option<ConfirmationDialog>,
    spv_clear_message: Option<SpvClearMessage>,
    db_clear_dialog: Option<ConfirmationDialog>,
    db_clear_message: Option<DatabaseClearMessage>,
    show_evonode_tools: bool,
    use_local_spv_node: bool,
    auto_start_spv: bool,
    close_dash_qt_on_exit: bool,
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
        let disable_zmq = settings.disable_zmq;
        let custom_dash_qt_path = settings.dash_qt_path;
        let show_evonode_tools = settings.show_evonode_tools;
        let use_local_spv_node = mainnet_app_context
            .db
            .get_use_local_spv_node()
            .unwrap_or(false);
        let auto_start_spv = mainnet_app_context
            .db
            .get_auto_start_spv()
            .unwrap_or(true);
        let close_dash_qt_on_exit = mainnet_app_context
            .db
            .get_close_dash_qt_on_exit()
            .unwrap_or(true);

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
            disable_zmq,
            developer_mode,
            theme_preference,
            should_reset_collapsing_states: true, // Start with collapsed state
            backend_modes,
            filter_headers_stage_start: None,
            spv_clear_dialog: None,
            spv_clear_message: None,
            db_clear_dialog: None,
            db_clear_message: None,
            show_evonode_tools,
            use_local_spv_node,
            auto_start_spv,
            close_dash_qt_on_exit,
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
    /// Render the simplified settings interface
    fn render_network_table(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Connection Settings Card
        StyledCard::new().padding(24.0).show(ui, |ui| {
            ui.heading("Connection Settings");
            ui.add_space(20.0);

            // Create a table with rows and 2 columns
            egui::Grid::new("connection_settings_grid")
                .num_columns(2)
                .spacing([40.0, 12.0])
                .striped(false)
                .show(ui, |ui| {
                    // Row 1: Connection Type
                    ui.label(
                        egui::RichText::new("Connection Type:")
                            .color(DashColors::text_primary(dark_mode)),
                    );

                    let current_backend_mode = *self
                        .backend_modes
                        .entry(self.current_network)
                        .or_insert(CoreBackendMode::Rpc);

                    let connection_text = match current_backend_mode {
                        CoreBackendMode::Spv => "SPV Client",
                        CoreBackendMode::Rpc => "Dash Core RPC",
                    };

                    let mut connection_mode = current_backend_mode;
                    egui::ComboBox::from_id_salt("connection_mode_selector")
                        .selected_text(connection_text)
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(
                                    &mut connection_mode,
                                    CoreBackendMode::Spv,
                                    "SPV Client",
                                )
                                .changed()
                            {
                                self.backend_modes
                                    .insert(self.current_network, CoreBackendMode::Spv);
                                let ctx = self.current_app_context();
                                ctx.set_core_backend_mode(CoreBackendMode::Spv);
                            }
                            if ui
                                .selectable_value(
                                    &mut connection_mode,
                                    CoreBackendMode::Rpc,
                                    "Dash Core RPC",
                                )
                                .changed()
                            {
                                self.backend_modes
                                    .insert(self.current_network, CoreBackendMode::Rpc);
                                let ctx = self.current_app_context();
                                ctx.set_core_backend_mode(CoreBackendMode::Rpc);
                                ctx.stop_spv();
                            }
                        });

                    ui.end_row();

                    // Row 2: Network
                    ui.label(
                        egui::RichText::new("Network:").color(DashColors::text_primary(dark_mode)),
                    );

                    // Check if currently connected via SPV (only SPV restricts network switching)
                    let is_spv_connected = if current_backend_mode == CoreBackendMode::Spv {
                        let ctx = self.current_app_context();
                        let snapshot = ctx.spv_manager().status();
                        snapshot.status.is_active()
                    } else {
                        false // Core mode doesn't restrict network switching
                    };

                    let network_text = match self.current_network {
                        Network::Dash => "Mainnet",
                        Network::Testnet => "Testnet",
                        Network::Devnet => "Devnet",
                        Network::Regtest => "Local",
                        _ => "Unknown",
                    };

                    let network_combo = egui::ComboBox::from_id_salt("network_selector")
                        .selected_text(network_text)
                        .width(200.0);

                    let response = ui.add_enabled_ui(!is_spv_connected, |ui| {
                        network_combo.show_ui(ui, |ui| {
                            if ui
                                .selectable_value(
                                    &mut self.current_network,
                                    Network::Dash,
                                    "Mainnet",
                                )
                                .clicked()
                            {
                                app_action = AppAction::SwitchNetwork(Network::Dash);
                            }
                            if self.testnet_app_context.is_some()
                                && ui
                                    .selectable_value(
                                        &mut self.current_network,
                                        Network::Testnet,
                                        "Testnet",
                                    )
                                    .clicked()
                            {
                                app_action = AppAction::SwitchNetwork(Network::Testnet);
                            }
                            if self.devnet_app_context.is_some()
                                && ui
                                    .selectable_value(
                                        &mut self.current_network,
                                        Network::Devnet,
                                        "Devnet",
                                    )
                                    .clicked()
                            {
                                app_action = AppAction::SwitchNetwork(Network::Devnet);
                            }
                            if self.local_app_context.is_some()
                                && ui
                                    .selectable_value(
                                        &mut self.current_network,
                                        Network::Regtest,
                                        "Local",
                                    )
                                    .clicked()
                            {
                                app_action = AppAction::SwitchNetwork(Network::Regtest);
                            }
                        });
                    });

                    if is_spv_connected {
                        response.response.on_hover_text("Disconnect from SPV first");
                    }

                    ui.end_row();
                });

            // Password input for Local network
            let current_backend_mode = *self
                .backend_modes
                .entry(self.current_network)
                .or_insert(CoreBackendMode::Rpc);
            if self.current_network == Network::Regtest
                && current_backend_mode == CoreBackendMode::Rpc
            {
                ui.add_space(20.0);
                ui.separator();
                ui.add_space(12.0);

                ui.label(
                    egui::RichText::new("Local Network Password")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.local_network_dashmate_password);

                    if ui.button("Save").clicked()
                        && let Ok(mut config) = Config::load()
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

                        // Update our local AppContext in memory
                        if let Some(local_app_context) = &self.local_app_context {
                            {
                                // Overwrite the config field with the new password
                                let mut cfg_lock = local_app_context.config.write().unwrap();
                                *cfg_lock = updated_local_config;
                            }

                            // Re-init the client & sdk from the updated config
                            if let Err(e) =
                                Arc::clone(local_app_context).reinit_core_client_and_sdk()
                            {
                                eprintln!("Failed to re-init local RPC client and sdk: {}", e);
                            } else {
                                // Trigger SwitchNetworks
                                app_action = AppAction::SwitchNetwork(Network::Regtest);
                            }
                        }
                    }
                });
            }
        });

        // Connection Status Card
        ui.add_space(16.0);

        StyledCard::new().padding(24.0).show(ui, |ui| {
            ui.heading("Connection Status");
            ui.add_space(10.0);

            let current_backend_mode = *self
                .backend_modes
                .entry(self.current_network)
                .or_insert(CoreBackendMode::Rpc);

            // Check connection status
            let (is_connected, snapshot) = match current_backend_mode {
                CoreBackendMode::Rpc => (self.check_network_status(self.current_network), None),
                CoreBackendMode::Spv => {
                    let ctx = self.current_app_context();
                    let snap = ctx.spv_manager().status();
                    let connected = snap.status.is_active() || snap.status == SpvStatus::Running;
                    (connected, Some(snap))
                }
            };

            // Button on the left with status
            ui.horizontal(|ui| {
                if is_connected {
                    if current_backend_mode == CoreBackendMode::Spv {
                        let disconnect_button = egui::Button::new(
                            egui::RichText::new("Disconnect").color(DashColors::WHITE),
                        )
                        .fill(DashColors::ERROR)
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(Shape::RADIUS_MD)
                        .min_size(egui::vec2(120.0, 36.0));

                        if ui.add(disconnect_button).clicked() {
                            self.current_app_context().stop_spv();
                        }

                        // Show sync status next to button
                        ui.add_space(12.0);

                        if let Some(snap) = &snapshot {
                            match snap.status {
                                SpvStatus::Running => {
                                    ui.colored_label(DashColors::SUCCESS, "Fully Synced - The SPV client can now be used for transacting and querying.");
                                }
                                SpvStatus::Syncing | SpvStatus::Starting => {
                                    ui.style_mut().visuals.widgets.inactive.fg_stroke.color =
                                        DashColors::DASH_BLUE;
                                    ui.style_mut().visuals.widgets.hovered.fg_stroke.color =
                                        DashColors::DASH_BLUE;
                                    ui.style_mut().visuals.widgets.active.fg_stroke.color =
                                        DashColors::DASH_BLUE;
                                    ui.spinner();
                                    ui.label(egui::RichText::new("Syncing..."));
                                }
                                SpvStatus::Stopping => {
                                    ui.style_mut().visuals.widgets.inactive.fg_stroke.color =
                                        DashColors::DASH_BLUE;
                                    ui.style_mut().visuals.widgets.hovered.fg_stroke.color =
                                        DashColors::DASH_BLUE;
                                    ui.style_mut().visuals.widgets.active.fg_stroke.color =
                                        DashColors::DASH_BLUE;
                                    ui.spinner();
                                    ui.label(egui::RichText::new("Disconnecting..."));
                                }
                                _ => {}
                            }
                        }
                    } else {
                        // For Core mode, just show status since it can switch networks freely
                        ui.colored_label(DashColors::DASH_BLUE, "✅ Connected");
                    }
                } else {
                    // Don't show Connect button for Local network in RPC mode
                    // (there's no Dash-Qt to start for local/regtest)
                    let show_connect_button = !(self.current_network == Network::Regtest
                        && current_backend_mode == CoreBackendMode::Rpc);

                    if show_connect_button {
                        let connect_button = egui::Button::new(
                            egui::RichText::new("Connect").color(DashColors::WHITE),
                        )
                        .fill(DashColors::DASH_BLUE)
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(Shape::RADIUS_MD)
                        .min_size(egui::vec2(120.0, 36.0));

                        if ui.add(connect_button).clicked() {
                            if current_backend_mode == CoreBackendMode::Spv {
                                if let Err(err) = self.current_app_context().start_spv() {
                                    app_action =
                                        AppAction::Custom(format!("Failed to start SPV: {}", err));
                                }
                            } else {
                                // Core mode connect
                                let settings =
                                    self.current_app_context().get_settings().ok().flatten();
                                let dash_qt_path = settings
                                    .and_then(|s| s.dash_qt_path)
                                    .or_else(|| self.custom_dash_qt_path.clone());
                                if let Some(path) = dash_qt_path {
                                    app_action = AppAction::BackendTask(BackendTask::CoreTask(
                                        CoreTask::StartDashQT(
                                            self.current_network,
                                            path,
                                            self.overwrite_dash_conf,
                                        ),
                                    ));
                                }
                            }
                        }
                    }
                }
            });

            if current_backend_mode == CoreBackendMode::Spv
                && let Some(snap) = snapshot.as_ref()
                    && (snap.status == SpvStatus::Syncing || snap.status == SpvStatus::Starting)
                {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(10.0);

                    self.render_spv_sync_progress(ui, snap);
                }
        });

        // Advanced Settings section with clean dropdown
        ui.add_space(16.0);

        StyledCard::new().padding(20.0).show(ui, |ui| {
            // Custom collapsing header
            let id = ui.make_persistent_id("advanced_settings_header");
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
                ui.ctx(),
                id,
                false,
            );

            // Custom expand/collapse icon
            let icon = if state.is_open() {
                "−" // Minus sign when open
            } else {
                "+" // Plus sign when closed
            };

            let response = ui.horizontal(|ui| {
                // Make the content area clickable
                let response = ui.allocate_response(
                    egui::vec2(ui.available_width(), 30.0),
                    egui::Sense::click(),
                );

                // Draw the content on top of the response area
                let painter = ui.painter_at(response.rect);
                let mut cursor = response.rect.min;

                // Icon with background
                let icon_size = egui::vec2(24.0, 24.0);
                let icon_rect = egui::Rect::from_min_size(cursor, icon_size);
                painter.rect_filled(
                    icon_rect,
                    egui::CornerRadius::from(4.0),
                    DashColors::glass_white(dark_mode),
                );

                let icon_text = painter.layout_no_wrap(
                    icon.to_string(),
                    egui::FontId::proportional(16.0),
                    DashColors::DASH_BLUE,
                );
                painter.galley(
                    icon_rect.center() - icon_text.size() / 2.0,
                    icon_text,
                    DashColors::DASH_BLUE,
                );

                cursor.x += icon_size.x + 8.0;

                // Advanced Settings text
                let text = painter.layout_no_wrap(
                    "Advanced Settings".to_string(),
                    egui::FontId::proportional(16.0),
                    DashColors::text_primary(dark_mode),
                );
                painter.galley(
                    cursor + egui::vec2(0.0, (icon_size.y - text.size().y) / 2.0),
                    text,
                    DashColors::text_primary(dark_mode),
                );

                response
            });

            if response.inner.clicked() {
                state.toggle(ui);
            }

            if response.inner.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            };
            state.show_body_unindented(ui, |ui| {
                ui.add_space(12.0);

                // Theme Selection
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("🎨").size(16.0));
                    ui.label("Theme:");

                    egui::ComboBox::from_id_salt("theme_selection")
                        .selected_text(match self.theme_preference {
                            ThemeMode::Light => "☀ Light",
                            ThemeMode::Dark => "🌙 Dark",
                            ThemeMode::System => "🖥 System",
                        })
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(
                                    &mut self.theme_preference,
                                    ThemeMode::System,
                                    "🖥 System",
                                )
                                .clicked()
                            {
                                app_action |= AppAction::BackendTask(BackendTask::SystemTask(
                                    SystemTask::UpdateThemePreference(ThemeMode::System),
                                ));
                            }
                            if ui
                                .selectable_value(
                                    &mut self.theme_preference,
                                    ThemeMode::Light,
                                    "☀ Light",
                                )
                                .clicked()
                            {
                                app_action |= AppAction::BackendTask(BackendTask::SystemTask(
                                    SystemTask::UpdateThemePreference(ThemeMode::Light),
                                ));
                            }
                            if ui
                                .selectable_value(
                                    &mut self.theme_preference,
                                    ThemeMode::Dark,
                                    "🌙 Dark",
                                )
                                .clicked()
                            {
                                app_action |= AppAction::BackendTask(BackendTask::SystemTask(
                                    SystemTask::UpdateThemePreference(ThemeMode::Dark),
                                ));
                            }
                        });
                });

                // Dash-QT Path
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                ui.label(
                    egui::RichText::new("Dash Core Executable Path")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui.button("Select File").clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_file()
                    {
                        let file_name = path.file_name().and_then(|f| f.to_str());
                        if let Some(file_name) = file_name {
                            self.custom_dash_qt_path = None;
                            self.custom_dash_qt_error_message = None;

                            // Handle macOS .app bundles
                            let resolved_path = if cfg!(target_os = "macos")
                                && path.extension().and_then(|s| s.to_str()) == Some("app")
                            {
                                path.join("Contents").join("MacOS").join("Dash-Qt")
                            } else {
                                path.clone()
                            };

                            // Check if the resolved path exists and is valid
                            let is_valid = if cfg!(target_os = "windows") {
                                file_name.to_ascii_lowercase().ends_with("dash-qt.exe")
                            } else if cfg!(target_os = "macos") {
                                file_name.eq_ignore_ascii_case("dash-qt")
                                    || (file_name.to_ascii_lowercase().ends_with(".app")
                                        && resolved_path.exists())
                            } else {
                                file_name.eq_ignore_ascii_case("dash-qt")
                            };

                            if is_valid {
                                self.custom_dash_qt_path = Some(resolved_path);
                                self.custom_dash_qt_error_message = None;
                                self.save().expect("Expected to save db settings");
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

                    if self.custom_dash_qt_path.is_some() && ui.button("Clear").clicked() {
                        self.custom_dash_qt_path = Some(PathBuf::new());
                        self.custom_dash_qt_error_message = None;
                        self.save().expect("Expected to save db settings");
                    }
                });

                if let Some(ref file) = self.custom_dash_qt_path {
                    if !file.as_os_str().is_empty() {
                        ui.horizontal(|ui| {
                            ui.label("Path:");
                            ui.label(
                                egui::RichText::new(format_path_for_display(file))
                                    .color(DashColors::SUCCESS)
                                    .italics(),
                            );
                        });
                    }
                } else if let Some(ref error) = self.custom_dash_qt_error_message {
                    let error_color = Color32::from_rgb(255, 100, 100);
                    let error = error.clone();
                    Frame::new()
                        .fill(error_color.gamma_multiply(0.1))
                        .inner_margin(Margin::symmetric(10, 8))
                        .corner_radius(5.0)
                        .stroke(egui::Stroke::new(1.0, error_color))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&error).color(error_color));
                                ui.add_space(10.0);
                                if ui.small_button("Dismiss").clicked() {
                                    self.custom_dash_qt_error_message = None;
                                }
                            });
                        });
                }

                // Configuration Options
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new("Configuration Options")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if StyledCheckbox::new(&mut self.overwrite_dash_conf, "Overwrite dash.conf")
                        .show(ui)
                        .clicked()
                    {
                        self.save().expect("Expected to save db settings");
                    }
                    ui.label(
                        egui::RichText::new("Auto-configure required settings")
                            .color(DashColors::TEXT_SECONDARY)
                            .italics(),
                    );
                });

                // Disable ZMQ toggle (requires restart)
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if StyledCheckbox::new(&mut self.disable_zmq, "Disable ZMQ (requires restart)")
                        .show(ui)
                        .clicked()
                    {
                        // Persist immediately via context
                        let _ = self
                            .current_app_context()
                            .update_disable_zmq(self.disable_zmq);
                    }
                });

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if StyledCheckbox::new(&mut self.developer_mode, "Developer mode")
                        .show(ui)
                        .clicked()
                        && let Ok(mut config) = Config::load()
                    {
                        config.developer_mode = Some(self.developer_mode);
                        if let Err(e) = config.save() {
                            eprintln!("Failed to save config: {e}");
                        }

                        // Update all contexts
                        self.mainnet_app_context
                            .enable_developer_mode(self.developer_mode);
                        if let Some(ref ctx) = self.testnet_app_context {
                            ctx.enable_developer_mode(self.developer_mode);
                        }
                        if let Some(ref ctx) = self.devnet_app_context {
                            ctx.enable_developer_mode(self.developer_mode);
                        }
                        if let Some(ref ctx) = self.local_app_context {
                            ctx.enable_developer_mode(self.developer_mode);
                        }
                    }
                    ui.label(
                        egui::RichText::new("Enable advanced features")
                            .color(DashColors::TEXT_SECONDARY)
                            .italics(),
                    );
                });

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if StyledCheckbox::new(
                        &mut self.close_dash_qt_on_exit,
                        "Close Dash-Qt when DET exits",
                    )
                    .show(ui)
                    .clicked()
                    {
                        // Save to database
                        let _ = self
                            .mainnet_app_context
                            .db
                            .update_close_dash_qt_on_exit(self.close_dash_qt_on_exit);
                    }
                    ui.label(
                        egui::RichText::new(if self.close_dash_qt_on_exit {
                            "Dash-Qt will close automatically"
                        } else {
                            "Dash-Qt will keep running"
                        })
                        .color(DashColors::TEXT_SECONDARY)
                        .italics(),
                    );
                });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(12.0);

                // SPV Peer Source
                ui.label(
                    egui::RichText::new("SPV Peer Source")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "Choose how SPV finds peers for blockchain sync on mainnet/testnet.",
                    )
                    .color(DashColors::text_secondary(dark_mode)),
                );
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if StyledCheckbox::new(&mut self.use_local_spv_node, "Use local Dash Core node")
                        .show(ui)
                        .clicked()
                    {
                        // Save to database
                        let _ = self
                            .mainnet_app_context
                            .db
                            .update_use_local_spv_node(self.use_local_spv_node);

                        // Update all network contexts
                        self.mainnet_app_context
                            .spv_manager()
                            .set_use_local_node(self.use_local_spv_node);
                        if let Some(ref ctx) = self.testnet_app_context {
                            ctx.spv_manager().set_use_local_node(self.use_local_spv_node);
                        }
                        if let Some(ref ctx) = self.devnet_app_context {
                            ctx.spv_manager().set_use_local_node(self.use_local_spv_node);
                        }
                        if let Some(ref ctx) = self.local_app_context {
                            ctx.spv_manager().set_use_local_node(self.use_local_spv_node);
                        }
                    }
                    ui.label(
                        egui::RichText::new(if self.use_local_spv_node {
                            "Connect to local node at 127.0.0.1"
                        } else {
                            "Use DNS seed discovery (default)"
                        })
                        .color(DashColors::TEXT_SECONDARY)
                        .italics(),
                    );
                });
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Note: Changes take effect on next SPV sync start. Devnet/local networks always use configured host.",
                    )
                    .size(11.0)
                    .color(DashColors::text_secondary(dark_mode))
                    .italics(),
                );

                // Auto-start SPV on startup
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(12.0);

                ui.label(
                    egui::RichText::new("SPV Auto-Start")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "Automatically start SPV sync when the app opens.",
                    )
                    .color(DashColors::text_secondary(dark_mode)),
                );
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if StyledCheckbox::new(&mut self.auto_start_spv, "Auto-start SPV on startup")
                        .show(ui)
                        .clicked()
                    {
                        // Save to database
                        let _ = self
                            .mainnet_app_context
                            .db
                            .update_auto_start_spv(self.auto_start_spv);
                    }
                    ui.label(
                        egui::RichText::new(if self.auto_start_spv {
                            "Enabled"
                        } else {
                            "Disabled"
                        })
                        .color(if self.auto_start_spv {
                            DashColors::DASH_BLUE
                        } else {
                            DashColors::text_secondary(dark_mode)
                        }),
                    );
                });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(12.0);

                ui.label(
                    egui::RichText::new("Database Maintenance")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Remove all local data for the current network (wallets, contacts, identities, tokens, etc.).")
                        .color(DashColors::text_secondary(dark_mode)),
                );
                ui.add_space(8.0);

                let button_label = format!("Clear {} Database", self.current_network_label());
                let clear_button = egui::Button::new(
                    egui::RichText::new(button_label).color(DashColors::WHITE),
                )
                .fill(DashColors::ERROR)
                .stroke(egui::Stroke::NONE)
                .corner_radius(Shape::RADIUS_MD)
                .min_size(egui::vec2(0.0, 36.0));

                if ui.add(clear_button).clicked() {
                    let message = format!(
                        "This permanently deletes all local database entries for {}. This includes wallets, tokens, contacts, and cached identity data. This cannot be undone.",
                        self.current_network_label()
                    );
                    self.db_clear_dialog = Some(
                        ConfirmationDialog::new("Clear Database", message)
                            .confirm_text(Some("Delete Data"))
                            .cancel_text(Some("Cancel"))
                            .danger_mode(true),
                    );
                    self.db_clear_message = None;
                }

                if let Some(feedback) = self.db_clear_message.clone() {
                    ui.add_space(8.0);
                    let (message, color) = match &feedback {
                        DatabaseClearMessage::Success(msg) => (msg.as_str(), DashColors::SUCCESS),
                        DatabaseClearMessage::Error(msg) => (msg.as_str(), DashColors::ERROR),
                    };

                    egui::Frame::new()
                        .fill(color.gamma_multiply(0.08))
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .stroke(egui::Stroke::new(1.0, color))
                        .corner_radius(Shape::RADIUS_MD)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(message).color(color));
                                ui.add_space(8.0);
                                if ui.small_button("Dismiss").clicked() {
                                    self.db_clear_message = None;
                                }
                            });
                        });
                }

                if self.db_clear_dialog.is_some() {
                    app_action |= self.show_database_clear_confirmation(ui);
                }

                // SPV Maintenance section
                let current_backend_mode = self.current_app_context().core_backend_mode();
                if current_backend_mode == CoreBackendMode::Spv {
                    let snapshot = self.current_app_context().spv_manager().status();
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(12.0);
                    app_action |= self.render_spv_maintenance_controls(ui, &snapshot);
                }
            });
        });

        app_action
    }

    fn render_spv_sync_progress(&mut self, ui: &mut Ui, snapshot: &SpvStatusSnapshot) {
        if let Some(detailed) = &snapshot.detailed_progress {
            match detailed.sync_stage {
                SyncStage::DownloadingFilterHeaders { current, target } => {
                    let baseline = current.min(target);
                    if let Some(existing) = self.filter_headers_stage_start {
                        self.filter_headers_stage_start = Some(existing.min(target));
                    } else {
                        self.filter_headers_stage_start = Some(baseline);
                    }
                }
                _ => {
                    self.filter_headers_stage_start = None;
                }
            }
        } else {
            self.filter_headers_stage_start = None;
        }

        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Raw sync status display
        egui::Frame::new()
            .fill(DashColors::glass_white(dark_mode))
            .corner_radius(Shape::RADIUS_SM)
            .inner_margin(12.0)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("SPV Sync Status")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );

                ui.add_space(8.0);

                // Display sync information in a grid
                egui::Grid::new("spv_sync_info")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        // Show current status detail
                        if let Some(detail) = self.spv_status_detail(snapshot) {
                            ui.label(
                                egui::RichText::new("Status:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(detail);
                            ui.end_row();
                        }

                        // Prefer detailed header progress when available
                        if snapshot.detailed_progress.is_some() {
                            // Add separator between status and progress bars
                            ui.separator();
                            ui.separator();
                            ui.end_row();

                            // Headers progress
                            ui.label(
                                egui::RichText::new("Headers:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            let headers_progress = self.calculate_headers_progress(snapshot);
                            ui.add(egui::ProgressBar::new(headers_progress).show_percentage());
                            ui.end_row();

                            // Validating headers progress (formerly masternode lists)
                            ui.label(
                                egui::RichText::new("Masternode Lists:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            let validating_progress =
                                self.calculate_validating_headers_progress(snapshot);
                            ui.add(egui::ProgressBar::new(validating_progress).show_percentage());
                            ui.end_row();

                            // Filter headers progress
                            ui.label(
                                egui::RichText::new("Filter Headers:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            let filter_headers_progress =
                                self.calculate_filter_headers_progress(snapshot);
                            ui.add(
                                egui::ProgressBar::new(filter_headers_progress).show_percentage(),
                            );
                            ui.end_row();

                            // Filters progress
                            ui.label(
                                egui::RichText::new("Filters:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            let filters_progress = self.calculate_filters_progress(snapshot);
                            ui.add(egui::ProgressBar::new(filters_progress).show_percentage());
                            ui.end_row();

                            // Blocks progress bar
                            ui.label(
                                egui::RichText::new("Blocks:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            let blocks_progress = self.calculate_blocks_progress(snapshot);
                            ui.add(egui::ProgressBar::new(blocks_progress).show_percentage());
                            ui.end_row();
                        } else if let Some(ev) = &snapshot.sync_progress {
                            // Event-driven progress (updates most frequently)
                            ui.label(
                                egui::RichText::new("Synced:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(format!("Headers height: {}", ev.header_height));
                            ui.end_row();

                            // Add separator between stats and progress bars
                            ui.separator();
                            ui.separator();
                            ui.end_row();

                            // Progress bars for different components
                            let headers_progress = self.calculate_headers_progress(snapshot);
                            ui.label(
                                egui::RichText::new("Headers:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.add(egui::ProgressBar::new(headers_progress).show_percentage());
                            ui.end_row();

                            let validating_progress =
                                self.calculate_validating_headers_progress(snapshot);
                            ui.label(
                                egui::RichText::new("Masternode Lists:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.add(egui::ProgressBar::new(validating_progress).show_percentage());
                            ui.end_row();

                            let filter_headers_progress =
                                self.calculate_filter_headers_progress(snapshot);
                            ui.label(
                                egui::RichText::new("Filter Headers:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.add(
                                egui::ProgressBar::new(filter_headers_progress).show_percentage(),
                            );
                            ui.end_row();

                            let filters_progress = self.calculate_filters_progress(snapshot);
                            ui.label(
                                egui::RichText::new("Filters:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.add(egui::ProgressBar::new(filters_progress).show_percentage());
                            ui.end_row();

                            let blocks_progress = self.calculate_blocks_progress(snapshot);
                            ui.label(
                                egui::RichText::new("Blocks:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.add(egui::ProgressBar::new(blocks_progress).show_percentage());
                            ui.end_row();
                        }
                    });
            });
    }

    fn render_spv_maintenance_controls(
        &mut self,
        ui: &mut Ui,
        snapshot: &SpvStatusSnapshot,
    ) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        ui.label(
            egui::RichText::new("SPV Maintenance")
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Clear cached headers and filter data for this network.")
                .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(8.0);

        let clear_button =
            egui::Button::new(egui::RichText::new("Clear SPV Data").color(DashColors::WHITE))
                .fill(DashColors::ERROR)
                .stroke(egui::Stroke::NONE)
                .corner_radius(Shape::RADIUS_MD)
                .min_size(egui::vec2(0.0, 36.0));

        let is_active = snapshot.status.is_active();
        let mut button_response = ui.add_enabled(!is_active, clear_button);
        if is_active {
            button_response =
                button_response.on_disabled_hover_text("Stop the SPV client before clearing data");
        }

        if button_response.clicked() {
            let network_label = self.current_network_label();
            let message = format!(
                "This will delete cached SPV data for {}. The next connection will trigger a full resync.",
                network_label
            );
            self.spv_clear_dialog = Some(
                ConfirmationDialog::new("Clear SPV Data", message)
                    .confirm_text(Some("Clear Data"))
                    .cancel_text(Some("Keep Data"))
                    .danger_mode(true),
            );
            self.spv_clear_message = None;
        }

        if let Some(feedback) = self.spv_clear_message.clone() {
            ui.add_space(8.0);

            let (message, color) = match &feedback {
                SpvClearMessage::Success(msg) => (msg.as_str(), DashColors::SUCCESS),
                SpvClearMessage::Error(msg) => (msg.as_str(), DashColors::ERROR),
            };

            egui::Frame::new()
                .fill(color.gamma_multiply(0.08))
                .inner_margin(egui::Margin::symmetric(10, 6))
                .stroke(egui::Stroke::new(1.0, color))
                .corner_radius(Shape::RADIUS_MD)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(message).color(color));
                        ui.add_space(8.0);
                        if ui.small_button("Dismiss").clicked() {
                            self.spv_clear_message = None;
                        }
                    });
                });
        }

        if self.spv_clear_dialog.is_some() {
            action |= self.show_spv_clear_confirmation(ui);
        }

        action
    }

    fn show_spv_clear_confirmation(&mut self, ui: &mut Ui) -> AppAction {
        if let Some(dialog) = self.spv_clear_dialog.as_mut() {
            let response = dialog.show(ui);
            if let Some(result) = response.inner.dialog_response {
                self.spv_clear_dialog = None;
                match result {
                    ConfirmationStatus::Confirmed => {
                        match self.current_app_context().clear_spv_data() {
                            Ok(_) => {
                                self.spv_clear_message = Some(SpvClearMessage::Success(format!(
                                    "Cleared SPV data for {}. Reconnect to start a new sync.",
                                    self.current_network_label()
                                )));
                            }
                            Err(err) => {
                                self.spv_clear_message = Some(SpvClearMessage::Error(format!(
                                    "Failed to clear SPV data: {}",
                                    err
                                )));
                            }
                        }
                    }
                    ConfirmationStatus::Canceled => {
                        // No-op
                    }
                }
            }
        }
        AppAction::None
    }

    fn show_database_clear_confirmation(&mut self, ui: &mut Ui) -> AppAction {
        if let Some(dialog) = self.db_clear_dialog.as_mut() {
            let response = dialog.show(ui);
            if let Some(result) = response.inner.dialog_response {
                self.db_clear_dialog = None;
                match result {
                    ConfirmationStatus::Confirmed => {
                        match self.current_app_context().clear_network_database() {
                            Ok(_) => {
                                self.db_clear_message =
                                    Some(DatabaseClearMessage::Success(format!(
                                        "Cleared {} database. Restart or resync to rebuild state.",
                                        self.current_network_label()
                                    )));
                                return AppAction::Refresh;
                            }
                            Err(err) => {
                                self.db_clear_message = Some(DatabaseClearMessage::Error(format!(
                                    "Failed to clear database: {}",
                                    err
                                )));
                            }
                        }
                    }
                    ConfirmationStatus::Canceled => {
                        // No-op
                    }
                }
            }
        }
        AppAction::None
    }

    fn current_network_label(&self) -> &'static str {
        match self.current_network {
            Network::Dash => "Mainnet",
            Network::Testnet => "Testnet",
            Network::Devnet => "Devnet",
            Network::Regtest => "Local",
            _ => "this network",
        }
    }

    fn calculate_headers_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        if let Some(detailed) = &snapshot.detailed_progress {
            match &detailed.sync_stage {
                SyncStage::DownloadingHeaders { start, end } => {
                    // Respect restored checkpoints: show progress relative to the download window.
                    if end > start {
                        let window = (end - start) as f32;
                        let current = detailed.sync_progress.header_height;
                        let clamped = current.clamp(*start, *end) - start;
                        (clamped as f32 / window).clamp(0.0, 1.0)
                    } else {
                        0.0
                    }
                }
                SyncStage::ValidatingHeaders { .. }
                | SyncStage::StoringHeaders { .. }
                | SyncStage::DownloadingFilterHeaders { .. }
                | SyncStage::DownloadingFilters { .. }
                | SyncStage::DownloadingBlocks { .. }
                | SyncStage::Complete => 1.0,
                SyncStage::Failed(_) => 0.0,
                _ => 0.0,
            }
        } else if let Some(progress) = &snapshot.sync_progress {
            if progress.header_height == 0 {
                0.0
            } else {
                // Without detailed context fall back to comparing against masternode progress
                (progress.masternode_height as f32 / progress.header_height as f32).clamp(0.0, 1.0)
            }
        } else {
            0.0
        }
    }

    fn calculate_filter_headers_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        if let Some(detailed) = &snapshot.detailed_progress {
            if detailed.peer_best_height == 0 {
                return 0.0;
            }
            match &detailed.sync_stage {
                SyncStage::DownloadingFilterHeaders { current, target } => {
                    let current = *current;
                    let target = *target;
                    if target == 0 {
                        return 0.0;
                    }

                    let start = self
                        .filter_headers_stage_start
                        .unwrap_or(current)
                        .min(target);
                    let span = target.saturating_sub(start);
                    if span == 0 {
                        if current >= target { 1.0 } else { 0.0 }
                    } else {
                        let progress = current.saturating_sub(start);
                        (progress as f32 / span as f32).clamp(0.0, 1.0)
                    }
                }
                SyncStage::DownloadingFilters { .. }
                | SyncStage::DownloadingBlocks { .. }
                | SyncStage::Complete => (detailed.sync_progress.filter_header_height as f32
                    / detailed.peer_best_height as f32)
                    .clamp(0.0, 1.0),
                SyncStage::Failed(_) => 0.0,
                _ => 0.0,
            }
        } else {
            0.0
        }
    }

    fn calculate_filters_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        if let Some(detailed) = &snapshot.detailed_progress {
            match &detailed.sync_stage {
                SyncStage::DownloadingFilters { completed, total } => {
                    if *total == 0 {
                        0.0
                    } else {
                        (*completed as f32 / *total as f32).clamp(0.0, 1.0)
                    }
                }
                SyncStage::DownloadingBlocks { .. } | SyncStage::Complete => 1.0,
                SyncStage::Failed(_) => 0.0,
                _ => 0.0,
            }
        } else {
            0.0
        }
    }

    fn calculate_validating_headers_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        if snapshot.status == SpvStatus::Running {
            return 1.0;
        }

        if let Some(detailed) = &snapshot.detailed_progress {
            match &detailed.sync_stage {
                SyncStage::ValidatingHeaders { .. } | SyncStage::StoringHeaders { .. } => {
                    if detailed.peer_best_height == 0 {
                        0.0
                    } else {
                        let best_height = detailed.peer_best_height as f32;
                        let validated = detailed.sync_progress.masternode_height as f32;
                        (validated / best_height).clamp(0.0, 1.0)
                    }
                }
                SyncStage::DownloadingFilterHeaders { .. }
                | SyncStage::DownloadingFilters { .. }
                | SyncStage::DownloadingBlocks { .. }
                | SyncStage::Complete => 1.0,
                SyncStage::Failed(_) => 0.0,
                _ => 0.0,
            }
        } else if let Some(progress) = &snapshot.sync_progress {
            if progress.header_height == 0 {
                0.0
            } else {
                (progress.masternode_height as f32 / progress.header_height as f32).clamp(0.0, 1.0)
            }
        } else {
            0.0
        }
    }

    fn calculate_blocks_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        if snapshot.status == SpvStatus::Running {
            return 1.0;
        }

        if let Some(detailed) = &snapshot.detailed_progress {
            match &detailed.sync_stage {
                SyncStage::DownloadingBlocks { .. } => {
                    if detailed.peer_best_height == 0 {
                        0.0
                    } else {
                        let processed_height = detailed
                            .sync_progress
                            .last_synced_filter_height
                            .unwrap_or(0);
                        (processed_height as f32 / detailed.peer_best_height as f32).clamp(0.0, 1.0)
                    }
                }
                SyncStage::Complete => 1.0,
                SyncStage::Failed(_) => 0.0,
                _ => 0.0,
            }
        } else {
            0.0
        }
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

    fn spv_status_detail(&self, snapshot: &SpvStatusSnapshot) -> Option<String> {
        if let SpvStatus::Error = snapshot.status
            && let Some(err) = &snapshot.last_error
        {
            return Some(err.clone());
        }

        if let Some(progress) = snapshot.detailed_progress.as_ref() {
            return Some(Self::format_detailed_progress(progress));
        }

        snapshot.last_error.clone()
    }

    fn format_detailed_progress(progress: &DetailedSyncProgress) -> String {
        let mut message = match &progress.sync_stage {
            SyncStage::Connecting => "Connecting to peers".to_string(),
            SyncStage::QueryingPeerHeight => "Querying peer heights".to_string(),
            SyncStage::DownloadingHeaders { .. } => {
                format!(
                    "Headers: {} / {}",
                    progress.sync_progress.header_height, progress.peer_best_height,
                )
            }
            SyncStage::ValidatingHeaders { batch_size } => {
                format!(
                    "Masternode lists (batch {batch_size}) | Height {}",
                    progress.sync_progress.masternode_height
                )
            }
            SyncStage::StoringHeaders { batch_size } => {
                format!(
                    "Storing headers (batch {batch_size}) | Height {}",
                    progress.sync_progress.header_height
                )
            }
            SyncStage::Complete => "Sync complete".to_string(),
            SyncStage::Failed(reason) => format!("Failed: {reason}"),
            SyncStage::DownloadingFilterHeaders { current, target } => {
                format!("Filter headers: {current} / {target}")
            }
            SyncStage::DownloadingFilters { completed, total } => {
                format!("Filters: {completed} / {total}")
            }
            SyncStage::DownloadingBlocks { pending } => {
                format!("Blocks: {pending}")
            }
        };

        if progress.sync_progress.peer_count > 0 {
            message = format!("{message} | Peers: {}", progress.sync_progress.peer_count);
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
                .auto_shrink([true; 2])
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
