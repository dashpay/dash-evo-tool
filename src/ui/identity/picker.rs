//! Identity picker grid — scaffold.
//!
//! Rendered when the hub detects ≥ 2 identities on the active network. See
//! design-spec §B.14. Full grid layout + `IdentityPickerCard` component lands in
//! T7.

use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::theme::DashColors;
use eframe::egui::{RichText, Ui};
use std::sync::Arc;

pub fn render(ui: &mut Ui, _app_context: &Arc<AppContext>) -> AppAction {
    let dark_mode = ui.ctx().style().visuals.dark_mode;
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        ui.label(
            RichText::new("Pick an identity")
                .size(24.0)
                .strong()
                .color(DashColors::text_primary(dark_mode)),
        );
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Each identity has its own balance, keys, and optional social profile. \
                 Choose one to open it, or add a new identity.",
            )
            .color(DashColors::text_secondary(dark_mode)),
        );
        ui.add_space(16.0);
        ui.label(
            RichText::new("Grid of identity cards arrives in a follow-up task.")
                .small()
                .color(DashColors::text_secondary(dark_mode)),
        );
    });
    AppAction::None
}
