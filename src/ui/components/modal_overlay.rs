use crate::ui::theme::DashColors;
use egui::Context;

/// Draws a semi-transparent dark overlay behind modal dialogs/popups.
///
/// Call this before rendering the modal window to dim the background
/// and improve focus on the dialog content.
///
/// The `id` parameter should be unique per overlay to avoid egui ID conflicts.
pub fn draw_modal_overlay(ctx: &Context, id: &str) {
    let screen_rect = ctx.screen_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new(id),
    ));
    painter.rect_filled(screen_rect, 0.0, DashColors::modal_overlay());
}
