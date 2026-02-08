use egui::{Frame, Margin, RichText, Ui};

use crate::ui::theme::DashColors;

/// A reusable component for displaying error messages with an optional expandable
/// "Show details" section for raw technical errors.
///
/// # Usage
/// ```ignore
/// ErrorDisplay::new("Connection to Platform failed.")
///     .with_details("Transport(Status { code: Unavailable, ... })")
///     .show(ui, &mut self.details_expanded);
/// ```
pub struct ErrorDisplay<'a> {
    summary: &'a str,
    details: Option<&'a str>,
}

impl<'a> ErrorDisplay<'a> {
    /// Create a new error display with a user-friendly summary message.
    pub fn new(summary: &'a str) -> Self {
        Self {
            summary,
            details: None,
        }
    }

    /// Add optional technical details that can be expanded by the user.
    pub fn with_details(mut self, details: &'a str) -> Self {
        if !details.is_empty() {
            self.details = Some(details);
        }
        self
    }

    /// Show the error display. The `details_expanded` parameter tracks whether
    /// the details section is currently expanded (caller owns the state).
    /// Returns `true` if the "Dismiss" button was clicked.
    pub fn show(&self, ui: &mut Ui, details_expanded: &mut bool) -> bool {
        let error_color = DashColors::ERROR;
        let mut dismissed = false;

        Frame::new()
            .fill(error_color.gamma_multiply(0.1))
            .inner_margin(Margin::symmetric(10, 8))
            .corner_radius(5.0)
            .stroke(egui::Stroke::new(1.0, error_color))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("Error: {}", self.summary)).color(error_color),
                        )
                        .wrap(),
                    );
                });

                // Show details toggle if details are available
                if let Some(details) = self.details {
                    ui.add_space(4.0);
                    let toggle_text = if *details_expanded {
                        "Hide details"
                    } else {
                        "Show details"
                    };
                    if ui.small_button(toggle_text).clicked() {
                        *details_expanded = !*details_expanded;
                    }

                    if *details_expanded {
                        ui.add_space(4.0);
                        let dark_mode = ui.ctx().style().visuals.dark_mode;
                        let detail_color = DashColors::text_secondary(dark_mode);
                        Frame::new()
                            .fill(DashColors::input_background(dark_mode))
                            .inner_margin(Margin::symmetric(8, 6))
                            .corner_radius(3.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(details).color(detail_color).small(),
                                    )
                                    .wrap(),
                                );
                            });
                    }
                }

                // Dismiss button
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.small_button("Dismiss").clicked() {
                        dismissed = true;
                    }
                });
            });

        dismissed
    }
}
