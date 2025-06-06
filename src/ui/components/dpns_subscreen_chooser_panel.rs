use crate::context::AppContext;
use crate::ui::dpns::dpns_contested_names_screen::DPNSSubscreen;
use crate::ui::theme::{DashColors, Shadow, Shape, Spacing, Typography};
use crate::ui::RootScreenType;
use crate::{app::AppAction, ui};
use egui::{Color32, Context, Frame, Margin, RichText, SidePanel};

pub fn add_dpns_subscreen_chooser_panel(ctx: &Context, app_context: &AppContext) -> AppAction {
    let mut action = AppAction::None;

    let subscreens = vec![
        DPNSSubscreen::Active,
        DPNSSubscreen::Past,
        DPNSSubscreen::Owned,
        DPNSSubscreen::ScheduledVotes,
    ];

    let active_screen = match app_context.get_settings() {
        Ok(Some(settings)) => match settings.1 {
            ui::RootScreenType::RootScreenDPNSActiveContests => DPNSSubscreen::Active,
            ui::RootScreenType::RootScreenDPNSPastContests => DPNSSubscreen::Past,
            ui::RootScreenType::RootScreenDPNSOwnedNames => DPNSSubscreen::Owned,
            ui::RootScreenType::RootScreenDPNSScheduledVotes => DPNSSubscreen::ScheduledVotes,
            _ => DPNSSubscreen::Active,
        },
        _ => DPNSSubscreen::Active, // Fallback to Active screen if settings unavailable
    };

    SidePanel::left("dpns_subscreen_chooser_panel")
        .default_width(270.0) // Increased to account for margins
        .frame(
            Frame::new()
                .fill(DashColors::BACKGROUND) // Light background instead of transparent
                .inner_margin(Margin::symmetric(10, 10)), // Add margins for island effect
        )
        .show(ctx, |ui| {
            // Create an island panel with rounded edges
            Frame::new()
                .fill(DashColors::SURFACE)
                .stroke(egui::Stroke::new(1.0, DashColors::BORDER_LIGHT))
                .inner_margin(Margin::same(Spacing::MD_I8))
                .corner_radius(egui::Rounding::same(Shape::RADIUS_LG))
                .shadow(Shadow::elevated())
                .show(ui, |ui| {
                    // Display subscreen names
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("DPNS Subscreens")
                                .font(Typography::heading_small())
                                .color(DashColors::TEXT_PRIMARY),
                        );
                        ui.add_space(Spacing::MD);

                        for subscreen in subscreens {
                            let is_active = active_screen == subscreen;

                            let button = if is_active {
                                egui::Button::new(
                                    RichText::new(subscreen.display_name())
                                        .color(DashColors::WHITE)
                                        .size(Typography::SCALE_BASE),
                                )
                                .fill(DashColors::DASH_BLUE)
                                .stroke(egui::Stroke::NONE)
                                .rounding(egui::Rounding::same(Shape::RADIUS_MD))
                                .min_size(egui::Vec2::new(200.0, 36.0))
                            } else {
                                egui::Button::new(
                                    RichText::new(subscreen.display_name())
                                        .color(DashColors::TEXT_PRIMARY)
                                        .size(Typography::SCALE_BASE),
                                )
                                .fill(DashColors::WHITE)
                                .stroke(egui::Stroke::new(1.0, DashColors::BORDER))
                                .rounding(egui::Rounding::same(Shape::RADIUS_MD))
                                .min_size(egui::Vec2::new(200.0, 36.0))
                            };

                            // Show the subscreen name as a clickable option
                            if ui.add(button).clicked() {
                                // Handle navigation based on which subscreen is selected
                                match subscreen {
                                    DPNSSubscreen::Active => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenDPNSActiveContests,
                                        )
                                    }
                                    DPNSSubscreen::Past => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenDPNSPastContests,
                                        )
                                    }
                                    DPNSSubscreen::Owned => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenDPNSOwnedNames,
                                        )
                                    }
                                    DPNSSubscreen::ScheduledVotes => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenDPNSScheduledVotes,
                                        )
                                    }
                                }
                            }

                            ui.add_space(Spacing::SM);
                        }
                    });
                }); // Close the island frame
        });

    action
}
