use crate::app::AppAction;
use crate::backend_task::core::CoreTask;
use crate::backend_task::dapi_discovery::persist_dapi_addresses;
use crate::backend_task::error::TaskError;
use crate::backend_task::system_task::SystemTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::context::connection_status::OverallConnectionState;
use crate::model::spv_status::{SpvStatus, SpvStatusSnapshot};
use crate::model::user_role::UserRole;
use crate::ui::components::MessageBanner;
use crate::ui::components::component_trait::Component;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::{
    ConfirmationDialog, ConfirmationStatus, StyledCard, StyledCheckbox, island_central_panel,
};
use crate::ui::components::top_panel::{add_top_panel_with_global_nav, subdued_everyday_spec};
use crate::ui::theme::{DashColors, ResponseExt, Shape, ThemeMode};
use crate::ui::{MessageType, RootScreenType, ScreenLike};
use dash_sdk::dash_spv::sync::{ProgressPercentage, SyncProgress as SpvSyncProgress, SyncState};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use eframe::egui::{self, Ui};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) fn chooser_network_label(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "Mainnet",
        Network::Testnet => "Testnet",
        Network::Devnet => "Devnet",
        Network::Regtest => "Local",
    }
}

#[derive(Debug, Clone)]
enum SpvClearMessage {
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
    pub network_contexts: BTreeMap<Network, Arc<AppContext>>,
    pub current_network: Network,
    pub recheck_time: Option<TimestampMillis>,
    selected_role: UserRole,
    theme_preference: ThemeMode,
    should_reset_collapsing_states: bool,
    spv_progress_network: Option<Network>,
    headers_stage_start: Option<u32>,
    filter_headers_stage_start: Option<u32>,
    filters_stage_start: Option<u32>,
    blocks_stage_start: Option<u32>,
    blocks_target_height: u32,
    spv_clear_dialog: Option<ConfirmationDialog>,
    spv_clear_message: Option<SpvClearMessage>,
    db_clear_dialog: Option<ConfirmationDialog>,
    db_clear_in_progress: bool,
    wipe_platform_data_dialog: Option<ConfirmationDialog>,
    auto_start_spv: bool,
    discovery_in_progress: bool,
    fetch_confirmation_dialog: Option<ConfirmationDialog>,
    /// Set when DAPI discovery completes and an SDK reinit is needed.
    /// Dispatched as a `BackendTask` from the next `ui()` call.
    pending_reinit_after_discovery: bool,
}

impl NetworkChooserScreen {
    pub fn new(contexts: &BTreeMap<Network, Arc<AppContext>>, current_network: Network) -> Self {
        let any_context = contexts
            .values()
            .next()
            .expect("BUG: NetworkChooserScreen requires at least one AppContext");

        let current_context = contexts.get(&current_network).unwrap_or(any_context);
        let selected_role = current_context.user_role();

        let settings = current_context.get_app_settings();
        let theme_preference = settings.theme_mode;
        let auto_start_spv = settings.auto_start_spv;

        Self {
            network_contexts: contexts.clone(),
            current_network,
            recheck_time: None,
            selected_role,
            theme_preference,
            should_reset_collapsing_states: true, // Start with collapsed state
            spv_progress_network: None,
            headers_stage_start: None,
            filter_headers_stage_start: None,
            filters_stage_start: None,
            blocks_stage_start: None,
            blocks_target_height: 0,
            spv_clear_dialog: None,
            spv_clear_message: None,
            db_clear_dialog: None,
            db_clear_in_progress: false,
            wipe_platform_data_dialog: None,
            auto_start_spv,
            discovery_in_progress: false,
            fetch_confirmation_dialog: None,
            pending_reinit_after_discovery: false,
        }
    }

    pub fn context_for_network(&self, network: Network) -> Option<&Arc<AppContext>> {
        self.network_contexts.get(&network)
    }

    /// Returns the AppContext for the current network.
    /// Falls back to any available context (should always succeed while the app is running).
    pub fn current_app_context(&self) -> &Arc<AppContext> {
        self.context_for_network(self.current_network)
            .or_else(|| self.network_contexts.values().next())
            .expect("BUG: no AppContext available for any network")
    }

    /// Render the simplified settings interface
    fn render_network_table(&mut self, ui: &mut Ui) -> AppAction {
        let mut app_action = AppAction::None;
        let dark_mode = ui.style().visuals.dark_mode;

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
                    // Row: Network
                    ui.label(
                        egui::RichText::new("Network:").color(DashColors::text_primary(dark_mode)),
                    );

                    // Chain sync is owned by upstream platform-wallet; the
                    // EventBridge feeds live SPV status into ConnectionStatus.
                    // While active, the network selector stays disabled so the
                    // user can't switch networks mid-sync.
                    let is_spv_connected = self
                        .current_app_context()
                        .connection_status()
                        .spv_status()
                        .is_active();

                    let network_text = match self.current_network {
                        Network::Mainnet => "Mainnet",
                        Network::Testnet => "Testnet",
                        Network::Devnet => "Devnet",
                        Network::Regtest => "Local",
                    };

                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        let network_combo = egui::ComboBox::from_id_salt("network_selector")
                            .selected_text(network_text)
                            .width(200.0);

                        let response = ui.add_enabled_ui(
                            !is_spv_connected && !self.db_clear_in_progress,
                            |ui| {
                                network_combo.show_ui(ui, |ui| {
                                    if ui
                                        .selectable_value(
                                            &mut self.current_network,
                                            Network::Mainnet,
                                            "Mainnet",
                                        )
                                        .clicked()
                                    {
                                        app_action = AppAction::SwitchNetwork(Network::Mainnet);
                                    }
                                    // Testnet always visible; Devnet/Local only in dev mode
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
                                    if self.selected_role.at_least(UserRole::Power)
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
                                    if self.selected_role.at_least(UserRole::Power)
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
                            },
                        );

                        if is_spv_connected {
                            response
                                .response
                                .disabled_tooltip("Disconnect from SPV first");
                        }
                    });

                    ui.end_row();
                });
        });

        // Connection Status Card
        ui.add_space(16.0);

        StyledCard::new().padding(24.0).show(ui, |ui| {
            ui.heading("Connection Status");
            ui.add_space(10.0);

            let ctx = self.current_app_context().clone();
            let status = ctx.connection_status();
            let spv_status = status.spv_status();
            let spv_connected = spv_status.is_active();
            let spv_error_detail = status.spv_last_error();
            // Chain sync is owned by upstream platform-wallet; the EventBridge
            // pushes live status + per-phase progress into ConnectionStatus.
            let snapshot: Option<SpvStatusSnapshot> = Some(status.spv_status_snapshot());
            let overall_state = status.overall_state();
            let dapi_total = status.dapi_total_endpoints();
            let dapi_available = status.dapi_available();
            let dapi_label = status.dapi_status_label();

            // Button on the left with status
            ui.horizontal(|ui| {
                if overall_state != OverallConnectionState::Disconnected {
                    let is_stopping = spv_status == SpvStatus::Stopping;
                    let disconnect_button = egui::Button::new(
                        egui::RichText::new("Disconnect").color(DashColors::WHITE),
                    )
                    .fill(DashColors::ERROR)
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(Shape::RADIUS_MD)
                    .min_size(egui::vec2(120.0, 36.0));

                    if ui.add_enabled(!is_stopping, disconnect_button).clicked() {
                        // The update loop owns the async teardown (upstream
                        // shutdown is async), so dispatch it as an action rather
                        // than blocking the frame loop. The indicator flips to
                        // Stopping → Disconnected as the teardown progresses.
                        app_action = AppAction::StopSpv;
                    }

                    // Show sync status next to button
                    ui.add_space(12.0);

                    if let Some(snap) = &snapshot {
                        match snap.status {
                            SpvStatus::Running => {
                                ui.colored_label(DashColors::SUCCESS, "Synced - The SPV client can now be used for transacting and querying.");
                            }
                            SpvStatus::Syncing | SpvStatus::Starting => {
                                let warning_color = DashColors::warning_color(dark_mode);
                                ui.style_mut().visuals.widgets.inactive.fg_stroke.color =
                                    warning_color;
                                ui.style_mut().visuals.widgets.hovered.fg_stroke.color =
                                    warning_color;
                                ui.style_mut().visuals.widgets.active.fg_stroke.color =
                                    warning_color;
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
                    // Chain sync is SPV-only.
                    let show_connect_button = true;

                    if show_connect_button {
                        let connect_button = egui::Button::new(
                            egui::RichText::new("Connect").color(DashColors::WHITE),
                        )
                        .fill(DashColors::DASH_BLUE)
                        .stroke(egui::Stroke::NONE)
                        .corner_radius(Shape::RADIUS_MD)
                        .min_size(egui::vec2(120.0, 36.0));

                        if ui.add(connect_button).clicked() {
                            // The update loop owns the `TaskResult` sender the
                            // backend-wiring step needs, so it lazily wires the
                            // backend then starts chain sync. A click during the
                            // brief not-yet-wired boot window no longer fast-fails.
                            app_action = AppAction::StartSpv;
                        }
                    }
                }
            });

            if let Some(snap) = snapshot.as_ref()
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
                {
                    ui.horizontal(|ui| {
                        ui.label("SPV:");
                        let color = if spv_connected {
                            DashColors::SUCCESS
                        } else {
                            DashColors::ERROR
                        };
                        if spv_status == SpvStatus::Error {
                            // Fixed, jargon-free label. The raw upstream sync
                            // error is offered only as a hover tooltip so the
                            // status line never renders internal error text.
                            let response = ui.colored_label(
                                color,
                                "Sync error — open Settings for details",
                            );
                            if let Some(detail) = spv_error_detail.as_ref() {
                                response.on_hover_text(detail);
                            }
                        } else {
                            ui.colored_label(color, spv_status.to_string());
                        }
                    });
                }

                // DAPI line (all modes)
                ui.horizontal(|ui| {
                    add_dapi_status_label(
                        ui,
                        dapi_total,
                        dapi_available,
                        &dapi_label,
                        dark_mode,
                    );
                });

                // "Refresh DAPI endpoints" button — Mainnet/Testnet only
                let is_discoverable = matches!(
                    self.current_network,
                    Network::Mainnet | Network::Testnet
                );
                if is_discoverable {
                    ui.add_space(8.0);
                    let button_text = if self.discovery_in_progress {
                        "Fetching..."
                    } else {
                        "Refresh DAPI endpoints"
                    };
                    let button =
                        egui::Button::new(button_text).corner_radius(Shape::RADIUS_MD);

                    let response = ui.add_enabled(!self.discovery_in_progress, button);
                    let clicked = response.clicked();
                    response.on_hover_text(
                        "Updates list of DAPI nodes using a centralized server managed by Dash Core Group.",
                    );
                    if clicked {
                        if dapi_total > 0 {
                            let message = format!(
                                "This will fetch a fresh list of DAPI nodes, replacing your current {} \
                                configured addresses in the config file.",
                                dapi_total
                            );
                            self.fetch_confirmation_dialog = Some(
                                ConfirmationDialog::new("Update Node Addresses?", message)
                                    .confirm_text(Some("Fetch"))
                                    .cancel_text(Some("Cancel")),
                            );
                        } else {
                            self.discovery_in_progress = true;
                            app_action = AppAction::BackendTask(
                                BackendTask::DiscoverDapiNodes {
                                    network: self.current_network,
                                },
                            );
                        }
                    }
                }
            });

            // Fetch confirmation dialog
            if self.fetch_confirmation_dialog.is_some() {
                app_action |= self.show_fetch_confirmation(ui);
            }
        });

        // Interface mode — rendered above Advanced Settings (which force-collapses
        // on every arrival) so the role selector is always discoverable.
        ui.add_space(16.0);
        StyledCard::new().padding(20.0).show(ui, |ui| {
            ui.label(
                egui::RichText::new("Interface mode")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.add_space(4.0);

            let previous_role = self.selected_role;
            ui.horizontal(|ui| {
                for role in [UserRole::Everyday, UserRole::Power, UserRole::Developer] {
                    ui.radio_value(&mut self.selected_role, role, role.label());
                }
            });

            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(self.selected_role.description())
                    .color(DashColors::text_secondary(dark_mode))
                    .italics(),
            );

            if self.selected_role != previous_role {
                // The role is app-global and shared by every per-network context;
                // persist it as the canonical AppSettings value and publish it to
                // the shared runtime role cell in one call.
                match self
                    .current_app_context()
                    .set_and_persist_user_role(self.selected_role)
                {
                    Ok(()) => {
                        // Raising the role also disables animations, which stops the
                        // continuous repaints, so request one so newly-gated nav entries
                        // (e.g. Masternodes) appear immediately.
                        ui.ctx().request_repaint();
                    }
                    Err(e) => {
                        // The selection never reached the store, so the radio group
                        // must fall back to the mode that is still in effect.
                        self.selected_role = previous_role;
                        MessageBanner::set_global(
                            ui.ctx(),
                            "Could not save your interface mode. Select it again, or restart the application if the problem continues.",
                            MessageType::Error,
                        )
                        .with_details(e);
                    }
                }
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

                // Developer-tools sub-panel — Developer role only.
                if self.selected_role.at_least(UserRole::Developer) {
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new("Developer Tools")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(6.0);

                    ui.horizontal(|ui| {
                        // The reason is carried by the always-visible label beside the
                        // button, not by a tooltip on a disabled control that is easy to
                        // miss — and it is one string, so it stays one translation unit.
                        ui.add_enabled(false, egui::Button::new("Clear Platform Addresses"));
                        ui.label(
                            egui::RichText::new(
                                "This tool is unavailable because earlier-version recovery data is kept read-only.",
                            )
                                .color(DashColors::TEXT_SECONDARY)
                                .italics(),
                        );
                    });
                }

                // Advanced SPV peer source configuration is Expert-only —
                // fresh-install users get auto-discovery, which is the correct default.
                if self.selected_role.at_least(UserRole::Power) {
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
                            // Save to the shared app k/v store
                            let _ = self
                                .current_app_context()
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

                if ui
                    .add_enabled(!self.db_clear_in_progress, clear_button)
                    .clicked()
                {
                    let message = format!(
                        "This removes the data used by this version for {}, including wallets, tokens, contacts, and cached identity data. If you updated from an earlier version, its read-only recovery database stays on this device and may still contain wallet recovery data. Continue?",
                        self.current_network_label()
                    );
                    self.db_clear_dialog = Some(
                        ConfirmationDialog::new("Clear Database", message)
                            .confirm_text(Some("Delete Data"))
                            .cancel_text(Some("Cancel"))
                            .danger_mode(true),
                    );
                }

                if self.db_clear_dialog.is_some() {
                    app_action |= self.show_database_clear_confirmation(ui);
                }

                if wipe_platform_data_available(self.selected_role, self.current_network) {
                    ui.add_space(8.0);
                    let wipe_button = egui::Button::new(
                        egui::RichText::new("Wipe Platform Data").color(DashColors::WHITE),
                    )
                    .fill(DashColors::ERROR)
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(Shape::RADIUS_MD)
                    .min_size(egui::vec2(0.0, 36.0));

                    if ui
                        .add(wipe_button)
                        .clickable_tooltip(
                            "Permanently remove identities, keys, tokens, and custom data stored by this app for your development network.",
                        )
                        .clicked()
                    {
                        self.wipe_platform_data_dialog = Some(
                            ConfirmationDialog::new(
                                "Wipe Platform Data?",
                                "This permanently removes every identity and its locally stored keys, along with tokens and custom Platform data, from this app's development network. Make sure you can recover any identity you still need. You cannot undo this action.",
                            )
                            .confirm_text(Some("Wipe Platform Data"))
                            .cancel_text(Some("Keep Data"))
                            .danger_mode(true)
                            .require_confirmation_text(
                                "WIPE",
                                "Type WIPE to confirm this action.",
                            ),
                        );
                    }
                }

                if self.wipe_platform_data_dialog.is_some() {
                    app_action |= self.show_wipe_platform_data_confirmation(ui);
                }

                // SPV maintenance (clear data, rescan) is Expert-only — these are
                // diagnostic tools that can destroy wallet sync state and should not
                // be exposed to fresh-install users.
                if self.selected_role.at_least(UserRole::Power) {
                    // Chain sync is owned by upstream platform-wallet; the
                    // EventBridge feeds live status into ConnectionStatus.
                    let snapshot = self
                        .current_app_context()
                        .connection_status()
                        .spv_status_snapshot();
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(12.0);
                    app_action |= self.render_spv_maintenance_controls(ui, &snapshot);
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

        let dark_mode = ui.style().visuals.dark_mode;

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
        let dark_mode = ui.style().visuals.dark_mode;

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
                button_response.disabled_tooltip("Stop the SPV client before clearing data");
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

    fn show_fetch_confirmation(&mut self, ui: &mut Ui) -> AppAction {
        let mut action = AppAction::None;
        if let Some(dialog) = self.fetch_confirmation_dialog.as_mut() {
            let response = dialog.show(ui);
            if let Some(result) = response.inner.dialog_response {
                self.fetch_confirmation_dialog = None;
                if matches!(result, ConfirmationStatus::Confirmed) {
                    self.discovery_in_progress = true;
                    action = AppAction::BackendTask(BackendTask::DiscoverDapiNodes {
                        network: self.current_network,
                    });
                }
            }
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
                                tracing::error!(error = ?err, "Failed to clear SPV data");
                                self.spv_clear_message =
                                    Some(SpvClearMessage::Error(err.to_string()));
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
                        self.db_clear_in_progress = true;
                        return AppAction::BackendTask(BackendTask::SystemTask(
                            SystemTask::ClearNetworkDatabase,
                        ));
                    }
                    ConfirmationStatus::Canceled => {
                        // No-op
                    }
                }
            }
        }
        AppAction::None
    }

    fn show_wipe_platform_data_confirmation(&mut self, ui: &mut Ui) -> AppAction {
        if !wipe_platform_data_available(self.selected_role, self.current_network) {
            self.wipe_platform_data_dialog = None;
            return AppAction::None;
        }

        if let Some(dialog) = self.wipe_platform_data_dialog.as_mut() {
            let response = dialog.show(ui);
            if let Some(result) = response.inner.dialog_response {
                self.wipe_platform_data_dialog = None;
                if matches!(result, ConfirmationStatus::Confirmed) {
                    return wipe_platform_data_action(self.selected_role, self.current_network);
                }
            }
        }
        AppAction::None
    }

    fn current_network_label(&self) -> &'static str {
        chooser_network_label(self.current_network)
    }

    /// Fraction in [0,1] of the download window from `stage_start` (default `current`) to `target`.
    ///
    /// Windowing makes checkpoint-resumed syncs start near 0% instead of jumping ahead. Pass
    /// `Some(0)` for a plain `current / target` ratio.
    fn window_fraction(stage_start: Option<u32>, current: u32, target: u32) -> f32 {
        if target == 0 {
            return 0.0;
        }
        let start = stage_start.unwrap_or(current).min(target);
        let span = target.saturating_sub(start);
        if span == 0 {
            return if current >= target { 1.0 } else { 0.0 };
        }
        (current.saturating_sub(start) as f32 / span as f32).clamp(0.0, 1.0)
    }

    /// Progress in [0,1] for a sync stage, from its state and windowed height range.
    fn stage_progress(
        state: SyncState,
        stage_start: Option<u32>,
        current: u32,
        target: u32,
    ) -> f32 {
        match state {
            SyncState::Syncing => Self::window_fraction(stage_start, current, target),
            SyncState::Synced => 1.0,
            SyncState::WaitingForConnections | SyncState::WaitForEvents | SyncState::Error => 0.0,
        }
    }

    fn calculate_headers_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        let Some(progress) = &snapshot.sync_progress else {
            return 0.0;
        };
        let Ok(headers) = progress.headers() else {
            return 0.0;
        };
        Self::stage_progress(
            headers.state(),
            self.headers_stage_start,
            headers.current_height(),
            headers.target_height(),
        )
    }

    fn calculate_filter_headers_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        let Some(progress) = &snapshot.sync_progress else {
            return 0.0;
        };
        let Ok(fh) = progress.filter_headers() else {
            return 0.0;
        };
        Self::stage_progress(
            fh.state(),
            self.filter_headers_stage_start,
            fh.current_height(),
            fh.target_height(),
        )
    }

    fn calculate_filters_progress(&self, snapshot: &SpvStatusSnapshot) -> f32 {
        let Some(progress) = &snapshot.sync_progress else {
            return 0.0;
        };
        let Ok(filters) = progress.filters() else {
            return 0.0;
        };
        Self::stage_progress(
            filters.state(),
            self.filters_stage_start,
            filters.current_height(),
            filters.target_height(),
        )
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
        // Masternode sync reports a plain current/target ratio, not a resume window.
        Self::stage_progress(mn.state(), Some(0), mn.current_height(), mn.target_height())
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
        // Track last_processed against blocks_target_height regardless of state: blocks can
        // transiently leave Syncing (e.g. WaitForEvents between batches) while still progressing.
        Self::window_fraction(
            self.blocks_stage_start,
            blocks.last_processed(),
            self.blocks_target_height,
        )
    }

    fn any_rpc_backend(&self) -> bool {
        // Chain sync is SPV-only; the RPC wallet backend was removed.
        false
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
                SyncState::Syncing | SyncState::Synced => "Syncing...".to_string(),
            }
        };

        if connected_peers > 0 {
            format!("{stage_message} | Peers: {connected_peers}")
        } else {
            stage_message
        }
    }
}

fn wipe_platform_data_available(role: UserRole, network: Network) -> bool {
    role.at_least(UserRole::Developer) && network == Network::Devnet
}

fn wipe_platform_data_action(role: UserRole, network: Network) -> AppAction {
    if wipe_platform_data_available(role, network) {
        AppAction::BackendTask(BackendTask::SystemTask(SystemTask::WipePlatformData))
    } else {
        AppAction::None
    }
}

impl ScreenLike for NetworkChooserScreen {
    fn refresh_on_arrival(&mut self) {
        // Reset collapsing states when arriving at this screen
        // This ensures dropdowns are closed when navigating back
        self.should_reset_collapsing_states = true;

        // Reload settings from the shared app k/v store to ensure we have the latest values
        let settings = self.current_app_context().get_app_settings();
        self.theme_preference = settings.theme_mode;
        // Re-sync the role selector with the app-global role, which may have
        // changed on another surface (e.g. the onboarding row) since this cached
        // root screen was constructed.
        self.selected_role = self.current_app_context().user_role();
    }

    fn ui(&mut self, ui: &mut egui::Ui) -> AppAction {
        let mut action = add_top_panel_with_global_nav(
            ui,
            self.current_app_context(),
            subdued_everyday_spec("Networks", RootScreenType::RootScreenNetworkChooser),
            vec![],
        );

        action |= add_left_panel(
            ui,
            self.current_app_context(),
            RootScreenType::RootScreenNetworkChooser,
        );

        action |= island_central_panel(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([true; 2])
                .show(ui, |ui| self.render_network_table(ui))
                .inner
        });

        // Dispatch deferred SDK reinit after DAPI discovery
        if self.pending_reinit_after_discovery {
            self.pending_reinit_after_discovery = false;
            action |= AppAction::BackendTask(BackendTask::ReinitCoreClientAndSdk);
        }

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

    fn display_task_result(&mut self, backend_task_success_result: BackendTaskSuccessResult) {
        // The post-DAPI-discovery SDK reinit (`CoreClientReinitialized`) needs
        // no banner of its own — the discovery result already confirmed the
        // refresh ("Updated to N node addresses.").

        // Handle DapiNodesDiscovered (from "Refresh DAPI endpoints" button)
        if let BackendTaskSuccessResult::NetworkDatabaseCleared { .. } =
            &backend_task_success_result
        {
            self.db_clear_in_progress = false;
            self.refresh();
        } else if let BackendTaskSuccessResult::DapiNodesDiscovered {
            network,
            count,
            addresses_csv,
        } = backend_task_success_result
        {
            self.discovery_in_progress = false;

            let persistence_result = self
                .context_for_network(network)
                .cloned()
                .ok_or(TaskError::DapiConfigContextUnavailable { network })
                .and_then(|app_context| persist_dapi_addresses(&app_context, addresses_csv));

            match persistence_result {
                Ok(()) => {
                    self.pending_reinit_after_discovery = true;
                    MessageBanner::set_global(
                        self.current_app_context().egui_ctx(),
                        format!("Updated to {count} node addresses."),
                        MessageType::Success,
                    );
                }
                Err(error) => {
                    MessageBanner::set_global(
                        self.current_app_context().egui_ctx(),
                        error.to_string(),
                        MessageType::Error,
                    )
                    .with_details(error);
                }
            }
        }
    }

    fn display_backend_task_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        if matches!(context, BackendTaskContext::ClearNetworkDatabase) {
            self.db_clear_in_progress = false;
        }
    }

    fn display_message(&mut self, _msg: &str, msg_type: MessageType) {
        // Only reset discovery state on errors — other message types may be unrelated
        if matches!(msg_type, MessageType::Error) && self.discovery_in_progress {
            self.discovery_in_progress = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooser_network_labels_match_the_labels_shown_before_database_clear() {
        assert_eq!(chooser_network_label(Network::Mainnet), "Mainnet");
        assert_eq!(chooser_network_label(Network::Testnet), "Testnet");
        assert_eq!(chooser_network_label(Network::Devnet), "Devnet");
        assert_eq!(chooser_network_label(Network::Regtest), "Local");
    }

    #[test]
    fn wipe_platform_data_is_available_only_to_developers_on_devnet() {
        assert!(!wipe_platform_data_available(
            UserRole::Everyday,
            Network::Devnet
        ));
        assert!(!wipe_platform_data_available(
            UserRole::Power,
            Network::Devnet
        ));
        assert!(!wipe_platform_data_available(
            UserRole::Developer,
            Network::Mainnet
        ));
        assert!(!wipe_platform_data_available(
            UserRole::Developer,
            Network::Testnet
        ));
        assert!(!wipe_platform_data_available(
            UserRole::Developer,
            Network::Regtest
        ));
        assert!(wipe_platform_data_available(
            UserRole::Developer,
            Network::Devnet
        ));
    }

    #[test]
    fn wipe_platform_data_dispatches_the_existing_system_task() {
        assert!(matches!(
            wipe_platform_data_action(UserRole::Developer, Network::Devnet),
            AppAction::BackendTask(BackendTask::SystemTask(SystemTask::WipePlatformData))
        ));
        assert!(matches!(
            wipe_platform_data_action(UserRole::Power, Network::Devnet),
            AppAction::None
        ));
        assert!(matches!(
            wipe_platform_data_action(UserRole::Developer, Network::Testnet),
            AppAction::None
        ));
    }
}
