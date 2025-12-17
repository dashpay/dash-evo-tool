use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::left_panel::load_svg_icon;
use crate::ui::theme::{DashColors, Shape, Spacing};
use crate::ui::{RootScreenType, ScreenType};
use egui::{Color32, Context, RichText, ScrollArea, Vec2};
use std::sync::Arc;

/// The action the user wants to take after onboarding
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingAction {
    LoadWallet,
    CreateWallet,
    ImportIdentity,
    JustBrowse,
}

pub struct WelcomeScreen {
    pub app_context: Arc<AppContext>,
    selected_action: Option<OnboardingAction>,
}

impl WelcomeScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self {
            app_context,
            selected_action: None,
        }
    }

    pub fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ctx.style().visuals.dark_mode;

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

                        ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
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

                            // Getting Started section
                            self.render_getting_started_section(ui, dark_mode);

                            ui.add_space(30.0);

                            // Continue button - centered
                            ui.vertical_centered(|ui| {
                                action |= self.render_continue_button(ui, dark_mode);
                            });

                            ui.add_space(20.0);
                        });
                    });
            });

        action
    }

    fn render_getting_started_section(&mut self, ui: &mut egui::Ui, dark_mode: bool) {
        // Section header - centered
        ui.vertical_centered(|ui| {
            ui.label(
                RichText::new("Get Started")
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
                            "Just Explore",
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
            let _ = self.app_context.db.update_onboarding_completed(true);

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
            };
        }

        if !can_continue && response.hovered() {
            response.on_hover_text("Please select how you'd like to get started");
        }

        AppAction::None
    }
}
