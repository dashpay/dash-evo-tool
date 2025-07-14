use crate::context::AppContext;
use crate::ui::RootScreenType;
use crate::ui::theme::{DashColors, Shadow, Shape, Spacing, Typography};
use crate::ui::tokens::tokens_screen::TokensSubscreen;
use crate::{app::AppAction, ui};
use egui::{Context, Frame, Margin, RichText, SidePanel};

pub fn add_tokens_subscreen_chooser_panel(ctx: &Context, app_context: &AppContext) -> AppAction {
    let mut action = AppAction::None;

    let subscreens = vec![
        TokensSubscreen::MyTokens,
        TokensSubscreen::SearchTokens,
        TokensSubscreen::TokenCreator,
    ];

    let active_screen = match app_context.get_settings() {
        Ok(Some(settings)) => match settings.1 {
            ui::RootScreenType::RootScreenMyTokenBalances => TokensSubscreen::MyTokens,
            ui::RootScreenType::RootScreenTokenSearch => TokensSubscreen::SearchTokens,
            ui::RootScreenType::RootScreenTokenCreator => TokensSubscreen::TokenCreator,
            _ => TokensSubscreen::MyTokens,
        },
        _ => TokensSubscreen::MyTokens, // Fallback to Active screen if settings unavailable
    };

    let dark_mode = ctx.style().visuals.dark_mode;

    SidePanel::left("tokens_subscreen_chooser_panel")
        .resizable(true)
        .default_width(270.0) // Increased to account for margins
        .frame(
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin::symmetric(10, 10)), // Add margins for island effect
        )
        .show(ctx, |ui| {
            // Fill the entire available height
            let available_height = ui.available_height();

            // Create an island panel with rounded edges that fills the height
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .inner_margin(Margin::same(Spacing::XL as i8))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                .shadow(Shadow::elevated())
                .show(ui, |ui| {
                    // Account for both outer margin (10px * 2) and inner margin
                    ui.set_min_height(available_height - 2.0 - (Spacing::XL * 2.0));
                    // Display subscreen names
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Tokens")
                                .font(Typography::heading_small())
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.add_space(Spacing::MD);

                        for subscreen in subscreens {
                            let is_active = active_screen == subscreen;

                            let button = if is_active {
                                egui::Button::new(
                                    RichText::new(subscreen.display_name())
                                        .color(DashColors::WHITE)
                                        .size(Typography::SCALE_SM),
                                )
                                .fill(DashColors::DASH_BLUE)
                                .stroke(egui::Stroke::NONE)
                                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                                .min_size(egui::Vec2::new(150.0, 28.0))
                            } else {
                                egui::Button::new(
                                    RichText::new(subscreen.display_name())
                                        .color(DashColors::text_primary(dark_mode))
                                        .size(Typography::SCALE_SM),
                                )
                                .fill(DashColors::glass_white(dark_mode))
                                .stroke(egui::Stroke::new(1.0, DashColors::border(dark_mode)))
                                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                                .min_size(egui::Vec2::new(150.0, 28.0))
                            };

                            // Show the subscreen name as a clickable option
                            if ui.add(button).clicked() {
                                // Handle navigation based on which subscreen is selected
                                match subscreen {
                                    TokensSubscreen::MyTokens => {
                                        action = AppAction::SetMainScreenThenGoToMainScreen(
                                            RootScreenType::RootScreenMyTokenBalances,
                                        )
                                    }
                                    TokensSubscreen::SearchTokens => {
                                        action = AppAction::SetMainScreenThenGoToMainScreen(
                                            RootScreenType::RootScreenTokenSearch,
                                        )
                                    }
                                    TokensSubscreen::TokenCreator => {
                                        action = AppAction::SetMainScreenThenGoToMainScreen(
                                            RootScreenType::RootScreenTokenCreator,
                                        )
                                    }
                                }
                            }

                            ui.add_space(Spacing::SM);
                        }
                    });
                });
        });

    action
}
