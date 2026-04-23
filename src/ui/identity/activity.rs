//! Activity tab — scaffold.
//!
//! The unified activity timeline (payments + funding + platform ops) depends on
//! a backend aggregator that does not exist yet. While that work is in flight,
//! this tab renders a gated-state message pointing users to the existing
//! DashPay Payments screen. Gated by the `identity-hub-activity-feed` feature.

use crate::app::AppAction;
use crate::context::AppContext;
use crate::ui::theme::DashColors;
use eframe::egui::{RichText, Ui};
use std::sync::Arc;

pub fn render(ui: &mut Ui, _app_context: &Arc<AppContext>) -> AppAction {
    let dark_mode = ui.ctx().style().visuals.dark_mode;

    ui.vertical_centered(|ui| {
        ui.add_space(12.0);

        // Filter chips — placeholder strip so the spec traceability works.
        ui.horizontal(|ui| {
            for chip in ["All", "Payments", "Funding"] {
                let _ = ui.selectable_label(chip == "All", chip);
            }
        });

        ui.add_space(16.0);

        #[cfg(feature = "identity-hub-activity-feed")]
        {
            ui.label(
                RichText::new("Unified activity timeline coming next.")
                    .color(DashColors::text_secondary(dark_mode)),
            );
        }

        #[cfg(not(feature = "identity-hub-activity-feed"))]
        {
            ui.label(
                RichText::new("Unified activity is coming soon.")
                    .strong()
                    .color(DashColors::text_primary(dark_mode)),
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new("For now, view activity on the existing DashPay Payments screen.")
                    .color(DashColors::text_secondary(dark_mode)),
            );
        }
    });
    AppAction::None
}
