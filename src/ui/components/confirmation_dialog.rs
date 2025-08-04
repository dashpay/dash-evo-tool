use crate::ui::components::component_trait::{Component, ComponentResponse};
use egui::{Color32, InnerResponse, Ui, WidgetText};

/// Response from showing a confirmation dialog
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmationStatus {
    /// Dialog is still open, no action taken
    None,
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
    pub dialog_response: ConfirmationStatus,
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
        // Return Some(status) if dialog has a response, None if still open
        static CONFIRMED: Option<ConfirmationStatus> = Some(ConfirmationStatus::Confirmed);
        static CANCELED: Option<ConfirmationStatus> = Some(ConfirmationStatus::Canceled);
        static NONE: Option<ConfirmationStatus> = None;

        match self.dialog_response {
            ConfirmationStatus::Confirmed => &CONFIRMED,
            ConfirmationStatus::Canceled => &CANCELED,
            ConfirmationStatus::None => &NONE,
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
        let changed = !matches!(inner_response.inner, ConfirmationStatus::None);
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
    pub fn show_dialog(&mut self, ui: &mut Ui) -> InnerResponse<ConfirmationStatus> {
        let mut is_open = self.is_open;

        if !is_open {
            return InnerResponse::new(
                ConfirmationStatus::Canceled,
                ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
            );
        }

        let mut final_response = ConfirmationStatus::None;
        let window_response = egui::Window::new(self.title.clone())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                // Set minimum width for the dialog
                ui.set_min_width(300.0);

                // Message content
                ui.add_space(10.0);
                ui.label(self.message.clone());
                ui.add_space(20.0);

                // Buttons
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Confirm button (only if text is provided)
                        if let Some(confirm_text) = &self.confirm_text {
                            let confirm_button = if self.danger_mode {
                                egui::Button::new(confirm_text.clone())
                                    .fill(Color32::from_rgb(220, 53, 69)) // Red for danger
                            } else {
                                egui::Button::new(confirm_text.clone())
                                    .fill(Color32::from_rgb(0, 128, 255)) // Blue for primary
                            };

                            if ui.add(confirm_button).clicked() {
                                final_response = ConfirmationStatus::Confirmed;
                            }

                            // Add space only if both buttons are present
                            if self.cancel_text.is_some() {
                                ui.add_space(10.0);
                            }
                        }

                        // Cancel button (only if text is provided)
                        if let Some(cancel_text) = &self.cancel_text {
                            let cancel_button = egui::Button::new(cancel_text.clone())
                                .fill(Color32::from_rgb(108, 117, 125)); // Gray for secondary

                            if ui.add(cancel_button).clicked() {
                                final_response = ConfirmationStatus::Canceled;
                            }
                        }
                    });
                });

                ui.add_space(10.0);
            });

        // Handle window being closed via X button - treat as cancel
        if !is_open && matches!(final_response, ConfirmationStatus::None) {
            final_response = ConfirmationStatus::Canceled;
        }

        // Handle Escape key press - always treat as cancel
        if matches!(final_response, ConfirmationStatus::None)
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            final_response = ConfirmationStatus::Canceled;
        }

        // Update the dialog's open state
        self.is_open = is_open;

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
