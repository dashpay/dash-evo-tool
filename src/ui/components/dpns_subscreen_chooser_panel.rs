use crate::context::AppContext;
use crate::{app::AppAction, ui::RootScreenType};
use egui::{Context, Frame, Margin, SidePanel};
use std::sync::Arc;

pub fn add_dpns_subscreen_chooser_panel(ctx: &Context, app_context: &Arc<AppContext>) -> AppAction {
    let mut action = AppAction::None;

    let subscreens = vec!["Active contests", "Past contests", "My usernames"];

    SidePanel::left("dpns_subscreen_chooser_panel")
        .default_width(250.0)
        .frame(
            Frame::none()
                .fill(ctx.style().visuals.panel_fill)
                .inner_margin(Margin::same(10.0)),
        )
        .show(ctx, |ui| {
            // Display subscreen names
            ui.vertical(|ui| {
                ui.label("DPNS Subscreens");
                ui.add_space(10.0);

                for subscreen in subscreens {
                    // Show the subscreen name as a clickable option
                    if ui.button(subscreen).clicked() {
                        // Handle navigation based on which subscreen is selected
                        match subscreen {
                            "Active contests" => {
                                action = AppAction::SetMainScreen(
                                    RootScreenType::RootScreenDPNSActiveContests,
                                )
                            }
                            "Past contests" => {
                                action = AppAction::SetMainScreen(
                                    RootScreenType::RootScreenDPNSPastContests,
                                )
                            }
                            "My usernames" => {
                                action = AppAction::SetMainScreen(
                                    RootScreenType::RootScreenDPNSOwnedNames,
                                )
                            }
                            _ => {}
                        }
                    }

                    ui.add_space(5.0);
                }
            });
        });

    action
}
