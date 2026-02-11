use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::left_panel::load_svg_icon;
use crate::ui::components::styled::island_central_panel;
use crate::ui::theme::{DashColors, Shadow, Shape, Spacing};
use crate::ui::{RootScreenType, ScreenType};
use egui::{Context, RichText, ScrollArea, Vec2};
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
}

impl WelcomeScreen {
    pub fn new(app_context: Arc<AppContext>) -> Self {
        Self { app_context }
    }

    pub fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = AppAction::None;
        let dark_mode = ctx.style().visuals.dark_mode;

        // Central panel with welcome content (using island style like other screens)
        island_central_panel(ctx, |ui| {
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(80.0);

                        // Logo
                        if let Some(logo) = load_svg_icon(ctx, "dashlogo.svg", 200, 80) {
                            ui.add(
                                egui::Image::new(&logo).fit_to_exact_size(Vec2::new(150.0, 60.0)),
                            );
                        }

                        ui.add_space(24.0);

                        // Title
                        ui.label(
                            RichText::new("Welcome to Dash Evo Tool")
                                .size(28.0)
                                .strong()
                                .color(DashColors::text_primary(dark_mode)),
                        );

                        ui.add_space(8.0);

                        // Subtitle
                        ui.label(
                            RichText::new("Your gateway to decentralized data")
                                .size(16.0)
                                .color(DashColors::text_secondary(dark_mode)),
                        );

                        ui.add_space(50.0);

                        // Instructional text
                        ui.label(
                            RichText::new("Select an option to get started:")
                                .size(14.0)
                                .color(DashColors::text_secondary(dark_mode)),
                        );

                        ui.add_space(16.0);

                        // Getting Started section - cards directly trigger navigation
                        action |= self.render_getting_started_section(ui, dark_mode);

                        ui.add_space(40.0);
                    });
                });
        });

        action
    }

    fn render_getting_started_section(&mut self, ui: &mut egui::Ui, dark_mode: bool) -> AppAction {
        let card_spacing = 16.0;
        // Card dimensions: 170 inner + 16*2 padding + ~2 border = ~204 per card
        let card_visual_width = 170.0 + (Spacing::MD * 2.0) + 2.0;
        let total_width = (card_visual_width * 3.0) + (card_spacing * 2.0);

        let mut action = AppAction::None;

        // Use a fixed-width horizontal layout so it can be centered properly
        ui.allocate_ui(Vec2::new(total_width, 100.0), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = card_spacing;

                action |= self.render_action_card(
                    ui,
                    dark_mode,
                    OnboardingAction::CreateWallet,
                    "Create Wallet",
                    "Start fresh with a new HD wallet",
                );

                action |= self.render_action_card(
                    ui,
                    dark_mode,
                    OnboardingAction::LoadWallet,
                    "Import Wallet",
                    "Load a wallet you already have",
                );

                action |= self.render_action_card(
                    ui,
                    dark_mode,
                    OnboardingAction::JustBrowse,
                    "Just Explore",
                    "Explore without setting up",
                );
            });
        });

        action
    }

    fn render_action_card(
        &self,
        ui: &mut egui::Ui,
        dark_mode: bool,
        onboarding_action: OnboardingAction,
        title: &str,
        description: &str,
    ) -> AppAction {
        let card_width = 170.0;
        let card_height = 60.0;

        let bg_color = DashColors::background(dark_mode);
        let border_color = DashColors::border_light(dark_mode);

        let response = egui::Frame::new()
            .fill(bg_color)
            .stroke(egui::Stroke::new(1.0, border_color))
            .corner_radius(Shape::RADIUS_LG)
            .shadow(Shadow::small())
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
                            .color(DashColors::text_primary(dark_mode)),
                    );

                    ui.add_space(6.0);

                    ui.label(
                        RichText::new(description)
                            .size(11.0)
                            .color(DashColors::text_secondary(dark_mode)),
                    );
                });
            });

        if response.response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        if response.response.interact(egui::Sense::click()).clicked() {
            // Save settings to database
            let _ = self.app_context.db.update_onboarding_completed(true);

            // Return OnboardingComplete with navigation based on selection
            let (main_screen, add_screen) = match onboarding_action {
                OnboardingAction::CreateWallet => (
                    RootScreenType::RootScreenWalletsBalances,
                    Some(Box::new(ScreenType::AddNewWallet)),
                ),
                OnboardingAction::LoadWallet => (
                    RootScreenType::RootScreenWalletsBalances,
                    Some(Box::new(ScreenType::ImportMnemonic)),
                ),
                OnboardingAction::ImportIdentity => (RootScreenType::RootScreenIdentities, None),
                OnboardingAction::JustBrowse => (RootScreenType::RootScreenDashPayProfile, None),
            };

            return AppAction::OnboardingComplete {
                main_screen,
                add_screen,
            };
        }

        AppAction::None
    }
}
