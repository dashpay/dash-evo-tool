use crate::context::AppContext;
use crate::ui::RootScreenType;
use crate::ui::dashpay::dashpay_screen::DashPaySubscreen;
use crate::ui::theme::{DashColors, Shadow, Shape, Spacing, Typography};
use crate::{app::AppAction, ui};
use egui::{Context, Frame, Margin, RichText, SidePanel};
use std::sync::Arc;

pub fn add_dashpay_subscreen_chooser_panel(
    ctx: &Context,
    app_context: &Arc<AppContext>,
    current_subscreen: DashPaySubscreen,
) -> AppAction {
    let mut action = AppAction::None;
    let dark_mode = ctx.style().visuals.dark_mode;

    let subscreens = vec![
        DashPaySubscreen::Contacts,
        DashPaySubscreen::Requests,
        DashPaySubscreen::Profile,
        DashPaySubscreen::Payments,
    ];

    let active_screen = current_subscreen;

    SidePanel::left("dashpay_subscreen_chooser_panel")
        .default_width(270.0) // Increased to account for margins
        .frame(
            Frame::new()
                .fill(DashColors::background(dark_mode)) // Light background instead of transparent
                .inner_margin(Margin::symmetric(10, 10)), // Add margins for island effect
        )
        .show(ctx, |ui| {
            // Fill the entire available height
            let available_height = ui.available_height();

            // Create an island panel with rounded edges that fills the height
            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .inner_margin(Margin::same(Spacing::MD_I8))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                .shadow(Shadow::elevated())
                .show(ui, |ui| {
                    // Account for both outer margin (10px * 2) and inner margin
                    ui.set_min_height(available_height - 2.0 - (Spacing::MD_I8 as f32 * 2.0));
                    // Display subscreen names
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("DashPay Sections")
                                .font(Typography::heading_small())
                                .color(DashColors::text_primary(dark_mode)),
                        );
                        ui.add_space(Spacing::MD);

                        for subscreen in subscreens {
                            let is_active = active_screen == subscreen;

                            let display_name = match subscreen {
                                DashPaySubscreen::Contacts => "My Contacts",
                                DashPaySubscreen::Requests => "Contact Requests",
                                DashPaySubscreen::Profile => "My Profile",
                                DashPaySubscreen::Payments => "Payment History",
                            };

                            let button = if is_active {
                                egui::Button::new(
                                    RichText::new(display_name)
                                        .color(DashColors::WHITE)
                                        .size(Typography::SCALE_SM),
                                )
                                .fill(DashColors::DASH_BLUE)
                                .stroke(egui::Stroke::NONE)
                                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_MD))
                                .min_size(egui::Vec2::new(150.0, 28.0))
                            } else {
                                egui::Button::new(
                                    RichText::new(display_name)
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
                                    DashPaySubscreen::Contacts => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenDashPayContacts,
                                        )
                                    }
                                    DashPaySubscreen::Requests => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenDashPayRequests,
                                        )
                                    }
                                    DashPaySubscreen::Profile => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenDashPayProfile,
                                        )
                                    }
                                    DashPaySubscreen::Payments => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenDashPayPayments,
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
