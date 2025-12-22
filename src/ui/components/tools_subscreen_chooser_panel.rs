use crate::context::AppContext;
use crate::ui::RootScreenType;
use crate::ui::theme::{DashColors, Shadow, Shape, Spacing, Typography};
use crate::{app::AppAction, ui};
use egui::{Context, Frame, Margin, RichText, SidePanel};

#[derive(PartialEq)]
pub enum ToolsSubscreen {
    PlatformInfo,
    AddressBalance,
    ProofLog,
    TransactionViewer,
    DocumentViewer,
    ProofViewer,
    ContractViewer,
    GroveSTARK,
    MasternodeListDiff,
}

impl ToolsSubscreen {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::PlatformInfo => "Platform info",
            Self::AddressBalance => "Address balance",
            Self::ProofLog => "Proof logs",
            Self::TransactionViewer => "Transaction deserializer",
            Self::ProofViewer => "Proof deserializer",
            Self::DocumentViewer => "Document deserializer",
            Self::ContractViewer => "Contract deserializer",
            Self::GroveSTARK => "ZK Proofs",
            Self::MasternodeListDiff => "Masternode list diff inspector",
        }
    }
}

pub fn add_tools_subscreen_chooser_panel(ctx: &Context, app_context: &AppContext) -> AppAction {
    let mut action = AppAction::None;
    let dark_mode = ctx.style().visuals.dark_mode;

    let subscreens = vec![
        ToolsSubscreen::PlatformInfo,
        ToolsSubscreen::AddressBalance,
        ToolsSubscreen::ProofLog,
        ToolsSubscreen::ProofViewer,
        ToolsSubscreen::TransactionViewer,
        ToolsSubscreen::DocumentViewer,
        ToolsSubscreen::ContractViewer,
        ToolsSubscreen::GroveSTARK,
        ToolsSubscreen::MasternodeListDiff,
    ];

    let active_screen = match app_context.get_settings() {
        Ok(Some(settings)) => match settings.root_screen_type {
            ui::RootScreenType::RootScreenToolsPlatformInfoScreen => ToolsSubscreen::PlatformInfo,
            ui::RootScreenType::RootScreenToolsAddressBalanceScreen => {
                ToolsSubscreen::AddressBalance
            }
            ui::RootScreenType::RootScreenToolsProofLogScreen => ToolsSubscreen::ProofLog,
            ui::RootScreenType::RootScreenToolsTransitionVisualizerScreen => {
                ToolsSubscreen::TransactionViewer
            }
            ui::RootScreenType::RootScreenToolsProofVisualizerScreen => ToolsSubscreen::ProofViewer,
            ui::RootScreenType::RootScreenToolsDocumentVisualizerScreen => {
                ToolsSubscreen::DocumentViewer
            }
            ui::RootScreenType::RootScreenToolsContractVisualizerScreen => {
                ToolsSubscreen::ContractViewer
            }
            ui::RootScreenType::RootScreenToolsMasternodeListDiffScreen => {
                ToolsSubscreen::MasternodeListDiff
            }
            ui::RootScreenType::RootScreenToolsGroveSTARKScreen => ToolsSubscreen::GroveSTARK,
            _ => ToolsSubscreen::PlatformInfo,
        },
        _ => ToolsSubscreen::PlatformInfo, // Fallback to Active screen if settings unavailable
    };

    SidePanel::left("tools_subscreen_chooser_panel")
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
                        ui.add_space(Spacing::SM);

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
                                    ToolsSubscreen::PlatformInfo => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenToolsPlatformInfoScreen,
                                        )
                                    }
                                    ToolsSubscreen::AddressBalance => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenToolsAddressBalanceScreen,
                                        )
                                    }
                                    ToolsSubscreen::ProofLog => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenToolsProofLogScreen,
                                        )
                                    }
                                    ToolsSubscreen::TransactionViewer => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenToolsTransitionVisualizerScreen,
                                        )
                                    }
                                    ToolsSubscreen::ProofViewer => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenToolsProofVisualizerScreen,
                                        )
                                    }
                                    ToolsSubscreen::DocumentViewer => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenToolsDocumentVisualizerScreen,
                                        )
                                    }
                                    ToolsSubscreen::ContractViewer => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenToolsContractVisualizerScreen,
                                        )
                                    }
                                    ToolsSubscreen::MasternodeListDiff => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenToolsMasternodeListDiffScreen)
                                    }
                                    ToolsSubscreen::GroveSTARK => {
                                        action = AppAction::SetMainScreen(
                                            RootScreenType::RootScreenToolsGroveSTARKScreen)
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
