use crate::ui::dpns_contested_names_screen::DPNSSubscreen;
use crate::{app::AppAction, ui::RootScreenType};
use egui::{Context, Frame, Margin, SidePanel};

pub fn add_dpns_subscreen_chooser_panel(ctx: &Context) -> AppAction {
    let mut action = AppAction::None;

    let subscreens = vec![
        DPNSSubscreen::Active,
        DPNSSubscreen::Past,
        DPNSSubscreen::Owned,
    ];

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
                    if ui.button(subscreen.display_name()).clicked() {
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
                            _ => {}
                        }
                    }

                    ui.add_space(5.0);
                }
            });
        });

    action
}
