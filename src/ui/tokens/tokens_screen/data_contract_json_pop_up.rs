use crate::ui::theme::{ComponentStyles, DashColors, Shape};
use crate::ui::tokens::tokens_screen::TokensScreen;
use egui::Ui;

impl TokensScreen {
    /// Renders a popup window displaying the data contract JSON.
    pub(super) fn render_data_contract_json_popup(&mut self, ui: &mut Ui) {
        if self.show_json_popup {
            let mut is_open = true;

            // Draw dark overlay behind the dialog for better visibility
            let screen_rect = ui.ctx().screen_rect();
            let painter = ui.ctx().layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("json_popup_overlay"),
            ));
            painter.rect_filled(
                screen_rect,
                0.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120), // Semi-transparent black overlay
            );

            egui::Window::new("Data Contract JSON")
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
                        color: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 100),
                    },
                    fill: ui.style().visuals.window_fill,
                    stroke: egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 30),
                    ),
                })
                .show(ui.ctx(), |ui| {
                    // Display the JSON in a multiline text box
                    let dark_mode = ui.ctx().style().visuals.dark_mode;
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

                    // Close button styled like ConfirmationDialog
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let close_button = egui::Button::new(
                                egui::RichText::new("Close")
                                    .color(ComponentStyles::secondary_button_text()),
                            )
                            .fill(ComponentStyles::secondary_button_fill())
                            .stroke(ComponentStyles::secondary_button_stroke())
                            .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
                            .min_size(egui::Vec2::new(80.0, 32.0));

                            if ui
                                .add(close_button)
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                self.show_json_popup = false;
                            }
                        });
                    });
                });

            // If the user closed the window via the "x" in the corner
            // we should reflect that in `show_json_popup`.
            if !is_open {
                self.show_json_popup = false;
            }
        }
    }
}
