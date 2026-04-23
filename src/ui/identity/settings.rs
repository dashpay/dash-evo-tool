//! Settings tab — scaffold.
//!
//! Social profile (left column) · username + aliases (right column) · advanced
//! expander · danger zone. See design-spec §B.8.

use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::theme::DashColors;
use eframe::egui::{RichText, Ui};
use std::sync::Arc;

pub fn render(ui: &mut Ui, _app_context: &Arc<AppContext>) -> AppAction {
    let dark_mode = ui.ctx().style().visuals.dark_mode;
    ui.vertical(|ui| {
        ui.add_space(12.0);
        ui.label(
            RichText::new("Social profile")
                .strong()
                .size(18.0)
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.label(
            RichText::new("This information is visible to everyone on Dash Platform.")
                .small()
                .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(16.0);
        ui.label(
            RichText::new("Username")
                .strong()
                .size(18.0)
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(16.0);
        ui.label(
            RichText::new("Aliases")
                .strong()
                .size(18.0)
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.label(
            RichText::new("Extra usernames that also point to your identity.")
                .small()
                .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(16.0);
        ui.collapsing(
            RichText::new("Advanced")
                .color(DashColors::text_primary(dark_mode))
                .strong(),
            |ui| {
                ui.label(
                    RichText::new("Keys, raw identifiers, and identity type.")
                        .small()
                        .color(DashColors::text_secondary(dark_mode)),
                );
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Content arrives in a follow-up task.")
                        .color(DashColors::text_secondary(dark_mode)),
                );
            },
        );
    });
    AppAction::None
}
