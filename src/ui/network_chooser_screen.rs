use crate::app::AppAction;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::system_task::SystemTask;
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::config::Config;
use crate::context::AppContext;
use crate::model::connection_type::ConnectionType;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::{island_central_panel, StyledCard, StyledCheckbox};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::theme::{DashColors, ThemeMode};
use crate::ui::{RootScreenType, ScreenLike};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use eframe::egui::{self, Context, Ui};
use egui::Color32;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    custom_dash_qt_path: Option<String>,
    custom_dash_qt_error_message: Option<String>,
    overwrite_dash_conf: bool,
    developer_mode: bool,
    theme_preference: ThemeMode,
    should_reset_collapsing_states: bool,
    is_switching_connection: bool,
    connection_switch_start_time: Option<Instant>,
    show_spv_diagnostics: bool,
    spv_diagnostics_text: String,
    previous_header_height: Option<u32>,
    previous_header_time: Option<Instant>,
    estimated_blocks_per_second: f32,
}

impl SettingsScreen {
    pub fn new(
        mainnet_app_context: &Arc<AppContext>,
        testnet_app_context: Option<&Arc<AppContext>>,
        devnet_app_context: Option<&Arc<AppContext>>,
        local_app_context: Option<&Arc<AppContext>>,
        current_network: Network,
        custom_dash_qt_path: Option<String>,
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
        let developer_mode = current_context.developer_mode.load(Ordering::Relaxed);

        // Load theme preference from settings
        let theme_preference = current_context
            .get_settings()
            .ok()
            .flatten()
            .map(|(_, _, _, _, _, theme, _)| theme)
            .unwrap_or(ThemeMode::System);

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
            is_switching_connection: false,
            connection_switch_start_time: None,
            show_spv_diagnostics: false,
            spv_diagnostics_text: String::new(),
            previous_header_height: None,
            previous_header_time: None,
            estimated_blocks_per_second: 0.0,
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
    /// Render the connection type selector and network selection table
    fn render_network_table(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let dark_mode = ui.ctx().style().visuals.dark_mode;

        // Connection Type Selection Section (moved from advanced settings)
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Connection type:")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );

            // Get current connection type
            let current_connection_type = self
                .current_app_context()
                .config
                .read()
                .unwrap()
                .connection_type
                .clone();
            let is_spv_supported = matches!(self.current_network, Network::Dash | Network::Testnet);

            egui::ComboBox::from_id_salt(format!(
                "connection_type_selection_{}",
                current_connection_type.as_str()
            ))
            .selected_text(current_connection_type.as_str())
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(
                        current_connection_type == ConnectionType::DashCore,
                        "Dash Core",
                    )
                    .clicked()
                    && current_connection_type != ConnectionType::DashCore
                {
                    tracing::info!(
                        "User clicked to switch to Dash Core from {:?}",
                        current_connection_type
                    );
                    app_action |= AppAction::BackendTask(BackendTask::SwitchConnectionType {
                        connection_type: ConnectionType::DashCore,
                    });
                    // self.is_switching_connection = true;
                    self.connection_switch_start_time = Some(Instant::now());
                }

                if is_spv_supported
                    && ui
                        .selectable_label(current_connection_type == ConnectionType::DashSpv, "SPV")
                        .clicked()
                    && current_connection_type != ConnectionType::DashSpv
                {
                    tracing::info!(
                        "User clicked to switch to SPV from {:?} on network {:?}",
                        current_connection_type,
                        self.current_network
                    );
                    app_action |= AppAction::BackendTask(BackendTask::SwitchConnectionType {
                        connection_type: ConnectionType::DashSpv,
                    });
                    // self.is_switching_connection = true;
                    self.connection_switch_start_time = Some(Instant::now());
                }
            });

            if !is_spv_supported {
                ui.colored_label(
                    DashColors::TEXT_SECONDARY,
                    "(SPV not available for this network)",
                );
            }

            // Show loading indicator if switching connection
            if self.is_switching_connection {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.style_mut().visuals.widgets.inactive.fg_stroke =
                        egui::Stroke::new(2.0, DashColors::DASH_BLUE);
                    ui.style_mut().visuals.widgets.hovered.fg_stroke =
                        egui::Stroke::new(2.0, DashColors::DASH_BLUE);
                    ui.style_mut().visuals.widgets.active.fg_stroke =
                        egui::Stroke::new(2.0, DashColors::DASH_BLUE);
                    ui.add(egui::Spinner::new());
                    ui.label(
                        egui::RichText::new("Switching connection type...")
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                });
            }

            // Show SPV sync status if SPV is selected
            if current_connection_type == ConnectionType::DashSpv && !self.is_switching_connection {
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                ui.label(
                    egui::RichText::new("SPV Sync Status")
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );

                let app_context = self.current_app_context();

                // Get SPV status data and immediately release the lock
                let (spv_is_running, header_height, filter_height) =
                    if let Ok(spv_status) = app_context.spv_status.lock() {
                        (
                            spv_status.is_running,
                            spv_status.header_height,
                            spv_status.filter_height,
                        )
                    } else {
                        (false, None, None)
                    };

                if spv_is_running {
                    // Calculate sync rate based on header height changes
                    let current_time = Instant::now();
                    let mut new_blocks_per_second = self.estimated_blocks_per_second;
                    
                    if let Some(height) = header_height {
                        if let (Some(prev_height), Some(prev_time)) = (self.previous_header_height, self.previous_header_time) {
                            if height > prev_height {
                                let height_diff = height - prev_height;
                                let time_diff = current_time.duration_since(prev_time).as_secs_f32();
                                if time_diff > 0.5 { // Only update if enough time has passed
                                    new_blocks_per_second = height_diff as f32 / time_diff;
                                }
                            }
                        }
                    }

                    // Estimate current blockchain height (mainnet is around 2.29M as of June 2025)
                    let estimated_chain_tip = match self.current_network {
                        Network::Dash => 2_294_200, // Updated estimate based on logs
                        Network::Testnet => 1_000_000, // Rough estimate for testnet
                        _ => 0,
                    };

                    // Show enhanced sync progress
                    ui.vertical(|ui| {
                        ui.add_space(4.0);
                        
                        // Status indicator with visual styling
                        ui.horizontal(|ui| {
                            ui.add_space(20.0);
                            
                            // Animated status indicator
                            let time = ui.input(|i| i.time);
                            let pulse = (time * 2.0).sin() * 0.5 + 0.5;
                            let _status_color = Color32::from_rgb(
                                (64.0 + pulse * 32.0) as u8,
                                (192.0 + pulse * 63.0) as u8,
                                (64.0 + pulse * 32.0) as u8,
                            );
                            
                            ui.label(
                                egui::RichText::new("SPV Connected & Syncing")
                                    .color(DashColors::success_color(dark_mode))
                                    .strong(),
                            );
                        });

                        if let Some(height) = header_height {
                            // Progress bar
                            if estimated_chain_tip > 0 {
                                let progress = (height as f32 / estimated_chain_tip as f32).min(1.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(20.0);
                                    ui.label("Sync Progress:");
                                    
                                    // Custom progress bar
                                    let bar_width = 200.0;
                                    let bar_height = 20.0;
                                    let (rect, _) = ui.allocate_exact_size(
                                        egui::Vec2::new(bar_width, bar_height),
                                        egui::Sense::hover(),
                                    );
                                    
                                    // Background
                                    ui.painter().rect_filled(
                                        rect,
                                        5.0,
                                        DashColors::surface_elevated(dark_mode),
                                    );
                                    
                                    // Progress fill
                                    let progress_width = bar_width * progress;
                                    let progress_rect = egui::Rect::from_min_size(
                                        rect.min,
                                        egui::Vec2::new(progress_width, bar_height),
                                    );
                                    ui.painter().rect_filled(
                                        progress_rect,
                                        5.0,
                                        DashColors::DASH_BLUE,
                                    );
                                    
                                    // Text overlay
                                    let text = format!("{:.1}%", progress * 100.0);
                                    ui.painter().text(
                                        rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        text,
                                        egui::FontId::proportional(12.0),
                                        Color32::WHITE,
                                    );
                                });
                            }
                            
                            // Block height and sync rate
                            ui.horizontal(|ui| {
                                ui.add_space(20.0);
                                ui.label("Block Height:");
                                ui.label(
                                    egui::RichText::new(format!("{}", height))
                                        .color(DashColors::DASH_BLUE)
                                        .monospace(),
                                );
                                
                                if estimated_chain_tip > 0 {
                                    ui.label(format!("/ {}", estimated_chain_tip));
                                }
                                
                                if new_blocks_per_second > 0.1 {
                                    ui.label(format!("({:.0} blocks/sec)", new_blocks_per_second));
                                    
                                    // ETA calculation
                                    if estimated_chain_tip > height && new_blocks_per_second > 0.1 {
                                        let remaining_blocks = estimated_chain_tip - height;
                                        let eta_seconds = remaining_blocks as f32 / new_blocks_per_second;
                                        let eta_minutes = eta_seconds / 60.0;
                                        
                                        if eta_minutes < 60.0 {
                                            ui.label(format!("ETA: {:.1}min", eta_minutes));
                                        } else {
                                            ui.label(format!("ETA: {:.1}h", eta_minutes / 60.0));
                                        }
                                    }
                                }
                            });
                        }

                        if let Some(height) = filter_height {
                            if height > 0 {
                                ui.horizontal(|ui| {
                                    ui.add_space(20.0);
                                    ui.label("Filter Height:");
                                    ui.label(
                                        egui::RichText::new(format!("{}", height))
                                            .color(DashColors::text_secondary(dark_mode))
                                            .monospace(),
                                    );
                                });
                            }
                        }

                    });

                    // Sync button
                    ui.add_space(8.0);
                    // Track button clicks to handle mutations outside closure
                    let mut show_diagnostics_clicked = false;
                    let mut start_sync_clicked = false;

                    // Track button clicks
                    let mut initialize_clicked = false;
                    
                    ui.horizontal(|ui| {
                        if ui.button("Show Diagnostics").clicked() {
                            show_diagnostics_clicked = true;
                        }
                        
                        // Add Initialize button if SPV is not initialized
                        let spv_initialized = app_context.spv_initialized.load(Ordering::Relaxed);
                        if !spv_initialized {
                            if ui.button("Initialize SPV").clicked() {
                                initialize_clicked = true;
                            }
                        }
                        
                        // Add "Start SPV Sync" button only after initialization
                        if spv_initialized && ui.button("Start SPV Sync").clicked() {
                            start_sync_clicked = true;
                        }
                    });

                    // Handle button clicks outside the closure to avoid borrow issues
                    if show_diagnostics_clicked {
                        tracing::info!("User clicked Show Diagnostics");
                        // Show basic diagnostics immediately to avoid blocking
                        self.spv_diagnostics_text = format!(
                            "SPV Diagnostics:\n\
                            Block Height: {}\n\
                            Filter Height: {}\n\
                            Status: {}\n\n\
                            For detailed diagnostics, check det.log",
                            header_height.unwrap_or(0),
                            filter_height.unwrap_or(0),
                            if spv_is_running { "Running" } else { "Stopped" }
                        );
                        self.show_spv_diagnostics = true;
                    }
                    
                    if initialize_clicked {
                        tracing::info!("User clicked Initialize SPV");
                        // Send the task to initialize SPV
                        app_action = AppAction::BackendTask(BackendTask::InitializeSpv);
                    }
                    
                    if start_sync_clicked {
                        tracing::info!("User clicked Start SPV Sync");
                        // Send the task to start SPV sync
                        app_action = AppAction::BackendTask(BackendTask::StartSpvSync);
                    }
                    
                    // Update sync rate tracking (after all borrowing is done)
                    self.estimated_blocks_per_second = new_blocks_per_second;
                    if let Some(height) = header_height {
                        self.previous_header_height = Some(height);
                        self.previous_header_time = Some(current_time);
                    }
                } else {
                    ui.label(
                        egui::RichText::new("SPV client is not running")
                            .color(DashColors::error_color(dark_mode)),
                    );
                }

                // Request repaint to update status
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_secs(1));
            }
        });

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
                        // Theme Selection Section
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


                        // Dash-QT Path Section
                        ui.add_space(16.0);
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
                                            egui::Button::new(egui::RichText::new("Select File")
                                        .strong()
                                        .color(Color32::WHITE))
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
                                                let required_file_name =
                                                    if cfg!(target_os = "windows") {
                                                        String::from("dash-qt.exe")
                                                    } else if cfg!(target_os = "macos") {
                                                        String::from("dash-qt")
                                                    } else {
                                                        //linux
                                                        String::from("dash-qt")
                                                    };
                                                if file_name.ends_with(required_file_name.as_str())
                                                {
                                                    self.custom_dash_qt_path =
                                                        Some(path.display().to_string());
                                                    self.custom_dash_qt_error_message = None;
                                                    self.save()
                                                        .expect("Expected to save db settings");
                                                } else {
                                                    self.custom_dash_qt_error_message =
                                                        Some(format!(
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
                                        self.custom_dash_qt_path = None;
                                        self.custom_dash_qt_error_message = None;
                                        self.save().expect("Expected to save db settings");
                                    }
                                });

                                ui.add_space(8.0);

                                if let Some(ref file) = self.custom_dash_qt_path {
                                    ui.horizontal(|ui| {
                                        ui.label("Selected:");
                                        ui.label(
                                            egui::RichText::new(file).color(DashColors::SUCCESS),
                                        );
                                    });
                                } else if let Some(ref error) = self.custom_dash_qt_error_message {
                                    ui.horizontal(|ui| {
                                        ui.label("Error:");
                                        ui.colored_label(DashColors::ERROR, error);
                                    });
                                } else {
                                    ui.label(
                                        egui::RichText::new(
                                            "No custom path selected (using system default)",
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
                                                .developer_mode
                                                .store(self.developer_mode, Ordering::Relaxed);

                                            if let Some(ref testnet_ctx) = self.testnet_app_context
                                            {
                                                testnet_ctx
                                                    .developer_mode
                                                    .store(self.developer_mode, Ordering::Relaxed);
                                            }

                                            if let Some(ref devnet_ctx) = self.devnet_app_context {
                                                devnet_ctx
                                                    .developer_mode
                                                    .store(self.developer_mode, Ordering::Relaxed);
                                            }

                                            if let Some(ref local_ctx) = self.local_app_context {
                                                local_ctx
                                                    .developer_mode
                                                    .store(self.developer_mode, Ordering::Relaxed);
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

        // Check if this is the current network and if it's using SPV
        let is_current_network = network == self.current_network;
        let is_spv = if is_current_network {
            // Get connection type from config (cached) instead of DB query
            let config = self.current_app_context().config.read().unwrap();
            matches!(config.connection_type, ConnectionType::DashSpv)
        } else {
            false
        };

        // Check network status - for SPV on current network, check cached status
        let is_working = if is_current_network && is_spv {
            // For SPV, check cached status
            self.current_app_context()
                .spv_status
                .lock()
                .map(|status| status.is_running)
                .unwrap_or(false)
        } else {
            self.check_network_status(network)
        };

        // Determine status color and text
        let (status_color, status_text) = if is_working {
            if is_spv {
                (DashColors::DASH_BLUE, "SPV")
            } else {
                (DashColors::success_color(dark_mode), "Online")
            }
        } else {
            (DashColors::error_color(dark_mode), "Offline")
        };

        // Display status indicator
        ui.colored_label(status_color, status_text);

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

        if network != Network::Regtest {
            if ui.button("Start").clicked() {
                app_action = AppAction::BackendTask(BackendTask::CoreTask(CoreTask::StartDashQT(
                    network,
                    self.custom_dash_qt_path.clone(),
                    self.overwrite_dash_conf,
                )));
            }
        } else {
            ui.label("");
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

        // Clear any lingering loading state
        self.is_switching_connection = false;
        self.connection_switch_start_time = None;

        // Reload settings from database to ensure we have the latest values
        if let Ok(Some((_, _, _, custom_dash_qt_path, overwrite_dash_conf, theme_preference, _))) =
            self.current_app_context().get_settings()
        {
            self.custom_dash_qt_path = custom_dash_qt_path;
            self.overwrite_dash_conf = overwrite_dash_conf;
            self.theme_preference = theme_preference;
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

        // Clear loading state if connection switch failed
        if message.contains("Failed to")
            && (message.contains("connection")
                || message.contains("SPV")
                || message.contains("Core"))
        {
            self.is_switching_connection = false;
            self.connection_switch_start_time = None;
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
            BackendTaskSuccessResult::Message(ref message) => {
                // Check if this is a connection type switch message
                if message.contains("Switched to Dash Core connection")
                    || message.contains("Successfully switched to SPV connection")
                {
                    // Force UI refresh by reloading settings
                    // This ensures the dropdown shows the correct connection type
                    self.refresh_on_arrival();
                    // Clear the loading state
                    self.is_switching_connection = false;
                    self.connection_switch_start_time = None;
                }
            }
            _ => {}
        }
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            self.current_app_context(),
            vec![("Settings", AppAction::None)],
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

        // Only check chain locks if we're NOT using SPV connection
        let current_connection_type = {
            let config = self.current_app_context().config.read().unwrap();
            config.connection_type.clone()
        };

        if current_connection_type != ConnectionType::DashSpv {
            // Recheck both network status every 3 seconds
            let recheck_time = Duration::from_secs(3);
            if action == AppAction::None {
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
            }
        }

        // Show SPV diagnostics popup if requested
        if self.show_spv_diagnostics {
            let mut is_open = true;
            egui::Window::new("SPV Diagnostics")
                .collapsible(false)
                .resizable(true)
                .default_width(600.0)
                .default_height(400.0)
                .open(&mut is_open)
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label("SPV Client Diagnostic Information:");
                    ui.add_space(8.0);

                    egui::ScrollArea::vertical()
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            ui.monospace(&self.spv_diagnostics_text);
                        });

                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        if ui.button("Refresh").clicked() {
                            tracing::info!("User clicked Refresh diagnostics");
                            let spv_manager = self.current_app_context().spv_manager.clone();
                            let diagnostics = tokio::task::block_in_place(|| {
                                let handle = tokio::runtime::Handle::current();
                                handle.block_on(async {
                                    match spv_manager.get_diagnostics().await {
                                        Ok(diag) => diag,
                                        Err(e) => format!("Failed to get diagnostics: {}", e),
                                    }
                                })
                            });
                            self.spv_diagnostics_text = diagnostics;
                        }

                        if ui.button("Close").clicked() {
                            self.show_spv_diagnostics = false;
                        }
                    });
                });

            if !is_open {
                self.show_spv_diagnostics = false;
            }
        }

        action
    }
}
