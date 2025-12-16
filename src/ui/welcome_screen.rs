use crate::app::AppAction;
use crate::backend_task::core::{CoreItem, CoreTask};
use crate::backend_task::{BackendTask, BackendTaskSuccessResult};
use crate::config::Config;
use crate::context::AppContext;
use crate::model::settings::UserMode;
use crate::ui::components::left_panel::load_svg_icon;
use crate::ui::theme::{DashColors, Shape, Spacing};
use crate::ui::{RootScreenType, ScreenType};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::identity::TimestampMillis;
use egui::{Color32, Context, RichText, ScrollArea, Vec2};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The action the user wants to take after onboarding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingAction {
    LoadWallet,
    CreateWallet,
    ImportIdentity,
    JustBrowse,
}

/// Connection status for a network
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Unknown,
    Checking,
    Connected,
    NotConnected,
}

pub struct WelcomeScreen {
    pub mainnet_app_context: Arc<AppContext>,
    pub testnet_app_context: Option<Arc<AppContext>>,
    pub devnet_app_context: Option<Arc<AppContext>>,
    pub local_app_context: Option<Arc<AppContext>>,
    selected_action: Option<OnboardingAction>,
    show_evonode_tools: bool,
    user_mode: UserMode,
    /// The currently selected network
    pub selected_network: Network,
    /// Connection status for each network
    pub mainnet_status: ConnectionStatus,
    pub testnet_status: ConnectionStatus,
    pub devnet_status: ConnectionStatus,
    pub local_status: ConnectionStatus,
    /// Local network dashmate password
    pub local_network_password: String,
    /// Time to recheck connection status
    pub recheck_time: Option<TimestampMillis>,
    /// Whether initial check has been triggered
    initial_check_triggered: bool,
}

impl WelcomeScreen {
    pub fn new(
        mainnet_app_context: Arc<AppContext>,
        testnet_app_context: Option<Arc<AppContext>>,
        devnet_app_context: Option<Arc<AppContext>>,
        local_app_context: Option<Arc<AppContext>>,
    ) -> Self {
        // Load local network password from config
        let local_network_password = if let Ok(config) = Config::load() {
            if let Some(local_config) = config.config_for_network(Network::Regtest) {
                local_config.core_rpc_password.clone()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        Self {
            mainnet_app_context,
            testnet_app_context,
            devnet_app_context,
            local_app_context,
            selected_action: None,
            show_evonode_tools: false,
            user_mode: UserMode::Beginner,
            selected_network: Network::Dash,
            mainnet_status: ConnectionStatus::Unknown,
            testnet_status: ConnectionStatus::Unknown,
            devnet_status: ConnectionStatus::Unknown,
            local_status: ConnectionStatus::Unknown,
            local_network_password,
            recheck_time: None,
            initial_check_triggered: false,
        }
    }

    /// Get the app context for the selected network
    pub fn current_app_context(&self) -> &Arc<AppContext> {
        match self.selected_network {
            Network::Dash => &self.mainnet_app_context,
            Network::Testnet => self
                .testnet_app_context
                .as_ref()
                .unwrap_or(&self.mainnet_app_context),
            Network::Devnet => self
                .devnet_app_context
                .as_ref()
                .unwrap_or(&self.mainnet_app_context),
            Network::Regtest => self
                .local_app_context
                .as_ref()
                .unwrap_or(&self.mainnet_app_context),
            _ => &self.mainnet_app_context,
        }
    }

    /// Handle task results from backend
    pub fn display_task_result(&mut self, result: BackendTaskSuccessResult) {
        if let BackendTaskSuccessResult::CoreItem(CoreItem::ChainLocks(
            mainnet_chainlock,
            testnet_chainlock,
            devnet_chainlock,
            local_chainlock,
        )) = result
        {
            self.mainnet_status = if mainnet_chainlock.is_some() {
                ConnectionStatus::Connected
            } else {
                ConnectionStatus::NotConnected
            };
            self.testnet_status = if testnet_chainlock.is_some() {
                ConnectionStatus::Connected
            } else {
                ConnectionStatus::NotConnected
            };
            self.devnet_status = if devnet_chainlock.is_some() {
                ConnectionStatus::Connected
            } else {
                ConnectionStatus::NotConnected
            };
            self.local_status = if local_chainlock.is_some() {
                ConnectionStatus::Connected
            } else {
                ConnectionStatus::NotConnected
            };
        }
    }

    pub fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ctx.style().visuals.dark_mode;

        // Trigger initial connection check
        if !self.initial_check_triggered {
            self.initial_check_triggered = true;
            self.mainnet_status = ConnectionStatus::Checking;
            self.testnet_status = ConnectionStatus::Checking;
            self.devnet_status = ConnectionStatus::Checking;
            self.local_status = ConnectionStatus::Checking;
            action = AppAction::BackendTask(BackendTask::CoreTask(CoreTask::GetBestChainLocks));
        }

        // Periodic recheck every 5 seconds (but don't change displayed status during recheck)
        let recheck_interval = Duration::from_secs(5);
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards");

        if action == AppAction::None {
            if let Some(time) = self.recheck_time {
                if current_time.as_millis() as u64 >= time {
                    action =
                        AppAction::BackendTask(BackendTask::CoreTask(CoreTask::GetBestChainLocks));
                    self.recheck_time = Some((current_time + recheck_interval).as_millis() as u64);
                }
            } else {
                self.recheck_time = Some((current_time + recheck_interval).as_millis() as u64);
            }
        }

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(DashColors::background(dark_mode))
                    .inner_margin(egui::Margin::same(20)),
            )
            .show(ctx, |ui| {
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Calculate centered content area
                        let content_width = 700.0_f32.min(ui.available_width());
                        let available_width = ui.available_width();
                        let left_margin = ((available_width - content_width) / 2.0).max(0.0);

                        // Create a centered rect for content
                        let content_rect = egui::Rect::from_min_size(
                            ui.cursor().min + egui::vec2(left_margin, 0.0),
                            egui::vec2(content_width, ui.available_height()),
                        );

                        ui.allocate_ui_at_rect(content_rect, |ui| {
                            ui.set_min_width(content_width);
                            ui.set_max_width(content_width);

                            ui.add_space(20.0);

                            // Logo - centered within content area
                            ui.vertical_centered(|ui| {
                                if let Some(logo) = load_svg_icon(ctx, "dashlogo.svg", 200, 80) {
                                    ui.add(
                                        egui::Image::new(&logo)
                                            .fit_to_exact_size(Vec2::new(150.0, 60.0)),
                                    );
                                }
                            });

                            ui.add_space(20.0);

                            // Title - centered
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("Welcome to Dash Evo Tool")
                                        .size(28.0)
                                        .strong()
                                        .color(DashColors::text_primary(dark_mode)),
                                );
                            });

                            ui.add_space(8.0);

                            // Subtitle - centered
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("Your gateway to decentralized data")
                                        .size(16.0)
                                        .color(DashColors::text_secondary(dark_mode)),
                                );
                            });

                            ui.add_space(30.0);

                            // Network Selection section
                            action |= self.render_network_selection_section(ui, dark_mode);

                            ui.add_space(30.0);

                            // Preferences section
                            self.render_preferences_section(ui, dark_mode);

                            ui.add_space(30.0);

                            // Getting Started section
                            self.render_getting_started_section(ui, dark_mode);

                            ui.add_space(30.0);

                            // Continue button - centered
                            ui.vertical_centered(|ui| {
                                action |= self.render_continue_button(ui, dark_mode);
                            });

                            ui.add_space(20.0);

                            // Footer text - centered
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new(
                                        "You can change these settings anytime in Settings.",
                                    )
                                    .size(12.0)
                                    .color(DashColors::text_secondary(dark_mode))
                                    .italics(),
                                );
                            });

                            ui.add_space(20.0);
                        });
                    });
            });

        action
    }

    fn render_network_selection_section(
        &mut self,
        ui: &mut egui::Ui,
        dark_mode: bool,
    ) -> AppAction {
        let action = AppAction::None;

        // Section header - centered
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("1. Select Network")
                    .size(18.0)
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
        });

        ui.add_space(16.0);

        // Network selection cards - use centered_and_justified to center the row
        // Card inner: 140x60, frame inner margin: 16*2=32, so visual width ~176 per card
        let card_visual_width = 140.0 + (Spacing::MD * 2.0) + 4.0; // +4 for frame stroke
        let card_spacing = 12.0;
        let total_width = (card_visual_width * 4.0) + (card_spacing * 3.0);
        let row_height = 60.0 + (Spacing::MD * 2.0) + 4.0;

        // Center the card row by using centered_and_justified layout
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), row_height),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(total_width, row_height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = card_spacing;

                        // Mainnet
                        self.render_network_card(
                            ui,
                            dark_mode,
                            Network::Dash,
                            "Mainnet",
                            "Production network",
                            true,
                        );

                        // Testnet
                        let testnet_available = self.testnet_app_context.is_some();
                        self.render_network_card(
                            ui,
                            dark_mode,
                            Network::Testnet,
                            "Testnet",
                            "Test network",
                            testnet_available,
                        );

                        // Devnet
                        let devnet_available = self.devnet_app_context.is_some();
                        self.render_network_card(
                            ui,
                            dark_mode,
                            Network::Devnet,
                            "Devnet",
                            "Development network",
                            devnet_available,
                        );

                        // Local
                        let local_available = self.local_app_context.is_some();
                        self.render_network_card(
                            ui,
                            dark_mode,
                            Network::Regtest,
                            "Local",
                            "Local dashmate",
                            local_available,
                        );
                    },
                );
            },
        );

        action
    }

    fn render_network_card(
        &mut self,
        ui: &mut egui::Ui,
        dark_mode: bool,
        network: Network,
        title: &str,
        description: &str,
        available: bool,
    ) {
        let is_selected = self.selected_network == network;
        let card_width = 140.0;
        let card_height = 60.0;

        let (bg_color, border_color) = if !available {
            (
                DashColors::surface(dark_mode).gamma_multiply(0.5),
                DashColors::border_light(dark_mode).gamma_multiply(0.5),
            )
        } else if is_selected {
            (
                DashColors::DASH_BLUE.gamma_multiply(0.15),
                DashColors::DASH_BLUE,
            )
        } else {
            (
                DashColors::surface(dark_mode),
                DashColors::border_light(dark_mode),
            )
        };

        let response = egui::Frame::new()
            .fill(bg_color)
            .stroke(egui::Stroke::new(
                if is_selected { 1.0 } else { 1.0 },
                border_color,
            ))
            .corner_radius(Shape::RADIUS_LG)
            .shadow(egui::Shadow::NONE)
            .inner_margin(Spacing::MD)
            .show(ui, |ui| {
                ui.set_min_size(Vec2::new(card_width, card_height));
                ui.set_max_size(Vec2::new(card_width, card_height));

                ui.vertical_centered(|ui| {
                    ui.add_space(5.0);

                    ui.label(
                        RichText::new(title)
                            .size(14.0)
                            .strong()
                            .color(if !available {
                                DashColors::text_secondary(dark_mode).gamma_multiply(0.5)
                            } else if is_selected {
                                DashColors::DASH_BLUE
                            } else {
                                DashColors::text_primary(dark_mode)
                            }),
                    );

                    ui.add_space(4.0);

                    ui.label(RichText::new(description).size(11.0).color(if !available {
                        DashColors::text_secondary(dark_mode).gamma_multiply(0.5)
                    } else {
                        DashColors::text_secondary(dark_mode)
                    }));
                });
            });

        if available {
            if response.response.interact(egui::Sense::click()).clicked() {
                self.selected_network = network;
            }

            if response.response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
    }

    fn render_getting_started_section(&mut self, ui: &mut egui::Ui, dark_mode: bool) {
        // Section header - centered
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("3. Get Setup")
                    .size(18.0)
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
        });

        ui.add_space(16.0);

        // Option cards - use same centering approach
        // Card inner: 170x60, frame inner margin: 16*2=32, so visual width ~206 per card
        let card_visual_width = 170.0 + (Spacing::MD * 2.0) + 4.0;
        let card_spacing = 16.0;
        let total_width = (card_visual_width * 3.0) + (card_spacing * 2.0);
        let row_height = 60.0 + (Spacing::MD * 2.0) + 4.0;

        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), row_height),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(total_width, row_height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = card_spacing;

                        self.render_action_card(
                            ui,
                            dark_mode,
                            OnboardingAction::CreateWallet,
                            "Create Wallet",
                            "Start fresh with a new HD wallet",
                        );

                        self.render_action_card(
                            ui,
                            dark_mode,
                            OnboardingAction::LoadWallet,
                            "Import Wallet",
                            "Load a wallet you already have",
                        );

                        self.render_action_card(
                            ui,
                            dark_mode,
                            OnboardingAction::JustBrowse,
                            "Skip For Now",
                            "Explore without setting up",
                        );
                    },
                );
            },
        );
    }

    fn render_action_card(
        &mut self,
        ui: &mut egui::Ui,
        dark_mode: bool,
        action: OnboardingAction,
        title: &str,
        description: &str,
    ) {
        let is_selected = self.selected_action == Some(action);
        let card_width = 170.0;
        let card_height = 60.0;

        let (bg_color, border_color) = if is_selected {
            (
                DashColors::DASH_BLUE.gamma_multiply(0.15),
                DashColors::DASH_BLUE,
            )
        } else {
            (
                DashColors::surface(dark_mode),
                DashColors::border_light(dark_mode),
            )
        };

        let response = egui::Frame::new()
            .fill(bg_color)
            .stroke(egui::Stroke::new(1.0, border_color))
            .corner_radius(Shape::RADIUS_LG)
            .shadow(egui::Shadow::NONE)
            .inner_margin(Spacing::MD)
            .show(ui, |ui| {
                ui.set_min_size(Vec2::new(card_width, card_height));
                ui.set_max_size(Vec2::new(card_width, card_height));

                ui.vertical_centered(|ui| {
                    ui.add_space(5.0);

                    ui.label(
                        RichText::new(title)
                            .size(14.0)
                            .strong()
                            .color(if is_selected {
                                DashColors::DASH_BLUE
                            } else {
                                DashColors::text_primary(dark_mode)
                            }),
                    );

                    ui.add_space(6.0);

                    ui.label(
                        RichText::new(description)
                            .size(11.0)
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                });
            });

        if response.response.interact(egui::Sense::click()).clicked() {
            self.selected_action = Some(action);
        }

        if response.response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }

    fn render_preferences_section(&mut self, ui: &mut egui::Ui, dark_mode: bool) {
        // Section header - centered
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("2. Select Preferences")
                    .size(18.0)
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
        });

        ui.add_space(16.0);

        // Center the preferences card with a constrained width
        let card_width = 420.0;
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 200.0), // height will auto-adjust
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                egui::Frame::new()
                    .fill(DashColors::surface(dark_mode))
                    .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                    .corner_radius(Shape::RADIUS_LG)
                    .inner_margin(Spacing::LG)
                    .show(ui, |ui| {
                        ui.set_max_width(card_width);
                        ui.set_min_width(card_width);
                        ui.vertical(|ui| {
                            // Experience Level
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Experience Level:")
                                        .color(DashColors::text_primary(dark_mode)),
                                );

                                ui.add_space(10.0);

                                // Beginner button
                                let beginner_selected = self.user_mode == UserMode::Beginner;
                                let beginner_btn = egui::Button::new(
                                    RichText::new("Beginner").color(if beginner_selected {
                                        Color32::WHITE
                                    } else {
                                        DashColors::text_primary(dark_mode)
                                    }),
                                )
                                .fill(if beginner_selected {
                                    DashColors::DASH_BLUE
                                } else {
                                    DashColors::glass_white(dark_mode)
                                })
                                .corner_radius(Shape::RADIUS_MD)
                                .min_size(Vec2::new(80.0, 28.0));

                                if ui.add(beginner_btn).clicked() {
                                    self.user_mode = UserMode::Beginner;
                                }

                                // Advanced button
                                let advanced_selected = self.user_mode == UserMode::Advanced;
                                let advanced_btn = egui::Button::new(
                                    RichText::new("Advanced").color(if advanced_selected {
                                        Color32::WHITE
                                    } else {
                                        DashColors::text_primary(dark_mode)
                                    }),
                                )
                                .fill(if advanced_selected {
                                    DashColors::DASH_BLUE
                                } else {
                                    DashColors::glass_white(dark_mode)
                                })
                                .corner_radius(Shape::RADIUS_MD)
                                .min_size(Vec2::new(80.0, 28.0));

                                if ui.add(advanced_btn).clicked() {
                                    self.user_mode = UserMode::Advanced;
                                }
                            });

                            ui.add_space(4.0);

                            ui.label(
                                RichText::new(if self.user_mode == UserMode::Beginner {
                                    "Simplified interface with guided options"
                                } else {
                                    "Full access to all features and options"
                                })
                                .size(11.0)
                                .color(DashColors::text_secondary(dark_mode))
                                .italics(),
                            );

                            ui.add_space(16.0);
                            ui.separator();
                            ui.add_space(16.0);

                            // Evonode tools toggle
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Are you running an Evonode?")
                                        .color(DashColors::text_primary(dark_mode)),
                                );

                                ui.add_space(10.0);

                                // No button
                                let no_selected = !self.show_evonode_tools;
                                let no_btn =
                                    egui::Button::new(RichText::new("No").color(if no_selected {
                                        Color32::WHITE
                                    } else {
                                        DashColors::text_primary(dark_mode)
                                    }))
                                    .fill(if no_selected {
                                        DashColors::DASH_BLUE
                                    } else {
                                        DashColors::glass_white(dark_mode)
                                    })
                                    .corner_radius(Shape::RADIUS_MD)
                                    .min_size(Vec2::new(50.0, 28.0));

                                if ui.add(no_btn).clicked() {
                                    self.show_evonode_tools = false;
                                }

                                // Yes button
                                let yes_selected = self.show_evonode_tools;
                                let yes_btn = egui::Button::new(RichText::new("Yes").color(
                                    if yes_selected {
                                        Color32::WHITE
                                    } else {
                                        DashColors::text_primary(dark_mode)
                                    },
                                ))
                                .fill(if yes_selected {
                                    DashColors::DASH_BLUE
                                } else {
                                    DashColors::glass_white(dark_mode)
                                })
                                .corner_radius(Shape::RADIUS_MD)
                                .min_size(Vec2::new(50.0, 28.0));

                                if ui.add(yes_btn).clicked() {
                                    self.show_evonode_tools = true;
                                }
                            });

                            ui.add_space(4.0);

                            ui.label(
                                RichText::new(if self.show_evonode_tools {
                                    "Evonode management tools will be shown"
                                } else {
                                    "Evonode tools will be hidden from the interface"
                                })
                                .size(11.0)
                                .color(DashColors::text_secondary(dark_mode))
                                .italics(),
                            );
                        });
                    });
            },
        );
    }

    fn render_continue_button(&mut self, ui: &mut egui::Ui, _dark_mode: bool) -> AppAction {
        let can_continue = self.selected_action.is_some();

        let button = egui::Button::new(RichText::new("Enter").size(16.0).color(Color32::WHITE))
            .fill(if can_continue {
                DashColors::DASH_BLUE
            } else {
                Color32::GRAY
            })
            .corner_radius(Shape::RADIUS_LG)
            .min_size(Vec2::new(200.0, 44.0));

        let response = ui.add_enabled(can_continue, button);

        if response.clicked() {
            // Save settings to database
            let _ = self
                .mainnet_app_context
                .db
                .update_onboarding_completed(true);
            let _ = self
                .mainnet_app_context
                .db
                .update_show_evonode_tools(self.show_evonode_tools);
            let _ = self
                .mainnet_app_context
                .db
                .update_user_mode(self.user_mode.as_str());

            // Return OnboardingComplete with navigation based on selection
            let (main_screen, add_screen) = match self.selected_action {
                Some(OnboardingAction::CreateWallet) => (
                    RootScreenType::RootScreenWalletsBalances,
                    Some(Box::new(ScreenType::AddNewWallet)),
                ),
                Some(OnboardingAction::LoadWallet) => (
                    RootScreenType::RootScreenWalletsBalances,
                    Some(Box::new(ScreenType::ImportMnemonic)),
                ),
                Some(OnboardingAction::ImportIdentity) => {
                    (RootScreenType::RootScreenIdentities, None)
                }
                Some(OnboardingAction::JustBrowse) | None => {
                    (RootScreenType::RootScreenDashPayProfile, None)
                }
            };

            return AppAction::OnboardingComplete {
                main_screen,
                add_screen,
                network: self.selected_network,
            };
        }

        if !can_continue && response.hovered() {
            response.on_hover_text("Please select how you'd like to get started");
        }

        AppAction::None
    }
}
