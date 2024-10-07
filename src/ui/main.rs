use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::model::qualified_identity::{IdentityType, QualifiedIdentity};
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{ScreenLike, ScreenType};
use dpp::identity::accessors::IdentityGettersV0;
use dpp::platform_value::string_encoding::Encoding;
use eframe::egui::{self, Context};
use eframe::emath::Align;
use egui::{Frame, Margin};
use egui_extras::{Column, TableBuilder};
use std::sync::{Arc, Mutex};

pub struct MainScreen {
    identities: Arc<Mutex<Vec<QualifiedIdentity>>>,
    app_context: Arc<AppContext>,
}

impl ScreenLike for MainScreen {
    fn refresh(&mut self) {
        let mut identities = self.identities.lock().unwrap();
        *identities = self.app_context.load_identities().unwrap_or_default();
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Home", AppAction::None)],
            Some((
                "Add Identity",
                DesiredAppAction::AddScreenType(ScreenType::AddIdentity),
            )),
        );

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            let identities = self.identities.lock().unwrap();

            egui::ScrollArea::vertical().show(ui, |ui| {
                // Define a frame with custom background color and border
                Frame::group(ui.style())
                    .fill(ui.visuals().panel_fill) // Use panel fill color
                    .stroke(egui::Stroke::new(
                        1.0,
                        ui.visuals().widgets.inactive.bg_stroke.color,
                    ))
                    .inner_margin(Margin::same(8.0))
                    .show(ui, |ui| {
                        // Build the table
                        TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(Align::Center))
                            // Define columns with resizing and alignment
                            .column(Column::initial(200.0).resizable(true)) // Identity ID
                            .column(Column::initial(100.0).resizable(true)) // Balance
                            .column(Column::initial(100.0).resizable(true)) // Type
                            .column(Column::initial(80.0).resizable(true)) // Keys
                            .column(Column::initial(80.0).resizable(true)) // Withdraw
                            .column(Column::initial(80.0).resizable(true)) // Transfer
                            .header(30.0, |mut header| {
                                header.col(|ui| {
                                    ui.heading("Identity ID");
                                });
                                header.col(|ui| {
                                    ui.heading("Balance");
                                });
                                header.col(|ui| {
                                    ui.heading("Type");
                                });
                                header.col(|ui| {
                                    ui.heading("Keys");
                                });
                                header.col(|ui| {
                                    ui.heading("Withdraw");
                                });
                                header.col(|ui| {
                                    ui.heading("Transfer");
                                });
                            })
                            .body(|mut body| {
                                for identity in identities.iter() {
                                    body.row(25.0, |mut row| {
                                        row.col(|ui| {
                                            let encoding = match identity.identity_type {
                                                IdentityType::User => Encoding::Base58,
                                                IdentityType::Masternode
                                                | IdentityType::Evonode => Encoding::Hex,
                                            };
                                            ui.label(format!(
                                                "{}",
                                                identity.identity.id().to_string(encoding)
                                            ));
                                        });
                                        row.col(|ui| {
                                            ui.label(format!("{}", identity.identity.balance()));
                                        });
                                        row.col(|ui| {
                                            ui.label(format!("{}", identity.identity_type));
                                        });
                                        row.col(|ui| {
                                            if ui.button("Keys").clicked() {
                                                // todo
                                            }
                                        });
                                        row.col(|ui| {
                                            if ui.button("Withdraw").clicked() {
                                                // Implement Withdraw functionality
                                            }
                                        });
                                        row.col(|ui| {
                                            if ui.button("Transfer").clicked() {
                                                // Implement Transfer functionality
                                            }
                                        });
                                    });
                                }
                            });
                    });
            });
        });

        action
    }
}

impl MainScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let identities = Arc::new(Mutex::new(
            app_context.load_identities().unwrap_or_default(),
        ));
        Self {
            identities,
            app_context: app_context.clone(),
        }
    }
}
