use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::ui::{RootScreenType, ScreenType};
use eframe::epaint::{Color32, Margin, Stroke};
use egui::{Context, Frame, Layout, RichText, SidePanel, TopBottomPanel};
use std::sync::Arc;

pub fn add_left_panel(
    ctx: &Context,
    app_context: &Arc<AppContext>,
    selected_screen: RootScreenType,
) -> AppAction {
    let mut action = AppAction::None;

    SidePanel::left("left_panel")
        .frame(Frame::none().fill(ctx.style().visuals.panel_fill)) // Optional: adjust styling
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
            });
        });

    action
}
