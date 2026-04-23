//! Identity Home tab — scaffold.
//!
//! Renders the hero card + quick actions row + secondary actions row +
//! onboarding checklist + recent activity preview. See design-spec §B.2 / §B.3.
//!
//! This file is the T3 scaffold; full behaviour lands in T8.

use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::theme::DashColors;
use eframe::egui::{RichText, Ui};
use std::sync::Arc;

pub fn render(ui: &mut Ui, _app_context: &Arc<AppContext>) -> AppAction {
    let dark_mode = ui.ctx().style().visuals.dark_mode;
    ui.vertical_centered(|ui| {
        ui.add_space(12.0);
        ui.label(
            RichText::new("Identity home is under construction.")
                .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "The hero, quick actions, and recent activity will arrive in a follow-up task.",
            )
            .small()
            .color(DashColors::text_secondary(dark_mode)),
        );
    });
    AppAction::None
}
