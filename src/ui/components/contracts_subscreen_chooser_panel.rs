use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::theme::{DashColors, Shadow, Shape, Spacing, Typography};
use crate::ui::{self, RootScreenType};
use egui::{Context, Frame, Margin, RichText, SidePanel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractsSubscreen {
    Contracts,
    DPNS,
    DashPay,
}

impl ContractsSubscreen {
    pub fn display_name(&self) -> &'static str {
        match self {
            ContractsSubscreen::Contracts => "All Contracts",
            ContractsSubscreen::DPNS => "DPNS",
            ContractsSubscreen::DashPay => "DashPay",
        }
    }
}

pub fn add_contracts_subscreen_chooser_panel(ctx: &Context, app_context: &AppContext) -> AppAction {
    let mut action = AppAction::None;

    let subscreens = vec![
        ContractsSubscreen::Contracts,
        ContractsSubscreen::DPNS,
        ContractsSubscreen::DashPay,
    ];

    // Determine active selection from settings; default to Contracts
    let active_screen = match app_context.get_settings() {
        Ok(Some(settings)) => match settings.root_screen_type {
            ui::RootScreenType::RootScreenDocumentQuery => ContractsSubscreen::Contracts,
            ui::RootScreenType::RootScreenDPNSActiveContests
            | ui::RootScreenType::RootScreenDPNSPastContests
            | ui::RootScreenType::RootScreenDPNSOwnedNames
            | ui::RootScreenType::RootScreenDPNSScheduledVotes => ContractsSubscreen::DPNS,
            ui::RootScreenType::RootScreenDashpay => ContractsSubscreen::DashPay,
            _ => ContractsSubscreen::Contracts,
        },
        _ => ContractsSubscreen::Contracts,
    };

    let dark_mode = ctx.style().visuals.dark_mode;

    SidePanel::left("contracts_subscreen_chooser_panel")
        .resizable(false)
        .default_width(270.0)
        .frame(
            Frame::new()
                .fill(DashColors::background(dark_mode))
                .inner_margin(Margin::symmetric(10, 10)),
        )
        .show(ctx, |ui| {
            let available_height = ui.available_height();

            Frame::new()
                .fill(DashColors::surface(dark_mode))
                .stroke(egui::Stroke::new(1.0, DashColors::border_light(dark_mode)))
                .inner_margin(Margin::same(Spacing::XL as i8))
                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_LG))
                .shadow(Shadow::elevated())
                .show(ui, |ui| {
                    ui.set_min_height(available_height - 2.0 - (Spacing::XL * 2.0));
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Contracts")
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

                            if ui.add(button).clicked() {
                                action = match subscreen {
                                    ContractsSubscreen::Contracts => {
                                        AppAction::SetMainScreenThenGoToMainScreen(
                                            RootScreenType::RootScreenDocumentQuery,
                                        )
                                    }
                                    ContractsSubscreen::DPNS => {
                                        AppAction::SetMainScreenThenGoToMainScreen(
                                            RootScreenType::RootScreenDPNSActiveContests,
                                        )
                                    }
                                    ContractsSubscreen::DashPay => {
                                        AppAction::SetMainScreenThenGoToMainScreen(
                                            RootScreenType::RootScreenDashpay,
                                        )
                                    }
                                };
                            }

                            ui.add_space(Spacing::SM);
                        }
                    });
                });
        });

    action
}
