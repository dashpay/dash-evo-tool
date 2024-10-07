use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::RootScreenType;
use eframe::epaint::{Color32, Margin};
use egui::{Context, Frame, SidePanel};
use std::sync::Arc;
pub fn add_left_panel(
    ctx: &Context,
    app_context: &Arc<AppContext>,
    selected_screen: RootScreenType,
) -> AppAction {
    let mut action = AppAction::None;

    let panel_width = 50.0 + 20.0; // Button width (50) + 10px margin on each side (20 total)

    SidePanel::left("left_panel")
        .default_width(panel_width)
        .frame(
            Frame::none()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(Margin {
                    left: 10.0,
                    right: 10.0,
                    top: 10.0,
                    bottom: 0.0, // No bottom margin since we'll manage the vertical space manually
                }),
        )
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                // "I" button for Identities screen
                let is_selected = selected_screen == RootScreenType::RootScreenIdentities;
                let button_color = if is_selected {
                    Color32::from_rgb(100, 149, 237) // A highlighted blue color for selected
                } else {
                    Color32::from_rgb(169, 169, 169) // Default gray color for unselected
                };

                let button = egui::Button::new("I")
                    .fill(button_color)
                    .min_size(egui::vec2(50.0, 50.0));

                if ui.add(button).clicked() {
                    action = AppAction::SetMainScreen(RootScreenType::RootScreenIdentities);
                }

                ui.add_space(10.0); // Add some space between buttons

                // "C" button for Contests screen
                let is_selected = selected_screen == RootScreenType::RootScreenDPNSContestedNames;
                let button_color = if is_selected {
                    Color32::from_rgb(100, 149, 237) // Highlighted blue color for selected
                } else {
                    Color32::from_rgb(169, 169, 169) // Default gray color for unselected
                };

                let button = egui::Button::new("C")
                    .fill(button_color)
                    .min_size(egui::vec2(50.0, 50.0));

                if ui.add(button).clicked() {
                    action = AppAction::SetMainScreen(RootScreenType::RootScreenDPNSContestedNames);
                }

                ui.add_space(10.0); // Add some space between buttons

                // "T" button for Transition visualizer
                let is_selected =
                    selected_screen == RootScreenType::RootScreenTransitionVisualizerScreen;
                let button_color = if is_selected {
                    Color32::from_rgb(100, 149, 237) // Highlighted blue color for selected
                } else {
                    Color32::from_rgb(169, 169, 169) // Default gray color for unselected
                };

                let button = egui::Button::new("T")
                    .fill(button_color)
                    .min_size(egui::vec2(50.0, 50.0));

                if ui.add(button).clicked() {
                    action = AppAction::SetMainScreen(
                        RootScreenType::RootScreenTransitionVisualizerScreen,
                    );
                }
            });
        });

    action
}
