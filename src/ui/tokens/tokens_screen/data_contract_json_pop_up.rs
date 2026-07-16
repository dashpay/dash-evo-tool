use crate::ui::helpers::clicked_outside_window_after_open;
use crate::ui::theme::{ComponentStyles, DashColors};
use crate::ui::tokens::tokens_screen::TokensScreen;
use egui::Ui;

impl TokensScreen {
    pub(super) fn open_data_contract_json_popup(&mut self, text: String) {
        self.json_popup_text = text;
        self.json_popup_opening_guard.arm();
        self.show_json_popup = true;
    }

    /// Renders a popup window displaying the data contract JSON.
    pub(super) fn render_data_contract_json_popup(&mut self, ui: &mut Ui) {
        if self.show_json_popup {
            let mut is_open = true;

            // Draw dark overlay behind the dialog for better visibility
            let screen_rect = ui.ctx().content_rect();
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("json_popup_overlay"),
            ));
            painter.rect_filled(screen_rect, 0.0, DashColors::modal_overlay());

            let window_response = egui::Window::new("Data Contract JSON")
                .collapsible(false)
                .resizable(true)
                .max_height(600.0)
                .max_width(800.0)
                .scroll(true)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .open(&mut is_open)
                .frame(egui::Frame {
                    inner_margin: egui::Margin::same(16),
                    outer_margin: egui::Margin::same(0),
                    corner_radius: egui::CornerRadius::same(8),
                    shadow: egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 16,
                        spread: 0,
                        color: DashColors::popup_shadow(),
                    },
                    fill: ui.style().visuals.window_fill,
                    stroke: egui::Stroke::new(1.0, DashColors::popup_border_glow()),
                })
                .show(ui.ctx(), |ui| {
                    // Display the JSON in a multiline text box
                    let dark_mode = ui.style().visuals.dark_mode;
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new("Below is the data contract JSON:")
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(10.0);

                    egui::Resize::default()
                        .id_salt("json_resize_area_for_contract")
                        .default_size([750.0, 550.0])
                        .max_height(ui.available_height() - 50.0)
                        .max_width(ui.available_height() - 20.0)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    ui.monospace(&mut self.json_popup_text);
                                });
                        });

                    ui.add_space(20.0);

                    // Close button — right-aligned to match ConfirmationDialog pattern
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let dark_mode = ui.style().visuals.dark_mode;
                        if ComponentStyles::add_secondary_button(ui, "Close", dark_mode).clicked() {
                            self.show_json_popup = false;
                        }
                    });
                });

            // Handle Escape key
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.show_json_popup = false;
            }

            // Handle click outside window
            if let Some(ref wr) = window_response
                && self.show_json_popup
                && clicked_outside_window_after_open(
                    ui.ctx(),
                    wr.response.rect,
                    &mut self.json_popup_opening_guard,
                )
            {
                self.show_json_popup = false;
            }

            // If the user closed the window via the "x" in the corner
            if !is_open {
                self.show_json_popup = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::helpers::ModalOpeningGuard;

    #[test]
    fn opening_click_does_not_immediately_close_json_popup() {
        let ctx = egui::Context::default();
        let window_rect =
            egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 100.0));
        let outside_pos = egui::pos2(0.0, 0.0);
        let raw = egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(outside_pos),
                egui::Event::PointerButton {
                    pos: outside_pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        let mut guard = ModalOpeningGuard::armed();
        let mut closed = true;

        let _ = ctx.run_ui(raw, |ui| {
            closed = clicked_outside_window_after_open(ui.ctx(), window_rect, &mut guard);
        });

        assert!(!closed, "the JSON popup must survive its opening click");
    }
}
