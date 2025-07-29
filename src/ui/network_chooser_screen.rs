use crate::app::AppAction;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::spv::{SpvTask, SpvTaskResult};
use dash_spv::types::SyncPhaseInfo;
use crate::backend_task::system_task::SystemTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::config::Config;
use crate::context::AppContext;
use crate::model::settings::ConnectionMode;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::{StyledCard, StyledCheckbox, island_central_panel};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::{DashColors, ThemeMode};
use crate::ui::{RootScreenType, ScreenLike};
use crate::utils::path::format_path_for_display;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use eframe::egui::{self, Color32, Context, Ui};
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
}

impl SettingsScreen {
    fn render_connection_mode_section(&mut self, ui: &mut Ui, app_action: &mut AppAction) {
        StyledCard::new().padding(20.0).show(ui, |ui| {
            ui.heading("Connection Mode");
            ui.separator();

            ui.horizontal(|ui| {
                ui.label("Select connection method:");

                let mut mode_changed = false;

                // Core RPC option
                if ui.selectable_value(&mut self.connection_mode, ConnectionMode::Core, "Dash Core RPC").clicked() {
                    mode_changed = true;
                }

                ui.add_space(20.0);

                // SPV option
                if ui.selectable_value(&mut self.connection_mode, ConnectionMode::Spv, "SPV Client").clicked() {
                    mode_changed = true;
                }

                if mode_changed {
                    // Update connection mode in context
                    if let Err(e) = self.current_app_context().set_connection_mode(self.connection_mode) {
                        self.spv_last_error = Some(format!("Failed to update connection mode: {}", e));
                    }

                    // Update all contexts
                    self.mainnet_app_context.set_connection_mode(self.connection_mode).ok();
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
            });

            ui.add_space(10.0);

            match self.connection_mode {
                ConnectionMode::Core => {
                    ui.label("Uses Dash Core's RPC interface for blockchain operations.");
                    ui.label("Requires a running Dash Core node with proper configuration.");
                }
                ConnectionMode::Spv => {
                    ui.label("SPV (Simplified Payment Verification) allows lightweight blockchain access.");
                    ui.label("No full node required - connects directly to the Dash network.");

                    ui.separator();

                    // SPV sync controls
                    self.render_spv_controls(ui, app_action);
                }
            }
        });
    }

    fn render_spv_controls(&mut self, ui: &mut Ui, action: &mut AppAction) {
        ui.heading("SPV Sync Controls");

        ui.horizontal(|ui| {
            if !self.spv_is_initialized || (!self.spv_is_syncing && self.spv_current_height == 0) {
                // Use network-appropriate checkpoint height
                // Start from genesis to avoid checkpoint validation issues
                let checkpoint_height = match self.current_network {
                    Network::Dash => 0,    // Start from genesis due to checkpoint validation issues
                    Network::Testnet => 0, // Start from genesis due to checkpoint validation issues
                    Network::Devnet => 0,  // Start from genesis for devnet
                    Network::Regtest => 0, // Start from genesis for regtest
                    _ => 0,
                };

                if ui.button("Initialize & Sync").clicked() {
                    *action =
                        AppAction::BackendTask(BackendTask::SpvTask(SpvTask::InitializeAndSync {
                            checkpoint_height,
                        }));
                    // Set syncing state immediately for UI feedback
                    self.spv_is_syncing = true;
                    self.spv_last_error = None;
                }
            } else if !self.spv_is_syncing {
                // For resume, we'll use the current height as the checkpoint
                let checkpoint_height = self.spv_current_height;

                if ui.button("Resume Sync").clicked() {
                    *action =
                        AppAction::BackendTask(BackendTask::SpvTask(SpvTask::InitializeAndSync {
                            checkpoint_height,
                        }));
                    // Set syncing state immediately for UI feedback
                    self.spv_is_syncing = true;
                    self.spv_last_error = None;
                }
            } else {
                ui.add_enabled(false, egui::Button::new("Syncing..."));
            }

        });

        if self.spv_is_syncing || self.spv_is_initialized {
            ui.separator();
            self.render_spv_sync_progress(ui);
        }

        if let Some(error) = &self.spv_last_error {
            ui.separator();
            ui.colored_label(Color32::RED, format!("Error: {}", error));
        }
    }

    fn render_spv_sync_progress(&self, ui: &mut Ui) {
        // Display phase name or generic label
        if let Some(ref phase_info) = self.spv_phase_info {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&phase_info.phase_name).strong());
                if self.spv_is_syncing {
                    ui.spinner();
                }
            });
        } else {
            ui.horizontal(|ui| {
                ui.label("Sync Progress");
                if self.spv_is_syncing {
                    ui.spinner();
                }
            });
        }

        // Add progress bar with proper width constraints
        ui.add_sized(
            [ui.available_width(), 20.0],
            egui::ProgressBar::new(self.spv_sync_progress / 100.0)
                .text(format!("{:.2}%", self.spv_sync_progress))
                .animate(self.spv_is_syncing),
        );

        // Display phase-specific information
        if let Some(ref phase_info) = self.spv_phase_info {
            ui.horizontal(|ui| {
                ui.label("Progress:");
                if let Some(total) = phase_info.items_total {
                    ui.label(format!(
                        "{} / {} items",
                        phase_info.items_completed.to_formatted_string(&Locale::en),
                        total.to_formatted_string(&Locale::en)
                    ));
                } else {
                    ui.label(format!(
                        "{} items",
                        phase_info.items_completed.to_formatted_string(&Locale::en)
                    ));
                }
                
                if phase_info.rate > 0.0 {
                    ui.label(format!(" @ {:.1} items/sec", phase_info.rate));
                }
            });

            // Display ETA if available
            if let Some(eta) = phase_info.eta_seconds {
                ui.horizontal(|ui| {
                    ui.label("ETA:");
                    let eta_text = if eta < 60 {
                        format!("{} seconds", eta)
                    } else if eta < 3600 {
                        format!("{:.1} minutes", eta as f64 / 60.0)
                    } else {
                        format!("{:.1} hours", eta as f64 / 3600.0)
                    };
                    ui.label(eta_text);
                });
            }

            // Display phase details if available
            if let Some(ref details) = phase_info.details {
                ui.add_space(5.0);
                ui.label(egui::RichText::new(details).small().color(Color32::GRAY));
            }
        } else if self.spv_target_height > 0 {
            // Fallback to basic height display
            ui.horizontal(|ui| {
                ui.label("Progress:");
                ui.label(format!(
                    "{} / {} blocks",
                    self.spv_current_height.to_formatted_string(&Locale::en),
                    self.spv_target_height.to_formatted_string(&Locale::en)
                ));
            });

            if self.spv_is_syncing
                && self.spv_current_height > 0
                && self.spv_target_height > self.spv_current_height
            {
                let blocks_remaining = self.spv_target_height - self.spv_current_height;
                ui.horizontal(|ui| {
                    ui.label("Blocks Remaining:");
                    ui.label(format!(
                        "{}",
                        blocks_remaining.to_formatted_string(&Locale::en)
                    ));
                });
            }
        }
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
    /// Render the network selection table
    fn render_network_table(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Connection Mode Section
        self.render_connection_mode_section(ui, &mut app_action);

        ui.add_space(20.0);

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
                // ui.label(egui::RichText::new("Wallet Count").strong().underline());
                // ui.label(egui::RichText::new("Add New Wallet").strong().underline());
                ui.label(
                    egui::RichText::new("Select")
                        .strong()
                        .underline()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.label(
                    egui::RichText::new("Start Dash Core")
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
                                    {
                                        if let Some(path) = rfd::FileDialog::new().pick_file() {
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

        // Check network status
        let is_working = self.check_network_status(network);
        let status_color = if is_working {
            DashColors::success_color(dark_mode) // Theme-aware green
        } else {
            DashColors::error_color(dark_mode) // Theme-aware red
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

        // Network selection
        let mut is_selected = self.current_network == network;
        if StyledCheckbox::new(&mut is_selected, "").show(ui).clicked() && is_selected {
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
                    app_action =
                        AppAction::BackendTask(BackendTask::CoreTask(CoreTask::StartDashQT(
                            network,
                            self.custom_dash_qt_path
                                .clone()
                                .expect("Some() checked above"),
                            self.overwrite_dash_conf,
                        )));
                }
            });
        }

        // Add a text field for the dashmate password
        if network == Network::Regtest {
            ui.spacing_mut().item_spacing.x = 5.0;
            let dark_mode = ui.ctx().style().visuals.dark_mode;
            ui.add(
                egui::TextEdit::singleline(&mut self.local_network_dashmate_password)
                    .desired_width(100.0)
                    .text_color(crate::ui::theme::DashColors::text_primary(dark_mode))
                    .background_color(crate::ui::theme::DashColors::input_background(dark_mode)),
            );
            if ui.button("Save Password").clicked() {
                // 1) Reload the config
                if let Ok(mut config) = Config::load() {
                    if let Some(local_cfg) = config.config_for_network(Network::Regtest).clone() {
                        let updated_local_config = local_cfg
                            .update_core_rpc_password(self.local_network_dashmate_password.clone());
                        config.update_config_for_network(
                            Network::Regtest,
                            updated_local_config.clone(),
                        );
                        if let Err(e) = config.save() {
                            eprintln!("Failed to save config to .env: {e}");
                        }

                        // 5) Update our local AppContext in memory
                        if let Some(local_app_context) = &self.local_app_context {
                            {
                                // Overwrite the config field with the new password
                                let mut cfg_lock = local_app_context.config.write().unwrap();
                                *cfg_lock = updated_local_config;
                            }

                            // 6) Re-init the client & sdk from the updated config
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
                }
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
}

impl ScreenLike for SettingsScreen {
    fn refresh_on_arrival(&mut self) {
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
                        // Check if phase has changed
                        let phase_changed = match (&self.spv_phase_info, &phase_info) {
                            (None, Some(_)) => true,
                            (Some(_), None) => true,
                            (Some(old), Some(new)) => old.phase_name != new.phase_name,
                            _ => false,
                        };
                        
                        // Only log significant changes
                        let height_changed = current_height != self.spv_current_height;
                        let progress_changed = (progress_percent - self.spv_sync_progress).abs() > 1.0;
                        
                        if phase_changed {
                            if let Some(ref phase) = phase_info {
                                tracing::info!(
                                    "🔄 Phase Change: {}",
                                    phase.phase_name
                                );
                                if let Some(ref details) = phase.details {
                                    tracing::info!("{}: {} ({:.1}%)", phase.phase_name, details, phase.progress_percentage);
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

                        self.spv_current_height = current_height;
                        self.spv_target_height = target_height;
                        self.spv_sync_progress = progress_percent;
                        self.spv_is_syncing = true;
                        self.spv_is_initialized = true;
                        self.spv_last_error = None;
                        self.spv_phase_info = phase_info;
                    }
                    SpvTaskResult::SyncComplete { final_height } => {
                        self.spv_current_height = final_height;
                        self.spv_target_height = final_height;
                        self.spv_sync_progress = 100.0;
                        self.spv_is_syncing = false;
                        self.spv_last_error = None;
                    }
                    SpvTaskResult::ProofVerificationResult { is_valid, details } => {
                        // Handle proof verification results if needed
                        if !is_valid {
                            self.spv_last_error = Some(details);
                        }
                    }
                    SpvTaskResult::Error(error) => {
                        self.spv_last_error = Some(error);
                        self.spv_is_syncing = false;
                    }
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

        // Auto-refresh progress while SPV is syncing (once per second)
        if self.spv_is_syncing && action == AppAction::None {
            let current_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as TimestampMillis;

            // Only check progress once per 3 seconds
            if current_time >= self.spv_last_progress_check + 3000 {
                self.spv_last_progress_check = current_time;
                action = AppAction::BackendTask(BackendTask::SpvTask(SpvTask::GetSyncProgress));
            }
            ctx.request_repaint_after(std::time::Duration::from_secs(3));
        } else if action == AppAction::None {
            // Recheck both network status every 3 seconds
            let recheck_time = Duration::from_secs(3);
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
