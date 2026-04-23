//! Contacts tab — scaffold.
//!
//! Renders either the populated Contacts page (received requests · active
//! contacts · sent requests) or a gated-state card when the selected identity
//! has no social profile. See design-spec §B.4 and §B.4.1.

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
            RichText::new("Set up a social profile first.")
                .strong()
                .size(20.0)
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Contacts use your display name and avatar to let people find you. A social \
                 profile only unlocks contacts. Without a social profile, you cannot add \
                 contacts or receive contact requests.",
            )
            .color(DashColors::text_secondary(dark_mode)),
        );
    });
    AppAction::None
}
