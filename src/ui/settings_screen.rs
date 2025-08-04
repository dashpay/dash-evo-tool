use crate::app::AppAction;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::spv::{SpvTask, SpvTaskResult};
use crate::backend_task::spv_v2::{SpvTaskResultV2, SpvTaskV2};
use crate::backend_task::system_task::SystemTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::config::Config;
use crate::context::AppContext;
use crate::model::settings::ConnectionMode;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::{StyledCard, StyledCheckbox, island_central_panel};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::{DashColors, Shape, ThemeMode};
use crate::ui::{RootScreenType, ScreenLike};
use crate::utils::path::format_path_for_display;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use dash_spv::types::SyncPhaseInfo;
use eframe::egui::{self, Context, Ui};
use egui::Vec2;
use num_format::{Locale, ToFormattedString};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct SettingsScreen {
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
    connection_mode: ConnectionMode,
    // SPV sync state
    spv_is_syncing: bool,
    spv_current_height: u32,
    spv_target_height: u32,
    spv_sync_progress: f32,
    spv_last_error: Option<String>,
    spv_is_initialized: bool,
    spv_last_progress_check: TimestampMillis,
    spv_phase_info: Option<SyncPhaseInfo>,
    // Checkpoint selection
    spv_selected_checkpoint: u32, // 0 = genesis, u32::MAX = latest
}

impl SettingsScreen {
    fn update_all_connection_modes(&mut self) {
        // Update connection mode in context
        if let Err(e) = self
            .current_app_context()
            .set_connection_mode(self.connection_mode)
        {
            self.spv_last_error = Some(format!("Failed to update connection mode: {}", e));
        }

        // Update all contexts
        self.mainnet_app_context
            .set_connection_mode(self.connection_mode)
            .ok();
        if let Some(ref ctx) = self.testnet_app_context {
            ctx.set_connection_mode(self.connection_mode).ok();
        }
        if let Some(ref ctx) = self.devnet_app_context {
            ctx.set_connection_mode(self.connection_mode).ok();
        }
        if let Some(ref ctx) = self.local_app_context {
            ctx.set_connection_mode(self.connection_mode).ok();
        }
    }

    fn render_spv_controls(&mut self, ui: &mut Ui, _action: &mut AppAction) {
        // Show progress if we're initialized and have any sync data
        if self.spv_is_initialized && (self.spv_target_height > 0 || self.spv_phase_info.is_some())
        {
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(10.0);

            self.render_spv_sync_progress(ui);
        }

        if let Some(error) = &self.spv_last_error {
            ui.add_space(12.0);
            egui::Frame::new()
                .fill(DashColors::ERROR.linear_multiply(0.1))
                .stroke(egui::Stroke::new(1.0, DashColors::ERROR))
                .corner_radius(Shape::RADIUS_SM)
                .inner_margin(12.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("⚠").color(DashColors::ERROR));
                        ui.label(
                            egui::RichText::new(format!("Error: {}", error))
                                .color(DashColors::ERROR),
                        );
                    });
                });
        }
    }

    fn render_spv_sync_progress(&self, ui: &mut Ui) {
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

                // Add progress bar
                // Calculate true blockchain sync progress
                let progress = if self.spv_target_height > 0 {
                    (self.spv_current_height as f32 / self.spv_target_height as f32)
                } else {
                    0.0_f32
                };

                let progress_bar = egui::ProgressBar::new(progress)
                    .show_percentage();

                ui.add(progress_bar);
                ui.add_space(8.0);

                // Display raw data
                egui::Grid::new("spv_raw_status")
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        // Phase info if available
                        if let Some(ref phase_info) = self.spv_phase_info {
                            // Phase (first)
                            ui.label(
                                egui::RichText::new("Phase:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(&phase_info.phase_name);
                            ui.end_row();

                            // Synced height
                            ui.label(
                                egui::RichText::new("Synced:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            // Use spv_current_height which contains the correct blockchain height
                            // phase_info.current_position seems to be incorrectly doubled when using checkpoints
                            let synced_value = self.spv_current_height;
                            ui.label(format!(
                                "{} / {}",
                                synced_value.to_formatted_string(&Locale::en),
                                self.spv_target_height.to_formatted_string(&Locale::en)
                            ));
                            ui.end_row();

                            // Rate
                            if phase_info.rate > 0.0 {
                                ui.label(
                                    egui::RichText::new("Rate:")
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                                // Use rate_units if available, otherwise default to "/sec"
                                let rate_display = if let Some(ref units) = phase_info.rate_units {
                                    format!("{:.1} {}", phase_info.rate, units)
                                } else {
                                    format!("{:.1}/sec", phase_info.rate)
                                };
                                ui.label(rate_display);
                                ui.end_row();
                            }

                            // // Details (last)
                            // if let Some(ref details) = phase_info.details {
                            //     ui.label(
                            //         egui::RichText::new("Details:")
                            //             .color(DashColors::text_secondary(dark_mode)),
                            //     );
                            //     ui.label(details);
                            //     ui.end_row();
                            // }
                        } else {
                            // If no phase info, show basic info
                            ui.label(
                                egui::RichText::new("Phase:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label("Initializing");
                            ui.end_row();

                            ui.label(
                                egui::RichText::new("Target Height:")
                                    .color(DashColors::text_secondary(dark_mode)),
                            );
                            ui.label(self.spv_target_height.to_formatted_string(&Locale::en));
                            ui.end_row();
                        }
                    });
            });
    }

    pub fn new(
        mainnet_app_context: &Arc<AppContext>,
        testnet_app_context: Option<&Arc<AppContext>>,
        devnet_app_context: Option<&Arc<AppContext>>,
        local_app_context: Option<&Arc<AppContext>>,
        current_network: Network,
        overwrite_dash_conf: bool,
        connection_mode: ConnectionMode,
    ) -> Self {
        tracing::info!("1");
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
        // Use the passed connection_mode parameter instead of reading from settings
        // This ensures we use the value loaded at app startup

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
            connection_mode,
            spv_is_syncing: false,
            spv_current_height: 0,
            spv_target_height: 0,
            spv_sync_progress: 0.0,
            spv_last_error: None,
            spv_is_initialized: false,
            spv_last_progress_check: 0,
            spv_phase_info: None,
            spv_selected_checkpoint: match current_network {
                Network::Dash => 1_900_000, // Default to most recent mainnet checkpoint
                Network::Testnet => 1_600_000, // Default to most recent testnet checkpoint
                _ => 0,                     // Genesis for other networks
            },
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
            .db
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

                    let connection_text = match self.connection_mode {
                        ConnectionMode::Spv => "SPV Client",
                        ConnectionMode::Core => "Dash Core RPC",
                    };

                    egui::ComboBox::from_id_salt("connection_mode_selector")
                        .selected_text(connection_text)
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(
                                    &mut self.connection_mode,
                                    ConnectionMode::Spv,
                                    "SPV Client",
                                )
                                .changed()
                            {
                                self.update_all_connection_modes();
                            }
                            if ui
                                .selectable_value(
                                    &mut self.connection_mode,
                                    ConnectionMode::Core,
                                    "Dash Core RPC",
                                )
                                .changed()
                            {
                                self.update_all_connection_modes();
                            }
                        });

                    ui.end_row();

                    // Row 2: SPV Checkpoint (only show for SPV mode)
                    if self.connection_mode == ConnectionMode::Spv {
                        ui.label(
                            egui::RichText::new("SPV Checkpoint:")
                                .color(DashColors::text_primary(dark_mode)),
                        );

                        // Determine checkpoint options based on network
                        let checkpoint_text = if self.spv_selected_checkpoint == 0 {
                            "Genesis Block"
                        } else {
                            // Format specific checkpoint heights
                            match self.current_network {
                                Network::Dash => match self.spv_selected_checkpoint {
                                    1_900_000 => "Height 1,900,000 (2023)",
                                    1_500_000 => "Height 1,500,000 (2022)",
                                    971_300 => "Height 971,300 (2020)",
                                    657_000 => "Height 657,000 (2019)",
                                    523_412 => "Height 523,412 (2018)",
                                    407_813 => "Height 407,813 (2018)",
                                    312_668 => "Height 312,668 (2017)",
                                    216_000 => "Height 216,000 (2017)",
                                    107_996 => "Height 107,996 (2016)",
                                    4_991 => "Height 4,991 (2014)",
                                    _ => "Custom Height",
                                },
                                Network::Testnet => match self.spv_selected_checkpoint {
                                    1_600_000 => "Height 1,600,000",
                                    1_480_000 => "Height 1,480,000",
                                    1_350_000 => "Height 1,350,000",
                                    1_270_000 => "Height 1,270,000",
                                    851_000 => "Height 851,000",
                                    797_400 => "Height 797,400",
                                    500_000 => "Height 500,000",
                                    _ => "Custom Height",
                                },
                                _ => "Custom Height",
                            }
                        };

                        let checkpoint_combo = egui::ComboBox::from_id_salt("checkpoint_selector")
                            .selected_text(checkpoint_text)
                            .width(200.0);

                        // Check if currently connected via SPV
                        let is_spv_connected = {
                            let ctx = self.current_app_context();
                            if let Ok(spv_manager) = ctx.spv_manager_v2.try_read() {
                                spv_manager.is_initialized() || spv_manager.is_syncing
                            } else {
                                false
                            }
                        };

                        let response = ui.add_enabled_ui(!is_spv_connected, |ui| {
                            checkpoint_combo.show_ui(ui, |ui| {
                                // Network-specific checkpoints
                                match self.current_network {
                                    Network::Dash => {
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 1_900_000, "Height 1,900,000 (2023)");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 1_500_000, "Height 1,500,000 (2022)");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 971_300, "Height 971,300 (2020)");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 657_000, "Height 657,000 (2019)");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 523_412, "Height 523,412 (2018)");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 407_813, "Height 407,813 (2018)");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 312_668, "Height 312,668 (2017)");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 216_000, "Height 216,000 (2017)");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 107_996, "Height 107,996 (2016)");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 4_991, "Height 4,991 (2014)");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 0, "Genesis Block");
                                    }
                                    Network::Testnet => {
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 1_600_000, "Height 1,600,000");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 1_480_000, "Height 1,480,000");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 1_350_000, "Height 1,350,000");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 1_270_000, "Height 1,270,000");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 851_000, "Height 851,000");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 797_400, "Height 797,400");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 500_000, "Height 500,000");
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 0, "Genesis Block");
                                    }
                                    _ => {
                                        // For other networks, only show genesis
                                        ui.selectable_value(&mut self.spv_selected_checkpoint, 0, "Genesis Block");
                                    }
                                }
                            });
                        });

                        if is_spv_connected {
                            response.response.on_hover_text("Disconnect from SPV to change checkpoint");
                        } else {
                            response.response.on_hover_text("Select a checkpoint to speed up initial sync. More recent checkpoints sync faster.");
                        }

                        ui.end_row();
                    }

                    // Row 3: Network
                    ui.label(
                        egui::RichText::new("Network:").color(DashColors::text_primary(dark_mode)),
                    );

                    // Check if currently connected via SPV (only SPV restricts network switching)
                    let is_spv_connected = if self.connection_mode == ConnectionMode::Spv {
                        let ctx = self.current_app_context();
                        if let Ok(spv_manager) = ctx.spv_manager_v2.try_read() {
                            spv_manager.is_initialized() || spv_manager.is_syncing
                        } else {
                            false
                        }
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
                            if ui
                                .selectable_value(
                                    &mut self.current_network,
                                    Network::Testnet,
                                    "Testnet",
                                )
                                .clicked()
                            {
                                app_action = AppAction::SwitchNetwork(Network::Testnet);
                            }
                            if ui
                                .selectable_value(
                                    &mut self.current_network,
                                    Network::Devnet,
                                    "Devnet",
                                )
                                .clicked()
                            {
                                app_action = AppAction::SwitchNetwork(Network::Devnet);
                            }
                            if ui
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
            if self.current_network == Network::Regtest
                && self.connection_mode == ConnectionMode::Core
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

                    if ui.button("Save").clicked() {
                        // Save the password to config
                        if let Ok(mut config) = Config::load() {
                            if let Some(local_cfg) =
                                config.config_for_network(Network::Regtest).clone()
                            {
                                let updated_local_config = local_cfg.update_core_rpc_password(
                                    self.local_network_dashmate_password.clone(),
                                );
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
                                        let mut cfg_lock =
                                            local_app_context.config.write().unwrap();
                                        *cfg_lock = updated_local_config;
                                    }

                                    // Re-init the client & sdk from the updated config
                                    if let Err(e) =
                                        Arc::clone(local_app_context).reinit_core_client_and_sdk()
                                    {
                                        eprintln!(
                                            "Failed to re-init local RPC client and sdk: {}",
                                            e
                                        );
                                    } else {
                                        // Trigger SwitchNetworks
                                        app_action = AppAction::SwitchNetwork(Network::Regtest);
                                    }
                                }
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
            ui.add_space(20.0);

            // Check connection status
            let (is_connected, is_syncing) = match self.connection_mode {
                ConnectionMode::Core => (self.check_network_status(self.current_network), false),
                ConnectionMode::Spv => {
                    // Use UI state first for immediate feedback
                    if self.spv_is_initialized {
                        (true, self.spv_is_syncing)
                    } else {
                        // Fall back to checking actual SPV manager V2 state
                        let ctx = self.current_app_context();
                        if let Ok(spv_manager) = ctx.spv_manager_v2.try_read() {
                            (spv_manager.is_initialized(), spv_manager.is_syncing)
                        } else {
                            (false, false)
                        }
                    }
                }
            };

            // Button on the left with status
            ui.horizontal(|ui| {
                if is_connected {
                    if self.connection_mode == ConnectionMode::Spv {
                        let disconnect_button = egui::Button::new(
                            egui::RichText::new("Disconnect").color(DashColors::WHITE),
                        )
                        .fill(DashColors::ERROR)
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(Shape::RADIUS_MD)
                        .min_size(egui::vec2(120.0, 36.0));

                        if ui.add(disconnect_button).clicked() {
                            tracing::info!("Disconnect clicked - stopping SPV");
                            app_action =
                                AppAction::BackendTask(BackendTask::SpvTaskV2(SpvTaskV2::Stop));
                            // Reset UI state immediately
                            self.spv_is_initialized = false;
                            self.spv_is_syncing = false;
                            self.spv_sync_progress = 0.0;
                            self.spv_current_height = 0;
                            self.spv_target_height = 0;
                            self.spv_last_error = None;
                            self.spv_phase_info = None;
                            self.spv_last_progress_check = 0;  // Reset progress check timer
                        }

                        // Show sync status next to button
                        ui.add_space(12.0);
                        // Check if we're in the initialization phase (no progress yet)
                        let is_initializing = self.spv_is_initialized
                            && self.spv_current_height == 0
                            && self.spv_phase_info.is_none();

                        if is_initializing {
                            // Show initialization status
                            ui.style_mut().visuals.widgets.inactive.fg_stroke.color =
                                DashColors::DASH_BLUE;
                            ui.style_mut().visuals.widgets.hovered.fg_stroke.color =
                                DashColors::DASH_BLUE;
                            ui.style_mut().visuals.widgets.active.fg_stroke.color =
                                DashColors::DASH_BLUE;
                            ui.spinner();
                            ui.label(egui::RichText::new("Initializing SPV client..."));
                        } else if self.spv_is_initialized
                            && (self.spv_is_syncing
                                || (self.spv_target_height > 0 && self.spv_sync_progress < 100.0))
                        {
                            // Show syncing status
                            ui.style_mut().visuals.widgets.inactive.fg_stroke.color =
                                DashColors::DASH_BLUE;
                            ui.style_mut().visuals.widgets.hovered.fg_stroke.color =
                                DashColors::DASH_BLUE;
                            ui.style_mut().visuals.widgets.active.fg_stroke.color =
                                DashColors::DASH_BLUE;
                            ui.spinner();
                            ui.label(egui::RichText::new("Syncing..."));
                        } else if self.spv_sync_progress >= 100.0 {
                            ui.colored_label(DashColors::SUCCESS, "✓ Fully Synced");
                        }
                    } else {
                        // For Core mode, just show status since it can switch networks freely
                        ui.colored_label(DashColors::DASH_BLUE, "✓ Connected");
                    }
                } else {
                    let connect_button =
                        egui::Button::new(egui::RichText::new("Connect").color(DashColors::WHITE))
                            .fill(DashColors::DASH_BLUE)
                            .stroke(egui::Stroke::NONE)
                            .corner_radius(Shape::RADIUS_MD)
                            .min_size(egui::vec2(120.0, 36.0));

                    if ui.add(connect_button).clicked() {
                        if self.connection_mode == ConnectionMode::Spv {
                            // Use the selected checkpoint height
                            let checkpoint_height = self.spv_selected_checkpoint;
                            tracing::info!(
                                "Connecting to SPV with checkpoint height: {}",
                                checkpoint_height
                            );
                            // Use SPV V2 for better concurrency
                            app_action = AppAction::BackendTask(BackendTask::SpvTaskV2(
                                SpvTaskV2::InitializeAndSync { checkpoint_height },
                            ));
                            // Set immediate UI feedback
                            self.spv_is_initialized = true;
                            self.spv_is_syncing = true;
                            self.spv_sync_progress = 0.0;
                            self.spv_current_height = 0;
                            self.spv_target_height = 0;
                            self.spv_last_error = None;
                            self.spv_phase_info = None;
                        } else {
                            // Core mode connect
                            let settings =
                                self.current_app_context().db.get_settings().ok().flatten();
                            let (custom_path, overwrite) = settings
                                .map(|(_, _, _, custom_path, overwrite, _, _)| {
                                    (custom_path, overwrite)
                                })
                                .unwrap_or((None, true));
                            if let Some(dash_qt_path) = custom_path {
                                app_action = AppAction::BackendTask(BackendTask::CoreTask(
                                    CoreTask::StartDashQT(
                                        self.current_network,
                                        dash_qt_path,
                                        overwrite,
                                    ),
                                ));
                            }
                        }
                    }
                }
            });

            // SPV sync controls (only show when SPV is selected)
            if self.connection_mode == ConnectionMode::Spv {
                self.render_spv_controls(ui, &mut app_action);
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
                    egui::Rounding::from(4.0),
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
                    if ui.button("Select File").clicked() {
                        if let Some(path) = rfd::FileDialog::new().pick_file() {
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
                    ui.colored_label(DashColors::ERROR, error);
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

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if StyledCheckbox::new(&mut self.developer_mode, "Developer mode")
                        .show(ui)
                        .clicked()
                    {
                        if let Ok(mut config) = Config::load() {
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
                    }
                    ui.label(
                        egui::RichText::new("Enable advanced features")
                            .color(DashColors::TEXT_SECONDARY)
                            .italics(),
                    );
                });
            });
        });

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

impl ScreenLike for SettingsScreen {
    fn refresh_on_arrival(&mut self) {
        tracing::info!("SettingsScreen::refresh_on_arrival called");

        // Reset collapsing states when arriving at this screen
        // This ensures dropdowns are closed when navigating back
        self.should_reset_collapsing_states = true;

        // Reload settings from database to ensure we have the latest values
        if let Ok(Some(settings)) = self.current_app_context().get_settings() {
            self.custom_dash_qt_path = settings.dash_qt_path;
            self.overwrite_dash_conf = settings.overwrite_dash_conf;
            self.theme_preference = settings.theme_mode;
            self.connection_mode = settings.connection_mode;
        }

        // Reset SPV state for the current network
        // This ensures we show the correct connection status when switching networks
        if self.connection_mode == ConnectionMode::Spv {
            // Only refresh if we're not in the middle of a connection attempt
            // This prevents overwriting the UI state during initialization
            if !self.spv_is_initialized {
                // Get the current app context's SPV manager state
                let ctx = self.current_app_context();

                tracing::debug!(
                    "Refreshing SPV state for network: {:?}, ctx network: {:?}",
                    self.current_network,
                    ctx.network
                );

                // Collect values from SPV manager V2 in a limited scope to avoid borrow issues
                let (is_initialized, is_syncing, current_height, target_height) = {
                    if let Ok(spv_manager) = ctx.spv_manager_v2.try_read() {
                        let result = (
                            spv_manager.is_initialized(),
                            spv_manager.is_syncing,
                            spv_manager.current_height,
                            spv_manager.target_height,
                        );
                        tracing::debug!(
                            "SPV V2 state for {:?}: initialized={}, syncing={}, height={}/{}",
                            ctx.network,
                            result.0,
                            result.1,
                            result.2,
                            result.3
                        );
                        result
                    } else {
                        tracing::debug!("Could not get SPV manager V2 lock for {:?}", ctx.network);
                        (false, false, 0, 0)
                    }
                }; // spv_manager_v2 lock is dropped here

                // Now update self without any borrows active
                self.spv_is_initialized = is_initialized;
                self.spv_is_syncing = is_syncing;
                self.spv_current_height = current_height;
                self.spv_target_height = target_height;
                // Don't calculate progress from target_height, use actual progress from SPV client
                self.spv_sync_progress = 0.0;
            } else {
                tracing::debug!("Skipping SPV state refresh - connection in progress");
            }
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
        tracing::info!(
            "SettingsScreen::display_task_result called with: {:?}",
            match &backend_task_success_result {
                BackendTaskSuccessResult::SpvResultV2(_) => "SpvResultV2",
                BackendTaskSuccessResult::SpvResult(_) => "SpvResult",
                BackendTaskSuccessResult::CoreItem(_) => "CoreItem",
                _ => "Other",
            }
        );

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
            BackendTaskSuccessResult::SpvResult(spv_result) => {
                match spv_result {
                    SpvTaskResult::SyncProgress {
                        current_height,
                        target_height,
                        progress_percent,
                        phase_info,
                    } => {
                        // Debug log the received phase info
                        if let Some(ref phase) = phase_info {
                            tracing::debug!(
                                "Received phase info: {} ({:.1}%)",
                                phase.phase_name,
                                phase.progress_percentage
                            );
                        }

                        self.spv_current_height = current_height;
                        self.spv_target_height = target_height;
                        self.spv_sync_progress = progress_percent;
                        self.spv_phase_info = phase_info;
                        self.spv_is_syncing = progress_percent < 100.0;
                        self.spv_is_initialized = true;

                        // Clear error on successful progress
                        self.spv_last_error = None;

                        if progress_percent >= 100.0 {
                            self.spv_is_syncing = false;
                            tracing::info!("SPV sync completed at height {}", current_height);
                        }
                    }
                    SpvTaskResult::SyncComplete { final_height } => {
                        self.spv_current_height = final_height;
                        self.spv_target_height = final_height;
                        self.spv_sync_progress = 100.0;
                        self.spv_is_syncing = false;
                        tracing::info!("SPV sync completed at height {}", final_height);
                    }
                    SpvTaskResult::Error(error) => {
                        self.spv_last_error = Some(error);
                        self.spv_is_syncing = false;
                    }
                    _ => {}
                }
            }
            BackendTaskSuccessResult::SpvResultV2(spv_result) => {
                match spv_result {
                    SpvTaskResultV2::SyncProgress {
                        current_height,
                        target_height,
                        progress_percent,
                        phase_info,
                    } => {
                        // Debug log all received values
                        tracing::info!(
                            "SettingsScreen received SPV Progress Update - current: {}, target: {}, percent: {:.1}%, has_phase: {}",
                            current_height,
                            target_height,
                            progress_percent,
                            phase_info.is_some()
                        );

                        // Debug log the received phase info
                        if let Some(ref phase) = phase_info {
                            tracing::debug!(
                                "Settings screen received phase: {} ({:.1}%)",
                                phase.phase_name,
                                phase.progress_percentage
                            );
                        } else {
                            tracing::debug!("Settings screen received no phase info");
                        }

                        // Check if phase has changed
                        let phase_changed = match (&self.spv_phase_info, &phase_info) {
                            (None, Some(_)) => true,
                            (Some(_), None) => true,
                            (Some(old), Some(new)) => old.phase_name != new.phase_name,
                            _ => false,
                        };

                        // Only log significant changes
                        let height_changed = current_height != self.spv_current_height;
                        let progress_changed =
                            (progress_percent - self.spv_sync_progress).abs() > 1.0;

                        if phase_changed {
                            if let Some(ref phase) = phase_info {
                                tracing::info!("🔄 Phase Change: {}", phase.phase_name);
                                if let Some(ref details) = phase.details {
                                    tracing::info!(
                                        "{}: {} ({:.1}%)",
                                        phase.phase_name,
                                        details,
                                        phase.progress_percentage
                                    );
                                }
                            }
                        } else if height_changed || progress_changed {
                            if let Some(ref phase) = phase_info {
                                tracing::debug!(
                                    "{}: {}/{:?} items ({:.1}%) @ {:.1}/sec",
                                    phase.phase_name,
                                    phase.items_completed,
                                    phase.items_total,
                                    phase.progress_percentage,
                                    phase.rate
                                );
                            } else {
                                tracing::info!(
                                    "SPV progress: {} / {} blocks ({:.1}%)",
                                    current_height,
                                    target_height,
                                    progress_percent
                                );
                            }
                        }

                        // Check if this is a stop result (all values are 0 and we're not expecting to be initialized)
                        if target_height == 0
                            && current_height == 0
                            && progress_percent == 0.0
                            && phase_info.is_none()
                        {
                            // Only treat as stop if we're not in the middle of initializing
                            let ctx = self.current_app_context();
                            let is_actually_initialized =
                                if let Ok(spv_manager) = ctx.spv_manager_v2.try_read() {
                                    spv_manager.is_initialized()
                                } else {
                                    false
                                };

                            if !is_actually_initialized && !self.spv_is_initialized {
                                // This is truly a stop result - both backend and UI agree
                                tracing::info!("SPV stop detected - resetting UI state");
                                self.spv_current_height = 0;
                                self.spv_target_height = 0;
                                self.spv_sync_progress = 0.0;
                                self.spv_is_syncing = false;
                                self.spv_is_initialized = false;
                                self.spv_last_error = None;
                                self.spv_phase_info = None;
                            } else {
                                // Either backend is initialized OR we just clicked connect
                                tracing::debug!("Maintaining initialized state - backend: {}, UI: {}", 
                                    is_actually_initialized, self.spv_is_initialized);
                                // Don't reset if we're expecting to be initialized
                                if self.spv_is_initialized || is_actually_initialized {
                                    self.spv_is_initialized = true;
                                }
                            }
                        } else {
                            // Normal sync progress update (including initialization)
                            self.spv_current_height = current_height;
                            self.spv_target_height = target_height;
                            self.spv_sync_progress = progress_percent;
                            self.spv_is_syncing = true; // Always syncing if we have a target
                            self.spv_is_initialized = true;
                            self.spv_last_error = None;
                            self.spv_phase_info = phase_info;
                        }
                    }
                    SpvTaskResultV2::SyncComplete { final_height } => {
                        self.spv_current_height = final_height;
                        self.spv_target_height = final_height;
                        self.spv_sync_progress = 100.0;
                        self.spv_is_syncing = false;
                        self.spv_last_error = None;
                    }
                    SpvTaskResultV2::ProofVerificationResult { is_valid, details } => {
                        // Handle proof verification results if needed
                        if !is_valid {
                            self.spv_last_error = Some(details);
                        }
                    }
                    SpvTaskResultV2::Error(error) => {
                        self.spv_last_error = Some(error);
                        self.spv_is_syncing = false;
                    }
                    _ => {}
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
            RootScreenType::RootScreenSettings,
        );

        action |= island_central_panel(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| self.render_network_table(ui))
                .inner
        });

        // Auto-refresh progress while SPV is syncing or connected
        if action == AppAction::None && self.connection_mode == ConnectionMode::Spv {
            // Check actual SPV manager state, not just UI state
            let should_check_progress = if self.spv_is_initialized {
                true
            } else {
                // Also check the actual SPV manager
                let ctx_app = self.current_app_context();
                if let Ok(spv_manager) = ctx_app.spv_manager_v2.try_read() {
                    spv_manager.is_initialized()
                } else {
                    false
                }
            };

            if should_check_progress {
                let current_time = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as TimestampMillis;

                // Check progress more frequently during initialization phase
                let is_initializing = self.spv_is_initialized
                    && self.spv_current_height == 0
                    && self.spv_phase_info.is_none();

                let check_interval = if is_initializing {
                    1000 // 1 second during initialization
                } else {
                    2000 // 2 seconds during normal sync
                };

                if current_time >= self.spv_last_progress_check + check_interval {
                    tracing::debug!(
                        "Scheduling SPV progress check - UI initialized: {}, should_check: {}",
                        self.spv_is_initialized,
                        should_check_progress
                    );
                    self.spv_last_progress_check = current_time;
                    // Use SPV V2 for progress checks
                    action =
                        AppAction::BackendTask(BackendTask::SpvTaskV2(SpvTaskV2::GetSyncProgress));
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(check_interval as u64));
            }
        }

        if action == AppAction::None {
            // Recheck both network status every 10 seconds
            let recheck_time = Duration::from_secs(10);
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
