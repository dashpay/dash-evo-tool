use crate::app::AppAction;
use crate::context::AppContext;
use crate::model::contested_name::ContestedName;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{RootScreenType, ScreenLike};
use egui::{Context, Ui};
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

    fn show_contested_name(ui: &mut Ui, contested_name: &ContestedName) -> AppAction {
        let mut action = AppAction::None;

        ui.heading(&contested_name.normalized_contested_name);
        ui.label(format!("Locked Votes: {}", contested_name.locked_votes));
        ui.label(format!("Abstain Votes: {}", contested_name.abstain_votes));

        if let Some(ending_time) = contested_name.ending_time {
            ui.label(format!("Ending Time: {}", ending_time));
        }

        // Iterate over contestants and create clickable buttons for each
        for contestant in &contested_name.contestants {
            let button_text = format!("{} - {} votes", contestant.name, contestant.votes);
            if ui.button(button_text).clicked() {
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
            None,
        );
        // Some((
        //     "Get Username",
        //     DesiredAppAction::AddScreenType(ScreenType::AddDPNSUsername),
        // )),

        let mut action = add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDPNSContestedNames,
        );

        // Main content
        egui::CentralPanel::default().show(ctx, |ui| {
            let contested_names = self.contested_names.lock().unwrap();

            egui::ScrollArea::vertical().show(ui, |ui| {
                // Iterate over contested names and display each
                for contested_name in contested_names.iter() {
                    action |= Self::show_contested_name(ui, contested_name);
                    ui.separator(); // Add a separator between contested names
                }
            });
        });

        action
    }
}
