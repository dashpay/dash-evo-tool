use std::sync::Arc;

use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::theme::{ComponentStyles, DashColors, Shape};
use egui::{InnerResponse, Ui, WidgetText};

/// Response from showing a confirmation dialog
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmationStatus {
    /// User clicked confirm button
    Confirmed,
    /// User clicked cancel button or closed dialog
    Canceled,
}

pub const NOTHING: Option<&str> = None;
/// Response struct for the ConfirmationDialog component following the Component trait pattern
#[derive(Debug, Clone)]
pub struct ConfirmationDialogComponentResponse {
    pub response: egui::Response,
    pub changed: bool,
    pub error_message: Option<String>,
    pub dialog_response: Option<ConfirmationStatus>,
}

impl ComponentResponse for ConfirmationDialogComponentResponse {
    type DomainType = ConfirmationStatus;

    fn has_changed(&self) -> bool {
        self.changed
    }

    fn is_valid(&self) -> bool {
        self.error_message.is_none()
    }

    fn changed_value(&self) -> &Option<Self::DomainType> {
        if self.has_changed() {
            &self.dialog_response
        } else {
            &None
        }
    }

    fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }
}
/// A reusable confirmation dialog component that implements the Component trait
///
/// This component provides a consistent modal dialog for confirming user actions
/// across the application. It supports customizable titles, messages, button text
/// with rich formatting (using WidgetText for styling), danger mode for destructive
/// actions, and optional buttons (confirm and cancel buttons can be hidden independently).
/// The dialog can be dismissed by pressing Escape (treated as cancel) or clicking the X button.
pub struct ConfirmationDialog {
    title: WidgetText,
    message: WidgetText,
    status: Option<ConfirmationStatus>,
    confirm_text: Option<WidgetText>,
    cancel_text: Option<WidgetText>,
    danger_mode: bool,
    is_open: bool,
}

impl Component for ConfirmationDialog {
    type DomainType = ConfirmationStatus;
    type Response = ConfirmationDialogComponentResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        let inner_response = self.show_dialog(ui);
        let changed = inner_response.inner.is_some();
        let response = inner_response.response;

        InnerResponse::new(
            ConfirmationDialogComponentResponse {
                response: response.clone(),
                changed,
                error_message: None, // Confirmation dialogs don't have validation errors
                dialog_response: inner_response.inner,
            },
            response,
        )
    }

    fn current_value(&self) -> Option<Self::DomainType> {
        // Return the current dialog state - None if still open, Some(status) if closed
        if self.is_open {
            None
        } else {
            Some(ConfirmationStatus::Canceled) // If dialog is closed, it was canceled
        }
    }
}

impl ConfirmationDialog {
    /// Create a new confirmation dialog with the given title and message
    pub fn new(title: impl Into<WidgetText>, message: impl Into<WidgetText>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            confirm_text: Some("Confirm".into()),
            cancel_text: Some("Cancel".into()),
            danger_mode: false,
            is_open: true,
            status: None, // No action taken yet
        }
    }

    /// Set the text for the confirm button, or None to hide it
    pub fn confirm_text(mut self, text: Option<impl Into<WidgetText>>) -> Self {
        self.confirm_text = text.map(|t| t.into());
        self
    }

    /// Set the text for the cancel button, or None to hide it
    pub fn cancel_text(mut self, text: Option<impl Into<WidgetText>>) -> Self {
        self.cancel_text = text.map(|t| t.into());
        self
    }

    /// Enable danger mode (red confirm button) for destructive actions
    pub fn danger_mode(mut self, enabled: bool) -> Self {
        self.danger_mode = enabled;
        self
    }

    /// Set whether the dialog is open
    pub fn open(mut self, open: bool) -> Self {
        self.is_open = open;
        self
    }
}

impl ConfirmationDialog {
    /// Show the dialog and return the user's response
    fn show_dialog(&mut self, ui: &mut Ui) -> InnerResponse<Option<ConfirmationStatus>> {
        let mut is_open = self.is_open;

        if !is_open {
            return InnerResponse::new(
                None, // no change
                ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
            );
        }

        // Draw dark overlay behind the dialog for better visibility
        let screen_rect = ui.ctx().screen_rect();
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("confirmation_dialog_overlay"),
        ));
        painter.rect_filled(
            screen_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120), // Semi-transparent black overlay
        );

        let mut final_response = None;
        let window_response = egui::Window::new(self.title.clone())
            .collapsible(false)
            .resizable(false)
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
                // Set minimum width for the dialog
                ui.set_min_width(300.0);

                let dark_mode = ui.ctx().style().visuals.dark_mode;

                // Message content with bold text and proper color
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(self.message.text())
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.add_space(20.0);

                // Buttons
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Confirm button (only if text is provided)
                        if let Some(confirm_text) = &self.confirm_text {
                            let (fill_color, text_color) = if self.danger_mode {
                                (
                                    ComponentStyles::danger_button_fill(),
                                    ComponentStyles::danger_button_text(),
                                )
                            } else {
                                (
                                    ComponentStyles::primary_button_fill(),
                                    ComponentStyles::primary_button_text(),
                                )
                            };
                            let confirm_label = if let WidgetText::RichText(rich_text) =
                                confirm_text
                            {
                                // preserve rich text formatting
                                rich_text.clone()
                            } else {
                                Arc::new(egui::RichText::new(confirm_text.text()).color(text_color))
                            };

                            let confirm_button = egui::Button::new(confirm_label)
                                .fill(fill_color)
                                .stroke(if self.danger_mode {
                                    egui::Stroke::NONE
                                } else {
                                    ComponentStyles::primary_button_stroke()
                                })
                                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
                                .min_size(egui::Vec2::new(80.0, 32.0));

                            if ui
                                .add(confirm_button)
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                final_response = Some(ConfirmationStatus::Confirmed);
                            }
                        }

                        // Cancel button (only if text is provided)
                        if let Some(cancel_text) = &self.cancel_text {
                            let cancel_label = if let WidgetText::RichText(rich_text) = cancel_text
                            {
                                // preserve rich text formatting
                                rich_text.clone()
                            } else {
                                egui::RichText::new(cancel_text.text())
                                    .color(ComponentStyles::secondary_button_text())
                                    .into()
                            };

                            let cancel_button = egui::Button::new(cancel_label)
                                .fill(ComponentStyles::secondary_button_fill())
                                .stroke(ComponentStyles::secondary_button_stroke())
                                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
                                .min_size(egui::Vec2::new(80.0, 32.0));

                            if ui
                                .add(cancel_button)
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                final_response = Some(ConfirmationStatus::Canceled);
                            }

                            ui.add_space(8.0); // Add spacing between buttons
                        }
                    });
                });
            });

        // Handle window being closed via X button - treat as cancel
        if !is_open && final_response.is_none() {
            final_response = Some(ConfirmationStatus::Canceled);
        }

        // Handle Escape key press - always treat as cancel
        if final_response.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            final_response = Some(ConfirmationStatus::Canceled);
        }

        // Update the dialog's state
        self.is_open = is_open;
        // if user actually did something, update the status
        if final_response.is_some() {
            self.status = final_response.clone();
        }

        if let Some(window_response) = window_response {
            InnerResponse::new(final_response, window_response.response)
        } else {
            InnerResponse::new(
                final_response,
                ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
            )
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_confirmation_dialog_creation() {
        let dialog = ConfirmationDialog::new("Test Title", "Test Message")
            .confirm_text(Some("Yes"))
            .cancel_text(Some("No"))
            .danger_mode(true);

        assert_eq!(dialog.title.text(), "Test Title");
        assert_eq!(dialog.message.text(), "Test Message");
        assert!(dialog.confirm_text.is_some_and(|t| t.text() == "Yes"));
        assert!(dialog.cancel_text.is_some_and(|t| t.text() == "No"));
        assert!(dialog.danger_mode);
        assert!(dialog.is_open);
    }

    #[test]
    fn test_confirmation_dialog_no_buttons() {
        let dialog = ConfirmationDialog::new("Test Title", "Test Message")
            .confirm_text(NOTHING)
            .cancel_text(NOTHING);

        assert_eq!(dialog.title.text(), "Test Title");
        assert_eq!(dialog.message.text(), "Test Message");
        assert!(dialog.confirm_text.is_none());
        assert!(dialog.cancel_text.is_none());
        assert!(!dialog.danger_mode);
        assert!(dialog.is_open);
    }

    #[test]
    fn test_confirmation_dialog_only_confirm_button() {
        let dialog = ConfirmationDialog::new("Test Title", "Test Message")
            .confirm_text(Some("OK"))
            .cancel_text(NOTHING);

        assert_eq!(dialog.title.text(), "Test Title");
        assert_eq!(dialog.message.text(), "Test Message");
        assert!(dialog.confirm_text.is_some());
        assert!(dialog.cancel_text.is_none());
        assert!(!dialog.danger_mode);
        assert!(dialog.is_open);
    }

    #[test]
    fn test_confirmation_dialog_only_cancel_button() {
        let dialog = ConfirmationDialog::new("Test Title", "Test Message")
            .confirm_text(NOTHING)
            .cancel_text(Some("Close"));

        assert_eq!(dialog.title.text(), "Test Title");
        assert_eq!(dialog.message.text(), "Test Message");
        assert!(dialog.confirm_text.is_none());
        assert!(dialog.cancel_text.is_some());
        assert!(!dialog.danger_mode);
        assert!(dialog.is_open);
    }
}
