use crate::ui::tokens::tokens_screen::TokensScreen;
use egui::Ui;

impl TokensScreen {
    /// Renders a popup window displaying the data contract JSON.
    pub(super) fn render_data_contract_json_popup(&mut self, ui: &mut Ui) {
        if self.show_json_popup {
            let mut is_open = true;
            egui::Window::new("Data Contract JSON")
                .collapsible(false)
                .resizable(true)
                .max_height(600.0)
                .max_width(800.0)
                .scroll(true)
                .open(&mut is_open)
                .show(ui.ctx(), |ui| {
                    // Display the JSON in a multiline text box
                    ui.add_space(4.0);
                    ui.label("Below is the data contract JSON:");
                    ui.add_space(4.0);

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

                    ui.add_space(10.0);

                    // A button to close
                    if ui.button("Close").clicked() {
                        self.show_json_popup = false;
                    }
                });

            // If the user closed the window via the "x" in the corner
            // we should reflect that in `show_json_popup`.
            if !is_open {
                self.show_json_popup = false;
            }
        }
    }
}
