use std::sync::Arc;

use crate::ui::components::component_trait::{Component, ComponentResponse};
use crate::ui::theme::{ComponentStyles, DashColors, Shape};
use egui::{InnerResponse, Ui, WidgetText};

/// Result of a selection dialog interaction
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionStatus {
    /// User selected an option (0-indexed)
    Selected(usize),
    /// User cancelled the dialog
    Canceled,
}

pub const NOTHING: Option<&str> = None;

/// Response struct for the SelectionDialog component following the Component trait pattern
#[derive(Debug, Clone)]
pub struct SelectionDialogComponentResponse {
    pub response: egui::Response,
    pub changed: bool,
    pub error_message: Option<String>,
    pub dialog_response: Option<SelectionStatus>,
}

impl ComponentResponse for SelectionDialogComponentResponse {
    type DomainType = SelectionStatus;

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

/// A reusable selection dialog component that implements the Component trait.
///
/// This component provides a modal dialog with a dropdown (ComboBox) for selecting
/// from a list of options. It follows the same visual pattern as `ConfirmationDialog`
/// but adds a selection mechanism between the message and buttons.
///
/// Supports customizable title, message, option labels, button text (using WidgetText
/// for styling), and preselection. The dialog can be dismissed by pressing Escape
/// (treated as cancel) or clicking the X button. Enter confirms the current selection.
pub struct SelectionDialog {
    title: WidgetText,
    message: WidgetText,
    options: Vec<String>,
    selected_index: usize,
    confirm_text: Option<WidgetText>,
    cancel_text: Option<WidgetText>,
    status: Option<SelectionStatus>,
    is_open: bool,
}

impl Component for SelectionDialog {
    type DomainType = SelectionStatus;
    type Response = SelectionDialogComponentResponse;

    fn show(&mut self, ui: &mut Ui) -> InnerResponse<Self::Response> {
        let inner_response = self.show_dialog(ui);
        let changed = inner_response.inner.is_some();
        let response = inner_response.response;

        InnerResponse::new(
            SelectionDialogComponentResponse {
                response: response.clone(),
                changed,
                error_message: None,
                dialog_response: inner_response.inner,
            },
            response,
        )
    }

    fn current_value(&self) -> Option<Self::DomainType> {
        if self.is_open {
            None
        } else {
            self.status.clone()
        }
    }
}

impl SelectionDialog {
    /// Create a new selection dialog with the given title, message, and options
    pub fn new(
        title: impl Into<WidgetText>,
        message: impl Into<WidgetText>,
        options: Vec<String>,
    ) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            options,
            selected_index: 0,
            confirm_text: Some("Select".into()),
            cancel_text: Some("Cancel".into()),
            is_open: true,
            status: None,
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

    /// Preselect an option by index (0-based). Clamped to valid range.
    pub fn preselect(mut self, index: usize) -> Self {
        if !self.options.is_empty() {
            self.selected_index = index.min(self.options.len() - 1);
        }
        self
    }

    /// Set whether the dialog is open
    pub fn open(mut self, open: bool) -> Self {
        self.is_open = open;
        self
    }

    /// Render the dialog as a modal overlay using its own `egui::Area`.
    ///
    /// Returns `Some(SelectionStatus)` when the user confirms or cancels,
    /// `None` while the dialog is still open.
    pub fn show_modal(&mut self, ctx: &egui::Context) -> Option<SelectionStatus> {
        use crate::ui::components::component_trait::{Component, ComponentResponse};

        let mut selection_result: Option<SelectionStatus> = None;
        egui::Area::new(egui::Id::new("selection_dialog_modal").with(self.title.text()))
            .fixed_pos(egui::Pos2::ZERO)
            .order(egui::Order::Middle)
            .interactable(true)
            .show(ctx, |ui| {
                ui.set_min_size(ctx.content_rect().size());
                let response = self.show(ui);
                if let Some(status) = response.inner.changed_value() {
                    selection_result = Some(status.clone());
                }
            });
        selection_result
    }
}

impl SelectionDialog {
    /// Show the dialog and return the user's response
    fn show_dialog(&mut self, ui: &mut Ui) -> InnerResponse<Option<SelectionStatus>> {
        let mut is_open = self.is_open;

        if !is_open {
            return InnerResponse::new(
                None,
                ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover()),
            );
        }

        // Draw dark overlay behind the dialog
        let screen_rect = ui.ctx().content_rect();
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Middle,
            egui::Id::new("selection_dialog_overlay"),
        ));
        painter.rect_filled(screen_rect, 0.0, DashColors::modal_overlay());

        let mut final_response = None;
        let mut combo_popup_id: Option<egui::Id> = None;
        let window_response = egui::Window::new(self.title.clone())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .order(egui::Order::Foreground)
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
                ui.set_min_width(300.0);

                let dark_mode = ui.ctx().style().visuals.dark_mode;

                // Message
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(self.message.text())
                        .strong()
                        .color(DashColors::text_primary(dark_mode)),
                );
                ui.add_space(12.0);

                // ComboBox for option selection
                if !self.options.is_empty() {
                    let selected_text = self
                        .options
                        .get(self.selected_index)
                        .map(|s| s.as_str())
                        .unwrap_or_default();

                    let salt = ui.id().with("selection_dialog_combo");
                    combo_popup_id = Some(ui.make_persistent_id(salt).with("popup"));
                    egui::ComboBox::from_id_salt(salt)
                        .selected_text(selected_text)
                        .width(ui.available_width() - 8.0)
                        .show_ui(ui, |ui| {
                            for (i, option) in self.options.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_index, i, option);
                            }
                        });
                }

                ui.add_space(20.0);

                // Buttons (same layout as ConfirmationDialog)
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Confirm button
                        if let Some(confirm_text) = &self.confirm_text {
                            let fill_color = ComponentStyles::primary_button_fill();
                            let text_color = ComponentStyles::primary_button_text();

                            let confirm_label = if let WidgetText::RichText(rich_text) =
                                confirm_text
                            {
                                rich_text.clone()
                            } else {
                                Arc::new(egui::RichText::new(confirm_text.text()).color(text_color))
                            };

                            let confirm_button = egui::Button::new(confirm_label)
                                .fill(fill_color)
                                .stroke(ComponentStyles::primary_button_stroke())
                                .corner_radius(egui::CornerRadius::same(Shape::RADIUS_SM))
                                .min_size(egui::Vec2::new(80.0, 32.0));

                            if ui
                                .add_enabled(!self.options.is_empty(), confirm_button)
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                                && !self.options.is_empty()
                            {
                                final_response =
                                    Some(SelectionStatus::Selected(self.selected_index));
                            }
                        }

                        // Cancel button
                        if let Some(cancel_text) = &self.cancel_text {
                            let cancel_label = if let WidgetText::RichText(rich_text) = cancel_text
                            {
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
                                final_response = Some(SelectionStatus::Canceled);
                            }

                            ui.add_space(8.0);
                        }
                    });
                });
            });

        // Handle window closed via X button
        if !is_open && final_response.is_none() {
            final_response = Some(SelectionStatus::Canceled);
        }

        // Handle Escape key
        if final_response.is_none() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            final_response = Some(SelectionStatus::Canceled);
        }

        // Handle Enter key (skip if ComboBox dropdown is open)
        let combo_open = combo_popup_id.is_some_and(|id| egui::Popup::is_id_open(ui.ctx(), id));
        if final_response.is_none()
            && !self.options.is_empty()
            && !combo_open
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
        {
            final_response = Some(SelectionStatus::Selected(self.selected_index));
        }

        // Update state: record status before closing so current_value() sees it
        if final_response.is_some() {
            self.status = final_response.clone();
            self.is_open = false;
        } else {
            self.is_open = is_open;
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
    fn test_selection_dialog_creation() {
        let dialog = SelectionDialog::new(
            "Pick Wallet",
            "Choose the wallet to use",
            vec!["Wallet A".into(), "Wallet B".into(), "Wallet C".into()],
        )
        .confirm_text(Some("Use"))
        .cancel_text(Some("Back"))
        .preselect(1);

        assert_eq!(dialog.title.text(), "Pick Wallet");
        assert_eq!(dialog.message.text(), "Choose the wallet to use");
        assert_eq!(dialog.options.len(), 3);
        assert_eq!(dialog.selected_index, 1);
        assert!(dialog.confirm_text.is_some_and(|t| t.text() == "Use"));
        assert!(dialog.cancel_text.is_some_and(|t| t.text() == "Back"));
        assert!(dialog.is_open);
    }

    #[test]
    fn test_selection_dialog_default_buttons() {
        let dialog = SelectionDialog::new("Title", "Message", vec!["A".into(), "B".into()]);

        assert!(dialog.confirm_text.is_some_and(|t| t.text() == "Select"));
        assert!(dialog.cancel_text.is_some_and(|t| t.text() == "Cancel"));
        assert_eq!(dialog.selected_index, 0);
    }

    #[test]
    fn test_selection_dialog_preselect() {
        // Normal preselection
        let dialog =
            SelectionDialog::new("Title", "Message", vec!["A".into(), "B".into(), "C".into()])
                .preselect(2);
        assert_eq!(dialog.selected_index, 2);

        // Out-of-bounds clamped to last index
        let dialog =
            SelectionDialog::new("Title", "Message", vec!["A".into(), "B".into()]).preselect(99);
        assert_eq!(dialog.selected_index, 1);

        // Empty options: stays at 0
        let dialog = SelectionDialog::new("Title", "Message", vec![]).preselect(5);
        assert_eq!(dialog.selected_index, 0);
    }

    #[test]
    fn test_selection_dialog_no_buttons() {
        let dialog = SelectionDialog::new("Title", "Message", vec!["Only".into()])
            .confirm_text(NOTHING)
            .cancel_text(NOTHING);

        assert!(dialog.confirm_text.is_none());
        assert!(dialog.cancel_text.is_none());
    }
}
