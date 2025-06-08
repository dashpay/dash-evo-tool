use crate::context::AppContext;
use crate::ui::theme::{DashColors, Shadow, Shape, Spacing, Typography};
use crate::ui::RootScreenType;
use crate::{app::AppAction, ui};
use egui::{Color32, Context, Frame, Margin, RichText, SidePanel};

#[derive(PartialEq)]
pub enum ToolsSubscreen {
    ProofLog,
    TransactionViewer,
    DocumentViewer,
    ProofViewer,
    ContractViewer,
}

impl ToolsSubscreen {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::ProofLog => "Proof logs",
            Self::TransactionViewer => "Transaction deserializer",
            Self::ProofViewer => "Proof deserializer",
            Self::DocumentViewer => "Document deserializer",
            Self::ContractViewer => "Contract deserializer",
        }
    }
}

pub fn add_tools_subscreen_chooser_panel(ctx: &Context, app_context: &AppContext) -> AppAction {
    let mut action = AppAction::None;

    let subscreens = vec![
        ToolsSubscreen::ProofLog,
        ToolsSubscreen::ProofViewer,
        ToolsSubscreen::TransactionViewer,
        ToolsSubscreen::DocumentViewer,
        ToolsSubscreen::ContractViewer,
    ];

    let active_screen = match app_context.get_settings() {
        Ok(Some(settings)) => match settings.1 {
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
            _ => ToolsSubscreen::ProofLog,
        },
        _ => ToolsSubscreen::ProofLog, // Fallback to Active screen if settings unavailable
    };

    SidePanel::left("tools_subscreen_chooser_panel")
        .default_width(270.0) // Increased to account for margins
        .frame(
            Frame::new()
                .fill(DashColors::BACKGROUND) // Light background instead of transparent
                .inner_margin(Margin::symmetric(10, 10)), // Add margins for island effect
        )
        .show(ctx, |ui| {
            // Fill the entire available height
            let available_height = ui.available_height();

            // Create an island panel with rounded edges that fills the height
            Frame::new()
                .fill(DashColors::SURFACE)
                .stroke(egui::Stroke::new(1.0, DashColors::BORDER_LIGHT))
                .inner_margin(Margin::same(Spacing::MD_I8))
                .corner_radius(egui::Rounding::same(Shape::RADIUS_LG))
                .shadow(Shadow::elevated())
                .show(ui, |ui| {
                    // Account for both outer margin (10px * 2) and inner margin
                    ui.set_min_height(available_height - 2.0 - (Spacing::MD_I8 as f32 * 2.0));
                    // Display subscreen names
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Tools")
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
                                        .size(Typography::SCALE_SM),
                                )
                                .fill(DashColors::DASH_BLUE)
                                .stroke(egui::Stroke::NONE)
                                .rounding(egui::Rounding::same(Shape::RADIUS_MD))
                                .min_size(egui::Vec2::new(150.0, 28.0))
                            } else {
                                egui::Button::new(
                                    RichText::new(subscreen.display_name())
                                        .color(DashColors::TEXT_PRIMARY)
                                        .size(Typography::SCALE_SM),
                                )
                                .fill(DashColors::WHITE)
                                .stroke(egui::Stroke::new(1.0, DashColors::BORDER))
                                .rounding(egui::Rounding::same(Shape::RADIUS_MD))
                                .min_size(egui::Vec2::new(150.0, 28.0))
                            };

                            // Show the subscreen name as a clickable option
                            if ui.add(button).clicked() {
                                // Handle navigation based on which subscreen is selected
                                match subscreen {
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
                        }
                            }

                            ui.add_space(Spacing::SM);
                        }
                    });
                }); // Close the island frame
        });

    action
}
