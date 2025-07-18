use egui::{Color32, InnerResponse, RichText, Ui, Widget};

/// Response from showing a confirmation dialog
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmationDialogResponse {
    /// Dialog is still open, no action taken
    None,
    /// User clicked confirm button
    Confirmed,
    /// User clicked cancel button or closed dialog
    Canceled,
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
/// Basic usage:
/// ```rust
/// # use dash_evo_tool::ui::components::confirmation_dialog::{ConfirmationDialog, ConfirmationDialogResponse};
/// # use egui::Ui;
/// # fn example(ui: &mut Ui) {
/// let response = ConfirmationDialog::new("Confirm Action", "Are you sure?")
///     .show(ui);
///
/// match response.inner {
///      ConfirmationDialogResponse::Confirmed => println!("User confirmed"),
///      ConfirmationDialogResponse::Canceled => println!("User canceled"),
///      ConfirmationDialogResponse::None => println!("Dialog still open"),
///      _ => {}
/// };
/// # }
/// ```
///
/// With custom styling:
/// ```rust
/// # use dash_evo_tool::ui::components::confirmation_dialog::ConfirmationDialog;
/// # use egui::Ui;
/// # fn example(ui: &mut Ui) {
/// let response = ConfirmationDialog::new("Delete Item", "This action cannot be undone")
///     .confirm_text("Delete")
///     .cancel_text("Keep")
///     .danger_mode(true)
///     .show(ui);
/// # }
/// ```
pub struct ConfirmationDialog<F = fn(), G = fn()> {
    title: String,
    message: String,
    confirm_text: String,
    cancel_text: String,
    danger_mode: bool,
    is_open: bool,
    on_confirm: Option<F>,
    on_cancel: Option<G>,
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
            on_confirm: None,
            on_cancel: None,
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

impl<F, G> ConfirmationDialog<F, G>
where
    F: FnOnce(),
    G: FnOnce(),
{
    /// Set a callback to execute when the user clicks the confirm button
    pub fn on_confirm<F2>(self, callback: F2) -> ConfirmationDialog<F2, G>
    where
        F2: FnOnce(),
    {
        ConfirmationDialog {
            title: self.title,
            message: self.message,
            confirm_text: self.confirm_text,
            cancel_text: self.cancel_text,
            danger_mode: self.danger_mode,
            is_open: self.is_open,
            on_confirm: Some(callback),
            on_cancel: self.on_cancel,
        }
    }

    /// Set a callback to execute when the user clicks the cancel button or closes the dialog
    pub fn on_cancel<G2>(self, callback: G2) -> ConfirmationDialog<F, G2>
    where
        G2: FnOnce(),
    {
        ConfirmationDialog {
            title: self.title,
            message: self.message,
            confirm_text: self.confirm_text,
            cancel_text: self.cancel_text,
            danger_mode: self.danger_mode,
            is_open: self.is_open,
            on_confirm: self.on_confirm,
            on_cancel: Some(callback),
        }
    }

    /// Show the dialog and return the user's response
    pub fn show(self, ui: &mut Ui) -> InnerResponse<ConfirmationDialogResponse> {
        let mut is_open = self.is_open;
        let mut on_ok = self.on_confirm;
        let mut on_cancel = self.on_cancel;

        if !is_open {
            return InnerResponse::new(
                ConfirmationDialogResponse::Canceled,
                ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
            );
        }

        let mut final_response = ConfirmationDialogResponse::None;
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
                            final_response = ConfirmationDialogResponse::Confirmed;
                            if let Some(callback) = on_ok.take() {
                                callback();
                            }
                        }

                        ui.add_space(10.0);

                        // Cancel button
                        let cancel_button = egui::Button::new(&self.cancel_text)
                            .fill(Color32::from_rgb(108, 117, 125)); // Gray for secondary

                        if ui.add(cancel_button).clicked() {
                            final_response = ConfirmationDialogResponse::Canceled;
                            if let Some(callback) = on_cancel.take() {
                                callback();
                            }
                        }
                    });
                });

                ui.add_space(10.0);
            });

        // Handle window being closed via X button - treat as cancel
        if !is_open && matches!(final_response, ConfirmationDialogResponse::None) {
            final_response = ConfirmationDialogResponse::Canceled;
            if let Some(callback) = on_cancel.take() {
                callback();
            }
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

impl<F, G> Widget for ConfirmationDialog<F, G>
where
    F: FnOnce(),
    G: FnOnce(),
{
    fn ui(self, ui: &mut Ui) -> egui::Response {
        let inner_response = self.show(ui);
        inner_response.response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

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

    #[test]
    fn test_confirmation_dialog_with_callbacks() {
        let ok_called = Arc::new(Mutex::new(false));
        let ok_called_clone = ok_called.clone();

        let cancel_called = Arc::new(Mutex::new(false));
        let cancel_called_clone = cancel_called.clone();

        let _dialog = ConfirmationDialog::new("Test", "Test message")
            .on_confirm(move || {
                *ok_called_clone.lock().unwrap() = true;
            })
            .on_cancel(move || {
                *cancel_called_clone.lock().unwrap() = true;
            });

        // Test that the dialog can be created with callbacks
        // (We can't easily test the actual callback execution without a UI context)
        assert!(!*ok_called.lock().unwrap());
        assert!(!*cancel_called.lock().unwrap());
    }
}
