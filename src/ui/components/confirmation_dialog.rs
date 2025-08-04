use crate::ui::components::component_trait::{Component, ComponentResponse};
use egui::{Color32, InnerResponse, RichText, Ui};

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
/// A reusable confirmation dialog component that implements the Widget trait
///
/// This component provides a consistent modal dialog for confirming user actions
/// across the application. It supports customizable titles, messages, button text,
/// styling options including a danger mode for destructive actions, and callback
/// functions for handling user responses.
///
/// # Examples
///
/// Basic usage with Component trait:
/// ```rust
/// # use dash_evo_tool::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationStatus};
/// # use dash_evo_tool::ui::components::component_trait::Component;
/// # use egui::Ui;
/// # fn example(ui: &mut Ui) {
/// // In your screen struct:
/// // confirmation_dialog: Option<ConfirmationDialog>,
///
/// // In your show method:
/// let confirmation_dialog = self.confirmation_dialog.get_or_insert_with(|| {
///     ConfirmationDialog::new("Confirm Action", "Are you sure?")
/// });
///
/// let response = confirmation_dialog.show(ui);
///
/// if let Some(status) = response.inner.changed_value() {
///     match status {
///         ConfirmationStatus::Confirmed => println!("User confirmed"),
///         ConfirmationStatus::Canceled => println!("User canceled/closed"),
///         ConfirmationStatus::None => {} // This won't happen in changed_value()
///     }
/// }
/// # }
/// ```
///
pub struct ConfirmationDialog {
    title: String,
    message: String,
    confirm_text: String,
    cancel_text: String,
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
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            confirm_text: "Confirm".to_string(),
            cancel_text: "Cancel".to_string(),
            danger_mode: false,
            is_open: true,
        }
    }

    /// Set the text for the confirm button
    pub fn confirm_text(mut self, text: impl Into<String>) -> Self {
        self.confirm_text = text.into();
        self
    }

    /// Set the text for the cancel button
    pub fn cancel_text(mut self, text: impl Into<String>) -> Self {
        self.cancel_text = text.into();
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
        let window_response = egui::Window::new(&self.title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut is_open)
            .show(ui.ctx(), |ui| {
                // Set minimum width for the dialog
                ui.set_min_width(300.0);

                // Message content
                ui.add_space(10.0);
                ui.label(&self.message);
                ui.add_space(20.0);

                // Buttons
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Confirm button
                        let confirm_button = if self.danger_mode {
                            egui::Button::new(
                                RichText::new(&self.confirm_text).color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(220, 53, 69)) // Red for danger
                        } else {
                            egui::Button::new(
                                RichText::new(&self.confirm_text).color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(0, 128, 255)) // Blue for primary
                        };

                        if ui.add(confirm_button).clicked() {
                            final_response = ConfirmationStatus::Confirmed;
                        }

                        ui.add_space(10.0);

                        // Cancel button
                        let cancel_button = egui::Button::new(&self.cancel_text)
                            .fill(Color32::from_rgb(108, 117, 125)); // Gray for secondary

                        if ui.add(cancel_button).clicked() {
                            final_response = ConfirmationStatus::Canceled;
                        }
                    });
                });

                ui.add_space(10.0);
            });

        // Handle window being closed via X button - treat as cancel
        if !is_open && matches!(final_response, ConfirmationStatus::None) {
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
            .confirm_text("Yes")
            .cancel_text("No")
            .danger_mode(true);

        assert_eq!(dialog.title, "Test Title");
        assert_eq!(dialog.message, "Test Message");
        assert_eq!(dialog.confirm_text, "Yes");
        assert_eq!(dialog.cancel_text, "No");
        assert!(dialog.danger_mode);
        assert!(dialog.is_open);
    }
}
