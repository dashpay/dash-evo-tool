use crate::app::AppAction;
use crate::backend_task::BackendTask;
use crate::backend_task::core::CoreTask;
use crate::backend_task::system_task::SystemTask;
use crate::config::Config;
use crate::context::AppContext;
use crate::context::connection_status::ConnectionStatus;
use crate::model::wallet::DerivationPathHelpers;
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
use dash_sdk::dash_spv::sync::{ProgressPercentage, SyncProgress as SpvSyncProgress, SyncState};
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

/// Renders DAPI endpoint status with appropriate color coding.
fn add_dapi_status_label(
    ui: &mut Ui,
    dapi_total: u16,
    dapi_available: bool,
    dapi_label: &str,
    dark_mode: bool,
) {
    ui.label("DAPI:");
    if dapi_total == 0 {
        ui.colored_label(DashColors::text_secondary(dark_mode), dapi_label);
    } else {
        let color = if dapi_available {
            DashColors::SUCCESS
        } else {
            DashColors::ERROR
        };
        ui.colored_label(color, dapi_label);
    }
}

pub struct NetworkChooserScreen {
    pub mainnet_app_context: Arc<AppContext>,
    pub testnet_app_context: Option<Arc<AppContext>>,
    pub devnet_app_context: Option<Arc<AppContext>>,
    pub local_app_context: Option<Arc<AppContext>>,
    pub local_network_dashmate_password: String,
    pub current_network: Network,
    pub recheck_time: Option<TimestampMillis>,
    custom_dash_qt_path: Option<PathBuf>,
    custom_dash_qt_error_message: Option<String>,
    overwrite_dash_conf: bool,
    disable_zmq: bool,
    developer_mode: bool,
    theme_preference: ThemeMode,
    should_reset_collapsing_states: bool,
    backend_modes: HashMap<Network, CoreBackendMode>,
    spv_progress_network: Option<Network>,
    headers_stage_start: Option<u32>,
    filter_headers_stage_start: Option<u32>,
    filters_stage_start: Option<u32>,
    blocks_stage_start: Option<u32>,
    blocks_target_height: u32,
    spv_clear_dialog: Option<ConfirmationDialog>,
    spv_clear_message: Option<SpvClearMessage>,
    db_clear_dialog: Option<ConfirmationDialog>,
    db_clear_message: Option<DatabaseClearMessage>,
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
        let use_local_spv_node = mainnet_app_context
            .db
            .get_use_local_spv_node()
            .unwrap_or(false);
        let auto_start_spv = mainnet_app_context.db.get_auto_start_spv().unwrap_or(false);
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
            recheck_time: None,
            custom_dash_qt_path,
            custom_dash_qt_error_message: None,
            overwrite_dash_conf,
            disable_zmq,
            developer_mode,
            theme_preference,
            should_reset_collapsing_states: true, // Start with collapsed state
            backend_modes,
            spv_progress_network: None,
            headers_stage_start: None,
            filter_headers_stage_start: None,
            filters_stage_start: None,
            blocks_stage_start: None,
            blocks_target_height: 0,
            spv_clear_dialog: None,
            spv_clear_message: None,
            db_clear_dialog: None,
            db_clear_message: None,
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
                    // TODO: SPV is currently hidden behind Developer Mode while still in development.
                    // Once SPV is production-ready, remove this developer_mode check and make SPV
                    // the default/primary connection method, with RPC as a fallback option.
                    let current_backend_mode = *self
                        .backend_modes
                        .entry(self.current_network)
                        .or_insert(CoreBackendMode::Rpc);

                    if self.developer_mode {
                        // Row 1: Connection Type (only shown in developer mode)
                        ui.label(
                            egui::RichText::new("Connection Type:")
                                .color(DashColors::text_primary(dark_mode)),
                        );

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

                        // Show experimental warning when SPV mode is selected
                        if current_backend_mode == CoreBackendMode::Spv {
                            ui.label(""); // Empty label for grid alignment
                            egui::Frame::new()
                                .fill(DashColors::WARNING.gamma_multiply(0.15))
                                .inner_margin(egui::Margin::symmetric(8, 4))
                                .stroke(egui::Stroke::new(1.0, DashColors::WARNING))
                                .corner_radius(4.0)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new("⚠")
                                                .color(DashColors::WARNING)
                                                .size(14.0),
                                        );
                                        ui.label(
                                            egui::RichText::new(
                                                "SPV mode is experimental and still in development",
                                            )
                                            .color(DashColors::WARNING)
                                            .size(12.0),
                                        );
                                    });
                                });
                            ui.end_row();
                        }
                    }

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

                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
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
                    });

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
                            tracing::error!("Failed to save config to .env: {e}");
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
                                tracing::error!(
                                    "Failed to re-init local RPC client and sdk: {}",
                                    e
                                );
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

            let ctx = self.current_app_context();
            let status = ctx.connection_status();
            let disable_zmq = status.disable_zmq();
            let rpc_online = status.rpc_online();
            let zmq_connected = status.zmq_connected();
            let spv_status = status.spv_status();
            let spv_connected = ConnectionStatus::spv_connected(spv_status);
            let snapshot = if current_backend_mode == CoreBackendMode::Spv {
                Some(ctx.spv_manager().status().clone())
            } else {
                None
            };
            let overall_connected = status.overall_connected();
            let dapi_total = status.dapi_total_endpoints();
            let dapi_available = status.dapi_available();
            let dapi_label = status.dapi_status_label();

            // Button on the left with status
            ui.horizontal(|ui| {
                if overall_connected {
                    if current_backend_mode == CoreBackendMode::Spv {
                        let is_stopping = spv_status == SpvStatus::Stopping;
                        let disconnect_button = egui::Button::new(
                            egui::RichText::new("Disconnect").color(DashColors::WHITE),
                        )
                        .fill(DashColors::ERROR)
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(Shape::RADIUS_MD)
                        .min_size(egui::vec2(120.0, 36.0));

                        if ui
                            .add_enabled(!is_stopping, disconnect_button)
                            .clicked()
                        {
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
                        let label = if disable_zmq {
                            "✅ Connected (RPC, ZMQ disabled)"
                        } else {
                            "✅ Connected (RPC + ZMQ)"
                        };
                        ui.colored_label(DashColors::DASH_BLUE, label);
                    }
                } else {
                    // Don't show Connect button for Local network in RPC mode
                    // (there's no Dash-Qt to start for local/regtest)
                    let show_connect_button = match current_backend_mode {
                        CoreBackendMode::Spv => true,
                        CoreBackendMode::Rpc => {
                            !rpc_online && self.current_network != Network::Regtest
                        }
                    };

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

            // TODO: SPV sync progress is hidden when developer mode is OFF.
            // Remove the developer_mode check once SPV is production-ready.
            if self.developer_mode
                && current_backend_mode == CoreBackendMode::Spv
                && let Some(snap) = snapshot.as_ref()
                && (snap.status == SpvStatus::Syncing || snap.status == SpvStatus::Starting)
            {
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(10.0);

                self.render_spv_sync_progress(ui, snap);
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            ui.vertical(|ui| {
                if current_backend_mode == CoreBackendMode::Rpc && !self.developer_mode {
                    ui.horizontal(|ui| {
                        ui.label("Core RPC:");
                        let rpc_color = if rpc_online {
                            DashColors::SUCCESS
                        } else {
                            DashColors::ERROR
                        };
                        let rpc_label = if rpc_online { "Connected" } else { "Disconnected" };
                        ui.colored_label(rpc_color, rpc_label);

                        ui.label(",");
                        ui.label("ZMQ:");
                        if disable_zmq {
                            ui.colored_label(DashColors::text_secondary(dark_mode), "Disabled");
                        } else {
                            let zmq_color = if zmq_connected {
                                DashColors::SUCCESS
                            } else {
                                DashColors::ERROR
                            };
                            let zmq_label = if zmq_connected { "Connected" } else { "Disconnected" };
                            ui.colored_label(zmq_color, zmq_label);
                        }

                        ui.label(",");
                        add_dapi_status_label(ui, dapi_total, dapi_available, &dapi_label, dark_mode);
                    });
                }

                if current_backend_mode == CoreBackendMode::Rpc && self.developer_mode {
                    ui.horizontal(|ui| {
                        ui.label("Dash Core RPC:");
                        let color = if rpc_online {
                            DashColors::SUCCESS
                        } else {
                            DashColors::ERROR
                        };
                        let label = if rpc_online { "Connected" } else { "Disconnected" };
                        ui.colored_label(color, label);
                    });

                    ui.horizontal(|ui| {
                        ui.label("ZMQ:");
                        if disable_zmq {
                            ui.colored_label(
                                DashColors::text_secondary(dark_mode),
                                "Disabled",
                            );
                        } else {
                            let color = if zmq_connected {
                                DashColors::SUCCESS
                            } else {
                                DashColors::ERROR
                            };
                            let label = if zmq_connected { "Connected" } else { "Disconnected" };
                            ui.colored_label(color, label);
                        }
                    });

                    ui.horizontal(|ui| {
                        add_dapi_status_label(ui, dapi_total, dapi_available, &dapi_label, dark_mode);
                    });
                }

                if current_backend_mode == CoreBackendMode::Spv {
                    ui.horizontal(|ui| {
                        ui.label("SPV:");
                        let color = if spv_connected {
                            DashColors::SUCCESS
                        } else {
                            DashColors::ERROR
                        };
                        ui.colored_label(color, spv_status.to_string());
                    });

                    ui.horizontal(|ui| {
                        add_dapi_status_label(ui, dapi_total, dapi_available, &dapi_label, dark_mode);
                    });
                }
            });
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

            // Reset to closed state when the screen is first opened
            if self.should_reset_collapsing_states {
                state.set_open(false);
                self.should_reset_collapsing_states = false;
            }

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

                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        ui.add_space(-6.0);
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
                    {
                        // Always update all contexts first to keep UI in sync
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

                        // Persist to config file (non-blocking for UI)
                        if let Ok(mut config) = Config::load() {
                            config.developer_mode = Some(self.developer_mode);
                            if let Err(e) = config.save() {
                                tracing::error!("Failed to save config: {e}");
                            }
                        }

                        // TODO: When developer mode is disabled, stop SPV and switch to RPC.
                        // Remove this block once SPV is production-ready.
                        if !self.developer_mode {
                            // Stop SPV and switch to RPC for all network contexts
                            self.mainnet_app_context.stop_spv();
                            if self.mainnet_app_context.core_backend_mode() == CoreBackendMode::Spv {
                                self.mainnet_app_context.set_core_backend_mode(CoreBackendMode::Rpc);
                            }
                            self.backend_modes.insert(Network::Dash, CoreBackendMode::Rpc);

                            if let Some(ref ctx) = self.testnet_app_context {
                                ctx.stop_spv();
                                if ctx.core_backend_mode() == CoreBackendMode::Spv {
                                    ctx.set_core_backend_mode(CoreBackendMode::Rpc);
                                }
                                self.backend_modes.insert(Network::Testnet, CoreBackendMode::Rpc);
                            }
                            if let Some(ref ctx) = self.devnet_app_context {
                                ctx.stop_spv();
                                if ctx.core_backend_mode() == CoreBackendMode::Spv {
                                    ctx.set_core_backend_mode(CoreBackendMode::Rpc);
                                }
                                self.backend_modes.insert(Network::Devnet, CoreBackendMode::Rpc);
                            }
                            if let Some(ref ctx) = self.local_app_context {
                                ctx.stop_spv();
                                if ctx.core_backend_mode() == CoreBackendMode::Spv {
                                    ctx.set_core_backend_mode(CoreBackendMode::Rpc);
                                }
                                self.backend_modes.insert(Network::Regtest, CoreBackendMode::Rpc);
                            }
                        }
                    }
                    ui.label(
                        egui::RichText::new("Enable advanced features")
                            .color(DashColors::TEXT_SECONDARY)
                            .italics(),
                    );
                });

                // Developer-only tools
                if self.developer_mode {
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new("Developer Tools")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(6.0);

                    ui.horizontal(|ui| {
                        if ui.button("Clear Platform Addresses").clicked() {
                            // Clear from database
                            let current_context = self.current_app_context();
                            match current_context
                                .db
                                .clear_all_platform_addresses(&current_context.network)
                            {
                                Ok(count) => {
                                    tracing::info!(
                                        "Cleared {} platform addresses from database",
                                        count
                                    );
                                    // Also clear from in-memory wallets
                                    if let Ok(wallets) = current_context.wallets.read() {
                                        for wallet_arc in wallets.values() {
                                            if let Ok(mut wallet) = wallet_arc.write() {
                                                // Clear platform address info
                                                wallet.platform_address_info.clear();

                                                // Remove platform addresses from known_addresses
                                                wallet.known_addresses.retain(|_, path| {
                                                    !path.is_platform_payment(current_context.network)
                                                });

                                                // Remove platform addresses from watched_addresses
                                                wallet.watched_addresses.retain(|path, _| {
                                                    !path.is_platform_payment(current_context.network)
                                                });

                                                // Remove platform addresses from address_balances
                                                let platform_addrs: Vec<_> = wallet
                                                    .address_balances
                                                    .keys()
                                                    .filter(|addr| {
                                                        // Check if this address was a platform address
                                                        // by seeing if it's not in known_addresses anymore
                                                        !wallet.known_addresses.contains_key(*addr)
                                                    })
                                                    .cloned()
                                                    .collect();
                                                for addr in platform_addrs {
                                                    wallet.address_balances.remove(&addr);
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Failed to clear platform addresses: {}", e);
                                }
                            }
                        }
                        ui.label(
                            egui::RichText::new("Removes all Platform addresses for testing sync")
                                .color(DashColors::TEXT_SECONDARY)
                                .italics(),
                        );
                    });
                }

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
                        match self
                            .mainnet_app_context
                            .db
                            .update_close_dash_qt_on_exit(self.close_dash_qt_on_exit)
                        {
                            Ok(_) => {
                                tracing::debug!(
                                    "close_dash_qt_on_exit setting saved: {}",
                                    self.close_dash_qt_on_exit
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Failed to save close_dash_qt_on_exit setting: {:?}",
                                    e
                                );
                            }
                        }
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

                // TODO: SPV settings are hidden when developer mode is OFF.
                // Remove the developer_mode checks once SPV is production-ready.
                if self.developer_mode {
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
                }

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
                // TODO: SPV maintenance is hidden when developer mode is OFF.
                // Remove the developer_mode check once SPV is production-ready.
                if self.developer_mode {
                    let current_backend_mode = self.current_app_context().core_backend_mode();
                    if current_backend_mode == CoreBackendMode::Spv {
                        let snapshot = self.current_app_context().spv_manager().status();
                        ui.add_space(12.0);
                        ui.separator();
                        ui.add_space(12.0);
                        app_action |= self.render_spv_maintenance_controls(ui, &snapshot);
                    }
                }
            });
        });

        app_action
    }

    /// Rebuild all SPV progress tracking state from the current snapshot.
    /// Called once when the active network changes so that stale values from
    /// one network don't leak into another, while preserving already-synced
    /// progress from the new network's SPV manager.
    fn rebuild_spv_progress_state(&mut self, snapshot: &SpvStatusSnapshot) {
        self.headers_stage_start = None;
        self.filter_headers_stage_start = None;
        self.filters_stage_start = None;
        self.blocks_stage_start = None;
        self.blocks_target_height = 0;

        // Seed from the new network's sync_progress so bars don't jump to 0.
        if let Some(progress) = &snapshot.sync_progress {
            if let Ok(headers) = progress.headers() {
                self.blocks_target_height = self.blocks_target_height.max(headers.target_height());
            }
            if let Ok(blocks) = progress.blocks() {
                self.blocks_target_height = self.blocks_target_height.max(blocks.last_processed());
                if blocks.state() == SyncState::Syncing {
                    self.blocks_stage_start = Some(blocks.last_processed());
                }
            }
        }
    }

    fn render_spv_sync_progress(&mut self, ui: &mut Ui, snapshot: &SpvStatusSnapshot) {
        // Rebuild progress state when the network changes.
        if self.spv_progress_network != Some(self.current_network) {
            self.rebuild_spv_progress_state(snapshot);
            self.spv_progress_network = Some(self.current_network);
        }

        if let Some(progress) = &snapshot.sync_progress {
            // Track headers download window start for checkpoint-aware progress
            if let Ok(headers) = progress.headers() {
                if headers.state() == SyncState::Syncing {
                    let current = headers.current_height();
                    let target = headers.target_height();
                    let baseline = current.min(target);
                    if let Some(existing) = self.headers_stage_start {
                        self.headers_stage_start = Some(existing.min(target));
                    } else {
                        self.headers_stage_start = Some(baseline);
                    }
                } else {
                    self.headers_stage_start = None;
                }
            } else {
                self.headers_stage_start = None;
            }

            // Track filter headers download window start
            if let Ok(fh) = progress.filter_headers() {
                if fh.state() == SyncState::Syncing {
                    let current = fh.current_height();
                    let target = fh.target_height();
                    let baseline = current.min(target);
                    if let Some(existing) = self.filter_headers_stage_start {
                        self.filter_headers_stage_start = Some(existing.min(target));
                    } else {
                        self.filter_headers_stage_start = Some(baseline);
                    }
                } else {
                    self.filter_headers_stage_start = None;
                }
            } else {
                self.filter_headers_stage_start = None;
            }

            // Track filters download window start
            if let Ok(filters) = progress.filters() {
                if filters.state() == SyncState::Syncing {
                    let current = filters.current_height();
                    let target = filters.target_height();
                    let baseline = current.min(target);
                    if let Some(existing) = self.filters_stage_start {
                        self.filters_stage_start = Some(existing.min(target));
                    } else {
                        self.filters_stage_start = Some(baseline);
                    }
                } else {
                    self.filters_stage_start = None;
                }
            } else {
                self.filters_stage_start = None;
            }

            // Capture target height from headers and blocks (only increases).
            if let Ok(headers) = progress.headers() {
                self.blocks_target_height = self.blocks_target_height.max(headers.target_height());
            }
            if let Ok(blocks) = progress.blocks() {
                // last_processed is a lower bound for chain height
                self.blocks_target_height = self.blocks_target_height.max(blocks.last_processed());

                if blocks.state() == SyncState::Syncing && self.blocks_stage_start.is_none() {
                    self.blocks_stage_start = Some(blocks.last_processed());
                }
                if matches!(blocks.state(), SyncState::Synced | SyncState::Error) {
                    self.blocks_stage_start = None;
                }
            }
        }

        let dark_mode = ui.ctx().style().visuals.dark_mode;

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

                egui::Grid::new("spv_sync_info")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        if let Some(detail) = self.spv_status_detail(snapshot) {
                            ui.label(
                                egui::RichText::new("Status:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(detail);
                            ui.end_row();
                        }

                        if snapshot.sync_progress.is_some() {
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

                            // Masternode Lists progress
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

                            // Blocks progress
                            ui.label(
                                egui::RichText::new("Blocks:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            let blocks_progress = self.calculate_blocks_progress(snapshot);
                            let blocks_text = snapshot
                                .sync_progress
                                .as_ref()
                                .and_then(|p| p.blocks().ok())
                                .map(|b| {
                                    format!(
                                        "{} / {}",
                                        b.last_processed(),
                                        self.blocks_target_height
                                    )
                                })
                                .unwrap_or_default();
                            ui.add(egui::ProgressBar::new(blocks_progress).text(blocks_text));
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
        let Some(progress) = &snapshot.sync_progress else {
            return 0.0;
        };
        let Ok(headers) = progress.headers() else {
            return 0.0;
        };
        match headers.state() {
            SyncState::Syncing => {
                let target = headers.target_height();
                if target == 0 {
                    return 0.0;
                }
                // Use download window to show progress relative to remaining work,
                // so checkpoint-resumed syncs start near 0% rather than jumping ahead.
                let start = self
                    .headers_stage_start
                    .unwrap_or(headers.current_height())
                    .min(target);
                let span = target.saturating_sub(start);
                if span == 0 {
                    if headers.current_height() >= target {
                        1.0
                    } else {
                        0.0
                    }
                } else {
                    let done = headers.current_height().saturating_sub(start);
                    (done as f32 / span as f32).clamp(0.0, 1.0)
                }
            }
            SyncState::Synced => 1.0,
            SyncState::Initializing
            | SyncState::WaitingForConnections
            | SyncState::WaitForEvents
            | SyncState::Error => 0.0,
        }
    }

    fn calculate_filter_headers_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        let Some(progress) = &snapshot.sync_progress else {
            return 0.0;
        };
        let Ok(fh) = progress.filter_headers() else {
            return 0.0;
        };
        match fh.state() {
            SyncState::Syncing => {
                let target = fh.target_height();
                if target == 0 {
                    return 0.0;
                }
                let start = self
                    .filter_headers_stage_start
                    .unwrap_or(fh.current_height())
                    .min(target);
                let span = target.saturating_sub(start);
                if span == 0 {
                    if fh.current_height() >= target {
                        1.0
                    } else {
                        0.0
                    }
                } else {
                    let done = fh.current_height().saturating_sub(start);
                    (done as f32 / span as f32).clamp(0.0, 1.0)
                }
            }
            SyncState::Synced => 1.0,
            SyncState::Initializing
            | SyncState::WaitingForConnections
            | SyncState::WaitForEvents
            | SyncState::Error => 0.0,
        }
    }

    fn calculate_filters_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        let Some(progress) = &snapshot.sync_progress else {
            return 0.0;
        };
        let Ok(filters) = progress.filters() else {
            return 0.0;
        };
        match filters.state() {
            SyncState::Syncing => {
                let target = filters.target_height();
                if target == 0 {
                    return 0.0;
                }
                // Use windowed progress so checkpoint-resumed syncs start near 0%.
                // current_height is the storage tip (not downloaded() which is a
                // session-level count).
                let start = self
                    .filters_stage_start
                    .unwrap_or(filters.current_height())
                    .min(target);
                let span = target.saturating_sub(start);
                if span == 0 {
                    if filters.current_height() >= target {
                        1.0
                    } else {
                        0.0
                    }
                } else {
                    let done = filters.current_height().saturating_sub(start);
                    (done as f32 / span as f32).clamp(0.0, 1.0)
                }
            }
            SyncState::Synced => 1.0,
            SyncState::Initializing
            | SyncState::WaitingForConnections
            | SyncState::WaitForEvents
            | SyncState::Error => 0.0,
        }
    }

    fn calculate_validating_headers_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        if snapshot.status == SpvStatus::Running {
            return 1.0;
        }
        let Some(progress) = &snapshot.sync_progress else {
            return 0.0;
        };
        let Ok(mn) = progress.masternodes() else {
            return 0.0;
        };
        match mn.state() {
            SyncState::Syncing => {
                let target = mn.target_height();
                if target == 0 {
                    return 0.0;
                }
                (mn.current_height() as f32 / target as f32).clamp(0.0, 1.0)
            }
            SyncState::Synced => 1.0,
            SyncState::Initializing
            | SyncState::WaitingForConnections
            | SyncState::WaitForEvents
            | SyncState::Error => 0.0,
        }
    }

    fn calculate_blocks_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        if snapshot.status == SpvStatus::Running {
            return 1.0;
        }
        let Some(progress) = &snapshot.sync_progress else {
            return 0.0;
        };
        let Ok(blocks) = progress.blocks() else {
            return 0.0;
        };
        if blocks.state() == SyncState::Synced {
            return 1.0;
        }
        // Use last_processed height relative to the tracked target height.
        // Don't branch on SyncState — blocks can transiently leave Syncing
        // (e.g. WaitForEvents between batches) while still making progress.
        let target = self.blocks_target_height;
        if target == 0 {
            return 0.0;
        }
        let current = blocks.last_processed();
        let start = self.blocks_stage_start.unwrap_or(current).min(target);
        let span = target.saturating_sub(start);
        if span == 0 {
            if current >= target { 1.0 } else { 0.0 }
        } else {
            let done = current.saturating_sub(start);
            (done as f32 / span as f32).clamp(0.0, 1.0)
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

        if let Some(progress) = snapshot.sync_progress.as_ref() {
            return Some(Self::format_sync_progress(
                progress,
                snapshot.connected_peers,
            ));
        }

        snapshot.last_error.clone()
    }

    fn format_sync_progress(progress: &SpvSyncProgress, connected_peers: usize) -> String {
        // Check each manager's state to determine what to display,
        // preferring later pipeline stages.
        let stage_message = if let Ok(blocks) = progress.blocks()
            && blocks.state() == SyncState::Syncing
        {
            format!(
                "Blocks: {} requested, {} processed",
                blocks.requested(),
                blocks.processed()
            )
        } else if let Ok(filters) = progress.filters()
            && filters.state() == SyncState::Syncing
        {
            format!(
                "Filters: {} / {}",
                filters.current_height(),
                filters.target_height()
            )
        } else if let Ok(fh) = progress.filter_headers()
            && fh.state() == SyncState::Syncing
        {
            format!(
                "Filter headers: {} / {}",
                fh.current_height(),
                fh.target_height()
            )
        } else if let Ok(mn) = progress.masternodes()
            && mn.state() == SyncState::Syncing
        {
            format!(
                "Masternode lists: {} diffs | Height {} / {}",
                mn.diffs_processed(),
                mn.current_height(),
                mn.target_height()
            )
        } else if let Ok(headers) = progress.headers()
            && headers.state() == SyncState::Syncing
        {
            format!(
                "Headers: {} / {}",
                headers.current_height(),
                headers.target_height()
            )
        } else if progress.is_synced() {
            "Sync complete".to_string()
        } else {
            match progress.state() {
                SyncState::WaitingForConnections => "Connecting to peers".to_string(),
                SyncState::WaitForEvents => "Querying peer heights".to_string(),
                SyncState::Error => "Sync error".to_string(),
                SyncState::Initializing | SyncState::Syncing | SyncState::Synced => {
                    "Syncing...".to_string()
                }
            }
        };

        if connected_peers > 0 {
            format!("{stage_message} | Peers: {connected_peers}")
        } else {
            stage_message
        }
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
                    .unwrap_or_default();
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
