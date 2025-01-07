use crate::context::AppContext;
use crate::ui::dpns::dpns_contested_names_screen::DPNSSubscreen;
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
                    let is_active = active_screen == subscreen;
                    let (button_color, text_color) = if is_active {
                        (Color32::from_rgb(0, 128, 255), Color32::WHITE)
                    } else {
                        (Color32::GRAY, Color32::WHITE)
                    };
                    let button = egui::Button::new(
                        RichText::new(subscreen.display_name()).color(text_color),
                    )
                    .fill(button_color);
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

                    ui.add_space(5.0);
                }
            });
        });

    action
}
