use crate::app::{AppAction, DesiredAppAction};
use crate::context::AppContext;
use crate::model::contested_name::ContestedName;
use crate::platform::contested_names::ContestedResourceTask;
use crate::platform::BackendTask;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{RootScreenType, ScreenLike};
use egui::{Context, Frame, Margin, Ui};
use egui_extras::{Column, TableBuilder};
use std::sync::{Arc, Mutex};

pub struct DPNSContestedNamesScreen {
    contested_names: Arc<Mutex<Vec<ContestedName>>>,
    app_context: Arc<AppContext>,
}

impl DPNSContestedNamesScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        let contested_names = Arc::new(Mutex::new(
            app_context.load_contested_names().unwrap_or_default(),
        ));
        Self {
            contested_names,
            app_context: app_context.clone(),
        }
    }

    fn show_contested_name_details(
        &self,
        ui: &mut Ui,
        contested_name: &ContestedName,
    ) -> AppAction {
        let mut action = AppAction::None;

        // Iterate over contestants and create clickable buttons for each
        for contestant in &contested_name.contestants {
            let button_text = format!("{} - {} votes", contestant.name, contestant.votes);
            if ui.button(button_text).clicked() {
                // Placeholder for action when a contestant is clicked
                action = AppAction::None;
            }
        }

        action
    }
}

impl ScreenLike for DPNSContestedNamesScreen {
    fn refresh(&mut self) {
        let mut contested_names = self.contested_names.lock().unwrap();
        *contested_names = self.app_context.load_contested_names().unwrap_or_default();
    }

    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![("Home", AppAction::None)],
            Some((
                "Refresh",
                DesiredAppAction::BackendTask(BackendTask::ContestedResourceTask(
                    ContestedResourceTask::QueryDPNSContestedResources,
                )),
            )),
        );

        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDPNSContestedNames,
        );

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            let contested_names = self.contested_names.lock().unwrap();

            egui::ScrollArea::vertical().show(ui, |ui| {
                // Define a frame for the table
                Frame::group(ui.style())
                    .fill(ui.visuals().panel_fill)
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
                            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                            // Define columns with resizing and alignment
                            .column(Column::initial(200.0).resizable(true)) // Contested Name
                            .column(Column::initial(100.0).resizable(true)) // Locked Votes
                            .column(Column::initial(100.0).resizable(true)) // Abstain Votes
                            .column(Column::initial(200.0).resizable(true)) // Ending Time
                            .column(Column::initial(200.0).resizable(true)) // Contestants
                            .header(30.0, |mut header| {
                                header.col(|ui| {
                                    ui.heading("Contested Name");
                                });
                                header.col(|ui| {
                                    ui.heading("Locked Votes");
                                });
                                header.col(|ui| {
                                    ui.heading("Abstain Votes");
                                });
                                header.col(|ui| {
                                    ui.heading("Ending Time");
                                });
                                header.col(|ui| {
                                    ui.heading("Contestants");
                                });
                            })
                            .body(|mut body| {
                                for contested_name in contested_names.iter() {
                                    body.row(25.0, |mut row| {
                                        row.col(|ui| {
                                            ui.label(&contested_name.normalized_contested_name);
                                        });
                                        row.col(|ui| {
                                            ui.label(format!("{}", contested_name.locked_votes));
                                        });
                                        row.col(|ui| {
                                            ui.label(format!("{}", contested_name.abstain_votes));
                                        });
                                        row.col(|ui| {
                                            if let Some(ending_time) = contested_name.ending_time {
                                                ui.label(format!("{}", ending_time));
                                            } else {
                                                ui.label("N/A");
                                            }
                                        });
                                        row.col(|ui| {
                                            action |= self
                                                .show_contested_name_details(ui, contested_name);
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
