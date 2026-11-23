use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::components::modal_chrome::{ModalChromeConfig, modal_chrome};
use crate::ui::components::styled::styled_text_edit_singleline;
use crate::ui::helpers::{ModalOpeningGuard, clicked_outside_window_after_open};
use crate::ui::theme::{ComponentStyles, DashColors, Spacing};
use egui::{InnerResponse, Ui, WidgetText};

pub use crate::ui::components::modal_chrome::NOTHING;

/// Response from showing a confirmation dialog
#[derive(Debug, Clone, PartialEq)]
pub enum ConfirmationStatus {
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
/// Label/value rows and separate notes for a structured confirmation.
#[derive(Debug, Clone, Default)]
pub struct ConfirmationSummary {
    /// Values remain selectable and never wrap inside a token.
    pub rows: Vec<(&'static str, String)>,
    /// Explanations shown below the summary, with normal wrapping.
    pub notes: Vec<String>,
}

impl std::fmt::Display for ConfirmationSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut lines: Vec<String> = self
            .rows
            .iter()
            .map(|(label, value)| format!("{label}: {value}"))
            .collect();
        lines.extend(self.notes.iter().cloned());
        f.write_str(&lines.join("\n"))
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
    summary: Option<ConfirmationSummary>,
    confirm_text: Option<WidgetText>,
    cancel_text: Option<WidgetText>,
    danger_mode: bool,
    required_confirmation_text: Option<String>,
    confirmation_prompt: Option<WidgetText>,
    confirmation_input: String,
    blocks_input: bool,
    is_open: bool,
    opening_guard: ModalOpeningGuard,
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
            summary: None,
            confirm_text: Some("Confirm".into()),
            cancel_text: Some("Cancel".into()),
            danger_mode: false,
            required_confirmation_text: None,
            confirmation_prompt: None,
            confirmation_input: String::new(),
            blocks_input: false,
            is_open: true,
            opening_guard: ModalOpeningGuard::armed(),
        }
    }

    /// Show aligned summary rows, keeping each value intact and notes separate.
    pub fn with_summary(mut self, summary: ConfirmationSummary) -> Self {
        self.summary = Some(summary);
        self
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

    /// Set whether the dialog blocks input to all controls behind it.
    pub fn blocks_input(mut self, enabled: bool) -> Self {
        self.blocks_input = enabled;
        self
    }

    /// Require the user to type an exact value before confirmation is enabled.
    pub fn require_confirmation_text(
        mut self,
        expected: impl Into<String>,
        prompt: impl Into<WidgetText>,
    ) -> Self {
        self.required_confirmation_text = Some(expected.into());
        self.confirmation_prompt = Some(prompt.into());
        self
    }

    /// Set whether the dialog is open
    pub fn open(mut self, open: bool) -> Self {
        self.is_open = open;
        self
    }

    /// The dialog's message as plain text. Test-only: lets a caller assert
    /// what a destructive prompt actually says without rendering a frame.
    #[cfg(test)]
    pub(crate) fn message_text(&self) -> &str {
        self.message.text()
    }

    /// Whether the dialog blocks input to the controls behind it. Test-only:
    /// lets a caller assert that a destructive prompt cannot be answered while
    /// the user has navigated elsewhere.
    #[cfg(test)]
    pub(crate) fn is_input_blocking(&self) -> bool {
        self.blocks_input
    }

    fn confirmation_text_matches(&self) -> bool {
        self.required_confirmation_text
            .as_ref()
            .is_none_or(|expected| self.confirmation_input == *expected)
    }
}

impl ConfirmationDialog {
    /// Show the dialog and return the user's response
    fn show_dialog(&mut self, ui: &mut Ui) -> InnerResponse<Option<ConfirmationStatus>> {
        if !self.is_open {
            return InnerResponse::new(
                None, // no change
                ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
            );
        }

        let mut final_response = None;
        let chrome = modal_chrome(
            ui.ctx(),
            ModalChromeConfig {
                title: self.title.clone(),
                overlay_id: egui::Id::unique("confirmation_dialog_overlay"),
                overlay_order: egui::Order::Background,
                window_order: if self.blocks_input {
                    egui::Order::Foreground
                } else {
                    egui::Order::Middle
                },
                resizable: false,
                show_close_button: true,
                blocks_input: self.blocks_input,
                inner_margin: 16,
            },
            |ui| {
                if self.summary.is_some() {
                    ui.set_width(
                        640.0_f32.min((ui.ctx().content_rect().width() - 64.0).max(240.0)),
                    );
                } else {
                    ui.set_min_width(300.0);
                }

                let dark_mode = ui.style().visuals.dark_mode;

                // Message content with bold text and proper color
                ui.add_space(10.0);
                if let Some(summary) = &self.summary {
                    egui::ScrollArea::horizontal()
                        .id_salt("confirmation_rows")
                        .show(ui, |ui| {
                            egui::Grid::new("confirmation_summary")
                                .spacing([Spacing::LG, Spacing::MD])
                                .show(ui, |ui| {
                                    for (label, value) in &summary.rows {
                                        ui.label(
                                            egui::RichText::new(*label)
                                                .color(DashColors::text_secondary(dark_mode)),
                                        );
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(value).monospace(),
                                            )
                                            .extend()
                                            .selectable(true),
                                        );
                                        ui.end_row();
                                    }
                                });
                        });
                    ui.add_space(Spacing::MD);
                    ui.separator();
                    for note in &summary.notes {
                        ui.add_space(Spacing::SM);
                        ui.label(note);
                    }
                } else {
                    ui.label(
                        egui::RichText::new(self.message.text())
                            .strong()
                            .color(DashColors::text_primary(dark_mode)),
                    );
                }
                ui.add_space(20.0);

                if let Some(prompt) = self.confirmation_prompt.clone() {
                    ui.label(
                        egui::RichText::new(prompt.text())
                            .color(DashColors::text_primary(dark_mode)),
                    );
                    ui.add_space(6.0);
                    ui.add(
                        styled_text_edit_singleline(&mut self.confirmation_input, dark_mode)
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(20.0);
                }

                let confirmation_text_matches = self.confirmation_text_matches();

                // Buttons
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Confirm button (only if text is provided)
                        if let Some(confirm_text) = &self.confirm_text {
                            let response = if self.danger_mode {
                                if confirmation_text_matches {
                                    ComponentStyles::add_danger_button(ui, confirm_text.clone())
                                } else {
                                    ui.add_enabled(
                                        false,
                                        ComponentStyles::danger_button(confirm_text.clone()),
                                    )
                                }
                            } else {
                                ComponentStyles::add_primary_button_enabled(
                                    ui,
                                    confirmation_text_matches,
                                    confirm_text.clone(),
                                )
                            };

                            if response.clicked() {
                                final_response = Some(ConfirmationStatus::Confirmed);
                            }
                        }

                        // Cancel button (only if text is provided)
                        if let Some(cancel_text) = &self.cancel_text {
                            if ComponentStyles::add_secondary_button(
                                ui,
                                cancel_text.clone(),
                                dark_mode,
                            )
                            .clicked()
                            {
                                final_response = Some(ConfirmationStatus::Canceled);
                            }

                            ui.add_space(8.0); // Add spacing between buttons
                        }
                    });
                });
            },
        );

        // Handle window being closed via X button - treat as cancel
        if chrome.closed_via_x && final_response.is_none() {
            final_response = Some(ConfirmationStatus::Canceled);
        }

        // Handle Escape key press - always treat as cancel
        if final_response.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            final_response = Some(ConfirmationStatus::Canceled);
        }

        // Handle Enter key press - confirm for non-danger dialogs,
        // but only when no other widget (e.g., text input) has focus
        if final_response.is_none()
            && !self.danger_mode
            && self.confirm_text.is_some()
            && self.confirmation_text_matches()
            && ui.ctx().memory(|m| m.focused().is_none())
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            final_response = Some(ConfirmationStatus::Confirmed);
        }

        // Handle click outside window - close for non-danger dialogs
        if let Some(ref wr) = chrome.window_response
            && final_response.is_none()
            && !self.danger_mode
            && clicked_outside_window_after_open(ui.ctx(), wr.rect, &mut self.opening_guard)
        {
            final_response = Some(ConfirmationStatus::Canceled);
        }

        // The dialog only closes itself via the X button; button clicks leave it to the caller.
        self.is_open = !chrome.closed_via_x;

        if let Some(window_response) = chrome.window_response {
            InnerResponse::new(final_response, window_response)
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

    #[test]
    fn required_confirmation_text_must_match_exactly() {
        let mut dialog = ConfirmationDialog::new("Wipe data?", "This cannot be undone.")
            .require_confirmation_text("WIPE", "Type WIPE to confirm this action.");

        assert!(!dialog.confirmation_text_matches());
        dialog.confirmation_input = "wipe".to_string();
        assert!(!dialog.confirmation_text_matches());
        dialog.confirmation_input = "WIPE".to_string();
        assert!(dialog.confirmation_text_matches());
    }

    #[test]
    fn enter_does_not_bypass_required_confirmation_text() {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut dialog = ConfirmationDialog::new("Confirm action?", "This action is sensitive.")
            .require_confirmation_text("CONFIRM", "Type CONFIRM to confirm this action.");
        let mut status = None;

        ctx.run_ui(raw, |ui| {
            status = dialog.show(ui).inner.dialog_response;
        })
        .drop_without_applying_deltas();

        assert_eq!(status, None);
    }

    #[test]
    fn opening_click_does_not_immediately_cancel_dialog() {
        let ctx = egui::Context::default();
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
        let mut dialog = ConfirmationDialog::new("Confirm", "Continue?");
        let mut status = None;

        ctx.run_ui(raw, |ui| {
            status = dialog.show(ui).inner.dialog_response;
        })
        .drop_without_applying_deltas();

        assert_eq!(
            status, None,
            "the click that opens a dialog must not also dismiss it",
        );
    }

    /// A private (Orchard) address is 83 characters on testnet. The whole
    /// recipient must be visible in the confirmation without scrolling.
    #[test]
    fn summary_shows_a_whole_private_address_without_scrolling() {
        use egui_kittest::kittest::Queryable;
        let address = format!("tdash1{}", "q".repeat(77));
        assert_eq!(address.len(), 83);
        let summary = ConfirmationSummary {
            rows: vec![("To", address.clone())],
            notes: vec!["Check the recipient. Payments can't be undone.".to_string()],
        };
        let mut dialog =
            ConfirmationDialog::new("Send 1 DASH?", summary.to_string()).with_summary(summary);
        let mut harness = crate::test_support::harness_builder_with_app_fonts()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_ui(move |ui| {
                let _ = dialog.show(ui);
            });
        harness.run();
        let window = harness.get_by_role(egui::accesskit::Role::Window).rect();
        let value = harness.get_by_label(&address).rect();
        assert!(
            value.right() <= window.right(),
            "the address runs past the dialog edge: value {value:?}, window {window:?}"
        );
    }
}
