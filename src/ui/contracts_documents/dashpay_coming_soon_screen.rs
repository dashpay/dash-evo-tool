use std::sync::Arc;

use eframe::egui::Context;

use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::components::left_panel::add_left_panel;
use crate::ui::components::styled::island_central_panel;
use crate::ui::components::top_panel::add_top_panel;
use crate::ui::{RootScreenType, ScreenLike};

pub struct DashpayScreen {
    pub app_context: Arc<AppContext>,
}

impl DashpayScreen {
    pub fn new(app_context: &Arc<AppContext>) -> Self {
        Self {
            app_context: app_context.clone(),
        }
    }
}

impl ScreenLike for DashpayScreen {
    fn ui(&mut self, ctx: &Context) -> AppAction {
        let mut action = add_top_panel(
            ctx,
            &self.app_context,
            vec![
                ("Contracts", AppAction::GoToMainScreen),
                ("Dashpay", AppAction::None),
            ],
            vec![],
        );

        // Keep Contracts highlighted in the main left panel
        action |= add_left_panel(
            ctx,
            &self.app_context,
            RootScreenType::RootScreenDocumentQuery,
        );

        // Contracts sub-left panel
        action |= crate::ui::components::contracts_subscreen_chooser_panel::add_contracts_subscreen_chooser_panel(
            ctx,
            &self.app_context,
        );

        action |= island_central_panel(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.heading("Coming Soon");
                ui.add_space(20.0);
            });
            AppAction::None
        });

        action
    }
}
